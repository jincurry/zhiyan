/**
 * 前后端解析口径的对照测试（前端这一侧）。
 *
 * 标签、行内引用、字数这三件事有两份实现：这里的 `util.js`，与 Rust 的
 * `zhiyan-db::parse`。分叉的表现很难查——界面上高亮着的标签点侧栏筛不出来，
 * 时间脊每天的字数加起来对不上统计面板的总数。
 *
 * 两边跑的是**同一张表** `tests/fixtures/parse-parity.json`，另一半在
 * `src-tauri/db/tests/parity.rs`。改任何一侧的规则，两边会同时红。
 *
 * 表里的期望值以前端为准：它是用户直接看到的那一份。要改口径就改这里，
 * 然后重新生成表，让 Rust 侧跟上。
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { parseTags, inlineRefs, countChars } from '../js/util.js';

const cases = JSON.parse(
  readFileSync(fileURLToPath(new URL('../../tests/fixtures/parse-parity.json', import.meta.url)), 'utf8'),
);

test('标签解析与对照表一致', () => {
  for (const c of cases) assert.deepEqual(parseTags(c.text), c.tags, JSON.stringify(c.text));
});

test('引用解析与对照表一致', () => {
  for (const c of cases) assert.deepEqual(inlineRefs(c.text), c.refs, JSON.stringify(c.text));
});

test('字数口径与对照表一致', () => {
  for (const c of cases) assert.equal(countChars(c.text), c.chars, JSON.stringify(c.text));
});

test('对照表本身没有退化成空表', () => {
  // 表被误清空的话上面三条会全部「通过」
  assert.ok(cases.length >= 20, `用例太少了：${cases.length}`);
  assert.ok(cases.some((c) => c.tags.length));
  assert.ok(cases.some((c) => c.refs.length));
  assert.ok(cases.some((c) => c.text.includes('\n- ')), '缺少列表符号的用例');
});
