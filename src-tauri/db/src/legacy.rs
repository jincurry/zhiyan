//! 从原型的整包 JSON 迁移过来（§7.3）。
//!
//! 原型把所有东西塞在 `window.storage` 的一个键里：
//!
//! ```json
//! { "memos": [{ "id": 4, "t": 1754380320000, "src": "app",
//!               "x": "正文…", "rel": [12], "pin": false,
//!               "imgs": ["data:image/jpeg;base64,…"] }],
//!   "trash": [{ "…": "…", "del": 1754380911000 }] }
//! ```
//!
//! 三件事必须一起做，缺一件数据就是坏的：
//!
//! **① `int` → UUIDv4。** 自增整数主键会把「这是第几条」直接告诉服务端，
//! 而且多设备下必然撞号。换成 UUIDv4（§7.1：不用 ULID / UUIDv7，
//! 那两者前 48 位是明文时间戳）。
//!
//! **② 正文里的 `[[摘要^数字]]` 要跟着改写。** 只换主键不改正文的话，
//! 所有行内引用会指向不存在的记录——而且是**静默**指空，界面上就是一堆
//! 点不开的引用卡片。
//!
//! **③ `imgs` 里的 base64 要落成内容寻址的附件。** 原型把 2MB 的图存成
//! 2.7MB 的文本塞进 JSON，每次读写整条搬运（§7.3 ①）。
//!
//! §7.3 的原话是「**趁数据量小尽早做**」。

use std::collections::HashMap;

use serde::Deserialize;
use uuid::Uuid;
use zhiyan_crypto::DataKey;

use crate::model::{MemoInput, Source};
use crate::{BlobStore, DbError, Result, Store};

/// 原型那一条记录的形状。字段名短是因为它们直接进了 localStorage。
#[derive(Debug, Deserialize)]
struct LegacyMemo {
    id: i64,
    /// 创建时间，毫秒。
    t: i64,
    #[serde(default)]
    x: String,
    #[serde(default)]
    src: Option<String>,
    #[serde(default)]
    pin: bool,
    /// 显式关联的记录 id。
    #[serde(default)]
    rel: Vec<i64>,
    /// base64 的 data URL。
    #[serde(default)]
    imgs: Vec<String>,
    /// 进回收站的时刻，只有 `trash` 里的有。
    #[serde(default)]
    del: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct LegacyDump {
    #[serde(default)]
    memos: Vec<LegacyMemo>,
    #[serde(default)]
    trash: Vec<LegacyMemo>,
}

/// 迁移结果。给用户看的，所以每一项都要能对应界面上的一句话。
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyReport {
    pub memos: usize,
    pub trashed: usize,
    /// 成功落盘的附件数。
    pub blobs: usize,
    /// 解不开的附件数（data URL 坏了 / 不是图片）。
    ///
    /// **单独报出来而不是算进成功数**：用户得知道有几张图没跟过来，
    /// 才有机会去原型里重新导一遍。
    pub blobs_failed: usize,
    /// 改写掉的行内引用数。
    pub refs_rewritten: usize,
}

impl Store {
    /// 导入原型导出的整包 JSON。
    ///
    /// 幂等性：**不保证**。重跑一次会得到另一批 UUID，也就是另一批记录。
    /// 调用方（IPC 层）负责只在空库上提供这个入口。
    pub fn import_legacy(
        &mut self,
        blobs: &BlobStore,
        dek: &DataKey,
        json: &str,
        now_ms: i64,
    ) -> Result<LegacyReport> {
        let dump: LegacyDump =
            serde_json::from_str(json).map_err(|e| DbError::Io(format!("解析导出文件: {e}")))?;

        // 映射表要**先建全**：正文里可以引用回收站里的记录，
        // 边建边引的话前面那些会查不到后面的
        let mut map: HashMap<i64, Uuid> = HashMap::new();
        for m in dump.memos.iter().chain(dump.trash.iter()) {
            map.entry(m.id).or_insert_with(Uuid::new_v4);
        }

        let mut report = LegacyReport::default();

        for (m, trashed) in dump
            .memos
            .iter()
            .map(|m| (m, false))
            .chain(dump.trash.iter().map(|m| (m, true)))
        {
            let id = map[&m.id];
            let (text, rewritten) = rewrite_refs(&m.x, &map);
            report.refs_rewritten += rewritten;

            let mut metas = Vec::new();
            for data_url in &m.imgs {
                match decode_data_url(data_url)
                    .ok_or(DbError::BlobCorrupted)
                    .and_then(|(mime, bytes)| self.put_blob(blobs, dek, &bytes, &mime, None, None))
                {
                    Ok(meta) => {
                        metas.push(meta);
                        report.blobs += 1;
                    }
                    // 一张图坏了不该让整条笔记进不来。正文比图重要
                    Err(_) => report.blobs_failed += 1,
                }
            }

            self.upsert_memo(
                &MemoInput {
                    id: Some(id),
                    text,
                    source: Some(Source::parse(m.src.as_deref().unwrap_or("app"))),
                    pinned: Some(m.pin),
                    blobs: metas,
                    created_at: Some(m.t),
                },
                now_ms,
            )?;

            // 显式关联写成 kind='rel'。`upsert_memo` 只重写 kind='inline'，
            // 所以之后再编辑正文不会把它们冲掉
            for dst in &m.rel {
                if let Some(dst) = map.get(dst) {
                    self.conn().execute(
                        "INSERT OR IGNORE INTO memo_ref(src, dst, kind) VALUES(?1,?2,'rel')",
                        rusqlite::params![id, dst],
                    )?;
                }
            }

            if trashed {
                self.soft_delete(id, m.del.unwrap_or(now_ms))?;
                report.trashed += 1;
            } else {
                report.memos += 1;
            }
        }

        Ok(report)
    }
}

/// 把正文里的 `[[摘要^数字]]` 换成 `[[摘要^uuid]]`。
///
/// 查不到的数字**原样留着**：那多半是引用了一条早就删掉的记录，
/// 换成一个假 UUID 只会把「引用失效」伪装成「引用有效但打不开」。
fn rewrite_refs(text: &str, map: &HashMap<i64, Uuid>) -> (String, usize) {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(text.len());
    let mut count = 0usize;
    let mut i = 0usize;

    while i < n {
        if i + 1 < n && chars[i] == '[' && chars[i + 1] == '[' {
            let mut j = i + 2;
            while j < n && !matches!(chars[j], ']' | '^' | '\n') {
                j += 1;
            }
            if j < n && chars[j] == '^' {
                let id_start = j + 1;
                let mut k = id_start;
                while k < n && chars[k].is_ascii_digit() {
                    k += 1;
                }
                if k > id_start && k + 1 < n && chars[k] == ']' && chars[k + 1] == ']' {
                    let old: String = chars[id_start..k].iter().collect();
                    if let Some(uuid) = old.parse::<i64>().ok().and_then(|v| map.get(&v)) {
                        out.push_str("[[");
                        out.extend(&chars[i + 2..j]);
                        out.push('^');
                        out.push_str(&uuid.to_string());
                        out.push_str("]]");
                        count += 1;
                        i = k + 2;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    (out, count)
}

/// `data:image/jpeg;base64,…` → `(mime, 字节)`。
///
/// 只认 base64 的图片。原型只会写出这一种，其余形态出现在这里就说明文件
/// 被人改过，不该照单全收。
fn decode_data_url(url: &str) -> Option<(String, Vec<u8>)> {
    use base64::Engine;

    let rest = url.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let mime = meta.strip_suffix(";base64")?;
    if !matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif"
    ) {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .ok()?;
    Some((mime.to_string(), bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Filter, View};
    use zhiyan_crypto::KEY_LEN;

    const NOW: i64 = 1_786_000_000_000;

    fn dek() -> DataKey {
        DataKey::from_bytes([0x7Au8; KEY_LEN])
    }

    fn fixture() -> (Store, BlobStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (crate::tests::store(), BlobStore::new(dir.path()), dir)
    }

    /// 一张 1×1 的 PNG。
    const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

    #[test]
    fn 整包导入() {
        let (mut s, bs, _d) = fixture();
        let json = r#"{
          "memos": [
            {"id": 4, "t": 1754380320000, "src": "quick", "pin": true,
             "x": "卡片笔记法 #读书/写作", "rel": [12]},
            {"id": 12, "t": 1754380000000, "x": "被引的那条"}
          ],
          "trash": [
            {"id": 99, "t": 1754300000000, "x": "删掉的", "del": 1754390000000}
          ]
        }"#;

        let r = s.import_legacy(&bs, &dek(), json, NOW).unwrap();
        assert_eq!(r.memos, 2);
        assert_eq!(r.trashed, 1);

        let live = s.list_memos(&Filter::default(), None, 10).unwrap();
        assert_eq!(live.len(), 2);
        assert!(live[0].pinned, "置顶要跟过来");
        assert_eq!(live[0].source, Source::Quick);
        assert_eq!(live[0].tags, vec!["读书/写作"], "标签要重新解析");

        let trash = s
            .list_memos(
                &Filter {
                    view: View::Trash,
                    ..Default::default()
                },
                None,
                10,
            )
            .unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].deleted_at, Some(1754390000000));
    }

    #[test]
    fn 主键换成了_uuid_而不是原来的整数() {
        // 自增整数会把「这是第几条」直接告诉服务端，多设备下还必然撞号
        let (mut s, bs, _d) = fixture();
        s.import_legacy(&bs, &dek(), r#"{"memos":[{"id":4,"t":1,"x":"a"}]}"#, NOW)
            .unwrap();
        let m = &s.list_memos(&Filter::default(), None, 10).unwrap()[0];
        assert_eq!(m.id.get_version_num(), 4, "必须是 UUIDv4");
        assert_ne!(m.id, Uuid::nil());
    }

    #[test]
    fn 正文里的行内引用跟着改写() {
        // 只换主键不改正文的话，所有引用会静默指空
        let (mut s, bs, _d) = fixture();
        let json = r#"{"memos":[
          {"id":4,"t":2,"x":"见 [[那天^12]] 说的"},
          {"id":12,"t":1,"x":"被引的"}
        ]}"#;
        let r = s.import_legacy(&bs, &dek(), json, NOW).unwrap();
        assert_eq!(r.refs_rewritten, 1);

        let all = s.list_memos(&Filter::default(), None, 10).unwrap();
        let citing = all.iter().find(|m| m.text.starts_with("见")).unwrap();
        let cited = all.iter().find(|m| m.text == "被引的").unwrap();

        assert_eq!(citing.refs, vec![cited.id], "引用必须落在真实记录上");
        assert!(!citing.text.contains("^12]]"), "正文里不该再有旧的数字 id");
        assert_eq!(s.backlinks(cited.id, 10).unwrap().len(), 1);
    }

    #[test]
    fn 指向已不存在记录的引用原样留着() {
        // 换成一个假 UUID 会把「引用失效」伪装成「引用有效但打不开」
        let (mut s, bs, _d) = fixture();
        let json = r#"{"memos":[{"id":4,"t":1,"x":"见 [[早没了^777]]"}]}"#;
        let r = s.import_legacy(&bs, &dek(), json, NOW).unwrap();
        assert_eq!(r.refs_rewritten, 0);
        assert!(s.list_memos(&Filter::default(), None, 10).unwrap()[0]
            .text
            .contains("^777]]"));
    }

    #[test]
    fn 显式关联落成_rel_不会被后续编辑冲掉() {
        let (mut s, bs, _d) = fixture();
        let json = r#"{"memos":[
          {"id":4,"t":2,"x":"甲","rel":[12]},
          {"id":12,"t":1,"x":"乙"}
        ]}"#;
        s.import_legacy(&bs, &dek(), json, NOW).unwrap();

        let n = |s: &Store| -> i64 {
            s.conn()
                .query_row("SELECT count(*) FROM memo_ref WHERE kind='rel'", [], |r| {
                    r.get(0)
                })
                .unwrap()
        };
        assert_eq!(n(&s), 1);

        // 编辑正文只重写 kind='inline'
        let id = s
            .list_memos(&Filter::default(), None, 10)
            .unwrap()
            .iter()
            .find(|m| m.text == "甲")
            .unwrap()
            .id;
        s.upsert_memo(
            &MemoInput {
                id: Some(id),
                text: "甲改过了".into(),
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
        assert_eq!(n(&s), 1, "显式关联不该被正文编辑冲掉");
    }

    #[test]
    fn 附件从_base64_落成内容寻址() {
        // 原型把 2MB 的图存成 2.7MB 文本塞进 JSON，每次读写整条搬运
        let (mut s, bs, _d) = fixture();
        let json = format!(r#"{{"memos":[{{"id":1,"t":1,"x":"配图","imgs":["{PNG}"]}}]}}"#);
        let r = s.import_legacy(&bs, &dek(), &json, NOW).unwrap();
        assert_eq!((r.blobs, r.blobs_failed), (1, 0));

        let m = &s.list_memos(&Filter::default(), None, 10).unwrap()[0];
        assert_eq!(m.blobs.len(), 1);
        assert_eq!(m.blobs[0].mime, "image/png");
        assert!(!m.text.contains("base64"), "正文里不该再有 base64");

        let hash =
            <[u8; 32]>::try_from(hex::decode(&m.blobs[0].sha256).unwrap().as_slice()).unwrap();
        assert!(bs.get(&dek(), &hash).unwrap().starts_with(b"\x89PNG"));
    }

    #[test]
    fn 坏掉的附件不连累整条笔记() {
        let (mut s, bs, _d) = fixture();
        let json = r#"{"memos":[{"id":1,"t":1,"x":"正文还在",
          "imgs":["data:image/png;base64,!!!不是base64",
                  "data:text/html;base64,PHNjcmlwdD4=",
                  "彻底不是 data url"]}]}"#;
        let r = s.import_legacy(&bs, &dek(), json, NOW).unwrap();
        assert_eq!((r.blobs, r.blobs_failed, r.memos), (0, 3, 1));
        assert_eq!(
            s.list_memos(&Filter::default(), None, 10).unwrap()[0].text,
            "正文还在"
        );
    }

    #[test]
    fn 只认图片类型的_data_url() {
        // 一个 text/html 的 data URL 走 zhiyan:// 出去就是一个 XSS
        assert!(decode_data_url("data:text/html;base64,PHNjcmlwdD4=").is_none());
        assert!(decode_data_url("data:image/svg+xml;base64,PHN2Zz4=").is_none());
        assert!(decode_data_url("data:image/png,notbase64").is_none());
        assert!(decode_data_url(PNG).is_some());
    }

    #[test]
    fn 空文件与缺字段都不崩() {
        let (mut s, bs, _d) = fixture();
        assert_eq!(
            s.import_legacy(&bs, &dek(), "{}", NOW).unwrap(),
            LegacyReport::default()
        );
        s.import_legacy(&bs, &dek(), r#"{"memos":[{"id":1,"t":1}]}"#, NOW)
            .unwrap();
    }

    #[test]
    fn 不是_json_时报得明白而不是_panic() {
        let (mut s, bs, _d) = fixture();
        let e = s
            .import_legacy(&bs, &dek(), "这不是 JSON", NOW)
            .unwrap_err();
        assert!(format!("{e}").contains("解析导出文件"), "{e}");
    }
}
