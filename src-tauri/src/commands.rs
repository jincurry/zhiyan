//! IPC 命令层（§9.6）。
//!
//! 这一层**只做三件事**：把前端的形状翻成存储层的形状、补上时间与时区、
//! 把错误翻成前端能分支的错误码。任何业务判断都不该出现在这里——
//! 它一旦开始判断，同一件事就会在 Rust 与 JS 两侧各有一个版本。
//!
//! ## 前端的筛选形状与存储层不一样，翻译在这里
//!
//! 前端有 `today` 与 `day` 两个视图，存储层没有：它们都要「本地时区的今天是
//! 哪一段毫秒」，而时区是运行环境状态，埋进存储层会让每个查询都隐式依赖系统
//! 时钟与 TZ 设置。所以偏移量由前端随调用传进来，在这里换算成毫秒区间。
//!
//! ## 时间戳也由这一层给
//!
//! `upsert_memo` 之类的写操作要 `now_ms`。存储层不读时钟（那样测试没法稳，
//! 也没法补录历史记录），由这里传 `SystemTime::now()`。

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;
use zhiyan_db::{
    BlobMeta, Clock, Filter, LegacyReport, Memo, MemoInput, Sort, Source, Stats, View,
};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

const DAY_MS: i64 = 86_400_000;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 前端传来的筛选条件。
///
/// 与 `state.js` 的 `filter()` 一一对应，多一个 `tzOffset`。
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IpcFilter {
    /// `all` | `today` | `untagged` | `trash` | `day`
    pub view: String,
    /// 标签；`view == "day"` 时它是日期键 `YYYY-M-D`。
    pub tag: Option<String>,
    pub query: Option<String>,
    /// `new` | `old`
    pub sort: Option<String>,
    /// 本地时区偏移，**分钟**。前端传 `-new Date().getTimezoneOffset()`。
    ///
    /// 缺省按 UTC。缺省值算出来的「今天」会和用户看到的差几个小时，
    /// 但那好过让这一层去猜一个可能和 WebView 不一致的时区。
    pub tz_offset: Option<i32>,
    /// 任意时间区间 `[from, to)`，毫秒。每周回顾这类「取某一段」的场景用。
    ///
    /// 有了它，前端就不必为了取一周的数据而去拉两千条再自己筛——
    /// 那种写法在 §9.6 的分页约束下迟早会被上限悄悄截断。
    pub from: Option<i64>,
    pub to: Option<i64>,
}

impl IpcFilter {
    fn utc_offset_secs(&self) -> i32 {
        self.tz_offset.unwrap_or(0) * 60
    }

    /// 本地某一天的 `[起, 止)`，毫秒。
    fn day_range(&self, day_number: i64) -> (i64, i64) {
        let off = self.utc_offset_secs() as i64 * 1000;
        let start = day_number * DAY_MS - off;
        (start, start + DAY_MS)
    }

    fn today_number(&self) -> i64 {
        (now_ms() + self.utc_offset_secs() as i64 * 1000).div_euclid(DAY_MS)
    }

    /// 解析前端的 `YYYY-M-D`（`util.js` 的 `dayKey`，**不补零**）。
    fn parse_day_key(key: &str) -> Option<i64> {
        let mut it = key.split('-');
        let y: i64 = it.next()?.parse().ok()?;
        let m: i64 = it.next()?.parse().ok()?;
        let d: i64 = it.next()?.parse().ok()?;
        if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        // days_from_civil（Howard Hinnant）
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = if m > 2 { m - 3 } else { m + 9 };
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        Some(era * 146_097 + doe - 719_468)
    }

    fn into_filter(self) -> AppResult<Filter> {
        let sort = match self.sort.as_deref() {
            Some("old") => Sort::Old,
            _ => Sort::New,
        };

        let (view, tag, range) = match self.view.as_str() {
            "trash" => (View::Trash, None, None),
            "untagged" => (View::Untagged, self.tag.clone(), None),
            "today" => (
                View::All,
                self.tag.clone(),
                Some(self.day_range(self.today_number())),
            ),
            "day" => {
                let key = self.tag.as_deref().ok_or(AppError::BadArgument("tag"))?;
                let day = Self::parse_day_key(key).ok_or(AppError::BadArgument("tag"))?;
                (View::All, None, Some(self.day_range(day)))
            }
            // 显式区间优先于视图：调用方已经说清楚要哪一段了
            _ if self.from.is_some() || self.to.is_some() => (
                View::All,
                self.tag.clone(),
                Some((self.from.unwrap_or(i64::MIN), self.to.unwrap_or(i64::MAX))),
            ),
            // 认不出的视图当「全部」。前端加了新视图而后端还没跟上时，
            // 用户看到的是全部记录，而不是一个空列表加一条报错
            _ => (View::All, self.tag.clone(), None),
        };

        Ok(Filter {
            view,
            tag: tag.filter(|t| !t.is_empty()),
            query: self.query.filter(|q| !q.is_empty()),
            sort,
            range,
        })
    }
}

/// 前端传来的写入参数。
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IpcMemoInput {
    pub id: Option<Uuid>,
    pub text: String,
    pub source: Option<Source>,
    pub pinned: Option<bool>,
    pub blobs: Vec<BlobMeta>,
}

// ── 片语（§9.6）─────────────────────────────────────────────────

#[tauri::command]
pub fn list_memos(
    state: State<'_, AppState>,
    filter: IpcFilter,
    cursor: Option<Uuid>,
    limit: u32,
) -> AppResult<Vec<Memo>> {
    // 上限钉死：前端传个 100000 进来就等于把整库经 JSON 搬一遍，
    // 而 §9.6 要求分页正是为了避免这个
    let limit = limit.clamp(1, 500);
    let f = filter.into_filter()?;
    state.read(|s| Ok(s.list_memos(&f, cursor, limit)?))
}

#[tauri::command]
pub fn get_memo(state: State<'_, AppState>, id: Uuid) -> AppResult<Option<Memo>> {
    state.read(|s| Ok(s.get_memo(id)?))
}

/// 新建或更新。**标签与引用由 Rust 从正文解析**（§9.6），前端不预先算。
#[tauri::command]
pub fn upsert_memo(state: State<'_, AppState>, input: IpcMemoInput) -> AppResult<Memo> {
    let now = now_ms();
    let input = MemoInput {
        id: input.id,
        text: input.text,
        source: input.source,
        pinned: input.pinned,
        blobs: input.blobs,
        created_at: None,
    };
    state.write(|s| Ok(s.upsert_memo(&input, now)?))
}

#[tauri::command]
pub fn soft_delete(state: State<'_, AppState>, id: Uuid) -> AppResult<()> {
    state.read(|s| Ok(s.soft_delete(id, now_ms())?))
}

#[tauri::command]
pub fn restore(state: State<'_, AppState>, id: Uuid) -> AppResult<()> {
    state.read(|s| Ok(s.restore(id, now_ms())?))
}

/// 彻底删除 = **写墓碑，不是物理删**（§7.3 ②）。
#[tauri::command]
pub fn purge(state: State<'_, AppState>, id: Uuid) -> AppResult<()> {
    state.write(|s| Ok(s.purge(id, now_ms())?))
}

/// 清空回收站。**不在 §9.6 的清单里**，理由见 `store.js` 上的注释：
/// 拆成 N 次 `purge` 会变成 N 次 IPC 往返，且会出现删到一半的中间态。
#[tauri::command]
pub fn purge_all(state: State<'_, AppState>) -> AppResult<usize> {
    state.write(|s| Ok(s.purge_all(now_ms())?))
}

/// 置顶开关。**不在 §9.6 的清单里**：走 `upsert_memo` 要把整条正文回传一遍，
/// 为翻一个布尔值搬运整篇文字在 IPC 边界上是明显的浪费。
#[tauri::command]
pub fn set_pinned(state: State<'_, AppState>, id: Uuid, pinned: bool) -> AppResult<()> {
    state.read(|s| Ok(s.set_pinned(id, pinned, now_ms())?))
}

#[tauri::command]
pub fn search(state: State<'_, AppState>, q: String, limit: u32) -> AppResult<Vec<Memo>> {
    let limit = limit.clamp(1, 500);
    state.read(|s| Ok(s.search(&q, limit)?))
}

/// 反向链接：哪些片语引用了这一条。
#[tauri::command]
pub fn backlinks(state: State<'_, AppState>, id: Uuid, limit: u32) -> AppResult<Vec<Memo>> {
    let limit = limit.clamp(1, 200);
    state.read(|s| Ok(s.backlinks(id, limit)?))
}

// ── 聚合（§9.6）─────────────────────────────────────────────────

/// 统计、热力图、标签分布、视图计数，一次给全。
///
/// 合成一个命令而不是拆四个：它们每次都一起刷新，拆开就是四次 IPC 往返
/// 换同一份数据。
///
/// `range` 是 §9.6 签名里的参数，目前热力图窗口固定为半年（与原型一致），
/// 先收着不用——改签名要动前端，等真需要按季/按年切换时一起改。
#[tauri::command]
pub fn stats(
    state: State<'_, AppState>,
    #[allow(unused_variables)] range: Option<String>,
    tz_offset: Option<i32>,
) -> AppResult<Stats> {
    let clock = Clock {
        now_ms: now_ms(),
        utc_offset_secs: tz_offset.unwrap_or(0) * 60,
    };
    state.read(|s| Ok(s.stats(clock)?))
}

// ── 附件（§9.6）─────────────────────────────────────────────────

/// 存一张图，返回内容寻址的元信息。正文里只留 hash。
///
/// 宽高由前端给——它从 `<img>` 上读得到。Rust 侧为了两个整数把 `image` 及其
/// 一串编解码器拖进依赖树，换来的是安装包体积和一批解析器攻击面。
#[tauri::command]
pub fn put_blob(
    state: State<'_, AppState>,
    bytes: Vec<u8>,
    mime: String,
    w: Option<i64>,
    h: Option<i64>,
) -> AppResult<BlobMeta> {
    // MIME 由前端给，会进 zhiyan:// 响应头。放任意串过去等于让前端决定
    // WebView 怎么解释这段字节
    if !matches!(
        mime.as_str(),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif"
    ) {
        return Err(AppError::BadArgument("mime"));
    }
    state.with_dek(|dek| state.read(|s| Ok(s.put_blob(state.blobs(), dek, &bytes, &mime, w, h)?)))
}

// ── 导出（§9.6 / §9.7）──────────────────────────────────────────

/// 导出。`fmt` 为 `markdown` / `json` / `zbk`。
///
/// 返回落盘路径；用户取消对话框时返回 `null`。
///
/// **路径由用户在系统对话框里选，不由前端传**（§12.1）。前端能传路径的话，
/// 一个 XSS 就能把正文写到任意位置。
#[tauri::command]
pub async fn export(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    fmt: String,
    tz_offset: Option<i32>,
) -> AppResult<Option<PathBuf>> {
    use tauri_plugin_dialog::DialogExt;

    let now = now_ms();
    let off = tz_offset.unwrap_or(0) * 60;

    // 先生成内容再弹对话框：**锁绝不能跨过 await**，否则用户盯着对话框发呆的
    // 十几秒里整个应用都在等这把锁
    let (bytes, ext, name) = match fmt.as_str() {
        "markdown" => (
            state.read(|s| Ok(s.export_markdown(off)?))?.into_bytes(),
            "md",
            "知言导出",
        ),
        "json" => (
            state.read(|s| Ok(s.export_json(now)?))?.into_bytes(),
            "json",
            "知言导出",
        ),
        "zbk" => (
            state.with_dek(|dek| state.read(|s| Ok(s.backup(dek, now)?)))?,
            "zbk",
            "知言备份",
        ),
        _ => return Err(AppError::BadArgument("fmt")),
    };

    let (tx, mut rx) = tauri::async_runtime::channel(1);
    app.dialog()
        .file()
        .set_file_name(format!("{name}.{ext}"))
        .add_filter(name, &[ext])
        .save_file(move |p| {
            let _ = tx.blocking_send(p);
        });

    let Some(path) = rx.recv().await.flatten() else {
        return Ok(None); // 用户取消了
    };
    let path = path
        .into_path()
        .map_err(|e| AppError::Vault(format!("解析保存路径：{e}")))?;

    std::fs::write(&path, &bytes)
        .map_err(|e| AppError::Db(zhiyan_db::DbError::Io(e.to_string())))?;

    // 明文导出必须留痕。用户以为「导出」总是加密的，那是这一步最危险的误解
    if ext != "zbk" {
        tracing::warn!("已明文导出 {} 字节（{}）", bytes.len(), ext);
    }
    Ok(Some(path))
}

// ── kv（§9.6）───────────────────────────────────────────────────

/// **不放敏感内容**：窗口位置、同步游标、UI 偏好可以；
/// 最近搜索词、标签列表不行（§9.1）。
#[tauri::command]
pub fn get_kv(state: State<'_, AppState>, k: String) -> AppResult<Option<String>> {
    state.read(|s| Ok(s.get_kv(&k)?))
}

#[tauri::command]
pub fn set_kv(state: State<'_, AppState>, k: String, v: String) -> AppResult<()> {
    state.read(|s| Ok(s.set_kv(&k, &v)?))
}

// ── 维护 ────────────────────────────────────────────────────────

/// 墓碑与孤儿附件的回收。启动时跑一次（§7.3 ②：墓碑保留 90 天）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GcReport {
    pub tombstones: usize,
    pub blobs: usize,
}

#[tauri::command]
pub fn run_gc(state: State<'_, AppState>) -> AppResult<GcReport> {
    let now = now_ms();
    state.read(|s| {
        Ok(GcReport {
            tombstones: s.gc_tombstones(now, 90)?,
            blobs: s.gc_blobs(state.blobs())?,
        })
    })
}

/// 从原型导出的整包 JSON 迁移（§7.3）。
///
/// **只允许在空库上跑。** 这个操作不幂等——重跑一次会得到另一批 UUID，
/// 也就是把每条笔记复制一份。把这条判断放在这里而不是存储层，是因为
/// 「空库才能导入」是一条产品规则，不是数据规则：将来支持「合并导入」时，
/// 改的应该是这里。
#[tauri::command]
pub fn import_legacy(state: State<'_, AppState>, json: String) -> AppResult<LegacyReport> {
    let now = now_ms();

    let existing = state.read(|s| Ok(s.list_memos(&Filter::default(), None, 1)?.len()))?;
    if existing > 0 {
        return Err(AppError::BadArgument("json"));
    }

    state.with_dek(|dek| state.write(|s| Ok(s.import_legacy(state.blobs(), dek, &json, now)?)))
}

// ── 运行时自检（§4.2）───────────────────────────────────────────

/// 目标最低 Chromium 大版本（§4.2）。
///
/// 原型用到的 `color-mix()`、`backdrop-filter`、`::-webkit-scrollbar`、
/// `aspect-ratio`、`:has()`、正则后行断言都要这个版本以上。
/// 低于它不是「样式差一点」，是整块整块的界面塌掉。
pub const CHROMIUM_MIN: u32 = 111;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeReport {
    /// 注册表里读到的 WebView2 Evergreen 运行时版本。
    ///
    /// `None` 有两种可能：真没装，或者装的是 Fixed Version（不写这个键）。
    /// 所以**它为 None 时不能直接说「没装 WebView2」**——应用都跑起来了，
    /// 显然是装了的。真正判「没装」是安装器的活。
    pub webview2: Option<String>,
    /// 上面那个版本的大版本号，与 Chromium 大版本一致。
    pub webview2_major: Option<u32>,
    pub chromium_min: u32,
    /// 版本够不够。读不到版本时按「够」处理——宁可漏报也不误报，
    /// 一个假的「请升级 Edge」会让用户去做一件没用的事。
    pub chromium_ok: bool,
}

#[tauri::command]
pub fn runtime_report() -> RuntimeReport {
    #[cfg(windows)]
    let version = crate::platform_win::webview2_version();
    #[cfg(not(windows))]
    let version: Option<String> = None;

    let major = version
        .as_deref()
        .and_then(|v| v.split('.').next())
        .and_then(|v| v.parse::<u32>().ok());

    RuntimeReport {
        webview2: version,
        webview2_major: major,
        chromium_min: CHROMIUM_MIN,
        chromium_ok: major.map(|m| m >= CHROMIUM_MIN).unwrap_or(true),
    }
}

// ── 锁定与设备（§9.4）───────────────────────────────────────────

/// 立刻锁定：清掉内存里的密钥并关掉库。
///
/// 对应「空闲 N 分钟后锁定」这个设置项。空闲计时在前端（它才知道用户有没有
/// 在动键盘），到点了调这条。
#[tauri::command]
pub fn lock_store(state: State<'_, AppState>) -> AppResult<()> {
    state.lock_now();
    Ok(())
}

/// 解锁。目前就是把密钥从凭据管理器里再取一次。
///
/// 接上服务端之后，这里会变成「用密码或恢复码解开 `protected_dek`」（§8.2）。
#[tauri::command]
pub fn unlock_store(state: State<'_, AppState>) -> AppResult<()> {
    state.unlock()
}

/// 本机停用：删掉凭据管理器里的密钥。
///
/// **它删不掉已经落在本地的数据**——库还在，只是再也打不开。界面上必须
/// 说明白：远程吊销能让服务端拒绝这台设备，但擦不掉本地已有的东西（§9.4）。
#[tauri::command]
pub fn forget_device(state: State<'_, AppState>) -> AppResult<()> {
    state.lock_now();
    crate::vault::forget_dek()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(view: &str, tag: Option<&str>) -> IpcFilter {
        IpcFilter {
            view: view.into(),
            tag: tag.map(str::to_string),
            tz_offset: Some(8 * 60), // 东八区
            ..Default::default()
        }
    }

    #[test]
    fn 运行时自检读不到版本时不误报() {
        // 假的「请升级 Edge」会让用户去做一件没用的事
        let r = runtime_report();
        assert_eq!(r.chromium_min, CHROMIUM_MIN);
        if r.webview2_major.is_none() {
            assert!(r.chromium_ok, "读不到版本时必须按「够」处理");
        }
    }

    #[test]
    fn 日期键解析与前端的_daykey_对得上() {
        // util.js 的 dayKey 不补零，两种写法都得认
        assert_eq!(IpcFilter::parse_day_key("1970-1-1"), Some(0));
        assert_eq!(IpcFilter::parse_day_key("1970-01-01"), Some(0));
        assert_eq!(
            IpcFilter::parse_day_key("2024-2-29"),
            IpcFilter::parse_day_key("2024-02-29")
        );
        assert!(IpcFilter::parse_day_key("2024-2-29").unwrap() > 19_000);
    }

    #[test]
    fn 坏的日期键被拒绝而不是当成第零天() {
        for bad in [
            "",
            "2024",
            "2024-13-1",
            "2024-1-32",
            "abc-1-1",
            "2024-1-1-1",
        ] {
            assert!(IpcFilter::parse_day_key(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn day_视图缺了日期键报参数错() {
        assert!(matches!(
            f("day", None).into_filter(),
            Err(AppError::BadArgument("tag"))
        ));
        assert!(matches!(
            f("day", Some("不是日期")).into_filter(),
            Err(AppError::BadArgument("tag"))
        ));
    }

    #[test]
    fn day_视图算出的区间正好是本地一天() {
        let filter = f("day", Some("2026-8-12")).into_filter().unwrap();
        let (start, end) = filter.range.unwrap();
        assert_eq!(end - start, DAY_MS);
        // 东八区的零点 = UTC 前一天 16:00
        assert_eq!(start.rem_euclid(DAY_MS), 16 * 3600 * 1000);
    }

    #[test]
    fn today_视图落在今天这一格里() {
        let filter = f("today", None).into_filter().unwrap();
        let (start, end) = filter.range.unwrap();
        let now = now_ms();
        assert!(start <= now && now < end, "现在必须落在今天这一段里");
    }

    #[test]
    fn 显式区间被原样带下去() {
        // 每周回顾靠它取某一周，不必拉两千条回来自己筛
        let raw = IpcFilter {
            view: "all".into(),
            from: Some(1000),
            to: Some(2000),
            ..Default::default()
        };
        assert_eq!(raw.into_filter().unwrap().range, Some((1000, 2000)));
    }

    #[test]
    fn 只给一端时另一端敞开() {
        let raw = IpcFilter {
            view: "all".into(),
            from: Some(1000),
            ..Default::default()
        };
        assert_eq!(raw.into_filter().unwrap().range, Some((1000, i64::MAX)));
    }

    #[test]
    fn 认不出的视图退回全部而不是空列表() {
        // 前端加了新视图而后端还没跟上时，看到全部记录好过看到一个空列表加报错
        let filter = f("从未来来的视图", None).into_filter().unwrap();
        assert_eq!(filter.view, View::All);
        assert!(filter.range.is_none());
    }

    #[test]
    fn 回收站视图不带标签筛选() {
        let filter = f("trash", Some("读书")).into_filter().unwrap();
        assert_eq!(filter.view, View::Trash);
        assert!(filter.tag.is_none(), "回收站里按标签筛没有意义");
    }

    #[test]
    fn 空字符串的标签与关键词当成没填() {
        let mut raw = f("all", Some(""));
        raw.query = Some(String::new());
        let filter = raw.into_filter().unwrap();
        assert!(filter.tag.is_none() && filter.query.is_none());
    }

    #[test]
    fn 缺时区时按_utc_而不是猜一个() {
        let raw = IpcFilter {
            view: "today".into(),
            ..Default::default()
        };
        let (start, _) = raw.into_filter().unwrap().range.unwrap();
        assert_eq!(start.rem_euclid(DAY_MS), 0);
    }

    #[test]
    fn 前端传来的筛选按_camel_case_反序列化() {
        let raw: IpcFilter =
            serde_json::from_str(r#"{"view":"day","tag":"2026-8-12","tzOffset":480}"#).unwrap();
        assert_eq!(raw.tz_offset, Some(480));
        assert!(raw.into_filter().is_ok());
    }

    #[test]
    fn 缺字段的筛选也能反序列化() {
        // 前端 state.js 的 filter() 可能不带 sort
        let raw: IpcFilter = serde_json::from_str("{}").unwrap();
        assert_eq!(raw.into_filter().unwrap().sort, Sort::New);
    }
}

// ── 自动更新（§11.3）────────────────────────────────────────────

/// 检查更新的结果。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    /// 有没有可用的新版本。
    pub available: bool,
    pub version: Option<String>,
    /// 这个构建**有没有配置更新源**。
    ///
    /// 单独报出来而不是混进 `available: false`：两者对用户的含义完全不同。
    /// 「已是最新」与「这个构建根本不会更新」不能显示成同一句话——
    /// 后者意味着他要自己去看有没有新版。
    pub configured: bool,
}

/// 这个构建配了更新源吗。
///
/// 仓库里 `plugins.updater` 是**空的**（`endpoints: []`、`pubkey: ""`）。
/// 不能整段省掉——插件初始化时会因为读不到配置直接 panic，应用根本起不来；
/// 也不能塞一个占位公钥——那看起来像配好了，实际谁都验不过，
/// 而失败信息是「签名不匹配」，排查方向会完全跑偏。
///
/// 空的 `endpoints` 是明确的「没配」，空的 `pubkey` 验什么都失败（fail closed）。
fn updater_configured(app: &tauri::AppHandle) -> bool {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|v| v.get("endpoints"))
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false)
}

#[tauri::command]
pub async fn check_update(app: tauri::AppHandle) -> AppResult<UpdateCheck> {
    use tauri_plugin_updater::UpdaterExt;

    if !updater_configured(&app) {
        return Ok(UpdateCheck {
            available: false,
            version: None,
            configured: false,
        });
    }

    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            tracing::info!("更新器不可用：{e}");
            return Ok(UpdateCheck {
                available: false,
                version: None,
                configured: false,
            });
        }
    };

    match updater.check().await {
        Ok(Some(update)) => Ok(UpdateCheck {
            available: true,
            version: Some(update.version.clone()),
            configured: true,
        }),
        Ok(None) => Ok(UpdateCheck {
            available: false,
            version: None,
            configured: true,
        }),
        Err(e) => Err(AppError::Window(format!("检查更新失败：{e}"))),
    }
}
