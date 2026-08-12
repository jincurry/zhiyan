/**
 * 生成 `tests/fixtures/parse-parity.json`。
 *
 * 标签 / 行内引用 / 字数这三件事有两份实现（前端 `util.js` 与 Rust
 * `zhiyan-db::parse`），这张表是它们之间的合同。**期望值以前端为准**——
 * 前端那份决定用户眼睛看到什么被高亮成标签，让后端去对齐它，反过来不行。
 *
 * 要改口径：先改 `util.js`，跑这个脚本重新生成表，然后 `cargo test -p zhiyan-db`
 * 会红，逼着把 Rust 侧一起改掉。
 *
 * ```sh
 * node scripts/gen-parse-fixtures.mjs
 * ```
 *
 * 用例只加不删。每条都对应一类曾经出过或者容易出的分歧，删掉就等于把那一类
 * 分歧重新放回来。
 */

import { writeFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname } from 'node:path';

import { parseTags, inlineRefs, countChars } from '../src/js/util.js';

const cases = [
  // 基本形
  '',
  '没有任何标签的一条',
  '#串首',
  '今天读了 #读书 和 #产品',
  '#读书/认知 很有意思',

  // 标签的边界：什么算标签开头、什么算标签结尾
  '记一笔 #工作，然后休息',
  '（#括号里的）',
  '用 C#写的', // 井号前是字母，不是标签
  'a#b',
  '# 一级标题', // 光一个井号是 Markdown 标题
  '## 二级标题',
  '#a #b #a #c', // 去重且保序
  '结尾也是标签 #收尾',
  '#带数字123 与 #带-连字符',
  '#百分之100%', // 标签里可以有 SQL/正则的元字符
  '多行\n#第二行的标签',
  '#标签后面紧跟句号。',
  '#标签！后面是感叹号',
  '#标签?半角问号',
  '#tag1#tag2',
  ' #前面是空格',
  '\t#前面是制表符',

  // 行内引用
  '见 [[那天的想法^3f9a1c2e-0000-4000-8000-000000000001]]',
  '[[^dead-beef]] 与 [[另一条^ABC123]]',
  '[[没有脱字符]] 和 [[label^]] 和 [[label^xyz]]',
  '混着 #标签 与 [[引用^0f0f0f0f-0000-4000-8000-00000000000a]] 的一条',
  '多个 [[a^aaa]] [[b^bbb]] [[a^aaa]]',

  // 字数：Markdown 标记不计入
  '**加粗** 与 `代码`',
  '[链接](https://example.com)',
  '[不完整的链接](',
  '[空文字]()',
  '~~删除线~~ 与 ==高亮==',
  '> 引用块 #引用里的标签',

  // 列表符号。`> - x` 这条卡的是替换顺序：`>` 先被去掉，那一行才认得出列表符号
  '- 列表项一\n- 列表项二',
  '  + 缩进的列表项',
  '> - 引用里的列表项',
  '-没有空格不是列表',
  'a - b 中间的减号',
  '#标签 后面\n- 列表 [[引用^0f0f0f0f-0000-4000-8000-00000000000b]]',

  // 中文按字、英文按词
  'hello world 两个词',
  '今天 hello 很好',
  '在东京的最后一个下午，我在便利店门口站了很久。#旅行/日本',
];

const out = cases.map((text) => ({
  text,
  tags: parseTags(text),
  refs: inlineRefs(text),
  chars: countChars(text),
}));

const path = fileURLToPath(new URL('../tests/fixtures/parse-parity.json', import.meta.url));
mkdirSync(dirname(path), { recursive: true });
writeFileSync(path, JSON.stringify(out, null, 2) + '\n');
console.log(`写出 ${out.length} 条用例 → tests/fixtures/parse-parity.json`);
