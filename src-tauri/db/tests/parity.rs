//! 前后端解析口径的对照测试。
//!
//! 标签、行内引用、字数这三件事**有两份实现**：前端 `src/js/util.js`（渲染高亮、
//! 时间脊节点大小要用）与 Rust `zhiyan-db::parse`（决定 `memo_tag` 表里存了什么、
//! 统计面板的累计字数是多少）。
//!
//! 两份实现分叉的表现很难查：
//!
//! - 界面上高亮着的标签，点侧栏筛不出来；
//! - 时间脊上每天的字数加起来，对不上统计面板的总数。
//!
//! 所以用例表放在仓库根的 `tests/fixtures/parse-parity.json`，**两边各跑一遍同一张表**
//! （另一半在 `src/test/parse-parity.test.js`）。改动任何一侧的规则时，
//! 两边会同时红，逼着人把另一侧一起改掉。
//!
//! 表里的期望值由 JS 那一侧生成——前端是用户直接看到的那一份，以它为准。

use std::path::PathBuf;

use serde::Deserialize;
use zhiyan_db::{count_chars, parse_refs, parse_tags};

#[derive(Debug, Deserialize)]
struct Case {
    text: String,
    tags: Vec<String>,
    refs: Vec<String>,
    chars: i64,
}

fn cases() -> Vec<Case> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/parse-parity.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读不到对照表 {}：{e}", path.display()));
    serde_json::from_str(&raw).expect("对照表不是合法 JSON")
}

#[test]
fn 标签解析与前端一致() {
    for c in cases() {
        assert_eq!(parse_tags(&c.text), c.tags, "输入：{:?}", c.text);
    }
}

#[test]
fn 引用解析与前端一致() {
    for c in cases() {
        assert_eq!(parse_refs(&c.text), c.refs, "输入：{:?}", c.text);
    }
}

#[test]
fn 字数口径与前端一致() {
    for c in cases() {
        assert_eq!(count_chars(&c.text), c.chars, "输入：{:?}", c.text);
    }
}

#[test]
fn 对照表本身没有退化成空表() {
    // 表被误清空的话上面三条会全部「通过」
    let all = cases();
    assert!(all.len() >= 20, "用例太少了：{}", all.len());
    assert!(all.iter().any(|c| !c.tags.is_empty()));
    assert!(all.iter().any(|c| !c.refs.is_empty()));
}
