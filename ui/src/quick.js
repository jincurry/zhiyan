/**
 * 速记浮窗。
 *
 * 它是**独立的 WebView 窗口**，不是主窗口里的一个 `div`。两条硬要求逼出了这个结构：
 * 主窗口最小化时它要能单独出现；它不能进任务栏。做成 `div` 两条都办不到。
 *
 * 代价是它有自己的 JS 上下文，取数要各走各的 IPC。所以这里刻意只做一件事：
 * 写一句、存下、收起。补全与标签这些「顺手」的东西共用主窗口的模块，不另写一套。
 */

import * as api from './api.js';
import { countChars, esc } from './util.js';

const $ = (id) => document.getElementById(id);

const ta = $('flInput');
const box = $('flSuggest');

/** 补全用的数据。开窗时取一次就够——浮窗的生命周期以「写一句话」为单位。 */
let tags = [];
let memos = [];

function refresh() {
  ta.style.height = 'auto';
  ta.style.height = Math.max(96, Math.min(300, ta.scrollHeight)) + 'px';
  $('flCount').textContent = countChars(ta.value) + ' 字';
  $('flSave').disabled = !ta.value.trim();
}

async function loadContext() {
  try {
    const counts = await api.tagCounts();
    tags = Object.keys(counts);
    // 常用的三个做成一键插入——速记的价值在于快，翻标签树就不叫速记了
    const top = Object.entries(counts).sort((a, b) => b[1] - a[1]).slice(0, 3);
    $('flTags').innerHTML = top
      .map(([t]) => `<button class="qtag" data-act="tag" data-arg="${esc(t)}">#${esc(t)}</button>`)
      .join('');
    memos = await api.listMemos({ view: 'all', sort: 'new' });
  } catch {
    // 取不到就退化成「只能写纯文本」，但存下这件事必须还能用
  }
}

async function save() {
  const text = ta.value.trim();
  if (!text) return;
  await api.upsertMemo({ text, source: 'quick' });
  ta.value = '';
  refresh();
  close();
}

function close() {
  if (api.inTauri) api.quickToggle();
}

function insertTag(tag) {
  const v = ta.value;
  const sep = v && !v.endsWith(' ') && !v.endsWith('\n') ? ' ' : '';
  ta.value = `${v}${sep}#${tag} `;
  ta.focus();
  refresh();
}

// ── 事件 ────────────────────────────────────────────────────────

document.addEventListener('click', (e) => {
  const el = e.target.closest('[data-act]');
  if (!el) return;
  if (el.dataset.act === 'save') save();
  else if (el.dataset.act === 'close') close();
  else if (el.dataset.act === 'tag') insertTag(el.dataset.arg);
});

ta.addEventListener('input', async () => {
  refresh();
  const ed = await import('./editor.js');
  ed.updateSuggest(ta, box, { tags, memos });
});

ta.addEventListener('keydown', async (e) => {
  const ed = await import('./editor.js');
  if (ed.suggestKey(e, ta, box)) return;

  if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') { e.preventDefault(); save(); }
  else if (e.key === 'Escape') { e.preventDefault(); close(); }
});

box.addEventListener('click', async (e) => {
  const b = e.target.closest('button');
  if (!b) return;
  const ed = await import('./editor.js');
  ed.applySuggest(ta, box, b);
  refresh();
});

// 窗口每次显示都重新聚焦并刷新上下文——它常驻但隐藏，不会重新加载页面
window.addEventListener('focus', () => {
  ta.focus();
  loadContext();
});

(async () => {
  await loadContext();
  refresh();
  ta.focus();
})();
