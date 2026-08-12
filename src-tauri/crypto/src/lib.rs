//! 知言的端到端加密（设计文档 §8）。
//!
//! ```text
//! 用户密码
//!    │  Argon2id(salt=服务端随机盐, m=64MiB, t=3, p=4)
//!    ▼
//! MK  主密钥 (32B) ─────────────── 永不离开设备
//!    ├─ HKDF(MK,"auth") ─▶ AuthKey ─▶ 服务端（再 Argon2id 后落库）
//!    └─ HKDF(MK,"wrap") ─▶ KEK ──解开──▶ protected_dek
//!                                           ▼
//! DEK  数据密钥 (32B, 注册时随机)
//!    ├─ HKDF(DEK,"record")            ─▶ 加密每条片语
//!    ├─ HKDF(DEK,"blob", plain_hash)  ─▶ CEK，加密附件（收敛，§8.5）
//!    ├─ HKDF(DEK,"index")             ─▶ DBKey，SQLCipher 本地库密钥
//!    └─ HKDF(DEK,"inbox")             ─▶ 解开 X25519 私钥（微信入口）
//! ```
//!
//! **为什么要 DEK 这一层**：直接用 MK 加密数据的话，改密码就要全量重加密。
//! 有了 DEK，改密码只需重新包一次 32 字节，数据一个字节不动。
//!
//! **内存卫生（§8.2）**：密钥类型一律 `Zeroize + ZeroizeOnDrop`，且用**定长数组
//! 不用 `Vec<u8>`**——`Vec` 扩容会在堆上留下未清零的旧缓冲区副本。
//!
//! ## 这个 crate 为什么独立
//!
//! 它不依赖 Tauri，因此 `cargo test -p zhiyan-crypto` 是秒级的。加密的正确性
//! 全靠测试兜底，把它绑在 Tauri 的编译时间上会让人不愿意频繁跑测试。

#![forbid(unsafe_code)]

pub mod envelope;
pub mod keys;
pub mod recovery;

pub use envelope::{open, seal, seal_with_nonce, Envelope, Identity};
pub use keys::{
    derive_master_key, AuthKey, BackupKey, DataKey, DbKey, InboxKey, KdfParams, KeyEncryptionKey,
    MasterKey, RecordKey, KDF_SALT_LEN, KEY_LEN,
};
pub use recovery::{RecoveryCode, RECOVERY_GROUPS, RECOVERY_GROUP_LEN};

/// 加密层的错误。
///
/// **不携带任何明文片段或密钥字节**——错误也会进日志（§13）。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("密钥派生失败")]
    KeyDerivation,

    /// 解密失败。
    ///
    /// **刻意不区分**「tag 不匹配」与「AAD 不匹配」——分开会给攻击者一个预言机，
    /// 让他能试出某段密文本来属于哪条记录。
    #[error("解密失败：数据被篡改、AAD 不匹配或密钥错误")]
    Decrypt,

    #[error("信封格式错误：{0}")]
    MalformedEnvelope(&'static str),
    #[error("不支持的信封版本 {version}")]
    UnsupportedVersion { version: u8 },
    #[error("不支持的算法编号 {alg}")]
    UnsupportedAlgorithm { alg: u8 },

    #[error("恢复码格式错误：{0}")]
    MalformedRecoveryCode(&'static str),
    #[error("恢复码校验位不匹配（很可能是抄错了一位）")]
    RecoveryChecksum,
}

pub type Result<T> = core::result::Result<T, CryptoError>;
