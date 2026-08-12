//! 加密信封（§8.3）。
//!
//! ```text
//! 偏移   长度   内容
//! 0      1      version = 0x01
//! 1      1      alg     = 0x01  (XChaCha20-Poly1305)
//! 2      24     nonce（随机）
//! 26     N      ciphertext
//! 26+N   16     tag
//! ```
//!
//! **为什么是 XChaCha20-Poly1305 而非 AES-GCM**：192 位 nonce 允许随机生成而不必
//! 担心碰撞。AES-GCM 的 96 位 nonce 在随机生成下有生日界约束，需要计数器管理，
//! 跨设备场景下计数器同步很麻烦。纯软件实现性能也好，不依赖 AES-NI。
//!
//! **AAD 必须绑定记录身份**：不加的话，攻击者可以把 A 记录的密文整块换成 B 记录的——
//! 内容读不懂，但能做定向破坏或让记录错位。加了 AAD 换位后解密直接失败。

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use rand::RngCore;
use uuid::Uuid;

use crate::keys::KEY_LEN;
use crate::{CryptoError, Result};

pub const ENVELOPE_V1: u8 = 0x01;
pub const ALG_XCHACHA20_POLY1305: u8 = 0x01;

const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
const HEADER_LEN: usize = 2 + NONCE_LEN;

/// 一个合法信封的最小长度：头部 + 空明文的 tag。
pub const MIN_ENVELOPE_LEN: usize = HEADER_LEN + TAG_LEN;

/// 绑进 AAD 的记录身份。
///
/// 片语用 16 字节 UUID，附件用 32 字节明文哈希——长度不同，因此不会互相冒充。
#[derive(Debug, Clone, Copy)]
pub enum Identity<'a> {
    /// 片语记录。
    Record(Uuid),
    /// 附件（§8.5 规定 `aad = plain_hash`）。
    Blob(&'a [u8; 32]),
    /// 包裹密钥等没有天然记录 id 的场合，用一个固定的域分隔串。
    Context(&'a [u8]),
}

impl Identity<'_> {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Identity::Record(id) => id.as_bytes(),
            Identity::Blob(h) => &h[..],
            Identity::Context(c) => c,
        }
    }
}

/// 解析出的信封头部。同步层在不持有密钥时用它读版本号。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Envelope {
    pub version: u8,
    pub alg: u8,
    pub nonce: [u8; NONCE_LEN],
    /// 密文 + tag 在整个信封里的起始偏移。
    pub body_offset: usize,
}

impl Envelope {
    pub fn parse_header(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < MIN_ENVELOPE_LEN {
            return Err(CryptoError::MalformedEnvelope("长度不足以容纳头部与 tag"));
        }
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&bytes[2..HEADER_LEN]);
        Ok(Self {
            version: bytes[0],
            alg: bytes[1],
            nonce,
            body_offset: HEADER_LEN,
        })
    }
}

fn build_aad(version: u8, alg: u8, identity: Identity<'_>) -> Vec<u8> {
    let id = identity.as_bytes();
    let mut aad = Vec::with_capacity(2 + id.len());
    aad.push(version);
    aad.push(alg);
    aad.extend_from_slice(id);
    aad
}

/// 用随机 nonce 封装。片语记录走这条路径。
pub fn seal(key: &[u8; KEY_LEN], identity: Identity<'_>, plaintext: &[u8]) -> Result<Vec<u8>> {
    let mut nonce = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    seal_with_nonce(key, identity, &nonce, plaintext)
}

/// 用指定 nonce 封装。
///
/// **只有收敛加密（附件）才可以传固定 nonce**——那里的密钥本身已与明文一一对应，
/// 同一 `(key, nonce)` 不会被用来加密两段不同的明文。其余场合一律用 [`seal`]。
pub fn seal_with_nonce(
    key: &[u8; KEY_LEN],
    identity: Identity<'_>,
    nonce: &[u8; NONCE_LEN],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let aad = build_aad(ENVELOPE_V1, ALG_XCHACHA20_POLY1305, identity);

    let sealed = cipher
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| CryptoError::Decrypt)?;

    let mut out = Vec::with_capacity(HEADER_LEN + sealed.len());
    out.push(ENVELOPE_V1);
    out.push(ALG_XCHACHA20_POLY1305);
    out.extend_from_slice(nonce);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// 拆开信封。`identity` 必须与封装时完全一致，否则解密失败。
pub fn open(key: &[u8; KEY_LEN], identity: Identity<'_>, envelope: &[u8]) -> Result<Vec<u8>> {
    let header = Envelope::parse_header(envelope)?;

    if header.version != ENVELOPE_V1 {
        return Err(CryptoError::UnsupportedVersion {
            version: header.version,
        });
    }
    if header.alg != ALG_XCHACHA20_POLY1305 {
        return Err(CryptoError::UnsupportedAlgorithm { alg: header.alg });
    }

    // AAD 用信封里**声明的**版本与算法，而不是常量——这样篡改头部的任何一个
    // 字节都会让 AAD 变化，进而 tag 校验失败
    let aad = build_aad(header.version, header.alg, identity);

    XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(&header.nonce),
            Payload {
                msg: &envelope[header.body_offset..],
                aad: &aad,
            },
        )
        .map_err(|_| CryptoError::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; KEY_LEN] = [0x42; KEY_LEN];

    fn id_a() -> Uuid {
        Uuid::parse_str("3f9a1c2e-0000-4000-8000-000000000001").unwrap()
    }
    fn id_b() -> Uuid {
        Uuid::parse_str("3f9a1c2e-0000-4000-8000-000000000002").unwrap()
    }

    #[test]
    fn 往返() {
        let msg = "在东京的最后一个下午，我在便利店门口站了很久。".as_bytes();
        let sealed = seal(&KEY, Identity::Record(id_a()), msg).unwrap();
        assert_eq!(open(&KEY, Identity::Record(id_a()), &sealed).unwrap(), msg);
    }

    #[test]
    fn 布局符合_8_3_的规格() {
        let sealed = seal(&KEY, Identity::Record(id_a()), b"hi").unwrap();
        assert_eq!(sealed[0], 0x01, "version");
        assert_eq!(sealed[1], 0x01, "alg");
        assert_eq!(sealed.len(), 2 + 24 + 2 + 16, "头 + nonce + 明文 + tag");
    }

    #[test]
    fn 密文里不含明文() {
        let msg = b"MAGIC_PLAINTEXT_MARKER";
        let sealed = seal(&KEY, Identity::Record(id_a()), msg).unwrap();
        assert!(sealed.windows(msg.len()).all(|w| w != msg));
    }

    #[test]
    fn 随机_nonce_使同一明文产生不同密文() {
        let a = seal(&KEY, Identity::Record(id_a()), b"same").unwrap();
        let b = seal(&KEY, Identity::Record(id_a()), b"same").unwrap();
        assert_ne!(a, b);
    }

    /// §8.3 的核心断言：**AAD 换位必须解密失败**。
    #[test]
    fn aad_换位必须解密失败() {
        let sealed = seal(&KEY, Identity::Record(id_a()), b"secret").unwrap();

        // 攻击者把 A 的密文整块塞到 B 的记录位置上
        assert_eq!(
            open(&KEY, Identity::Record(id_b()), &sealed),
            Err(CryptoError::Decrypt)
        );
        // 用正确身份仍打得开，说明失败确实来自 AAD 而非别的
        assert!(open(&KEY, Identity::Record(id_a()), &sealed).is_ok());
    }

    #[test]
    fn 片语与附件的身份不可互换() {
        let h = [0u8; 32];
        let sealed = seal(&KEY, Identity::Blob(&h), b"bytes").unwrap();
        assert!(open(&KEY, Identity::Record(id_a()), &sealed).is_err());
        assert!(open(&KEY, Identity::Blob(&h), &sealed).is_ok());
    }

    #[test]
    fn 篡改任意一个字节都会失败() {
        let sealed = seal(&KEY, Identity::Record(id_a()), b"tamper me please").unwrap();
        for i in 0..sealed.len() {
            let mut bad = sealed.clone();
            bad[i] ^= 0x01;
            assert!(
                open(&KEY, Identity::Record(id_a()), &bad).is_err(),
                "第 {i} 个字节被翻转后仍然解密成功"
            );
        }
    }

    #[test]
    fn 密钥错误则失败() {
        let sealed = seal(&KEY, Identity::Record(id_a()), b"x").unwrap();
        assert_eq!(
            open(&[0x43; KEY_LEN], Identity::Record(id_a()), &sealed),
            Err(CryptoError::Decrypt)
        );
    }

    #[test]
    fn 截断的信封被识别而不是崩溃() {
        let sealed = seal(&KEY, Identity::Record(id_a()), b"hello").unwrap();
        for cut in 0..MIN_ENVELOPE_LEN {
            assert!(
                matches!(
                    open(&KEY, Identity::Record(id_a()), &sealed[..cut]),
                    Err(CryptoError::MalformedEnvelope(_))
                ),
                "cut={cut}"
            );
        }
    }

    #[test]
    fn 未来版本被明确拒绝而不是当成损坏() {
        // 跨版本兼容的前提：老客户端必须能分辨「读不懂」与「被篡改」
        let mut sealed = seal(&KEY, Identity::Record(id_a()), b"x").unwrap();
        sealed[0] = 0x02;
        assert_eq!(
            open(&KEY, Identity::Record(id_a()), &sealed),
            Err(CryptoError::UnsupportedVersion { version: 2 })
        );

        let mut sealed = seal(&KEY, Identity::Record(id_a()), b"x").unwrap();
        sealed[1] = 0x7F;
        assert_eq!(
            open(&KEY, Identity::Record(id_a()), &sealed),
            Err(CryptoError::UnsupportedAlgorithm { alg: 0x7F })
        );
    }

    #[test]
    fn 空明文也能往返() {
        let sealed = seal(&KEY, Identity::Record(id_a()), b"").unwrap();
        assert_eq!(sealed.len(), MIN_ENVELOPE_LEN);
        assert!(open(&KEY, Identity::Record(id_a()), &sealed)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn 收敛加密下同一明文产生同一密文() {
        use crate::keys::{
            cipher_id, derive_blob_key, deterministic_blob_nonce, plain_hash, DataKey,
        };

        let dek = DataKey::from_bytes([1u8; KEY_LEN]);
        let bytes = b"pretend this is a jpeg";
        let h = plain_hash(bytes);

        let once = || {
            let cek = derive_blob_key(&dek, &h).unwrap();
            let nonce = deterministic_blob_nonce(&cek);
            seal_with_nonce(cek.as_bytes(), Identity::Blob(&h), &nonce, bytes).unwrap()
        };

        // cipher_id 恒定 → 上传前 HEAD 一下就能跳过重复上传
        assert_eq!(cipher_id(&once()), cipher_id(&once()));
    }
}
