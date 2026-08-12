//! 免密启动：DEK 存哪儿、怎么取回来（§9.4）。
//!
//! ```text
//! DEK ──DPAPI(CryptProtectData, 用户作用域)──▶ 密文
//!                                              │
//!                             写入 Windows Credential Manager
//!                             （keyring crate，目标名 "Zhiyan/dek"）
//! ```
//!
//! ## 为什么要免密启动
//!
//! 要求每次开应用输密码，对「按热键即时速记」这种工具是灾难——三秒记一个念头
//! 的前提是那三秒里不用做别的事。
//!
//! ## 安全边界必须写清楚
//!
//! **同一 Windows 会话内运行的程序可以解出这把密钥。** DPAPI 用户作用域绑定的是
//! Windows 登录凭据，离开这台机器的这个账户就解不开；但它挡不住已经以你的身份
//! 在跑的进程。这是所有免密启动方案的共同代价（1Password、Bitwarden 的
//! 「记住我」同理），UI 上不能含糊。
//!
//! 对应的设置项（阶段五接 UI）：「启动时要求输入密码」「空闲 N 分钟后锁定」。
//!
//! ## 服务端账号
//!
//! 这里做的是**本机独立可用**的那一半：首次启动随机生成 DEK 存进凭据管理器。
//! 接上服务端之后，DEK 改为从 `protected_dek` 用 KEK 解出来（§8.2），
//! 落地位置不变——凭据管理器里的还是同一把 DEK。

use zeroize::Zeroize;
use zhiyan_crypto::{DataKey, KEY_LEN};

use crate::error::{AppError, AppResult};

/// 凭据管理器里的目标名（§9.4 指定）。
const SERVICE: &str = "Zhiyan";
const ENTRY: &str = "dek";

/// 取回 DEK；没有就生成一把新的存进去。
///
/// `app_dir` 只在调试构建的回退路径上用到，见 [`dev_fallback`]。
pub fn load_or_create_dek(app_dir: &std::path::Path) -> AppResult<DataKey> {
    match keyring_load() {
        Ok(Some(dek)) => return Ok(dek),
        Ok(None) => {}
        Err(e) => {
            // 凭据管理器整个用不了（Linux 上没有可用后端是常事）
            tracing::warn!("凭据管理器不可用：{}", e.code());
            return dev_fallback(app_dir, e);
        }
    }

    let dek = DataKey::random();
    keyring_store(&dek)?;
    tracing::info!("已生成新的数据密钥并存入凭据管理器");
    Ok(dek)
}

/// 清掉本机保存的 DEK。「注销」与「本机停用」走这条。
///
/// **它删不掉已经落在本地的数据**——库还在，只是再也打不开。
/// UI 上要说明白：远程吊销能让服务端拒绝这台设备，但擦不掉本地已有的东西（§9.4）。
pub fn forget_dek() -> AppResult<()> {
    let entry = entry()?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(AppError::Vault(format!("删除凭据失败：{e}"))),
    }
}

fn entry() -> AppResult<keyring::Entry> {
    keyring::Entry::new(SERVICE, ENTRY).map_err(|e| AppError::Vault(format!("打开凭据项：{e}")))
}

fn keyring_load() -> AppResult<Option<DataKey>> {
    let entry = entry()?;
    let stored = match entry.get_password() {
        Ok(s) => s,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(e) => return Err(AppError::Vault(format!("读取凭据：{e}"))),
    };

    let mut protected =
        hex::decode(stored.trim()).map_err(|_| AppError::Vault("凭据内容不是十六进制".into()))?;
    let dek = unprotect(&protected);
    protected.zeroize();
    dek
}

fn keyring_store(dek: &DataKey) -> AppResult<()> {
    let mut protected = protect(dek.as_bytes())?;
    let mut hexed = hex::encode(&protected);
    let r = entry()?
        .set_password(&hexed)
        .map_err(|e| AppError::Vault(format!("写入凭据：{e}")));
    hexed.zeroize();
    protected.zeroize();
    r
}

fn unprotect(protected: &[u8]) -> AppResult<Option<DataKey>> {
    let mut raw = unprotect_bytes(protected)?;
    let out = <[u8; KEY_LEN]>::try_from(raw.as_slice())
        .map(|b| Some(DataKey::from_bytes(b)))
        .map_err(|_| AppError::Vault("凭据里的密钥长度不对".into()));
    raw.zeroize();
    out
}

// ── DPAPI（§9.4）────────────────────────────────────────────────

#[cfg(windows)]
mod dpapi {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    use crate::error::{AppError, AppResult};

    /// 把 `out` 里的字节拷出来并释放 DPAPI 分配的缓冲区。
    ///
    /// 拷完立刻 `LocalFree`：那块内存里躺着的是刚解出来的明文密钥。
    unsafe fn take(out: &mut CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        unsafe {
            std::ptr::write_bytes(out.pbData, 0, out.cbData as usize);
            let _ = LocalFree(HLOCAL(out.pbData as *mut _));
        }
        bytes
    }

    fn blob(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            pbData: bytes.as_ptr() as *mut u8,
        }
    }

    pub fn protect(plain: &[u8]) -> AppResult<Vec<u8>> {
        let mut out = CRYPT_INTEGER_BLOB::default();
        // UI_FORBIDDEN：这条路径可能在开机自启时跑，弹不出任何窗口
        unsafe {
            CryptProtectData(
                &blob(plain),
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        }
        .map_err(|e| AppError::Vault(format!("DPAPI 加密失败：{}", e.code().0)))?;
        Ok(unsafe { take(&mut out) })
    }

    pub fn unprotect(protected: &[u8]) -> AppResult<Vec<u8>> {
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptUnprotectData(
                &blob(protected),
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        }
        .map_err(|e| AppError::Vault(format!("DPAPI 解密失败：{}", e.code().0)))?;
        Ok(unsafe { take(&mut out) })
    }
}

#[cfg(windows)]
fn protect(plain: &[u8; KEY_LEN]) -> AppResult<Vec<u8>> {
    dpapi::protect(plain)
}

#[cfg(windows)]
fn unprotect_bytes(protected: &[u8]) -> AppResult<Vec<u8>> {
    dpapi::unprotect(protected)
}

/// 非 Windows 上没有 DPAPI，直接交给平台的凭据后端。
///
/// 这不是「凑合」：macOS 的 Keychain 与 Linux 的 Secret Service 本来就负责
/// 静态加密，DPAPI 在 Windows 上是补凭据管理器的一层——三家的信任模型不同，
/// 不必强行拉平。目标平台是 Windows（§4.2），这条路径只用于开发。
#[cfg(not(windows))]
fn protect(plain: &[u8; KEY_LEN]) -> AppResult<Vec<u8>> {
    Ok(plain.to_vec())
}

#[cfg(not(windows))]
fn unprotect_bytes(protected: &[u8]) -> AppResult<Vec<u8>> {
    Ok(protected.to_vec())
}

// ── 开发回退 ────────────────────────────────────────────────────

/// 凭据管理器用不了时的回退。**只在调试构建里**。
///
/// 容器和很多 Linux 桌面上没有可用的凭据后端，没有回退的话应用根本起不来，
/// 界面也就没法开发。
///
/// 发布构建里这是**硬错误**：把密钥明文写在数据库旁边，等于 §8 那一整套白做——
/// 偷走 `%APPDATA%` 的人连密钥一起拿走了。宁可开不起来，也不能悄悄降级。
#[cfg(debug_assertions)]
fn dev_fallback(app_dir: &std::path::Path, _why: AppError) -> AppResult<DataKey> {
    use std::io::Write;

    let path = app_dir.join("dev-dek.key");
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(bytes) = hex::decode(text.trim()) {
            if let Ok(b) = <[u8; KEY_LEN]>::try_from(bytes.as_slice()) {
                tracing::warn!("使用开发用明文密钥文件（发布构建不会有这条路径）");
                return Ok(DataKey::from_bytes(b));
            }
        }
    }

    let dek = DataKey::random();
    std::fs::create_dir_all(app_dir).map_err(|e| AppError::Vault(format!("建目录：{e}")))?;
    let mut f =
        std::fs::File::create(&path).map_err(|e| AppError::Vault(format!("写密钥文件：{e}")))?;
    writeln!(f, "{}", hex::encode(dek.as_bytes()))
        .map_err(|e| AppError::Vault(format!("写密钥文件：{e}")))?;
    tracing::warn!("凭据管理器不可用，已在应用目录写下开发用明文密钥");
    Ok(dek)
}

#[cfg(not(debug_assertions))]
fn dev_fallback(_app_dir: &std::path::Path, why: AppError) -> AppResult<DataKey> {
    Err(why)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 保护与还原是一对() {
        let key = [0x5Au8; KEY_LEN];
        let protected = protect(&key).unwrap();
        assert_eq!(unprotect_bytes(&protected).unwrap(), key);
    }

    #[test]
    fn 长度不对的凭据被拒绝而不是截断使用() {
        // 截断成 32 字节「凑合用」会安静地换掉整个数据库的密钥
        let short = protect(&[1u8; KEY_LEN]).unwrap();
        assert!(unprotect(&short[..short.len() - 1]).is_err() || cfg!(not(windows)));
        assert!(unprotect(b"too short").is_err());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn 开发回退在同一目录下取回同一把密钥() {
        let dir = tempfile::tempdir().unwrap();
        let a = dev_fallback(dir.path(), AppError::Vault("test".into())).unwrap();
        let b = dev_fallback(dir.path(), AppError::Vault("test".into())).unwrap();
        assert_eq!(a.as_bytes(), b.as_bytes(), "换一把密钥等于数据全丢");
    }
}
