//! 应用状态：库、附件目录、内存里的 DEK。
//!
//! ## 锁
//!
//! `rusqlite::Connection` 是 `Send` 但**不是** `Sync`，所以 `Store` 要 `Mutex`
//! 包一层才能进 Tauri 的托管状态。命令里持锁的时间必须短——Tauri 的命令跑在
//! 线程池上，一个长查询持着锁会把其余命令全堵住。
//!
//! 更要紧的是：**绝不能把 `MutexGuard` 跨 `.await` 持着**。文件对话框那类命令
//! 中间要 await，中途持锁的话整个应用会在用户盯着对话框发呆的十几秒里全卡住。
//! 下面的 `read` / `write` 只借出一个闭包的时间，从形状上堵死这种写法。
//!
//! ## 时区
//!
//! 状态里**不存时区**。日期分桶的口径必须与前端 `util.js` 的 `dayKey` 完全一致，
//! 而那份用的是 WebView 的本地时区。Rust 侧自己去问系统有两个问题：
//! `time` 的 `local-offset` 在多线程进程里读环境变量是不安全的（Unix 上直接返回
//! 错误），而且就算读到了，也未必与 WebView 认为的一致。
//!
//! 所以偏移量由前端随每次调用传进来（`tzOffset`，分钟）。
//!
//! ## 加锁顺序
//!
//! 需要同时拿密钥和库时，**一律先 `with_dek` 再 `read` / `write`**。
//! 附件那几条命令两把锁都要，顺序反过来的话迟早会撞上死锁——而死锁只在
//! 并发压上来时才出现，正是最难复现的那一类。目前所有调用点都是这个顺序。

use std::path::PathBuf;
use std::sync::Mutex;

use zhiyan_crypto::DataKey;
use zhiyan_db::{BlobStore, Store};

use crate::error::{AppError, AppResult};

/// 托管状态。
pub struct AppState {
    /// `None` 表示还没解锁（密钥取不到、或者用户主动锁定）。
    store: Mutex<Option<Store>>,
    blobs: BlobStore,
    /// 数据密钥。**只在内存里**，进程退出时 `ZeroizeOnDrop` 清掉（§8.2）。
    dek: Mutex<Option<DataKey>>,
    app_dir: PathBuf,
    encrypted: Mutex<bool>,
}

impl AppState {
    /// 建状态但**不开库**。
    ///
    /// 开库要密钥，密钥可能取不到（凭据管理器不可用、用户还没登录）。
    /// 那时应该起一个能显示引导界面的空壳，而不是让整个进程起不来。
    pub fn new(app_dir: PathBuf) -> Self {
        Self {
            store: Mutex::new(None),
            blobs: BlobStore::new(&app_dir),
            dek: Mutex::new(None),
            app_dir,
            encrypted: Mutex::new(false),
        }
    }

    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    /// 拿密钥、开库。启动时调一次。
    pub fn unlock(&self) -> AppResult<()> {
        std::fs::create_dir_all(&self.app_dir)
            .map_err(|e| AppError::Vault(format!("建应用目录：{e}")))?;

        let dek = crate::vault::load_or_create_dek(&self.app_dir)?;
        let store = Store::open(&self.app_dir.join("zhiyan.db"), &dek.db_key()?)?;

        *self.encrypted.lock().unwrap() = store.is_encrypted();
        *self.store.lock().unwrap() = Some(store);
        *self.dek.lock().unwrap() = Some(dek);
        Ok(())
    }

    /// 本地库是否真的加密了。
    ///
    /// 运行时探测出来的，不是编译期 feature 猜的——`PRAGMA key` 在普通 SQLite 上
    /// 会被静默忽略，库照开、数据照写，只是全是明文。显示一个假的「已加密」
    /// 比不显示更糟。
    pub fn is_encrypted(&self) -> bool {
        *self.encrypted.lock().unwrap()
    }

    pub fn is_unlocked(&self) -> bool {
        self.store.lock().unwrap().is_some()
    }

    /// 借库做一次只读操作。
    ///
    /// 闭包形式而不是返回 guard：guard 一旦能被调用方拿走，就一定会有人把它
    /// 跨 `.await` 持着。
    pub fn read<T>(&self, f: impl FnOnce(&Store) -> AppResult<T>) -> AppResult<T> {
        let guard = self.store.lock().unwrap();
        f(guard.as_ref().ok_or(AppError::Locked)?)
    }

    /// 借库做一次写操作。
    pub fn write<T>(&self, f: impl FnOnce(&mut Store) -> AppResult<T>) -> AppResult<T> {
        let mut guard = self.store.lock().unwrap();
        f(guard.as_mut().ok_or(AppError::Locked)?)
    }

    /// 借 DEK。附件加解密用。
    pub fn with_dek<T>(&self, f: impl FnOnce(&DataKey) -> AppResult<T>) -> AppResult<T> {
        let guard = self.dek.lock().unwrap();
        f(guard.as_ref().ok_or(AppError::Locked)?)
    }

    /// 锁定：清掉内存里的密钥并关掉库（§9.4 的「空闲 N 分钟后锁定」）。
    ///
    /// `DataKey` 是 `ZeroizeOnDrop` 的，`take()` 之后随即析构清零。
    pub fn lock_now(&self) {
        *self.store.lock().unwrap() = None;
        *self.dek.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 未解锁时读写都报_locked_而不是_panic() {
        let s = AppState::new(PathBuf::from("/nonexistent/zhiyan"));
        assert!(!s.is_unlocked());
        assert!(matches!(s.read(|_| Ok(())), Err(AppError::Locked)));
        assert!(matches!(s.write(|_| Ok(())), Err(AppError::Locked)));
        assert!(matches!(s.with_dek(|_| Ok(())), Err(AppError::Locked)));
    }

    #[test]
    fn 解锁后能读写再锁上就读不了了() {
        let dir = tempfile::tempdir().unwrap();
        let s = AppState::new(dir.path().to_path_buf());
        s.unlock().unwrap();
        assert!(s.is_unlocked());
        s.read(|st| Ok(st.get_kv("x")?)).unwrap();

        s.lock_now();
        assert!(!s.is_unlocked());
        assert!(matches!(s.read(|_| Ok(())), Err(AppError::Locked)));
    }

    #[test]
    fn 重开同一目录能读到上次写的东西() {
        // 密钥必须是取回来的同一把，否则每次启动都是一个空库
        let dir = tempfile::tempdir().unwrap();
        {
            let s = AppState::new(dir.path().to_path_buf());
            s.unlock().unwrap();
            s.read(|st| Ok(st.set_kv("theme", "dark")?)).unwrap();
        }
        let s = AppState::new(dir.path().to_path_buf());
        s.unlock().unwrap();
        let v = s.read(|st| Ok(st.get_kv("theme")?)).unwrap();
        assert_eq!(v.as_deref(), Some("dark"));
    }
}
