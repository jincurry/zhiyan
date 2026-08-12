//! 片语的读写（§9.6 的前七条命令）。

use rusqlite::{params, params_from_iter, types::Value, Connection, Row, Transaction};
use uuid::Uuid;

use crate::model::{BlobMeta, Filter, Memo, MemoInput, Op, Sort, Source, View};
use crate::parse::{parse_refs, parse_tags};
use crate::{DbError, Result, Store};

/// `memo` 表的列，顺序与 [`row_to_memo`] 一一对应。
const COLS: &str =
    "id, created_at, updated_at, deleted_at, purged_at, rev, text, pinned, source, dirty";

fn row_to_memo(row: &Row<'_>) -> rusqlite::Result<Memo> {
    let purged_at: Option<i64> = row.get(4)?;
    Ok(Memo {
        id: row.get(0)?,
        created_at: row.get(1)?,
        updated_at: row.get(2)?,
        deleted_at: row.get(3)?,
        rev: row.get(5)?,
        text: row.get(6)?,
        pinned: row.get::<_, i64>(7)? != 0,
        source: Source::parse(&row.get::<_, String>(8)?),
        dirty: row.get::<_, i64>(9)? != 0,
        tags: Vec::new(),
        refs: Vec::new(),
        blobs: Vec::new(),
        tombstone: purged_at.is_some(),
    })
}

/// `LIKE` 的元字符转义。
///
/// 不转的话，用户搜一个 `100%` 会变成「匹配任意内容」，搜 `a_b` 会把 `axb` 也搜出来。
fn like_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// 把用户输入包成 FTS5 的**短语**字面量。
///
/// 不包的话，`-`、`*`、`AND`、`NEAR` 全会被当成查询语法：用户搜 `a-b` 得到的是
/// 「含 a 不含 b」，搜一个孤零零的 `"` 直接语法错误。
fn fts_phrase(q: &str) -> String {
    let mut out = String::with_capacity(q.len() + 2);
    out.push('"');
    for c in q.chars() {
        if c == '"' {
            out.push('"'); // FTS5 里双写来转义
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// trigram 分词器的下限。短于它的串在 FTS 索引里查不到，要退回 `LIKE` 扫描。
const TRIGRAM_MIN: usize = 3;

impl Store {
    // ── 读 ──────────────────────────────────────────────────────

    /// 取一条。回收站里的、以及墓碑都取得到（前者要能预览，后者同步层要读 rev）。
    pub fn get_memo(&self, id: Uuid) -> Result<Option<Memo>> {
        let sql = format!("SELECT {COLS} FROM memo WHERE id=?1");
        let mut st = self.conn().prepare_cached(&sql)?;
        let mut memo = match st.query_row([id], row_to_memo) {
            Ok(m) => m,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        self.load_relations(&mut memo)?;
        Ok(Some(memo))
    }

    /// 取一页（§9.6：**必须分页**）。
    ///
    /// # 为什么是键集分页而不是 OFFSET
    ///
    /// `OFFSET n` 要求 SQLite 先数着跳过前 n 行，翻到第 200 页时每次都得重扫
    /// 一万行。而且中途有人新写了一条，后面所有页都会整体错位，无限滚动会
    /// 看到重复的卡片。
    ///
    /// 游标是上一页最后一条的 id；这里把它的排序键读出来，用元组比较接着往下取。
    /// 游标对应的记录已被删掉时返回空页——比退回第一页好，后者会让无限滚动打转。
    pub fn list_memos(
        &self,
        filter: &Filter,
        cursor: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<Memo>> {
        let mut where_sql = String::new();
        let mut args: Vec<Value> = Vec::new();

        match filter.view {
            View::All => where_sql.push_str("deleted_at IS NULL"),
            View::Untagged => where_sql.push_str(
                "deleted_at IS NULL AND NOT EXISTS \
                 (SELECT 1 FROM memo_tag t WHERE t.memo_id = memo.id)",
            ),
            // 墓碑对用户不可见：它是同步用的残骸，不是「还能恢复的东西」
            View::Trash => where_sql.push_str("deleted_at IS NOT NULL AND purged_at IS NULL"),
        }

        if let Some(tag) = &filter.tag {
            // 前缀匹配用 substr 而不是 LIKE/GLOB：标签里可以有 `%` `_` `*` `[`，
            // 交给模式匹配的话这些字符会被当通配符
            where_sql.push_str(
                " AND EXISTS (SELECT 1 FROM memo_tag t WHERE t.memo_id = memo.id \
                 AND (t.tag = ?  OR substr(t.tag, 1, length(?) + 1) = ? || '/'))",
            );
            args.push(Value::Text(tag.clone()));
            args.push(Value::Text(tag.clone()));
            args.push(Value::Text(tag.clone()));
        }

        if let Some(q) = filter.query.as_deref().filter(|q| !q.is_empty()) {
            where_sql.push_str(" AND text LIKE ? ESCAPE '\\'");
            args.push(Value::Text(format!("%{}%", like_escape(q))));
        }

        if let Some((start, end)) = filter.range {
            where_sql.push_str(" AND created_at >= ? AND created_at < ?");
            args.push(Value::Integer(start));
            args.push(Value::Integer(end));
        }

        let trash = filter.view == View::Trash;
        let asc = filter.sort == Sort::Old;

        if let Some(c) = cursor {
            let Some(k) = self.sort_key(c)? else {
                return Ok(Vec::new());
            };
            if trash {
                where_sql.push_str(" AND (deleted_at, id) < (?, ?)");
                args.push(Value::Integer(k.deleted_at.unwrap_or(0)));
                args.push(Value::Blob(c.as_bytes().to_vec()));
            } else {
                // 置顶恒在最前，所以第一维永远是 pinned DESC；第二维才随 sort 变向
                let cmp = if asc { '>' } else { '<' };
                where_sql.push_str(&format!(
                    " AND (pinned < ? OR (pinned = ? AND (created_at, id) {cmp} (?, ?)))"
                ));
                let p = Value::Integer(k.pinned as i64);
                args.push(p.clone());
                args.push(p);
                args.push(Value::Integer(k.created_at));
                args.push(Value::Blob(c.as_bytes().to_vec()));
            }
        }

        let order = if trash {
            "deleted_at DESC, id DESC".to_string()
        } else if asc {
            "pinned DESC, created_at ASC, id ASC".to_string()
        } else {
            "pinned DESC, created_at DESC, id DESC".to_string()
        };

        let sql = format!("SELECT {COLS} FROM memo WHERE {where_sql} ORDER BY {order} LIMIT ?");
        args.push(Value::Integer(limit as i64));

        let mut st = self.conn().prepare(&sql)?;
        let rows = st.query_map(params_from_iter(args.iter()), row_to_memo)?;

        let mut out = Vec::with_capacity(limit as usize);
        for m in rows {
            out.push(m?);
        }
        for m in &mut out {
            self.load_relations(m)?;
        }
        Ok(out)
    }

    /// 全文检索（§9.6 的 `search`）。
    ///
    /// # 中文分词
    ///
    /// `tokenize='trigram'`。默认的 `unicode61` **不切分中文**，会把整句当一个
    /// token——「认知」搜不到「注意力认知负荷」。trigram 索引膨胀约 3 倍，
    /// 万级数据下无所谓。
    ///
    /// # 一两个字的查询
    ///
    /// trigram 建的是三字窗口，短于 3 个字符的串**在索引里根本不存在**。
    /// 直接 MATCH 会返回空，而「搜一个『书』字」在中文里再正常不过，
    /// 所以这里退回 `LIKE` 全表扫描。万级数据下一次几毫秒，可以接受；
    /// 真成瓶颈时的出路是编译 `simple` 分词器，而不是把这个 case 判死。
    pub fn search(&self, q: &str, limit: u32) -> Result<Vec<Memo>> {
        let q = q.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }

        let sql = if q.chars().count() < TRIGRAM_MIN {
            format!(
                "SELECT {COLS} FROM memo \
                 WHERE deleted_at IS NULL AND text LIKE ?1 ESCAPE '\\' \
                 ORDER BY created_at DESC LIMIT ?2"
            )
        } else {
            // rank 是 FTS5 内建的 bm25 排序；同分时按时间兜底，免得顺序在两次查询间抖动
            format!(
                "SELECT {} FROM memo_fts f JOIN memo ON memo.rowid = f.rowid \
                 WHERE f.text MATCH ?1 AND memo.deleted_at IS NULL \
                 ORDER BY f.rank, memo.created_at DESC LIMIT ?2",
                COLS.split(", ")
                    .map(|c| format!("memo.{c}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let arg = if q.chars().count() < TRIGRAM_MIN {
            format!("%{}%", like_escape(q))
        } else {
            fts_phrase(q)
        };

        let mut st = self.conn().prepare_cached(&sql)?;
        let rows = st.query_map(params![arg, limit], row_to_memo)?;
        let mut out = Vec::new();
        for m in rows {
            out.push(m?);
        }
        for m in &mut out {
            self.load_relations(m)?;
        }
        Ok(out)
    }

    /// 反向链接：哪些片语引用了 `id`。
    ///
    /// `idx_ref_dst` 就是为这一条建的。
    pub fn backlinks(&self, id: Uuid, limit: u32) -> Result<Vec<Memo>> {
        let sql = format!(
            "SELECT {} FROM memo_ref r JOIN memo ON memo.id = r.src \
             WHERE r.dst = ?1 AND memo.deleted_at IS NULL \
             ORDER BY memo.created_at DESC LIMIT ?2",
            COLS.split(", ")
                .map(|c| format!("memo.{c}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut st = self.conn().prepare_cached(&sql)?;
        let rows = st.query_map(params![id, limit], row_to_memo)?;
        let mut out = Vec::new();
        for m in rows {
            out.push(m?);
        }
        for m in &mut out {
            self.load_relations(m)?;
        }
        Ok(out)
    }

    // ── 写 ──────────────────────────────────────────────────────

    /// 新建或更新（§9.6 的 `upsert_memo`）。
    ///
    /// **标签与引用在这里从正文解析**，前端不预先算（§9.6 的注释就是这么写的）。
    /// 让前端传的话，正文与标签的一致性就落到了前端头上——而正文还会被同步、
    /// 导入、迁移改写，那些路径根本不经过前端。
    ///
    /// `now_ms` 由调用方给：IPC 层传系统时钟，导入与测试传固定值。
    /// 函数内部读时钟的话，「补录一条昨天的笔记」就没法做，测试也没法稳。
    pub fn upsert_memo(&mut self, input: &MemoInput, now_ms: i64) -> Result<Memo> {
        let tx = self.conn.transaction()?;
        let id = input.id.unwrap_or_else(Uuid::new_v4);

        let existing: Option<(i64, i64, String, i64)> = tx
            .query_row(
                "SELECT created_at, rev, source, pinned FROM memo WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .ok();

        let (created_at, rev, source, pinned) = match &existing {
            Some((c, rev, src, pin)) => (
                input.created_at.unwrap_or(*c),
                rev + 1,
                input
                    .source
                    .map(|s| s.as_str().to_string())
                    .unwrap_or_else(|| src.clone()),
                input.pinned.map(|p| p as i64).unwrap_or(*pin),
            ),
            None => (
                input.created_at.unwrap_or(now_ms),
                1,
                input.source.unwrap_or_default().as_str().to_string(),
                input.pinned.unwrap_or(false) as i64,
            ),
        };

        tx.execute(
            "INSERT INTO memo(id, created_at, updated_at, deleted_at, purged_at, rev, text, pinned, source, dirty) \
             VALUES(?1, ?2, ?3, NULL, NULL, ?4, ?5, ?6, ?7, 1) \
             ON CONFLICT(id) DO UPDATE SET \
               created_at=excluded.created_at, updated_at=excluded.updated_at, \
               rev=excluded.rev, text=excluded.text, pinned=excluded.pinned, \
               source=excluded.source, dirty=1",
            params![id, created_at, now_ms, rev, input.text, pinned, source],
        )?;

        write_tags(&tx, id, &input.text)?;
        write_refs(&tx, id, &input.text)?;
        write_blobs(&tx, id, &input.blobs)?;
        queue(&tx, id, Op::Upsert, now_ms)?;

        tx.commit()?;
        self.get_memo(id)?.ok_or(DbError::NotFound)
    }

    /// 进回收站（§9.6 的 `soft_delete`）。
    pub fn soft_delete(&self, id: Uuid, now_ms: i64) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE memo SET deleted_at=?2, updated_at=?2, rev=rev+1, dirty=1 \
             WHERE id=?1 AND deleted_at IS NULL",
            params![id, now_ms],
        )?;
        if n == 0 {
            return Err(DbError::NotFound);
        }
        queue(self.conn(), id, Op::Delete, now_ms)
    }

    /// 从回收站恢复。墓碑恢复不了——正文已经没了。
    pub fn restore(&self, id: Uuid, now_ms: i64) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE memo SET deleted_at=NULL, updated_at=?2, rev=rev+1, dirty=1 \
             WHERE id=?1 AND deleted_at IS NOT NULL AND purged_at IS NULL",
            params![id, now_ms],
        )?;
        if n == 0 {
            return Err(DbError::NotFound);
        }
        queue(self.conn(), id, Op::Upsert, now_ms)
    }

    /// 彻底删除 = **写墓碑，不是物理删**（§7.3 ②）。
    ///
    /// 物理删的后果是同步应用最经典的那个 bug：A 机删干净了，B 机不知道，
    /// 下次同步又把它推回来——「删不掉的笔记」。
    ///
    /// 墓碑留下 id、rev、`purged_at`，正文与标签、引用、附件引用全部清掉。
    /// 满 90 天且已同步出去之后由 [`Store::gc_tombstones`] 真删。
    pub fn purge(&mut self, id: Uuid, now_ms: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        let n = purge_one(&tx, id, now_ms)?;
        tx.commit()?;
        if n == 0 {
            return Err(DbError::NotFound);
        }
        Ok(())
    }

    /// 清空回收站。
    ///
    /// **不在 §9.6 的清单里**，是补的。替代方案是前端遍历逐条 `purge`：那会变成
    /// N 次 IPC 往返，而且「清空」在语义上本来就是一次事务，拆开会出现删到
    /// 一半失败的中间态。
    pub fn purge_all(&mut self, now_ms: i64) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let ids: Vec<Uuid> = {
            let mut st = tx.prepare(
                "SELECT id FROM memo WHERE deleted_at IS NOT NULL AND purged_at IS NULL",
            )?;
            let rows = st.query_map([], |r| r.get::<_, Uuid>(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for id in &ids {
            purge_one(&tx, *id, now_ms)?;
        }
        tx.commit()?;
        Ok(ids.len())
    }

    /// 置顶开关。
    ///
    /// **不在 §9.6 的清单里**，是补的。走 `upsert_memo` 也行，但那要前端把整条
    /// 正文回传一遍——为翻一个布尔值搬运整篇文字，在 IPC 边界上是明显的浪费。
    pub fn set_pinned(&self, id: Uuid, pinned: bool, now_ms: i64) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE memo SET pinned=?2, updated_at=?3, rev=rev+1, dirty=1 \
             WHERE id=?1 AND deleted_at IS NULL",
            params![id, pinned as i64, now_ms],
        )?;
        if n == 0 {
            return Err(DbError::NotFound);
        }
        queue(self.conn(), id, Op::Upsert, now_ms)
    }

    /// 真删满 90 天的墓碑（§7.3 ②「墓碑保留 90 天后由后台任务真删」）。
    ///
    /// **只删已经推上去的**（`dirty = 0`）。还带着 dirty 标记就真删的话，
    /// 那次删除永远传不到别的设备，笔记会从别处被推回来——正是墓碑要防的事。
    pub fn gc_tombstones(&self, now_ms: i64, retain_days: i64) -> Result<usize> {
        let cutoff = now_ms - retain_days * 86_400_000;
        let n = self.conn().execute(
            "DELETE FROM memo WHERE purged_at IS NOT NULL AND purged_at < ?1 AND dirty = 0",
            [cutoff],
        )?;
        Ok(n)
    }

    // ── 内部 ────────────────────────────────────────────────────

    /// 排序键，键集分页用。
    fn sort_key(&self, id: Uuid) -> Result<Option<SortKey>> {
        let mut st = self
            .conn()
            .prepare_cached("SELECT pinned, created_at, deleted_at FROM memo WHERE id=?1")?;
        match st.query_row([id], |r| {
            Ok(SortKey {
                pinned: r.get::<_, i64>(0)? != 0,
                created_at: r.get(1)?,
                deleted_at: r.get(2)?,
            })
        }) {
            Ok(k) => Ok(Some(k)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 补齐标签、引用、附件三张关联表。
    ///
    /// 每条三次小查询，一页 50 条就是 150 次。看着多，但都是主键/索引直取，
    /// 全在进程内，量级是微秒——比拼一条带三个 `IN (…)` 的动态 SQL 划算，
    /// 后者的参数偏移量是这类代码最爱出错的地方。
    fn load_relations(&self, m: &mut Memo) -> Result<()> {
        {
            let mut st = self
                .conn()
                .prepare_cached("SELECT tag FROM memo_tag WHERE memo_id=?1 ORDER BY tag")?;
            m.tags = st
                .query_map([m.id], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<_>>()?;
        }
        {
            let mut st = self.conn().prepare_cached(
                "SELECT dst FROM memo_ref WHERE src=?1 AND kind='inline' ORDER BY rowid",
            )?;
            m.refs = st
                .query_map([m.id], |r| r.get::<_, Uuid>(0))?
                .collect::<rusqlite::Result<_>>()?;
        }
        {
            let mut st = self.conn().prepare_cached(
                "SELECT b.plain_hash, b.cipher_id, b.mime, b.size, b.width, b.height \
                 FROM memo_blob mb JOIN blob b ON b.plain_hash = mb.plain_hash \
                 WHERE mb.memo_id=?1 ORDER BY mb.ord",
            )?;
            m.blobs = st
                .query_map([m.id], |r| {
                    Ok(BlobMeta {
                        sha256: hex::encode(r.get::<_, Vec<u8>>(0)?),
                        cipher_id: Some(hex::encode(r.get::<_, Vec<u8>>(1)?)),
                        mime: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        size: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                        w: r.get(4)?,
                        h: r.get(5)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
        }
        Ok(())
    }
}

struct SortKey {
    pinned: bool,
    created_at: i64,
    deleted_at: Option<i64>,
}

fn write_tags(tx: &Transaction<'_>, id: Uuid, text: &str) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM memo_tag WHERE memo_id=?1", [id])?;
    let mut st = tx.prepare_cached("INSERT OR IGNORE INTO memo_tag(memo_id, tag) VALUES(?1,?2)")?;
    for tag in parse_tags(text) {
        st.execute(params![id, tag])?;
    }
    Ok(())
}

fn write_refs(tx: &Transaction<'_>, id: Uuid, text: &str) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM memo_ref WHERE src=?1 AND kind='inline'", [id])?;
    let mut st =
        tx.prepare_cached("INSERT OR IGNORE INTO memo_ref(src, dst, kind) VALUES(?1,?2,'inline')")?;
    for r in parse_refs(text) {
        // 解析不出 UUID 的跳过：正文里可能残留 §7.3 迁移前的旧数字 id，
        // 那不该让整条保存失败
        if let Ok(dst) = Uuid::parse_str(&r) {
            st.execute(params![id, dst])?;
        }
    }
    Ok(())
}

/// 重写附件引用并维护 refcount。
///
/// refcount 归零的 blob 不在这里删——真删要连磁盘上的密文一起删，那是
/// [`crate::BlobStore::gc`] 的事，而且得在事务外做（文件系统不参与回滚）。
fn write_blobs(tx: &Transaction<'_>, id: Uuid, blobs: &[BlobMeta]) -> Result<()> {
    {
        let mut dec = tx.prepare_cached(
            "UPDATE blob SET refcount = MAX(0, refcount - 1) WHERE plain_hash = ?1",
        )?;
        let mut st = tx.prepare_cached("SELECT plain_hash FROM memo_blob WHERE memo_id=?1")?;
        let olds: Vec<Vec<u8>> = st
            .query_map([id], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<rusqlite::Result<_>>()?;
        for h in olds {
            dec.execute([h])?;
        }
    }
    tx.execute("DELETE FROM memo_blob WHERE memo_id=?1", [id])?;

    let mut ins =
        tx.prepare_cached("INSERT INTO memo_blob(memo_id, plain_hash, ord) VALUES(?1,?2,?3)")?;
    let mut inc =
        tx.prepare_cached("UPDATE blob SET refcount = refcount + 1 WHERE plain_hash=?1")?;
    for (ord, b) in blobs.iter().enumerate() {
        let hash = hex::decode(&b.sha256).map_err(|_| DbError::BlobCorrupted)?;
        // 附件字节必须先经 put_blob 落盘。这里认不出的 hash 只能是前端拼错了，
        // 静默跳过会让正文里的图永远打不开，所以直接报错
        if inc.execute([&hash])? == 0 {
            return Err(DbError::NotFound);
        }
        ins.execute(params![id, hash, ord as i64])?;
    }
    Ok(())
}

fn purge_one(tx: &Transaction<'_>, id: Uuid, now_ms: i64) -> Result<usize> {
    write_blobs(tx, id, &[])?; // 先减 refcount 再清行
    tx.execute("DELETE FROM memo_tag WHERE memo_id=?1", [id])?;
    tx.execute("DELETE FROM memo_ref WHERE src=?1", [id])?;
    let n = tx.execute(
        "UPDATE memo SET text='', purged_at=?2, deleted_at=COALESCE(deleted_at, ?2), \
         updated_at=?2, rev=rev+1, pinned=0, dirty=1 \
         WHERE id=?1 AND purged_at IS NULL",
        params![id, now_ms],
    )?;
    if n > 0 {
        queue(tx, id, Op::Purge, now_ms)?;
    }
    Ok(n)
}

/// 排队等同步（§10.3）。
///
/// 同一条记录只留最后一次操作：连着改五次再删掉，推上去的应该是一次删除，
/// 而不是六个事件。
fn queue(conn: &Connection, id: Uuid, op: Op, now_ms: i64) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO outbox(memo_id, op, queued_at) VALUES(?1,?2,?3) \
         ON CONFLICT(memo_id) DO UPDATE SET \
           op=excluded.op, queued_at=excluded.queued_at, attempts=0, last_error=NULL",
    )?
    .execute(params![id, op.as_str(), now_ms])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Sort;

    const T0: i64 = 1_754_380_320_000; // 2025-08-05 附近，与 §7.2 的示例同一天

    fn store() -> Store {
        crate::tests::store()
    }

    fn write(s: &mut Store, text: &str, at: i64) -> Uuid {
        s.upsert_memo(
            &MemoInput {
                text: text.into(),
                created_at: Some(at),
                ..Default::default()
            },
            at,
        )
        .unwrap()
        .id
    }

    fn ids(v: &[Memo]) -> Vec<Uuid> {
        v.iter().map(|m| m.id).collect()
    }

    #[test]
    fn 新建与读回() {
        let mut s = store();
        let m = s
            .upsert_memo(
                &MemoInput {
                    text: "在东京的最后一个下午 #旅行/日本".into(),
                    source: Some(Source::Quick),
                    ..Default::default()
                },
                T0,
            )
            .unwrap();

        assert_eq!(m.rev, 1);
        assert_eq!(m.source, Source::Quick);
        assert!(m.dirty);
        assert_eq!(m.tags, vec!["旅行/日本"]);

        let back = s.get_memo(m.id).unwrap().unwrap();
        assert_eq!(back.text, m.text);
        assert_eq!(back.tags, m.tags);
    }

    #[test]
    fn 标签由正文解析而不是前端传() {
        let mut s = store();
        let id = write(&mut s, "#读书/认知 与 #产品", T0);
        let tags: Vec<String> = s
            .conn()
            .prepare("SELECT tag FROM memo_tag WHERE memo_id=?1 ORDER BY tag")
            .unwrap()
            .query_map([id], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(tags, vec!["产品", "读书/认知"]);
    }

    #[test]
    fn 改正文时旧标签被清掉() {
        // 漏了这一步的话，改过标签的笔记会同时挂在新旧两个标签下
        let mut s = store();
        let id = write(&mut s, "#旧标签", T0);
        s.upsert_memo(
            &MemoInput {
                id: Some(id),
                text: "#新标签".into(),
                ..Default::default()
            },
            T0 + 1,
        )
        .unwrap();
        assert_eq!(s.get_memo(id).unwrap().unwrap().tags, vec!["新标签"]);
    }

    #[test]
    fn 更新会涨_rev_且不动_created_at() {
        let mut s = store();
        let id = write(&mut s, "一稿", T0);
        let m = s
            .upsert_memo(
                &MemoInput {
                    id: Some(id),
                    text: "二稿".into(),
                    ..Default::default()
                },
                T0 + 5_000,
            )
            .unwrap();
        assert_eq!(m.rev, 2);
        assert_eq!(m.created_at, T0, "创建时间不该被编辑改掉");
        assert_eq!(m.updated_at, T0 + 5_000);
    }

    #[test]
    fn 引用写进反向链接() {
        let mut s = store();
        let a = write(&mut s, "被引的那条", T0);
        let b = write(&mut s, &format!("见 [[那天^{a}]]"), T0 + 1);

        assert_eq!(s.get_memo(b).unwrap().unwrap().refs, vec![a]);
        assert_eq!(ids(&s.backlinks(a, 10).unwrap()), vec![b]);
    }

    #[test]
    fn 引用里的非_uuid_被跳过而不是让保存失败() {
        // §7.3 的迁移之前，正文里是 [[…^数字]]
        let mut s = store();
        let id = write(&mut s, "旧格式 [[某条^123abc]] 还在", T0);
        assert!(s.get_memo(id).unwrap().unwrap().refs.is_empty());
    }

    #[test]
    fn 列表按时间倒序且置顶在前() {
        let mut s = store();
        let a = write(&mut s, "早", T0);
        let b = write(&mut s, "中", T0 + 1000);
        let c = write(&mut s, "晚", T0 + 2000);
        s.set_pinned(a, true, T0 + 3000).unwrap();

        let got = s.list_memos(&Filter::default(), None, 10).unwrap();
        assert_eq!(ids(&got), vec![a, c, b]);
    }

    #[test]
    fn 正序也把置顶放在最前() {
        let mut s = store();
        let a = write(&mut s, "早", T0);
        let b = write(&mut s, "中", T0 + 1000);
        let c = write(&mut s, "晚", T0 + 2000);
        s.set_pinned(c, true, T0 + 3000).unwrap();

        let f = Filter {
            sort: Sort::Old,
            ..Default::default()
        };
        assert_eq!(ids(&s.list_memos(&f, None, 10).unwrap()), vec![c, a, b]);
    }

    #[test]
    fn 键集分页不重不漏() {
        let mut s = store();
        let all: Vec<Uuid> = (0..25)
            .map(|i| write(&mut s, &format!("第{i}条"), T0 + i))
            .collect();

        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let page = s.list_memos(&Filter::default(), cursor, 7).unwrap();
            if page.is_empty() {
                break;
            }
            cursor = Some(page.last().unwrap().id);
            seen.extend(ids(&page));
        }

        let mut expect = all;
        expect.reverse(); // 时间倒序
        assert_eq!(seen, expect);
    }

    #[test]
    fn 同一毫秒写入的多条也不会在分页里丢() {
        // 只按 created_at 做游标的话，同毫秒的那几条会被整体跳过或重复。
        // 排序键第三维是 id，就是为了这个
        let mut s = store();
        for i in 0..10 {
            write(&mut s, &format!("同毫秒{i}"), T0);
        }
        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let page = s.list_memos(&Filter::default(), cursor, 3).unwrap();
            if page.is_empty() {
                break;
            }
            cursor = Some(page.last().unwrap().id);
            seen.extend(ids(&page));
        }
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 10);
    }

    #[test]
    fn 游标指向已消失的记录时返回空页而不是打转() {
        // 无限滚动正翻页时，那条记录在另一台设备上被删了并同步了过来。
        // 退回第一页的话滚动条会永远在原地打转
        let mut s = store();
        write(&mut s, "一", T0);
        let b = write(&mut s, "二", T0 + 1);
        s.conn()
            .execute("DELETE FROM memo WHERE id=?1", [b])
            .unwrap();

        assert!(s
            .list_memos(&Filter::default(), Some(b), 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn 按标签筛选含子标签() {
        let mut s = store();
        let a = write(&mut s, "#读书", T0);
        let b = write(&mut s, "#读书/认知", T0 + 1);
        write(&mut s, "#产品", T0 + 2);

        let f = Filter {
            tag: Some("读书".into()),
            ..Default::default()
        };
        let got = ids(&s.list_memos(&f, None, 10).unwrap());
        assert_eq!(got.len(), 2);
        assert!(got.contains(&a) && got.contains(&b));
    }

    #[test]
    fn 标签里的通配符不被当模式() {
        let mut s = store();
        let a = write(&mut s, "#百分之100%", T0);
        write(&mut s, "#百分之", T0 + 1);
        let f = Filter {
            tag: Some("百分之100%".into()),
            ..Default::default()
        };
        assert_eq!(ids(&s.list_memos(&f, None, 10).unwrap()), vec![a]);
    }

    #[test]
    fn 未打标签视图() {
        let mut s = store();
        write(&mut s, "#有标签", T0);
        let b = write(&mut s, "没有标签", T0 + 1);
        let f = Filter {
            view: View::Untagged,
            ..Default::default()
        };
        assert_eq!(ids(&s.list_memos(&f, None, 10).unwrap()), vec![b]);
    }

    #[test]
    fn 时间区间筛选() {
        let mut s = store();
        write(&mut s, "区间外", T0);
        let b = write(&mut s, "区间内", T0 + 5_000);
        write(&mut s, "区间外", T0 + 20_000);

        let f = Filter {
            range: Some((T0 + 1_000, T0 + 10_000)),
            ..Default::default()
        };
        assert_eq!(ids(&s.list_memos(&f, None, 10).unwrap()), vec![b]);
    }

    #[test]
    fn 列表关键词里的百分号不被当通配符() {
        let mut s = store();
        let a = write(&mut s, "涨了 100% 呢", T0);
        write(&mut s, "无关的一条", T0 + 1);
        let f = Filter {
            query: Some("100%".into()),
            ..Default::default()
        };
        assert_eq!(ids(&s.list_memos(&f, None, 10).unwrap()), vec![a]);
    }

    #[test]
    fn 软删与恢复() {
        let mut s = store();
        let a = write(&mut s, "先删掉", T0);
        s.soft_delete(a, T0 + 1).unwrap();

        assert!(s
            .list_memos(&Filter::default(), None, 10)
            .unwrap()
            .is_empty());
        let trash = Filter {
            view: View::Trash,
            ..Default::default()
        };
        assert_eq!(ids(&s.list_memos(&trash, None, 10).unwrap()), vec![a]);

        s.restore(a, T0 + 2).unwrap();
        assert_eq!(
            ids(&s.list_memos(&Filter::default(), None, 10).unwrap()),
            vec![a]
        );
        assert!(s.list_memos(&trash, None, 10).unwrap().is_empty());
    }

    #[test]
    fn 重复软删报找不到而不是静默成功() {
        let mut s = store();
        let a = write(&mut s, "x", T0);
        s.soft_delete(a, T0 + 1).unwrap();
        assert!(matches!(s.soft_delete(a, T0 + 2), Err(DbError::NotFound)));
    }

    #[test]
    fn 彻底删除留下墓碑而不是物理删() {
        // §7.3 ②：物理删的话别的设备下次同步会把它推回来
        let mut s = store();
        let a = write(&mut s, "秘密内容 #秘密", T0);
        s.soft_delete(a, T0 + 1).unwrap();
        s.purge(a, T0 + 2).unwrap();

        let row = s.get_memo(a).unwrap().expect("行必须还在");
        assert!(row.tombstone);
        assert_eq!(row.text, "", "正文必须清干净");
        assert!(row.tags.is_empty());

        // 用户看不到它
        let trash = Filter {
            view: View::Trash,
            ..Default::default()
        };
        assert!(s.list_memos(&trash, None, 10).unwrap().is_empty());
        assert!(s
            .list_memos(&Filter::default(), None, 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn 清空回收站是一次事务() {
        let mut s = store();
        for i in 0..5 {
            let id = write(&mut s, &format!("{i}"), T0 + i);
            s.soft_delete(id, T0 + 100).unwrap();
        }
        assert_eq!(s.purge_all(T0 + 200).unwrap(), 5);
        let n: i64 = s
            .conn()
            .query_row(
                "SELECT count(*) FROM memo WHERE purged_at IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 5, "五条都该只剩墓碑");
    }

    #[test]
    fn 墓碑满九十天且已同步才真删() {
        let mut s = store();
        let a = write(&mut s, "x", T0);
        s.soft_delete(a, T0 + 1).unwrap();
        s.purge(a, T0 + 2).unwrap();

        let long_after = T0 + 2 + 91 * 86_400_000;

        // 还带着 dirty：删了的话这次删除永远传不到别的设备
        assert_eq!(s.gc_tombstones(long_after, 90).unwrap(), 0);

        s.conn().execute("UPDATE memo SET dirty=0", []).unwrap();
        assert_eq!(s.gc_tombstones(T0 + 3, 90).unwrap(), 0, "没满 90 天不能删");
        assert_eq!(s.gc_tombstones(long_after, 90).unwrap(), 1);
        assert!(s.get_memo(a).unwrap().is_none());
    }

    #[test]
    fn outbox_只留最后一次操作() {
        let mut s = store();
        let a = write(&mut s, "一稿", T0);
        write(&mut s, "无关", T0 + 1);
        s.upsert_memo(
            &MemoInput {
                id: Some(a),
                text: "二稿".into(),
                ..Default::default()
            },
            T0 + 2,
        )
        .unwrap();
        s.soft_delete(a, T0 + 3).unwrap();

        let (op, n): (String, i64) = s
            .conn()
            .query_row(
                "SELECT op, (SELECT count(*) FROM outbox WHERE memo_id=?1) FROM outbox WHERE memo_id=?1",
                [a],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(op, "delete");
    }

    #[test]
    fn 全文检索中文() {
        // unicode61 分词器在这条上会挂：它把整句当一个 token
        let mut s = store();
        let a = write(&mut s, "注意力与认知负荷的关系", T0);
        write(&mut s, "完全无关的一条", T0 + 1);
        assert_eq!(ids(&s.search("认知", 10).unwrap()), vec![a]);
    }

    #[test]
    fn 全文检索跟着正文更新() {
        // 外部内容表没配触发器的话，这条会挂在「改过的笔记按旧文还搜得到」
        let mut s = store();
        let a = write(&mut s, "原来的内容", T0);
        s.upsert_memo(
            &MemoInput {
                id: Some(a),
                text: "换过的内容".into(),
                ..Default::default()
            },
            T0 + 1,
        )
        .unwrap();
        assert!(s.search("原来", 10).unwrap().is_empty());
        assert_eq!(ids(&s.search("换过", 10).unwrap()), vec![a]);
    }

    #[test]
    fn 单字检索退回_like_扫描() {
        // trigram 建的是三字窗口，一个字在索引里根本不存在
        let mut s = store();
        let a = write(&mut s, "读了一本书", T0);
        assert_eq!(ids(&s.search("书", 10).unwrap()), vec![a]);
    }

    #[test]
    fn 检索里的语法字符不被当操作符() {
        let mut s = store();
        let a = write(&mut s, "a-b-c 这一串", T0);
        write(&mut s, "只有 a 没有别的", T0 + 1);
        assert_eq!(ids(&s.search("a-b-c", 10).unwrap()), vec![a]);
        // 一个孤零零的引号在 FTS5 里是语法错误
        assert!(s.search("\"", 10).is_ok());
        assert!(s.search("NEAR AND OR *", 10).is_ok());
    }

    #[test]
    fn 检索不返回回收站里的() {
        let mut s = store();
        let a = write(&mut s, "会被删掉的内容", T0);
        s.soft_delete(a, T0 + 1).unwrap();
        assert!(s.search("会被删掉", 10).unwrap().is_empty());
    }
}
