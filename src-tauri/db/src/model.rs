//! 跨 IPC 边界的数据形状。
//!
//! 全部 `camelCase` 序列化——前端 `store.js` / `mock.js` 已按 §7.2 的规范形态写好，
//! 这边对齐它，而不是让前端去适应 Rust 的命名习惯。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 记录的来源（§9.5 的 `source` 列）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    #[default]
    App,
    /// 速记浮窗（§5.3）。
    Quick,
    Wechat,
    Tg,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::App => "app",
            Source::Quick => "quick",
            Source::Wechat => "wechat",
            Source::Tg => "tg",
        }
    }

    /// 认不出的来源退回 `app`。
    ///
    /// 库里的值可能来自更新的版本（多设备同步），把整条记录读失败比认错来源糟得多。
    pub fn parse(s: &str) -> Self {
        match s {
            "quick" => Source::Quick,
            "wechat" => Source::Wechat,
            "tg" => Source::Tg,
            _ => Source::App,
        }
    }
}

/// 附件元信息（§7.2 的 `blobs[]`）。
///
/// `sha256` 是**明文**哈希的十六进制。正文里只留它，字节在 `blobs\` 目录下（§7.3 ①）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobMeta {
    /// `hex(SHA256(明文))`。
    pub sha256: String,
    /// `hex(SHA256(密文))`，服务端对象键。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cipher_id: Option<String>,
    pub mime: String,
    pub size: i64,
    /// 宽高由调用方给（前端从 `<img>` 上读得到）。
    ///
    /// 这里不引图片解码库：为了两个整数把 `image` 及其一串编解码器拖进依赖树，
    /// 换来的是安装包体积和一批解析器攻击面。
    pub w: Option<i64>,
    pub h: Option<i64>,
}

/// 一条片语的完整形态（§7.2）。
///
/// `Debug` 是**手写的**：这个类型会穿过 IPC 边界，也会被顺手 `dbg!`，
/// 而 `text` 是用户正文（§14 ①）。
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Memo {
    pub id: Uuid,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    pub rev: i64,
    pub text: String,
    pub pinned: bool,
    pub source: Source,
    /// outbox 标记：本地改过、还没推上去。
    pub dirty: bool,
    /// 从正文解析出来的，不由前端传入。
    pub tags: Vec<String>,
    /// 行内引用 `[[摘要^uuid]]` 里的 uuid。
    pub refs: Vec<Uuid>,
    pub blobs: Vec<BlobMeta>,
    /// 已被彻底删除，只剩墓碑（§7.3 ②）。正文为空。
    #[serde(default)]
    pub tombstone: bool,
}

impl core::fmt::Debug for Memo {
    /// **只打 id 与长度**。正文、标签名都不出现——它们会落进 WebView 的日志。
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Memo")
            .field("id", &self.id)
            .field("chars", &self.text.chars().count())
            .field("tags", &self.tags.len())
            .field("refs", &self.refs.len())
            .field("blobs", &self.blobs.len())
            .field("rev", &self.rev)
            .field("deleted", &self.deleted_at.is_some())
            .field("tombstone", &self.tombstone)
            .finish()
    }
}

/// 写入用的入参（§9.6 的 `MemoInput`）。
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoInput {
    /// `None` 表示新建。
    #[serde(default)]
    pub id: Option<Uuid>,
    pub text: String,
    #[serde(default)]
    pub source: Option<Source>,
    #[serde(default)]
    pub pinned: Option<bool>,
    /// 附件。**标签与引用不在这里**——那两样由 Rust 从正文解析（§9.6）。
    #[serde(default)]
    pub blobs: Vec<BlobMeta>,
    /// 补录历史记录时指定创建时间（导入、迁移用）。日常写入留空。
    #[serde(default)]
    pub created_at: Option<i64>,
}

impl core::fmt::Debug for MemoInput {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MemoInput")
            .field("id", &self.id)
            .field("chars", &self.text.chars().count())
            .field("blobs", &self.blobs.len())
            .finish()
    }
}

/// 列表视图。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum View {
    #[default]
    All,
    /// 没有任何标签的。
    Untagged,
    /// 回收站。**不含墓碑**——墓碑对用户不可见。
    Trash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    #[default]
    New,
    Old,
}

/// 列表筛选条件。
///
/// ## 为什么没有 `today` / `day` 这两个视图
///
/// 前端有，这一层没有：它们都要「本地时区的今天是哪一段毫秒」，而时区是个
/// **运行环境状态**。把它埋进这一层，等于让每个查询都隐式依赖系统时钟与
/// 时区设置，测试要么跟着漂，要么得去改进程的 TZ。
///
/// 所以这里只收 [`Filter::range`]——一段左闭右开的毫秒区间，由 IPC 层按本地
/// 时区算好传进来。`today` 与 `day` 都是它的特例。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Filter {
    pub view: View,
    /// 标签前缀。`读书` 同时命中 `读书` 与 `读书/认知`（§2.2 的两级标签）。
    pub tag: Option<String>,
    /// 关键词。**走 `LIKE` 而非 FTS**——列表里的即时筛选要能匹配一两个字，
    /// 而 trigram 索引对短于 3 字符的串无能为力。全文检索走 [`crate::Store::search`]。
    pub query: Option<String>,
    pub sort: Sort,
    /// `created_at ∈ [start, end)`，毫秒。
    pub range: Option<(i64, i64)>,
}

/// 热力图的一格。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatCell {
    /// `YYYY-M-D`，与前端 `util.js` 的 `dayKey` 同格式（**不补零**）。
    pub day: String,
    /// 当地零点的毫秒时间戳。
    pub at: i64,
    pub count: i64,
    /// 档位 0–4。阈值固定而非按当期最大值归一，否则低产期看起来和高产期一样满。
    pub level: u8,
}

/// 侧栏四个视图的计数。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub all: i64,
    pub today: i64,
    pub untagged: i64,
    pub trash: i64,
}

/// 统计面板要的全部数字（§9.6：在 Rust 侧用 SQL 聚合算完再返回）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub total: i64,
    pub week: i64,
    /// 当前连续天数。
    pub streak: i64,
    /// 历史最长连续天数。
    pub longest: i64,
    pub chars: i64,
    pub per_day_avg: f64,
    pub heat: Vec<HeatCell>,
    /// 标签 → 条数。
    pub tags: std::collections::BTreeMap<String, i64>,
    pub counts: Counts,
}

/// 统计的时间基准。
///
/// 显式传进来而不是在函数里读时钟：读时钟的话「跨年那天的热力图」这种
/// 边界情形就没法写测试了。
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    /// 现在的毫秒时间戳。
    pub now_ms: i64,
    /// 本地时区相对 UTC 的偏移，秒。东八区是 `8 * 3600`。
    pub utc_offset_secs: i32,
}

/// outbox 里的操作类型（§10.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Upsert,
    Delete,
    Purge,
}

impl Op {
    pub fn as_str(self) -> &'static str {
        match self {
            Op::Upsert => "upsert",
            Op::Delete => "delete",
            Op::Purge => "purge",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memo_的_debug_不含正文() {
        let m = Memo {
            id: Uuid::nil(),
            created_at: 0,
            updated_at: 0,
            deleted_at: None,
            rev: 1,
            text: "MAGIC_PLAINTEXT_MARKER 与 #秘密标签".into(),
            pinned: false,
            source: Source::App,
            dirty: true,
            tags: vec!["秘密标签".into()],
            refs: vec![],
            blobs: vec![],
            tombstone: false,
        };
        let s = format!("{m:?}");
        assert!(!s.contains("MAGIC_PLAINTEXT_MARKER"), "{s}");
        assert!(!s.contains("秘密标签"), "{s}");
        assert!(s.contains("chars"), "{s}");
    }

    #[test]
    fn 序列化用_camel_case() {
        let m = Memo {
            id: Uuid::nil(),
            created_at: 1,
            updated_at: 2,
            deleted_at: None,
            rev: 1,
            text: "x".into(),
            pinned: false,
            source: Source::Quick,
            dirty: true,
            tags: vec![],
            refs: vec![],
            blobs: vec![],
            tombstone: false,
        };
        let v = serde_json::to_value(&m).unwrap();
        assert!(v.get("createdAt").is_some(), "{v}");
        assert!(v.get("created_at").is_none());
        assert_eq!(v["source"], "quick");
    }

    #[test]
    fn 认不出的来源退回_app() {
        assert_eq!(Source::parse("从未来来的"), Source::App);
        assert_eq!(Source::parse("quick"), Source::Quick);
    }
}
