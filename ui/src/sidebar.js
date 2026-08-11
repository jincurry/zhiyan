/**
 * 侧栏：搜索、四个固定视图、两级标签树、每日目标。
 */

import { esc } from './util.js';
import { state } from './state.js';
import * as api from './api.js';

const $ = (id) => document.getElementById(id);

/** 标签计数缓存。写入后由 `refreshTags` 刷新，避免每次重绘都过一次 IPC。 */
let counts = {};

export async function refreshTags() {
  counts = await api.tagCounts();
}

/** 由扁平计数构建两级树。 */
export function buildTree(flat) {
  const tree = {};
  for (const [tag, n] of Object.entries(flat)) {
    const [parent, ...rest] = tag.split('/');
    const child = rest.join('/');
    tree[parent] ??= { n: 0, own: 0, kids: {} };
    tree[parent].n += n;
    if (child) tree[parent].kids[child] = (tree[parent].kids[child] || 0) + n;
    else tree[parent].own += n;
  }
  return tree;
}

export function drawSidebar() {
  drawNav();
  drawTags();
  drawGoal();
}

function drawNav() {
  const c = state.counts;
  $('nAll').textContent = c.all;
  $('nToday').textContent = c.today;
  $('nUn').textContent = c.untagged;
  $('nTrash').textContent = c.trash;

  const active = !state.tag && !state.dayFilter;
  for (const b of document.querySelectorAll('.navitem[data-view]')) {
    b.classList.toggle('on', active && b.dataset.view === state.view);
  }
}

function drawTags() {
  const tree = buildTree(counts);
  const keys = Object.keys(tree).sort((a, b) => {
    // 置顶的在前；其余按计数降序，同频按名字——顺序必须稳定，
    // 否则侧栏会在每次刷新时抖
    const pa = state.tagMeta.pinned.includes(a);
    const pb = state.tagMeta.pinned.includes(b);
    if (pa !== pb) return pa ? -1 : 1;
    return tree[b].n - tree[a].n || a.localeCompare(b, 'zh');
  });

  if (!keys.length) {
    $('tagList').innerHTML =
      '<p style="padding:2px 9px;font-size:12px;color:var(--ink-3)">用 #主题 归类，标签会出现在这里。</p>';
    return;
  }

  $('tagList').innerHTML = keys
    .map((parent) => {
      const node = tree[parent];
      const kids = Object.entries(node.kids).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0], 'zh'));
      const closed = state.tagMeta.closed.includes(parent);
      const pinned = state.tagMeta.pinned.includes(parent);

      let h =
        `<button class="navitem tag ${state.tag === parent ? 'on' : ''}" data-act="tag" data-arg="${esc(parent)}">` +
        (kids.length
          ? `<span class="caret ${closed ? 'closed' : ''}" data-act="tag-fold" data-arg="${esc(parent)}">` +
            '<svg width="9" height="9" viewBox="0 0 12 12" fill="currentColor"><path d="M2 4l4 4 4-4z"/></svg></span>'
          : '<span class="dot"></span>') +
        `<span class="lbl">${esc(parent)}</span>` +
        (pinned ? '<span class="pin">◆</span>' : '') +
        `<span class="more" data-act="tag-menu" data-arg="${esc(parent)}">⋯</span>` +
        `<span class="n">${node.n}</span></button>`;

      if (!closed) {
        h += kids
          .map(([child, n]) => {
            const full = `${parent}/${child}`;
            return (
              `<button class="navitem tag sub ${state.tag === full ? 'on' : ''}" style="padding-left:32px" ` +
              `data-act="tag" data-arg="${esc(full)}">` +
              `<span class="dot"></span><span class="lbl">${esc(child)}</span><span class="n">${n}</span></button>`
            );
          })
          .join('');
      }
      return h;
    })
    .join('');
}

function drawGoal() {
  const done = state.counts.today;
  $('goalTxt').textContent = `${done} / ${state.goal} 今日`;
  // goal 为 0 时不能除零，done 超过 goal 时进度条不能超过满格
  const pct = state.goal > 0 ? Math.min(100, (done / state.goal) * 100) : 0;
  $('goalBar').style.width = pct + '%';
}

/** 全部标签名，供命令面板与补全用。 */
export const allTags = () => Object.keys(counts);

/** 标签计数快照，供每周回顾与速记浮窗的常用标签用。 */
export const tagCountSnapshot = () => ({ ...counts });
