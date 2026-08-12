//! 表结构与迁移（§9.5）。
//!
//! SQLCipher 已经整库加密，所以本地表存**明文列**——FTS、索引、`ORDER BY`、
//! 聚合全部照常可用。密文只在推送到服务端的那一刻生成（§9.5 开头）。
//!
//! 这正是页级透明加密相对「应用层字段加密」的决定性优势：后者会让全文搜索和
//! `ORDER BY created_at` 直接失效。

use rusqlite::{Connection, Result as SqlResult};

/// 本程序认识的最高 schema 版本。
///
/// 存在 `PRAGMA user_version` 里。**遇到更高的版本必须拒绝打开**，而不是
/// 硬着头皮读——同步场景下用户很可能刚在另一台机器上装了新版，
/// 老版本写进去的数据会被新版当成损坏。
pub const SCHEMA_VERSION: i32 = 1;

/// v1 的全部 DDL。
///
/// 与 §9.5 的清单逐条对应，两处补充在下面注明。
const V1: &str = r#"
CREATE TABLE memo (
  id          BLOB PRIMARY KEY,              -- UUIDv4，16 字节（§7.1）
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  deleted_at  INTEGER,                       -- 进回收站的时刻
  purged_at   INTEGER,                       -- 墓碑时刻，见下方说明
  rev         INTEGER NOT NULL DEFAULT 1,
  text        TEXT NOT NULL,
  pinned      INTEGER NOT NULL DEFAULT 0,
  source      TEXT NOT NULL DEFAULT 'app',   -- app | quick | wechat | tg
  dirty       INTEGER NOT NULL DEFAULT 1,    -- outbox 标记
  server_seq  INTEGER
);
CREATE INDEX idx_memo_time  ON memo(created_at DESC) WHERE deleted_at IS NULL;
CREATE INDEX idx_memo_dirty ON memo(dirty) WHERE dirty = 1;
CREATE INDEX idx_memo_trash ON memo(deleted_at DESC) WHERE deleted_at IS NOT NULL AND purged_at IS NULL;
CREATE INDEX idx_memo_purged ON memo(purged_at) WHERE purged_at IS NOT NULL;

CREATE TABLE memo_tag (memo_id BLOB, tag TEXT, PRIMARY KEY(memo_id, tag));
CREATE INDEX idx_tag ON memo_tag(tag);

CREATE TABLE memo_ref (src BLOB, dst BLOB, kind TEXT, PRIMARY KEY(src, dst, kind));
CREATE INDEX idx_ref_dst ON memo_ref(dst);      -- 反向链接

CREATE TABLE blob (
  plain_hash BLOB PRIMARY KEY,   -- SHA256(明文)
  cipher_id  BLOB NOT NULL,      -- SHA256(密文)，服务端对象键
  mime TEXT, size INTEGER, width INTEGER, height INTEGER,
  refcount INTEGER NOT NULL DEFAULT 0,
  uploaded INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_blob_unref ON blob(refcount) WHERE refcount = 0;

CREATE TABLE memo_blob (memo_id BLOB, plain_hash BLOB, ord INTEGER, PRIMARY KEY(memo_id, ord));
CREATE INDEX idx_memo_blob_hash ON memo_blob(plain_hash);

CREATE VIRTUAL TABLE memo_fts USING fts5(
  text, content='memo', content_rowid='rowid', tokenize='trigram'
);

CREATE TABLE outbox (
  memo_id BLOB PRIMARY KEY, op TEXT NOT NULL, queued_at INTEGER NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0, last_error TEXT
);

CREATE TABLE kv (k TEXT PRIMARY KEY, v BLOB NOT NULL);
"#;

/// 外部内容表的同步触发器。
///
/// **§9.5 的清单里没有，但 `content='memo'` 逼着必须有。** 外部内容模式下
/// FTS5 不会自己跟着基表走，官方文档要求配一组触发器（或每次手工维护索引）。
/// 漏了的表现是：新写的笔记搜不到，改过的笔记按旧文搜得到——两个都很难在
/// 开发期撞见，因为刚写完的那条正显示在列表里。
const V1_FTS_TRIGGERS: &str = r#"
CREATE TRIGGER memo_ai AFTER INSERT ON memo BEGIN
  INSERT INTO memo_fts(rowid, text) VALUES (new.rowid, new.text);
END;
CREATE TRIGGER memo_ad AFTER DELETE ON memo BEGIN
  INSERT INTO memo_fts(memo_fts, rowid, text) VALUES ('delete', old.rowid, old.text);
END;
CREATE TRIGGER memo_au AFTER UPDATE OF text ON memo BEGIN
  INSERT INTO memo_fts(memo_fts, rowid, text) VALUES ('delete', old.rowid, old.text);
  INSERT INTO memo_fts(rowid, text) VALUES (new.rowid, new.text);
END;
"#;

/// 建表或升级到 [`SCHEMA_VERSION`]。
pub fn migrate(conn: &Connection) -> SqlResult<()> {
    let found: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;

    if found > SCHEMA_VERSION {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
            Some(format!(
                "数据库 schema 版本 {found} 高于本程序支持的 {SCHEMA_VERSION}，请升级知言"
            )),
        ));
    }

    if found < 1 {
        conn.execute_batch(V1)?;
        conn.execute_batch(V1_FTS_TRIGGERS)?;
    }

    // 后续版本在这里接 `if found < 2 { … }`。
    // 每一档都必须能从任意更低的版本跑上来，所以是一串独立的 if，不是 match。

    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        c
    }

    #[test]
    fn 建表后版本号被写上() {
        let c = mem();
        migrate(&c).unwrap();
        let v: i32 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn 重复迁移是幂等的() {
        let c = mem();
        migrate(&c).unwrap();
        migrate(&c).unwrap(); // 每次启动都会跑，不能炸
    }

    #[test]
    fn 更高的版本被拒绝而不是硬读() {
        let c = mem();
        migrate(&c).unwrap();
        c.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert!(migrate(&c).is_err());
    }

    #[test]
    fn 编译进来的_sqlite_支持_fts5_的_trigram() {
        // trigram 要 SQLite 3.34+。默认的 unicode61 不切分中文，
        // 「认知」搜不到「注意力认知负荷」——这条测试守着编译选项
        let c = mem();
        migrate(&c).unwrap();
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='memo_fts'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn 表与索引齐全() {
        let c = mem();
        migrate(&c).unwrap();
        for t in [
            "memo",
            "memo_tag",
            "memo_ref",
            "blob",
            "memo_blob",
            "memo_fts",
            "outbox",
            "kv",
        ] {
            let n: i64 = c
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name=?1",
                    [t],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "缺表 {t}");
        }
    }
}
