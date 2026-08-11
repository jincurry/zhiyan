/**
 * 命令面板（Ctrl+K）。
 *
 * 三段结果拼在一起：命令、标签、片语命中。片语命中要回 Rust 侧搜——
 * 前端手上只有当前视图那一份快照，拿它当搜索源会漏掉大半。
 */

import { esc, snippet } from './util.js';
import { state } from './state.js';
import * as api from './api.js';

const $ = (id) => document.getElementById(id);

let rows = [];
let index = 0;
/** 命令表由 main.js 注入——面板不该知道每条命令具体做什么。 */
let commands = [];
let tagsOf = () => [];
/** 输入变化后异步取搜索结果，用序号丢弃过期的响应。 */
let seq = 0;

export function initPalette({ commands: cmds, tags }) {
  commands = cmds;
  tagsOf = tags;
}

export function openPalette() {
  $('scrimSheet').classList.remove('show');
  $('scrimPal').classList.add('show');
  const pq = $('pq');
  pq.value = '';
  fill('');
  setTimeout(() => pq.focus(), 20);
}

export async function fill(query) {
  const q = query.trim();
  const mine = ++seq;

  const cmdRows = commands
    .filter((c) => !q || c.label.includes(q))
    .map((c) => ({ icon: c.icon, text: c.label, kbd: c.kbd ?? '', run: c.run }));

  const tagRows = tagsOf()
    .filter((t) => !q || t.includes(q))
    .slice(0, 5)
    .map((t) => ({ icon: '◈', text: '#' + t, kbd: '', run: () => pickTag(t) }));

  // 先把命令与标签画出来，搜索结果到了再补——搜索要过 IPC，
  // 等它回来再一起画会让面板有明显的延迟感
  rows = [...cmdRows, ...tagRows];
  index = 0;
  paint();

  if (!q) return;
  let hits = [];
  try {
    hits = await api.listMemos({ view: 'all', query: q, sort: 'new' });
  } catch {
    hits = [];
  }
  if (mine !== seq) return; // 输入已经变了，这次结果作废

  rows = [
    ...cmdRows,
    ...tagRows,
    ...hits.slice(0, 6).map((m) => ({
      icon: '❞',
      text: snippet(m.text, 44),
      kbd: '',
      run: () => jumpTo(m.id),
    })),
  ];
  paint();
}

function paint() {
  $('plist').innerHTML = rows.length
    ? rows
        .map(
          (r, i) =>
            `<button class="pitem ${i === index ? 'hl' : ''}" data-act="palette-run" data-arg="${i}">` +
            `<span class="ic">${esc(r.icon)}</span><span class="tx">${esc(r.text)}</span>` +
            (r.kbd ? `<span class="kbd">${esc(r.kbd)}</span>` : '') +
            '</button>'
        )
        .join('')
    : '<div style="padding:22px;text-align:center;font-size:13px;color:var(--ink-3)">没有匹配的结果。按 ⏎ 把它写成新片语。</div>';
}

export function highlightRow(i) {
  index = i;
  document.querySelectorAll('.pitem').forEach((el, j) => el.classList.toggle('hl', j === i));
}

export function runRow(i) {
  const r = rows[i];
  if (r) r.run();
}

export function currentIndex() {
  return index;
}

export function rowCount() {
  return rows.length;
}

/** 这两个由 main.js 覆盖成真正的实现，避免 palette 反向依赖导航模块。 */
export let pickTag = () => {};
export let jumpTo = () => {};

export function bindNavigation(handlers) {
  pickTag = handlers.pickTag;
  jumpTo = handlers.jumpTo;
}
