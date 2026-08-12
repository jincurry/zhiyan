//! 备份与导出（§9.7）。
//!
//! | 类型 | 加密 | 说明 |
//! |---|---|---|
//! | 本地自动备份 `.zbk` | ✅ DEK | 每日 + 启动检查，保留 30 日 + 12 月 |
//! | 导出 JSON | ⚠️ 可选 | 加密导出（需恢复码）/ 明文导出 |
//! | 导出 Markdown | ❌ 明文 | 本质如此，导出前必须明确提示 |
//!
//! ## 一句必须写在 UI 上的话
//!
//! `.zbk` 用 DEK 加密，而 DEK 依赖密码 / 恢复码。**用户忘了密码又丢了恢复码时，
//! 本地备份同样打不开。** 所以「注销账号」与「重置密码」两个流程必须先引导
//! 导出明文 Markdown——那是唯一不依赖任何密钥的出口。
//!
//! 排期与保留策略（每日、启动检查、30 日 + 12 月）属于应用层，在阶段五接。
//! 这里只负责格式本身。

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};
use zhiyan_crypto::{open, seal, DataKey, Identity};

use crate::model::{Memo, MemoInput};
use crate::{DbError, Result, Store};

/// `.zbk` 的 16 字节 magic。
///
/// 认得出文件类型，杀毒软件与备份工具也不会把它当成随机垃圾。
const MAGIC: &[u8; 16] = b"ZHIYAN-BACKUP\x00\x00\x00";
/// 紧跟 magic 的一字节格式版本。
const BACKUP_V1: u8 = 1;
const HEADER_LEN: usize = MAGIC.len() + 1;

/// 备份的 AAD 域。防的是把别处的信封整块塞进 `.zbk` 冒充备份。
const BACKUP_AAD: &[u8] = b"zhiyan/backup/v1";

/// zstd 压缩级别。
///
/// 3 是 zstd 的默认档。备份跑在后台且每天一次，再高的档位省下的那点体积
/// 换不来卡住用户几百毫秒。
const ZSTD_LEVEL: i32 = 3;

/// 解压后的快照上限。万级记录的快照是几 MB 量级，256 MiB 留了三个数量级的余量。
const MAX_SNAPSHOT_BYTES: u64 = 256 * 1024 * 1024;

/// 备份的全量内容。
///
/// **只备份用户数据**：`server_seq` 与 outbox 不进来，恢复时 `dirty` 一律重新
/// 置位。那几样描述的是「这台机器与服务端同步到哪了」，照搬到另一台机器上会
/// 让同步状态错乱——恢复之后本来就该重新走一次全量同步。
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub v: u32,
    /// 生成时刻，毫秒。
    pub at: i64,
    pub memos: Vec<Memo>,
    /// `k → v`，只含不敏感项（§9.1）。
    pub kv: std::collections::BTreeMap<String, String>,
}

impl Store {
    /// 导出全量快照。**包含回收站，不含墓碑**——墓碑没有正文，恢复它毫无意义。
    pub fn snapshot(&self, now_ms: i64) -> Result<Snapshot> {
        let mut memos = Vec::new();
        let ids: Vec<uuid::Uuid> = {
            let mut st = self
                .conn()
                .prepare("SELECT id FROM memo WHERE purged_at IS NULL ORDER BY created_at")?;
            let rows = st
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        for id in ids {
            if let Some(m) = self.get_memo(id)? {
                memos.push(m);
            }
        }

        let mut kv = std::collections::BTreeMap::new();
        {
            let mut st = self.conn().prepare("SELECT k, v FROM kv")?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                kv.insert(r.get::<_, String>(0)?, r.get::<_, String>(1)?);
            }
        }

        Ok(Snapshot {
            v: 1,
            at: now_ms,
            memos,
            kv,
        })
    }

    /// 生成 `.zbk` 的字节。
    ///
    /// `magic(16) | version(1) | 信封`，信封内容是 zstd 压缩的全量 JSON。
    ///
    /// 版本号在**信封外面**：老版本程序拿到新格式的备份，要能读出「这是 v2，
    /// 我不认识」并如实告诉用户，而不是报一个「解密失败」让人以为文件坏了。
    pub fn backup(&self, dek: &DataKey, now_ms: i64) -> Result<Vec<u8>> {
        let json = serde_json::to_vec(&self.snapshot(now_ms)?)
            .map_err(|e| DbError::Io(format!("序列化快照: {e}")))?;

        let mut enc = zstd::Encoder::new(Vec::new(), ZSTD_LEVEL)
            .map_err(|e| DbError::Io(format!("zstd 初始化: {e}")))?;
        enc.write_all(&json)
            .map_err(|e| DbError::Io(format!("zstd 压缩: {e}")))?;
        let packed = enc
            .finish()
            .map_err(|e| DbError::Io(format!("zstd 收尾: {e}")))?;

        let key = dek.backup_key()?;
        let sealed = seal(key.as_bytes(), Identity::Context(BACKUP_AAD), &packed)?;

        let mut out = Vec::with_capacity(HEADER_LEN + sealed.len());
        out.extend_from_slice(MAGIC);
        out.push(BACKUP_V1);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    /// 解开 `.zbk`。
    pub fn read_backup(dek: &DataKey, bytes: &[u8]) -> Result<Snapshot> {
        if bytes.len() < HEADER_LEN || &bytes[..MAGIC.len()] != MAGIC {
            return Err(DbError::Io("不是知言的备份文件".into()));
        }
        let version = bytes[MAGIC.len()];
        if version != BACKUP_V1 {
            return Err(DbError::Io(format!(
                "备份格式版本 {version} 比本程序新，请升级知言"
            )));
        }

        let key = dek.backup_key()?;
        let packed = open(
            key.as_bytes(),
            Identity::Context(BACKUP_AAD),
            &bytes[HEADER_LEN..],
        )?;

        // 解压**必须封顶**：一个精心构造的 zstd 流能把几 KB 膨胀成几十 GB 把内存吃光。
        // 备份文件是本机自己写的，但恢复入口让用户选任意文件——那就是不可信输入了。
        // （信封的 tag 已经先过了一道，所以这里挡的是「攻击者拿到了 DEK」之外的
        // 剩余情形，比如用户被骗着导入了一个别人给的 .zbk）
        let mut json = Vec::new();
        let dec = zstd::Decoder::new(&packed[..])
            .map_err(|e| DbError::Io(format!("zstd 初始化: {e}")))?;
        std::io::Read::take(dec, MAX_SNAPSHOT_BYTES + 1)
            .read_to_end(&mut json)
            .map_err(|e| DbError::Io(format!("zstd 解压: {e}")))?;
        if json.len() as u64 > MAX_SNAPSHOT_BYTES {
            return Err(DbError::Io("备份内容超过上限，已中止".into()));
        }

        serde_json::from_slice(&json).map_err(|e| DbError::Io(format!("解析快照: {e}")))
    }

    /// 把快照写回库里。**逐条 upsert，不清空现有数据**。
    ///
    /// 清空重建看着更「干净」，但恢复是个高风险操作：用户在恢复到一半时断电、
    /// 或者选错了备份文件，清空过的库就再也回不来了。逐条合并的代价是可能留下
    /// 几条备份里没有的记录，那远比丢数据好收拾。
    pub fn restore_snapshot(&mut self, snap: &Snapshot, now_ms: i64) -> Result<usize> {
        let mut n = 0;
        for m in &snap.memos {
            // 附件字节不在快照里（它们在 blobs\ 目录下，另行备份）。引用一个本地
            // 没有的 hash 会让 upsert 直接失败，那样一条丢了图的记录会连正文一起
            // 恢复不进来——正文比图重要，所以这里只带本机确实有的
            let mut blobs = Vec::new();
            for b in &m.blobs {
                let have = hex::decode(&b.sha256)
                    .ok()
                    .and_then(|h| <[u8; 32]>::try_from(h.as_slice()).ok())
                    .map(|h| self.blob_meta(&h).map(|o| o.is_some()))
                    .transpose()?
                    .unwrap_or(false);
                if have {
                    blobs.push(b.clone());
                }
            }

            let input = MemoInput {
                id: Some(m.id),
                text: m.text.clone(),
                source: Some(m.source),
                pinned: Some(m.pinned),
                blobs,
                created_at: Some(m.created_at),
            };
            self.upsert_memo(&input, now_ms)?;
            if let Some(deleted_at) = m.deleted_at {
                self.soft_delete(m.id, deleted_at)?;
            }
            n += 1;
        }
        for (k, v) in &snap.kv {
            self.set_kv(k, v)?;
        }
        Ok(n)
    }

    /// 明文 Markdown 导出（§9.7）。
    ///
    /// **这是唯一不依赖任何密钥的出口**，所以它必须永远能用——包括在用户
    /// 打算注销账号、或者已经不记得密码只是还开着应用的时候。
    ///
    /// 一条一个 `## 时间` 小节。不做 front-matter：导出的东西是给人读的，
    /// 以及丢进别的笔记软件里的，YAML 头在多数编辑器里只会碍眼。
    pub fn export_markdown(&self, utc_offset_secs: i32) -> Result<String> {
        let mut out = String::from("# 知言导出\n");
        let mut st = self.conn().prepare(
            "SELECT created_at, text FROM memo \
             WHERE deleted_at IS NULL AND purged_at IS NULL ORDER BY created_at",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let at: i64 = r.get(0)?;
            let text: String = r.get(1)?;
            out.push_str(&format!(
                "\n## {}\n\n",
                crate::stats::stamp(at, utc_offset_secs)
            ));
            out.push_str(text.trim_end());
            out.push('\n');
        }
        Ok(out)
    }

    /// 明文 JSON 导出（§9.7 的「明文导出」那一档）。
    pub fn export_json(&self, now_ms: i64) -> Result<String> {
        serde_json::to_string_pretty(&self.snapshot(now_ms)?)
            .map_err(|e| DbError::Io(format!("序列化: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MemoInput;
    use zhiyan_crypto::KEY_LEN;

    const T0: i64 = 1_754_380_320_000;

    fn dek() -> DataKey {
        DataKey::from_bytes([0x33u8; KEY_LEN])
    }

    fn seeded() -> Store {
        let mut s = crate::tests::store();
        s.upsert_memo(
            &MemoInput {
                text: "第一条 #读书/认知".into(),
                created_at: Some(T0),
                ..Default::default()
            },
            T0,
        )
        .unwrap();
        let b = s
            .upsert_memo(
                &MemoInput {
                    text: "第二条，进了回收站".into(),
                    created_at: Some(T0 + 1000),
                    ..Default::default()
                },
                T0 + 1000,
            )
            .unwrap();
        s.soft_delete(b.id, T0 + 2000).unwrap();
        s.set_kv("theme", "dark").unwrap();
        s
    }

    #[test]
    fn 备份往返() {
        let s = seeded();
        let bytes = s.backup(&dek(), T0 + 9000).unwrap();
        let snap = Store::read_backup(&dek(), &bytes).unwrap();

        assert_eq!(snap.memos.len(), 2, "回收站里的也要备份");
        assert_eq!(snap.memos[0].text, "第一条 #读书/认知");
        assert_eq!(snap.kv.get("theme").map(String::as_str), Some("dark"));
    }

    #[test]
    fn 备份文件里没有明文() {
        let mut s = crate::tests::store();
        s.upsert_memo(
            &MemoInput {
                text: "MAGIC_PLAINTEXT_MARKER".into(),
                ..Default::default()
            },
            T0,
        )
        .unwrap();
        let bytes = s.backup(&dek(), T0).unwrap();
        let needle = b"MAGIC_PLAINTEXT_MARKER";
        assert!(!bytes.windows(needle.len()).any(|w| w == needle));
    }

    #[test]
    fn 头部在信封外面() {
        let s = seeded();
        let bytes = s.backup(&dek(), T0).unwrap();
        assert_eq!(&bytes[..16], MAGIC);
        assert_eq!(bytes[16], BACKUP_V1);
    }

    #[test]
    fn 换把密钥打不开() {
        let s = seeded();
        let bytes = s.backup(&dek(), T0).unwrap();
        assert!(Store::read_backup(&DataKey::from_bytes([0u8; KEY_LEN]), &bytes).is_err());
    }

    #[test]
    fn 不是备份文件时报得明白() {
        let e = Store::read_backup(&dek(), b"just some random bytes here ok").unwrap_err();
        assert!(format!("{e}").contains("不是知言的备份"), "{e}");
    }

    #[test]
    fn 更高的格式版本被单独识别出来() {
        // 「这是新版写的」和「文件坏了」必须给用户两种不同的话
        let s = seeded();
        let mut bytes = s.backup(&dek(), T0).unwrap();
        bytes[16] = 2;
        let e = Store::read_backup(&dek(), &bytes).unwrap_err();
        assert!(format!("{e}").contains("请升级"), "{e}");
    }

    #[test]
    fn 篡改一个字节就打不开() {
        let s = seeded();
        let mut bytes = s.backup(&dek(), T0).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        assert!(Store::read_backup(&dek(), &bytes).is_err());
    }

    #[test]
    fn 恢复到一个空库() {
        let s = seeded();
        let bytes = s.backup(&dek(), T0 + 9000).unwrap();
        let snap = Store::read_backup(&dek(), &bytes).unwrap();

        let mut fresh = crate::tests::store();
        assert_eq!(fresh.restore_snapshot(&snap, T0 + 10_000).unwrap(), 2);

        let live = fresh.list_memos(&Default::default(), None, 10).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].text, "第一条 #读书/认知");
        assert_eq!(live[0].tags, vec!["读书/认知"], "标签要重新解析出来");

        let trash = fresh
            .list_memos(
                &crate::Filter {
                    view: crate::View::Trash,
                    ..Default::default()
                },
                None,
                10,
            )
            .unwrap();
        assert_eq!(trash.len(), 1, "回收站状态要保住");
        assert_eq!(fresh.get_kv("theme").unwrap().as_deref(), Some("dark"));
    }

    #[test]
    fn 恢复保住原来的_id_与创建时间() {
        // id 变了的话，正文里的 [[…^uuid]] 引用会全部指空
        let s = seeded();
        let snap = Store::read_backup(&dek(), &s.backup(&dek(), T0).unwrap()).unwrap();
        let mut fresh = crate::tests::store();
        fresh.restore_snapshot(&snap, T0 + 10_000).unwrap();

        let m = fresh.get_memo(snap.memos[0].id).unwrap().unwrap();
        assert_eq!(m.created_at, T0);
    }

    #[test]
    fn 恢复不清空现有数据() {
        let mut fresh = crate::tests::store();
        let mine = fresh
            .upsert_memo(
                &MemoInput {
                    text: "备份里没有的一条".into(),
                    ..Default::default()
                },
                T0,
            )
            .unwrap();

        let s = seeded();
        let snap = Store::read_backup(&dek(), &s.backup(&dek(), T0).unwrap()).unwrap();
        fresh.restore_snapshot(&snap, T0 + 10_000).unwrap();

        assert!(fresh.get_memo(mine.id).unwrap().is_some(), "不该被清掉");
    }

    #[test]
    fn 快照不带同步状态() {
        // 恢复到另一台机器时，dirty / server_seq 会让同步错乱
        let s = seeded();
        let json = s.export_json(T0).unwrap();
        assert!(!json.contains("serverSeq"), "{json}");
        assert!(!json.contains("outbox"));
    }

    #[test]
    fn markdown_导出是明文且不含回收站() {
        let s = seeded();
        let md = s.export_markdown(8 * 3600).unwrap();
        assert!(md.contains("第一条 #读书/认知"));
        assert!(!md.contains("进了回收站"));
        assert!(md.starts_with("# 知言导出"));
    }
}
