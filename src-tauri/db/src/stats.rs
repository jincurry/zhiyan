//! 统计与热力图（§9.6：**在 Rust 侧算完再返回**）。
//!
//! 把原始数据搬到前端算的话，万级记录要整包过一次 JSON 序列化——这正是
//! IPC 边界最贵的地方。所以统计面板要的十来个数字全在这里出。

use std::collections::BTreeMap;

use crate::model::{Clock, Counts, HeatCell, Stats};
use crate::parse::count_chars;
use crate::{Result, Store};

const DAY_MS: i64 = 86_400_000;
/// 半年热力图的格数，与原型一致。
const HEAT_DAYS: i64 = 182;

/// 毫秒时间戳 → 本地日序号（1970-01-01 为 0）。
///
/// 用 `div_euclid` 而不是 `/`：1970 年之前的时间戳是负数，`/` 向零取整会把
/// 那一天算到后一天去。日常用不到，但导入历史笔记时会撞上。
fn day_number(ms: i64, utc_offset_secs: i32) -> i64 {
    (ms.div_euclid(1000) + utc_offset_secs as i64).div_euclid(86_400)
}

/// 日序号 → 当地零点的毫秒时间戳。
fn day_start_ms(day: i64, utc_offset_secs: i32) -> i64 {
    (day * 86_400 - utc_offset_secs as i64) * 1000
}

/// 日序号 → `(年, 月, 日)`。Howard Hinnant 的 `civil_from_days`。
///
/// 自己算而不是拉一个日期库：这里只需要这一个方向的转换，
/// 而 `chrono` / `time` 的本地时区支持在多线程下还有已知的 `getenv` 竞态。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]，3 月为 0
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `YYYY-M-D`，**不补零**——与前端 `util.js` 的 `dayKey` 同格式。
///
/// 补零看着更规整，但那样两边生成的键就对不上了，热力图会整片空掉。
fn day_key(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y}-{m}-{d}")
}

/// `YYYY-MM-DD HH:MM`，本地时区。Markdown 导出的小节标题用。
///
/// 这里**补零**——它是给人读的时间戳，不是要和前端 `dayKey` 对上的键。
pub(crate) fn stamp(ms: i64, utc_offset_secs: i32) -> String {
    let local = ms.div_euclid(1000) + utc_offset_secs as i64;
    let (y, m, d) = civil_from_days(local.div_euclid(86_400));
    let secs = local.rem_euclid(86_400);
    format!(
        "{y}-{m:02}-{d:02} {:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60
    )
}

/// 星期几，0 = 周日。1970-01-01 是周四。
fn weekday(day: i64) -> i64 {
    (day + 4).rem_euclid(7)
}

/// 热力档位。**阈值固定而非按当期最大值归一**——归一化会让低产的半年
/// 看起来和高产的半年一样满，那张图就不再说明任何事情了。
fn heat_level(n: i64) -> u8 {
    match n {
        0 => 0,
        1 => 1,
        2 => 2,
        3..=4 => 3,
        _ => 4,
    }
}

impl Store {
    /// 统计面板要的全部数字。
    ///
    /// # 为什么不用 SQL 的 `GROUP BY` 分桶
    ///
    /// `chars` 的口径是「中文按字、英文按词」，还要先剥掉 Markdown 标记——
    /// SQL 表达不了，无论如何都得把正文过一遍。既然已经在扫这一遍，
    /// 顺手按天分桶比再发一条 `GROUP BY` 便宜。
    ///
    /// 标签分布与回收站计数则确实走 SQL：它们不碰正文。
    pub fn stats(&self, clock: Clock) -> Result<Stats> {
        let off = clock.utc_offset_secs;
        let today = day_number(clock.now_ms, off);

        let mut total = 0i64;
        let mut week = 0i64;
        let mut chars = 0i64;
        let mut per_day: BTreeMap<i64, i64> = BTreeMap::new();

        {
            let mut st = self
                .conn()
                .prepare_cached("SELECT created_at, text FROM memo WHERE deleted_at IS NULL")?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                let created_at: i64 = r.get(0)?;
                let text: String = r.get(1)?;
                total += 1;
                if created_at > clock.now_ms - 7 * DAY_MS {
                    week += 1;
                }
                chars += count_chars(&text);
                *per_day.entry(day_number(created_at, off)).or_insert(0) += 1;
            }
        }

        // ── 连续天数 ──
        //
        // 今天还没写**不算断**：早上打开应用就看到归零，是在惩罚用户还没开始写。
        let mut cur = if per_day.contains_key(&today) {
            today
        } else {
            today - 1
        };
        let mut streak = 0i64;
        while per_day.contains_key(&cur) {
            streak += 1;
            cur -= 1;
        }

        let mut longest = 0i64;
        let mut run = 0i64;
        let mut prev: Option<i64> = None;
        for &d in per_day.keys() {
            run = if prev == Some(d - 1) { run + 1 } else { 1 };
            longest = longest.max(run);
            prev = Some(d);
        }

        // ── 热力图 ──
        //
        // 窗口右端补齐到本周的周六，这样最后一列是完整的一周，格子不会长短不齐
        let end = today + (6 - weekday(today));
        let heat: Vec<HeatCell> = (0..HEAT_DAYS)
            .map(|i| {
                let d = end - (HEAT_DAYS - 1 - i);
                let count = per_day.get(&d).copied().unwrap_or(0);
                HeatCell {
                    day: day_key(d),
                    at: day_start_ms(d, off),
                    count,
                    level: heat_level(count),
                }
            })
            .collect();

        // ── 标签分布 ──
        let mut tags = BTreeMap::new();
        {
            let mut st = self.conn().prepare_cached(
                "SELECT t.tag, count(*) FROM memo_tag t JOIN memo m ON m.id = t.memo_id \
                 WHERE m.deleted_at IS NULL GROUP BY t.tag",
            )?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                tags.insert(r.get::<_, String>(0)?, r.get::<_, i64>(1)?);
            }
        }

        // ── 侧栏计数 ──
        let untagged: i64 = self.conn().query_row(
            "SELECT count(*) FROM memo WHERE deleted_at IS NULL \
             AND NOT EXISTS (SELECT 1 FROM memo_tag t WHERE t.memo_id = memo.id)",
            [],
            |r| r.get(0),
        )?;
        let trash: i64 = self.conn().query_row(
            "SELECT count(*) FROM memo WHERE deleted_at IS NOT NULL AND purged_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        let today_count = per_day.get(&today).copied().unwrap_or(0);

        // 日均：分母是「从第一条到今天」的天数，不是有记录的天数——
        // 后者会让一年只写过三天的人看到「日均 5 条」
        let span = per_day
            .keys()
            .next()
            .map(|&first| (today - first + 1).max(1))
            .unwrap_or(1);

        Ok(Stats {
            total,
            week,
            streak,
            longest,
            chars,
            per_day_avg: total as f64 / span as f64,
            heat,
            tags,
            counts: Counts {
                all: total,
                today: today_count,
                untagged,
                trash,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MemoInput;

    /// 东八区。
    const CST: i32 = 8 * 3600;

    /// 2026-08-12 12:00 北京时间。
    const NOW: i64 = 1_786_000_000_000 + 4 * 3600 * 1000;

    fn clock(now: i64) -> Clock {
        Clock {
            now_ms: now,
            utc_offset_secs: CST,
        }
    }

    fn store_with(days_ago: &[i64]) -> Store {
        let mut s = crate::tests::store();
        for (i, d) in days_ago.iter().enumerate() {
            s.upsert_memo(
                &MemoInput {
                    text: format!("第{i}条 #日记"),
                    created_at: Some(NOW - d * DAY_MS),
                    ..Default::default()
                },
                NOW,
            )
            .unwrap();
        }
        s
    }

    #[test]
    fn 日序号与日期互相对得上() {
        // 1970-01-01
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(day_key(0), "1970-1-1");
        // 闰日
        let d = day_number(1_709_164_800_000, 0); // 2024-02-29 00:00 UTC
        assert_eq!(civil_from_days(d), (2024, 2, 29));
    }

    #[test]
    fn 日期键不补零与前端一致() {
        // 补零的话前端 dayKey 生成的 "2026-8-5" 与这里的 "2026-08-05" 对不上，
        // 热力图会整片空掉
        let d = day_number(1_754_380_320_000, CST);
        assert_eq!(day_key(d), "2025-8-5");
    }

    #[test]
    fn 一九七零年之前的时间戳不串到后一天() {
        // div_euclid 而非 / 的原因
        let ms = -1000; // 1969-12-31 23:59:59 UTC
        assert_eq!(day_number(ms, 0), -1);
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn 星期几() {
        assert_eq!(weekday(0), 4); // 1970-01-01 是周四
    }

    #[test]
    fn 时区决定一条笔记算哪一天() {
        // 北京时间 8 月 6 日 01:00 == UTC 8 月 5 日 17:00
        let ms = 1_754_413_200_000;
        assert_ne!(day_number(ms, CST), day_number(ms, 0));
    }

    #[test]
    fn 总数与字数() {
        let s = store_with(&[0, 1, 2]);
        let st = s.stats(clock(NOW)).unwrap();
        assert_eq!(st.total, 3);
        assert_eq!(st.week, 3);
        // 「第0条 #日记」→ plain 去掉标签 → 「第0条」→ 第 / 0 / 条 = 3
        assert_eq!(st.chars, 9);
    }

    #[test]
    fn 今天还没写不算断了() {
        // 早上打开应用就看到连续天数归零，是在惩罚用户还没开始写
        let s = store_with(&[1, 2, 3]);
        assert_eq!(s.stats(clock(NOW)).unwrap().streak, 3);
    }

    #[test]
    fn 今天写了也接着数() {
        let s = store_with(&[0, 1, 2]);
        assert_eq!(s.stats(clock(NOW)).unwrap().streak, 3);
    }

    #[test]
    fn 断掉一天就断了() {
        let s = store_with(&[1, 2, 4, 5]);
        let st = s.stats(clock(NOW)).unwrap();
        assert_eq!(st.streak, 2);
        assert_eq!(st.longest, 2);
    }

    #[test]
    fn 最长连续取历史最大() {
        let s = store_with(&[1, 10, 11, 12, 13]);
        let st = s.stats(clock(NOW)).unwrap();
        assert_eq!(st.streak, 1);
        assert_eq!(st.longest, 4);
    }

    #[test]
    fn 空库不崩且各项归零() {
        let s = crate::tests::store();
        let st = s.stats(clock(NOW)).unwrap();
        assert_eq!((st.total, st.streak, st.longest, st.chars), (0, 0, 0, 0));
        assert_eq!(st.heat.len(), HEAT_DAYS as usize);
        assert!(st.heat.iter().all(|c| c.count == 0 && c.level == 0));
    }

    #[test]
    fn 热力图窗口右端补到本周周六() {
        let s = store_with(&[0]);
        let st = s.stats(clock(NOW)).unwrap();
        assert_eq!(st.heat.len(), HEAT_DAYS as usize);
        let last = day_number(st.heat.last().unwrap().at, CST);
        assert_eq!(weekday(last), 6, "最后一格必须是周六");
        assert_eq!(st.heat.len() % 7, 0, "格数要能整除一周");
    }

    #[test]
    fn 热力档位不随当期最大值归一() {
        assert_eq!(
            (
                heat_level(0),
                heat_level(1),
                heat_level(2),
                heat_level(4),
                heat_level(5),
                heat_level(500)
            ),
            (0, 1, 2, 3, 4, 4)
        );
    }

    #[test]
    fn 标签分布与侧栏计数() {
        let mut s = crate::tests::store();
        let a = s
            .upsert_memo(
                &MemoInput {
                    text: "#读书 一".into(),
                    created_at: Some(NOW),
                    ..Default::default()
                },
                NOW,
            )
            .unwrap();
        s.upsert_memo(
            &MemoInput {
                text: "#读书 二".into(),
                created_at: Some(NOW),
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
        s.upsert_memo(
            &MemoInput {
                text: "没有标签".into(),
                created_at: Some(NOW),
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
        s.soft_delete(a.id, NOW).unwrap();

        let st = s.stats(clock(NOW)).unwrap();
        assert_eq!(st.tags.get("读书"), Some(&1), "回收站里的不该计入标签分布");
        assert_eq!(st.counts.all, 2);
        assert_eq!(st.counts.untagged, 1);
        assert_eq!(st.counts.trash, 1);
        assert_eq!(st.counts.today, 2);
    }

    #[test]
    fn 日均的分母是跨度不是有记录的天数() {
        // 一年只写过三天的人不该看到「日均 1 条」
        let s = store_with(&[0, 99]);
        let st = s.stats(clock(NOW)).unwrap();
        assert!(
            (st.per_day_avg - 2.0 / 100.0).abs() < 1e-9,
            "{}",
            st.per_day_avg
        );
    }

    #[test]
    fn 墓碑不计入任何统计() {
        let mut s = store_with(&[0, 1]);
        let id = s.list_memos(&Default::default(), None, 1).unwrap()[0].id;
        s.soft_delete(id, NOW).unwrap();
        s.purge(id, NOW).unwrap();
        let st = s.stats(clock(NOW)).unwrap();
        assert_eq!(st.total, 1);
        assert_eq!(st.counts.trash, 0, "墓碑不该出现在回收站计数里");
    }
}
