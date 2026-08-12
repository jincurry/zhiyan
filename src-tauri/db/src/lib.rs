//! 知言的本地存储（设计文档 §9）。
//!
//! ```text
//! %APPDATA%\Zhiyan\
//! ├─ zhiyan.db                    SQLCipher，整库加密（§9.2）
//! ├─ blobs\a3\a3f9c2e1…           附件密文，按 plain_hash 前两位分桶
//! ├─ cache\thumbs\a3\a3f9c2e1…    缩略图密文
//! └─ state.json                   明文，只放不敏感内容
//! ```
//!
//! ## 为什么整库加密而不是字段加密
//!
//! 不加密的话，`memo` 表和 `memo_fts` 索引里躺着完整明文——笔记本被偷、磁盘被
//! 恢复、或者备份软件把 `%APPDATA%` 同步到别处，§8 那一整套就全白做了。
//!
//! SQLCipher 做的是**页级**透明加密：SQLite 引擎看到的是解密后的页，所以 FTS5、
//! 索引、事务、WAL 全部照常工作，SQL 一行不用改。换成应用层字段加密的话，
//! 全文搜索和 `ORDER BY created_at` 会直接失效。
//!
//! ## 这个 crate 为什么独立
//!
//! 与 `zhiyan-crypto` 同理：`cargo test -p zhiyan-db` 不必先编一遍 Tauri。
//! 存储层的回归几乎全靠测试发现，跑一次要几分钟的话就没人跑了。

#![forbid(unsafe_code)]

mod backup;
mod blobstore;
mod memo;
mod model;
mod parse;
mod schema;
mod stats;

pub use backup::Snapshot;
pub use blobstore::BlobStore;
pub use model::{
    BlobMeta, Clock, Counts, Filter, HeatCell, Memo, MemoInput, Op, Sort, Source, Stats, View,
};
pub use parse::{count_chars, parse_refs, parse_tags};
pub use schema::SCHEMA_VERSION;

use rusqlite::Connection;
use std::path::Path;
use zhiyan_crypto::DbKey;

/// 存储层的错误。
///
/// **不携带正文片段**（§14 ①）：错误会穿过 IPC 边界，再经 `console.error`
/// 落进 WebView 的日志。`Sqlite` 变体带的是 SQLite 自己的消息——它讲的是
/// 约束名与表名，不含被绑定的参数值。
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("SQL 错误：{0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("找不到记录")]
    NotFound,

    #[error("加密层错误：{0}")]
    Crypto(#[from] zhiyan_crypto::CryptoError),

    #[error("附件读写失败：{0}")]
    Io(String),

    /// 密文与它的哈希对不上。
    #[error("附件内容与哈希不符")]
    BlobCorrupted,
}

pub type Result<T> = core::result::Result<T, DbError>;

impl DbError {
    /// 稳定的错误码，供前端分支。
    pub fn code(&self) -> &'static str {
        match self {
            DbError::Sqlite(_) => "sql",
            DbError::NotFound => "not_found",
            DbError::Crypto(_) => "crypto",
            DbError::Io(_) => "io",
            DbError::BlobCorrupted => "blob_corrupted",
        }
    }
}

/// 本地库。
///
/// 内含的 `Connection` 是 `Send` 但**不是** `Sync`，所以进 Tauri 托管状态要
/// `Mutex` 包一层。命令里持锁的时间必须短：Tauri 的命令跑在线程池上，
/// 一个长查询持着锁会把其余命令全堵住。
pub struct Store {
    conn: Connection,
    encrypted: bool,
}

impl core::fmt::Debug for Store {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Store")
            .field("encrypted", &self.encrypted)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// 打开（或新建）本地库。
    ///
    /// # 裸密钥模式（§9.3）
    ///
    /// `PRAGMA key = x'…'` 直接给 32 字节裸密钥，**跳过 SQLCipher 自带的
    /// 25.6 万次 PBKDF2**。用 `PRAGMA key = 'passphrase'` 的话每开一次连接都要
    /// 跑一遍那 25.6 万次，连接池场景下启动时间会劣化到秒级。
    ///
    /// `DbKey` 本身已是 Argon2id 派生的强密钥（`HKDF(DEK,"index")`），无需再拉伸。
    pub fn open(path: &Path, key: &DbKey) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn, key)
    }

    /// 内存库。测试与「未登录时的空壳」用。
    pub fn open_in_memory(key: &DbKey) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn, key)
    }

    fn init(conn: Connection, key: &DbKey) -> Result<Self> {
        // 必须是第一条语句：SQLCipher 要在任何读写之前拿到密钥
        conn.pragma_update(None, "key", format!("x'{}'", hex::encode(key.as_bytes())))?;

        let encrypted = detect_sqlcipher(&conn);
        if encrypted {
            conn.pragma_update(None, "cipher_page_size", 4096)?;
        }

        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // 触发器里要更新 FTS 的影子表，递归深度默认够用，但显式写上免得被外部 PRAGMA 改掉
        conn.pragma_update(None, "recursive_triggers", "ON")?;

        schema::migrate(&conn)?;

        if !encrypted {
            // 不 return Err：开发构建（`--no-default-features` 之外的默认 feature）就是明文跑的。
            // 但这件事必须留下痕迹，否则「发布版忘了加 --features sqlcipher」会悄无声息地过去
            tracing::warn!(
                "本地库未加密：当前二进制没有编译 SQLCipher。\
                 发布构建必须开 --features sqlcipher（§9.2）"
            );
        }

        Ok(Self { conn, encrypted })
    }

    /// 本地库是否真的加密了。
    ///
    /// **不能靠「编译时开了 feature」来推断**：`PRAGMA key` 在普通 SQLite 上是
    /// 未知 pragma，会被**静默忽略**——库照开、数据照写，只是全是明文。
    /// 这是这套方案里最容易无声出错的一处，所以做成运行时探测并上报给界面。
    pub fn is_encrypted(&self) -> bool {
        self.encrypted
    }

    /// 借出连接。事务、备份一类需要它。
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// 借出可变连接（`transaction()` 需要）。
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    // ── kv（§9.6）────────────────────────────────────────────────

    /// 读一个 kv。
    ///
    /// **不放敏感内容**：它对应 §9.1 里那份明文 `state.json` 的定位——
    /// 窗口位置、同步游标、UI 偏好可以；最近搜索词、标签列表不行。
    /// （库本身是加密的，但这些键会被导出、被同步、被日志顺手打出来。）
    pub fn get_kv(&self, k: &str) -> Result<Option<String>> {
        let mut st = self.conn.prepare_cached("SELECT v FROM kv WHERE k=?1")?;
        let v = st
            .query_row([k], |r| r.get::<_, String>(0))
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        Ok(v)
    }

    pub fn set_kv(&self, k: &str, v: &str) -> Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO kv(k,v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=excluded.v",
            )?
            .execute(rusqlite::params![k, v])?;
        Ok(())
    }

    pub fn del_kv(&self, k: &str) -> Result<()> {
        self.conn
            .prepare_cached("DELETE FROM kv WHERE k=?1")?
            .execute([k])?;
        Ok(())
    }
}

/// 探测当前连接是不是真的 SQLCipher。
///
/// `PRAGMA cipher_version` 只有 SQLCipher 认识；普通 SQLite 返回空结果集。
fn detect_sqlcipher(conn: &Connection) -> bool {
    conn.query_row("PRAGMA cipher_version", [], |r| r.get::<_, String>(0))
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zhiyan_crypto::KEY_LEN;

    pub(crate) fn store() -> Store {
        Store::open_in_memory(&DbKey::from_bytes([7u8; KEY_LEN])).unwrap()
    }

    #[test]
    fn 开库并建表() {
        let s = store();
        let n: i64 = s
            .conn()
            .query_row("SELECT count(*) FROM memo", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn kv_往返() {
        let s = store();
        assert_eq!(s.get_kv("sync_cursor").unwrap(), None);
        s.set_kv("sync_cursor", "42").unwrap();
        assert_eq!(s.get_kv("sync_cursor").unwrap().as_deref(), Some("42"));
        s.set_kv("sync_cursor", "43").unwrap();
        assert_eq!(s.get_kv("sync_cursor").unwrap().as_deref(), Some("43"));
        s.del_kv("sync_cursor").unwrap();
        assert_eq!(s.get_kv("sync_cursor").unwrap(), None);
    }

    #[test]
    fn 加密状态是运行时探测出来的() {
        // 默认 feature 下是普通 SQLite，这里必须如实报 false 而不是靠 cfg 猜。
        // 反过来开了 sqlcipher feature 就必须是 true——两个方向都不能撒谎
        let s = store();
        assert_eq!(s.is_encrypted(), cfg!(feature = "sqlcipher"));
    }

    #[test]
    fn store_的_debug_不泄漏路径与密钥() {
        let s = store();
        let d = format!("{s:?}");
        assert!(!d.contains("07070707"), "{d}");
    }
}
