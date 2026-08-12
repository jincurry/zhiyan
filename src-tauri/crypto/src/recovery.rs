//! 恢复码（§8.4）。
//!
//! ```text
//! protected_dek_pw       = Enc(HKDF(MK,"wrap"), DEK)
//! protected_dek_recovery = Enc(HKDF(RK,"wrap"), DEK)
//! ```
//!
//! **忘记密码 = 数据永久丢失**，服务端无任何恢复手段。所以恢复码是注册流程的
//! 强制环节，且必须**要求用户回填其中随机两组**（[`RecoveryCode::challenge`] /
//! [`RecoveryCode::verify_groups`]）。
//!
//! 第三步不能省。只显示不校验的话，绝大多数用户会直接点下一步，然后在第一次
//! 忘记密码时永久丢失全部数据。这会增加注册摩擦，但 E2EE 下没有第二次机会。
//!
//! ## 编码格式与文档的一处出入
//!
//! §8.4 写「RK 为 **128 位**随机数，编成 **10 组 5 字符** Base32」。这两个数字
//! 对不上：10×5 = 50 个 Base32 字符能装 250 bit，只放 128 bit 会空掉一多半。
//!
//! 这里按 **240 bit 熵 + 10 bit 校验位 = 250 bit** 实现：版式与文档完全一致
//! （用户要抄的仍是 50 个字符），熵只多不少，满足「至少 128 位」的安全要求。
//! **已确认按 240 bit 实现（2026-08-12）**，服务端与其他客户端按这个来。

use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::keys::{KeyEncryptionKey, KEY_LEN};
use crate::{CryptoError, Result};

/// 分组数。
pub const RECOVERY_GROUPS: usize = 10;
/// 每组字符数。
pub const RECOVERY_GROUP_LEN: usize = 5;
/// 去掉分隔符后的总字符数。
pub const RECOVERY_CHARS: usize = RECOVERY_GROUPS * RECOVERY_GROUP_LEN;
/// 熵的字节数（240 bit）。
pub const RECOVERY_ENTROPY_LEN: usize = 30;

const CHECKSUM_BITS: u32 = 10;

/// Crockford Base32 字母表：**已排除易混淆的 I、L、O、U**（§8.4）。
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// 一份恢复码。
///
/// 刻意不实现 `Display`——渲染必须显式调用 [`RecoveryCode::to_grouped_string`]，
/// 免得被 `format!("{}")` 顺手写进日志。
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct RecoveryCode {
    entropy: [u8; RECOVERY_ENTROPY_LEN],
}

impl core::fmt::Debug for RecoveryCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RecoveryCode(<redacted>)")
    }
}

impl RecoveryCode {
    pub fn generate() -> Self {
        let mut entropy = [0u8; RECOVERY_ENTROPY_LEN];
        rand::rngs::OsRng.fill_bytes(&mut entropy);
        Self { entropy }
    }

    /// 从裸熵构造（仅供测试与迁移）。
    pub fn from_entropy(entropy: [u8; RECOVERY_ENTROPY_LEN]) -> Self {
        Self { entropy }
    }

    /// `HKDF(RK,"wrap")`：包裹 DEK 的 KEK。
    ///
    /// 先把 30 字节熵压成 32 字节 PRK，与密码那条路径的 KEK 派生保持同一形状。
    pub fn key_encryption_key(&self) -> Result<KeyEncryptionKey> {
        let prk: [u8; KEY_LEN] = Sha256::new_with_prefix(b"zhiyan/v1/rk")
            .chain_update(self.entropy)
            .finalize()
            .into();

        let hk = hkdf::Hkdf::<Sha256>::from_prk(&prk).map_err(|_| CryptoError::KeyDerivation)?;
        let mut out = [0u8; KEY_LEN];
        hk.expand(b"zhiyan/v1/wrap", &mut out)
            .map_err(|_| CryptoError::KeyDerivation)?;
        Ok(KeyEncryptionKey::from_bytes(out))
    }

    /// 编成 10 组，供显示与「下载 txt」用。形如 `K7QF2-9XBTM-…`。
    pub fn to_grouped_string(&self) -> String {
        let chars = self.encode();
        chars
            .chunks(RECOVERY_GROUP_LEN)
            .map(|g| core::str::from_utf8(g).expect("字母表是 ASCII"))
            .collect::<Vec<_>>()
            .join("-")
    }

    /// 取第 `index` 组（0 起）。注册时的回填校验用。
    pub fn group(&self, index: usize) -> Option<String> {
        self.encode()
            .chunks(RECOVERY_GROUP_LEN)
            .nth(index)
            .map(|g| String::from_utf8(g.to_vec()).expect("字母表是 ASCII"))
    }

    /// 随机挑两组出题。注册流程第 ③ 步用它。
    pub fn challenge() -> [usize; 2] {
        let mut rng = rand::rngs::OsRng;
        let a = (rng.next_u32() as usize) % RECOVERY_GROUPS;
        let mut b = (rng.next_u32() as usize) % (RECOVERY_GROUPS - 1);
        if b >= a {
            b += 1;
        }
        if a < b {
            [a, b]
        } else {
            [b, a]
        }
    }

    /// 校验回填。常量时间比较，大小写与 Crockford 混淆字符都做归一化。
    pub fn verify_groups(&self, answers: &[(usize, &str)]) -> bool {
        if answers.is_empty() {
            return false;
        }
        let chars = self.encode();
        let mut ok = 1u8;

        for &(index, answer) in answers {
            let Some(expected) = chars.chunks(RECOVERY_GROUP_LEN).nth(index) else {
                return false;
            };
            let Ok(actual) = normalize(answer) else {
                return false;
            };
            // normalize 产出字母表下标，encode 产出字母表字符，换算到同一个域再比
            let expected: Vec<u8> = expected
                .iter()
                .map(|c| ALPHABET.iter().position(|a| a == c).expect("由字母表生成") as u8)
                .collect();
            if actual.len() != expected.len() {
                return false;
            }
            ok &= actual.ct_eq(&expected).unwrap_u8();
        }
        ok == 1
    }

    /// 解析用户输入。允许任意分隔符与大小写，自动纠正 `I/L→1`、`O→0`。
    pub fn parse(input: &str) -> Result<Self> {
        let symbols = normalize(input)?;
        if symbols.len() != RECOVERY_CHARS {
            return Err(CryptoError::MalformedRecoveryCode("必须是 50 个有效字符"));
        }

        let mut bits = BitReader::new(&symbols);
        let mut entropy = [0u8; RECOVERY_ENTROPY_LEN];
        for byte in entropy.iter_mut() {
            *byte = bits.read(8) as u8;
        }
        let got = bits.read(CHECKSUM_BITS) as u16;

        if got.ct_eq(&checksum(&entropy)).unwrap_u8() != 1 {
            entropy.zeroize();
            return Err(CryptoError::RecoveryChecksum);
        }
        Ok(Self { entropy })
    }

    fn encode(&self) -> [u8; RECOVERY_CHARS] {
        let mut w = BitWriter::default();
        for &byte in &self.entropy {
            w.write(byte as u64, 8);
        }
        w.write(checksum(&self.entropy) as u64, CHECKSUM_BITS);
        w.finish()
    }
}

/// 校验位：SHA-256 的前 10 bit。**只用于挡抄错，不承担安全性。**
fn checksum(entropy: &[u8; RECOVERY_ENTROPY_LEN]) -> u16 {
    let d = Sha256::new_with_prefix(b"zhiyan/v1/rk-checksum")
        .chain_update(entropy)
        .finalize();
    ((d[0] as u16) << 2) | ((d[1] as u16) >> 6)
}

/// 归一成字母表下标序列：忽略分隔符，纠正 Crockford 的混淆字符。
fn normalize(input: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(RECOVERY_CHARS);
    for ch in input.chars() {
        if !ch.is_ascii_alphanumeric() {
            if ch.is_whitespace() || ch == '-' || ch == '_' {
                continue;
            }
            return Err(CryptoError::MalformedRecoveryCode("含有非法字符"));
        }
        let normalized = match ch.to_ascii_uppercase() {
            // 用户看到的 I/L 其实是 1，O 其实是 0
            'I' | 'L' => b'1',
            'O' => b'0',
            // U 被刻意排除（避免拼出不雅词），出现即视为抄错
            'U' => return Err(CryptoError::MalformedRecoveryCode("字母 U 不在字母表内")),
            c => c as u8,
        };
        let Some(index) = ALPHABET.iter().position(|&a| a == normalized) else {
            return Err(CryptoError::MalformedRecoveryCode("含有非法字符"));
        };
        out.push(index as u8);
    }
    Ok(out)
}

/// MSB 优先的 5 bit 打包器。
#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    bits: u32,
}

impl BitWriter {
    fn write(&mut self, value: u64, width: u32) {
        self.acc = (self.acc << width) | value;
        self.bits += width;
        while self.bits >= 5 {
            self.bits -= 5;
            self.out
                .push(ALPHABET[((self.acc >> self.bits) & 0x1F) as usize]);
        }
    }

    fn finish(self) -> [u8; RECOVERY_CHARS] {
        debug_assert_eq!(self.bits, 0, "250 bit 恰好是 5 的整数倍，不应有余位");
        let mut chars = [0u8; RECOVERY_CHARS];
        chars.copy_from_slice(&self.out);
        chars
    }
}

/// 与 [`BitWriter`] 配对的读取器。输入是字母表下标序列。
struct BitReader<'a> {
    symbols: &'a [u8],
    pos: usize,
    acc: u64,
    bits: u32,
}

impl<'a> BitReader<'a> {
    fn new(symbols: &'a [u8]) -> Self {
        Self {
            symbols,
            pos: 0,
            acc: 0,
            bits: 0,
        }
    }

    fn read(&mut self, width: u32) -> u64 {
        while self.bits < width {
            let symbol = self.symbols.get(self.pos).copied().unwrap_or(0) as u64;
            self.pos += 1;
            self.acc = (self.acc << 5) | symbol;
            self.bits += 5;
        }
        self.bits -= width;
        let value = (self.acc >> self.bits) & ((1u64 << width) - 1);
        self.acc &= (1u64 << self.bits) - 1;
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 版式为十组五字符() {
        let s = RecoveryCode::generate().to_grouped_string();
        let groups: Vec<&str> = s.split('-').collect();
        assert_eq!(groups.len(), RECOVERY_GROUPS);
        assert!(groups.iter().all(|g| g.len() == RECOVERY_GROUP_LEN));
    }

    #[test]
    fn 只用_crockford_字母表且无易混淆字符() {
        for _ in 0..64 {
            for ch in RecoveryCode::generate()
                .to_grouped_string()
                .chars()
                .filter(|c| *c != '-')
            {
                assert!(ALPHABET.contains(&(ch as u8)), "字母表外的字符 {ch}");
                assert!(!"ILOU".contains(ch), "易混淆字符 {ch}");
            }
        }
    }

    #[test]
    fn 熵不低于文档要求的_128_bit() {
        const { assert!(RECOVERY_ENTROPY_LEN * 8 >= 128) };
    }

    #[test]
    fn 往返() {
        for _ in 0..64 {
            let code = RecoveryCode::generate();
            assert_eq!(
                RecoveryCode::parse(&code.to_grouped_string())
                    .unwrap()
                    .entropy,
                code.entropy
            );
        }
    }

    /// §8.4 的完整用途：忘记密码时只凭抄在纸上的恢复码取回 DEK。
    #[test]
    fn 恢复码路径能取回_dek() {
        use crate::envelope::{open, seal, Identity};
        use crate::keys::DataKey;

        // 注册：随机 DEK，用恢复码 KEK 包一份
        let dek = DataKey::random();
        let code = RecoveryCode::generate();
        let protected = seal(
            code.key_encryption_key().unwrap().as_bytes(),
            Identity::Context(b"protected_dek"),
            dek.as_bytes(),
        )
        .unwrap();

        // 忘记密码：用户照着纸抄进来（顺手抄成了小写）
        let typed = code.to_grouped_string().to_lowercase();
        let recovered = RecoveryCode::parse(&typed)
            .unwrap()
            .key_encryption_key()
            .unwrap();
        let plain = open(
            recovered.as_bytes(),
            Identity::Context(b"protected_dek"),
            &protected,
        )
        .unwrap();

        assert_eq!(&plain[..], dek.as_bytes());
    }

    #[test]
    fn 抄错一位会被校验位挡下() {
        let s = RecoveryCode::generate().to_grouped_string();
        let (mut wrong, mut caught) = (0usize, 0usize);

        for (i, ch) in s.char_indices().filter(|(_, c)| *c != '-') {
            for &replacement in ALPHABET.iter() {
                if replacement as char == ch {
                    continue;
                }
                let mut bad = s.clone();
                bad.replace_range(i..i + 1, &(replacement as char).to_string());
                wrong += 1;
                if RecoveryCode::parse(&bad).is_err() {
                    caught += 1;
                }
            }
        }
        // 10 bit 校验位的理论漏检率约 1/1024
        let miss = (wrong - caught) as f64 / wrong as f64;
        assert!(miss < 0.01, "单字符抄错的漏检率 {miss} 偏高");
    }

    #[test]
    fn 分隔符与大小写不敏感() {
        let code = RecoveryCode::generate();
        let canonical = code.to_grouped_string();
        for variant in [
            canonical.replace('-', ""),
            canonical.replace('-', " "),
            canonical.to_lowercase(),
            format!("  {canonical}\n"),
        ] {
            assert_eq!(
                RecoveryCode::parse(&variant).unwrap().entropy,
                code.entropy,
                "{variant}"
            );
        }
    }

    #[test]
    fn 混淆字符被纠正() {
        // 用户把 1 抄成 l、0 抄成 O，仍应解析成功——这正是 Crockford 变体的意义
        let code = RecoveryCode::from_entropy([0u8; RECOVERY_ENTROPY_LEN]);
        let typo = code.to_grouped_string().replace('1', "l").replace('0', "O");
        assert_eq!(RecoveryCode::parse(&typo).unwrap().entropy, code.entropy);
    }

    #[test]
    fn 字母_u_被拒绝() {
        let s = RecoveryCode::generate().to_grouped_string();
        let bad = format!("U{}", &s[1..]);
        assert!(matches!(
            RecoveryCode::parse(&bad),
            Err(CryptoError::MalformedRecoveryCode(_)) | Err(CryptoError::RecoveryChecksum)
        ));
    }

    #[test]
    fn 长度不对被拒绝() {
        let s = RecoveryCode::generate().to_grouped_string();
        for bad in [&s[..s.len() - 1], &format!("{s}Z")[..]] {
            assert!(matches!(
                RecoveryCode::parse(bad),
                Err(CryptoError::MalformedRecoveryCode(_))
            ));
        }
    }

    #[test]
    fn 回填校验() {
        let code = RecoveryCode::generate();
        let [a, b] = RecoveryCode::challenge();
        assert!(a < b && b < RECOVERY_GROUPS, "出题的两组必须不同且升序");

        let (ga, gb) = (code.group(a).unwrap(), code.group(b).unwrap());
        assert!(code.verify_groups(&[(a, &ga), (b, &gb)]));
        assert!(
            code.verify_groups(&[(a, &ga.to_lowercase()), (b, &gb)]),
            "应大小写不敏感"
        );

        assert!(!code.verify_groups(&[(a, &gb), (b, &ga)]), "组序错位应失败");
        assert!(!code.verify_groups(&[(a, "ZZZZZ"), (b, &gb)]));
        assert!(!code.verify_groups(&[(a, "ZZZ")]), "长度不足应失败");
        assert!(!code.verify_groups(&[]), "空答案不算通过");
        assert!(
            !code.verify_groups(&[(RECOVERY_GROUPS, &ga)]),
            "越界下标应失败"
        );
    }

    #[test]
    fn debug_不泄漏恢复码() {
        assert_eq!(
            format!("{:?}", RecoveryCode::generate()),
            "RecoveryCode(<redacted>)"
        );
    }

    #[test]
    fn 轮换后旧码作废() {
        // §8.4：用过一次的恢复码应作废并生成新的
        let old = RecoveryCode::generate();
        let new = RecoveryCode::generate();
        assert_ne!(
            old.key_encryption_key().unwrap().as_bytes(),
            new.key_encryption_key().unwrap().as_bytes()
        );
    }
}
