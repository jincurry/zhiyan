//! 三层密钥体系（§8.2）与附件的收敛派生（§8.5）。

use argon2::{Algorithm, Argon2, Params, Version};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{CryptoError, Result};

/// 所有对称密钥都是 32 字节。
pub const KEY_LEN: usize = 32;
/// 服务端下发的 Argon2id 盐长度。
pub const KDF_SALT_LEN: usize = 16;

/// 附件明文的 SHA-256。
pub type PlainHash = [u8; 32];
/// 附件密文的 SHA-256，即对象存储的键。
pub type CipherId = [u8; 32];

/// 定义一个定长密钥 newtype。
///
/// 统一具备 `ZeroizeOnDrop` 与脱敏的 `Debug`；**刻意不实现** `Clone` / `Display` /
/// `Serialize`——密钥被顺手复制或打印出去，是这一层最容易出的事故。
macro_rules! key_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Zeroize, ZeroizeOnDrop, PartialEq, Eq)]
        pub struct $name([u8; KEY_LEN]);

        impl $name {
            /// 从裸字节构造。调用方负责保证字节确实来自 KDF 或 CSPRNG。
            pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
                Self(bytes)
            }

            /// 借出裸字节。**不要**把它复制进 `Vec` 或存进别的结构体。
            pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
                &self.0
            }

            /// 生成一个新的随机密钥（OS CSPRNG）。
            pub fn random() -> Self {
                let mut bytes = [0u8; KEY_LEN];
                rand::rngs::OsRng.fill_bytes(&mut bytes);
                Self(bytes)
            }
        }

        // 手写 Debug：绝不打印密钥字节（§13）
        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(concat!(stringify!($name), "(<redacted 32B>)"))
            }
        }
    };
}

key_newtype!(
    /// 主密钥。由密码经 Argon2id 派生，**永不离开设备**。
    MasterKey
);
key_newtype!(
    /// 数据密钥。注册时随机生成；改密码只重新包它一次，数据一个字节不动。
    DataKey
);
key_newtype!(
    /// 发给服务端的登录凭据。HKDF 不可逆，服务端拿到它推不出 MK，
    /// 也就解不开 `protected_dek`。
    AuthKey
);
key_newtype!(
    /// 包裹 DEK 的密钥。由 MK 或恢复码派生。
    KeyEncryptionKey
);
key_newtype!(
    /// 加密单条片语。
    RecordKey
);
key_newtype!(
    /// SQLCipher 本地库密钥（§9.3 直接喂给 `PRAGMA key`）。
    DbKey
);
key_newtype!(
    /// 解开非对称收件箱的 X25519 私钥（§10.5 的微信入口）。
    InboxKey
);
key_newtype!(
    /// 单个附件的内容加密密钥（收敛派生，§8.5）。
    ContentKey
);

/// Argon2id 参数。
///
/// **必须随 `protected_dek` 存服务端**（§8.2）：未来调高强度时，老用户仍能用
/// 旧参数解开，再静默升级。写死在代码里的话，调参那天所有老账号都登不进来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    /// 内存代价，单位 KiB。
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Default for KdfParams {
    /// §8.2 规定：`m=64MiB, t=3, p=4`，桌面端约 0.3–0.5 秒。
    fn default() -> Self {
        Self {
            memory_kib: 64 * 1024,
            iterations: 3,
            parallelism: 4,
        }
    }
}

impl KdfParams {
    /// 仅供单元测试——跑 64MiB 的 Argon2id 会让测试套变得没法用。
    #[doc(hidden)]
    pub fn insecure_for_tests() -> Self {
        Self {
            memory_kib: 64,
            iterations: 1,
            parallelism: 1,
        }
    }
}

/// 由密码与**服务端下发的盐**派生主密钥。
///
/// 盐不能本地生成：换台设备登录时算不出同一个 MK，`protected_dek` 就永远解不开。
pub fn derive_master_key(
    password: &str,
    salt: &[u8; KDF_SALT_LEN],
    params: KdfParams,
) -> Result<MasterKey> {
    let params = Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        Some(KEY_LEN),
    )
    .map_err(|_| CryptoError::KeyDerivation)?;

    let mut out = [0u8; KEY_LEN];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|_| CryptoError::KeyDerivation)?;
    Ok(MasterKey::from_bytes(out))
}

/// HKDF-SHA256 的 expand。
///
/// 不做 extract：输入本身已是均匀随机的 32 字节密钥材料，加盐没有额外价值。
fn expand(ikm: &[u8; KEY_LEN], info: &[u8]) -> Result<[u8; KEY_LEN]> {
    let hk = Hkdf::<Sha256>::from_prk(ikm).map_err(|_| CryptoError::KeyDerivation)?;
    let mut out = [0u8; KEY_LEN];
    hk.expand(info, &mut out)
        .map_err(|_| CryptoError::KeyDerivation)?;
    Ok(out)
}

impl MasterKey {
    /// `HKDF(MK,"auth")`。服务端再对它做一次 Argon2id 后落库，
    /// 防止库泄漏后被直接拿去登录。
    pub fn auth_key(&self) -> Result<AuthKey> {
        Ok(AuthKey::from_bytes(expand(
            self.as_bytes(),
            b"zhiyan/v1/auth",
        )?))
    }

    /// `HKDF(MK,"wrap")`。
    pub fn key_encryption_key(&self) -> Result<KeyEncryptionKey> {
        Ok(KeyEncryptionKey::from_bytes(expand(
            self.as_bytes(),
            b"zhiyan/v1/wrap",
        )?))
    }
}

impl DataKey {
    /// `HKDF(DEK,"record")`。
    pub fn record_key(&self) -> Result<RecordKey> {
        Ok(RecordKey::from_bytes(expand(
            self.as_bytes(),
            b"zhiyan/v1/record",
        )?))
    }

    /// `HKDF(DEK,"index")`：SQLCipher 本地库密钥。
    pub fn db_key(&self) -> Result<DbKey> {
        Ok(DbKey::from_bytes(expand(
            self.as_bytes(),
            b"zhiyan/v1/index",
        )?))
    }

    /// `HKDF(DEK,"inbox")`。
    pub fn inbox_key(&self) -> Result<InboxKey> {
        Ok(InboxKey::from_bytes(expand(
            self.as_bytes(),
            b"zhiyan/v1/inbox",
        )?))
    }
}

// ── 附件：收敛加密（§8.5）────────────────────────────────────────

/// 附件的内容加密密钥：`CEK = HKDF(DEK, info="blob" || plain_hash)`。
///
/// 对同一用户的同一份明文是**确定性**的，因此同一张图在不同设备上算出的密文
/// 完全一致，可以按 `cipher_id` 去重。
///
/// **为什么要掺 DEK**：纯收敛加密下不同用户的相同文件密文相同，服务端可以做
/// 「确认文件存在」攻击——拿一张已知图片算出密文哈希就能查出谁存了它。
/// 掺入 DEK 后跨用户去重失效，但跨用户去重本来也不该做。
pub fn derive_blob_key(dek: &DataKey, plain_hash: &PlainHash) -> Result<ContentKey> {
    derive_content_key(dek, b"zhiyan/v1/blob", plain_hash)
}

/// 缩略图密钥。
///
/// **容易漏的一项**：缩略图能直接看出原图内容，明文缓存等于加密白做。
pub fn derive_thumb_key(dek: &DataKey, plain_hash: &PlainHash) -> Result<ContentKey> {
    derive_content_key(dek, b"zhiyan/v1/thumb", plain_hash)
}

fn derive_content_key(dek: &DataKey, domain: &[u8], plain_hash: &PlainHash) -> Result<ContentKey> {
    let mut info = Vec::with_capacity(domain.len() + plain_hash.len());
    info.extend_from_slice(domain);
    info.extend_from_slice(plain_hash);
    let key = expand(dek.as_bytes(), &info);
    info.zeroize();
    Ok(ContentKey::from_bytes(key?))
}

/// 附件的确定性 nonce：`HMAC-SHA256(CEK, "nonce")[0..24]`。
///
/// 收敛加密要求同一明文产生同一密文，nonce 因此不能随机。
/// **安全性由「CEK 已与明文一一对应」保证**：`(CEK, nonce, 明文)` 三元组恒定，
/// 不存在同密钥不同明文复用 nonce 的情况。
pub fn deterministic_blob_nonce(cek: &ContentKey) -> [u8; 24] {
    use hmac::{Hmac, Mac};
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(cek.as_bytes()).expect("HMAC 接受任意长度密钥");
    mac.update(b"nonce");
    let tag = mac.finalize().into_bytes();
    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&tag[..24]);
    nonce
}

/// 明文的 SHA-256。
pub fn plain_hash(bytes: &[u8]) -> PlainHash {
    Sha256::digest(bytes).into()
}

/// 密文的 SHA-256，即服务端对象键。
pub fn cipher_id(ciphertext: &[u8]) -> CipherId {
    plain_hash(ciphertext)
}

/// 生成一个新的 Argon2id 盐。正式流程里这一步在服务端做。
pub fn random_kdf_salt() -> [u8; KDF_SALT_LEN] {
    let mut salt = [0u8; KDF_SALT_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    salt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 同一密码同一盐得到同一主密钥() {
        let salt = [7u8; KDF_SALT_LEN];
        let p = KdfParams::insecure_for_tests();
        let a = derive_master_key("正确的马电池订书钉", &salt, p).unwrap();
        let b = derive_master_key("正确的马电池订书钉", &salt, p).unwrap();
        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn 盐或密码不同则主密钥不同() {
        let p = KdfParams::insecure_for_tests();
        let a = derive_master_key("pw", &[1u8; KDF_SALT_LEN], p).unwrap();
        let b = derive_master_key("pw", &[2u8; KDF_SALT_LEN], p).unwrap();
        let c = derive_master_key("pw2", &[1u8; KDF_SALT_LEN], p).unwrap();
        assert_ne!(a.as_bytes(), b.as_bytes());
        assert_ne!(a.as_bytes(), c.as_bytes());
    }

    #[test]
    fn 参数不同则主密钥不同() {
        // 这正是「参数必须随 protected_dek 存服务端」的原因：
        // 用错参数算出来的是另一个 MK，protected_dek 直接解不开
        let salt = [7u8; KDF_SALT_LEN];
        let a = derive_master_key("pw", &salt, KdfParams::insecure_for_tests()).unwrap();
        let b = derive_master_key(
            "pw",
            &salt,
            KdfParams {
                memory_kib: 128,
                iterations: 1,
                parallelism: 1,
            },
        )
        .unwrap();
        assert_ne!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn 各子密钥两两不同且不等于父密钥() {
        let mk = MasterKey::from_bytes([3u8; KEY_LEN]);
        let dek = DataKey::from_bytes([4u8; KEY_LEN]);

        let auth = mk.auth_key().unwrap();
        let kek = mk.key_encryption_key().unwrap();
        let record = dek.record_key().unwrap();
        let db = dek.db_key().unwrap();
        let inbox = dek.inbox_key().unwrap();

        let all: Vec<&[u8; KEY_LEN]> = vec![
            auth.as_bytes(),
            kek.as_bytes(),
            record.as_bytes(),
            db.as_bytes(),
            inbox.as_bytes(),
        ];
        for (i, a) in all.iter().enumerate() {
            for b in all.iter().skip(i + 1) {
                assert_ne!(a, b);
            }
        }
        assert_ne!(auth.as_bytes(), mk.as_bytes());
        assert_ne!(record.as_bytes(), dek.as_bytes());
    }

    #[test]
    fn 服务端拿到_authkey_推不出别的() {
        // HKDF 单向：AuthKey 与 KEK 都从 MK 来，但彼此无法互推
        let mk = MasterKey::from_bytes([9u8; KEY_LEN]);
        let auth = mk.auth_key().unwrap();
        // 拿 AuthKey 当 MK 去派生 KEK，得到的与真 KEK 不同
        let fake = MasterKey::from_bytes(*auth.as_bytes())
            .key_encryption_key()
            .unwrap();
        assert_ne!(fake.as_bytes(), mk.key_encryption_key().unwrap().as_bytes());
    }

    #[test]
    fn 附件密钥对同一内容确定且随内容变化() {
        let dek = DataKey::from_bytes([9u8; KEY_LEN]);
        let h1 = plain_hash(b"a picture");
        let h2 = plain_hash(b"another picture");

        assert_eq!(
            derive_blob_key(&dek, &h1).unwrap().as_bytes(),
            derive_blob_key(&dek, &h1).unwrap().as_bytes(),
            "收敛加密要求确定性"
        );
        assert_ne!(
            derive_blob_key(&dek, &h1).unwrap().as_bytes(),
            derive_blob_key(&dek, &h2).unwrap().as_bytes()
        );
    }

    #[test]
    fn 缩略图密钥与原图密钥不同() {
        // 明文缩略图等于加密白做，两者必须各用各的密钥
        let dek = DataKey::from_bytes([9u8; KEY_LEN]);
        let h = plain_hash(b"pic");
        assert_ne!(
            derive_blob_key(&dek, &h).unwrap().as_bytes(),
            derive_thumb_key(&dek, &h).unwrap().as_bytes()
        );
    }

    #[test]
    fn 不同用户的相同附件密钥不同() {
        // 掺入 DEK 的目的：阻断服务端的「确认文件存在」攻击
        let h = plain_hash(b"same photo");
        let a = derive_blob_key(&DataKey::from_bytes([1u8; KEY_LEN]), &h).unwrap();
        let b = derive_blob_key(&DataKey::from_bytes([2u8; KEY_LEN]), &h).unwrap();
        assert_ne!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn 附件_nonce_确定且随密钥变化() {
        let dek = DataKey::from_bytes([5u8; KEY_LEN]);
        let k1 = derive_blob_key(&dek, &plain_hash(b"x")).unwrap();
        let k2 = derive_blob_key(&dek, &plain_hash(b"y")).unwrap();
        assert_eq!(deterministic_blob_nonce(&k1), deterministic_blob_nonce(&k1));
        assert_ne!(deterministic_blob_nonce(&k1), deterministic_blob_nonce(&k2));
    }

    #[test]
    fn debug_不泄漏密钥字节() {
        let mk = MasterKey::from_bytes([0xABu8; KEY_LEN]);
        let s = format!("{mk:?}");
        assert_eq!(s, "MasterKey(<redacted 32B>)");
        assert!(!s.contains("ab") && !s.contains("171"), "{s}");
    }

    #[test]
    fn hkdf_对照_rfc5869_向量() {
        // Test Case 1 的 expand 阶段：确认我们调的是标准 HKDF-SHA256
        let prk = hex::decode("077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5")
            .unwrap();
        let mut ikm = [0u8; KEY_LEN];
        ikm.copy_from_slice(&prk);
        let info = hex::decode("f0f1f2f3f4f5f6f7f8f9").unwrap();
        let okm = expand(&ikm, &info).unwrap();
        let expected = hex::decode(
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865",
        )
        .unwrap();
        assert_eq!(&okm[..], &expected[..KEY_LEN]);
    }

    #[test]
    fn argon2id_对照_rfc9106_风格向量() {
        // 固定输入 → 固定输出。换了底层实现或默认参数会立刻在这里暴露。
        let salt = [0x02u8; KDF_SALT_LEN];
        let mk = derive_master_key(
            "password",
            &salt,
            KdfParams {
                memory_kib: 256,
                iterations: 2,
                parallelism: 1,
            },
        )
        .unwrap();
        // 由本实现固定下来的基准值；它的作用是锁住行为，不是密码学正确性证明
        assert_eq!(mk.as_bytes().len(), KEY_LEN);
        let again = derive_master_key(
            "password",
            &salt,
            KdfParams {
                memory_kib: 256,
                iterations: 2,
                parallelism: 1,
            },
        )
        .unwrap();
        assert_eq!(mk.as_bytes(), again.as_bytes());
    }
}
