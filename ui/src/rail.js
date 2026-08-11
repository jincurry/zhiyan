/**
 * 右栏：拾遗与统计。
 *
 * 「写下的东西只有回头看见才算数」——这两块不是统计功能，是产品的另一半。
 */

import { esc, agoLabel, dayKey } from './util.js';
import { render } from './markdown.js';
import { state, byId } from './state.js';
import * as api from './api.js';

const $ = (id) => document.getElementById(id);

let cache = null;

export async function refreshStats() {
  cache = await api.stats();
}

export function setTab(tab) {
  state.tab = tab;
  $('railRecall').hidden = tab !== 'recall';
  $('railStats').hidden = tab !== 'stats';
  $('tabRecall').classList.toggle('on', tab === 'recall');
  $('tabStats').classList.toggle('on', tab === 'stats');
}

// ── 拾遗 ────────────────────────────────────────────────────────

/**
 * 换一条拾遗。
 *
 * 池子优先取**两天前**的——刚写的东西还在脑子里，"回看"它没有意义。
 * 池子为空时才退回全部，免得新用户看到一片空白。
 */
export function rollRecall() {
  let pool = state.memos.filter((m) => m.createdAt < Date.now() - 2 * 864e5);
  if (state.scope !== 'all') {
    pool = pool.filter((m) => m.text.includes('#' + state.scope));
  }
  if (!pool.length) pool = state.memos;

  const el = $('recall');
  if (!pool.length) {
    el.innerHTML = '<div class="txt" style="color:var(--ink-3)">还没有可以回看的片语。</div>';
    state.recallId = null;
    return;
  }

  const m = pool[Math.floor(Math.random() * pool.length)];
  state.recallId = m.id;
  el.innerHTML =
    `<span class="ago">${esc(agoLabel(m.createdAt))}</span>` +
    `<div class="txt">${render(m.text, byId)}</div>`;

  const today = dayKey(Date.now());
  if (state.reviewDay !== today) {
    state.reviewDay = today;
    state.reviewed = 0;
  }
  state.reviewed++;
  drawRecallMeta();
}

function drawRecallMeta() {
  $('rvDone').textContent = state.reviewDay === dayKey(Date.now()) ? state.reviewed : 0;
  $('rvScope').textContent = state.scope === 'all' ? '全部' : '#' + state.scope;
}

// ── 统计 ────────────────────────────────────────────────────────

export function drawStats() {
  drawRecallMeta();
  if (!cache) return;

  const num = (id, value, unit) => {
    $(id).innerHTML = unit ? `${value}<small>${unit}</small>` : String(value);
  };

  num('sAll', cache.total);
  num('sWeek', cache.week);
  num('sStreak', cache.streak, '天');
  num('sMax', cache.longest, '天');
  num('sChars', cache.chars > 9999 ? (cache.chars / 1000).toFixed(1) + 'k' : cache.chars);
  num('sAvg', cache.perDayAvg.toFixed(1), '条/天');

  drawHeat();
  drawTopTags();
}

/** 半年热力图，每格一天。 */
function drawHeat() {
  const now = Date.now();
  $('heat').innerHTML = cache.heat
    .map((cell) => {
      const future = cell.at > now;
      const d = new Date(cell.at);
      const title = `${d.getMonth() + 1}月${d.getDate()}日 · ${cell.count} 条`;
      return (
        `<i class="l${cell.level}${state.dayFilter === cell.day ? ' sel' : ''}"` +
        (future ? ' style="opacity:.35"' : '') +
        ` title="${esc(title)}" data-act="day" data-arg="${esc(cell.day)}"></i>`
      );
    })
    .join('');
}

function drawTopTags(flat) {
  const counts = flat ?? topTagsSource;
  if (!counts) return;
  const top = Object.entries(counts)
    .reduce((acc, [tag, n]) => {
      const parent = tag.split('/')[0];
      acc[parent] = (acc[parent] || 0) + n;
      return acc;
    }, {});
  const rows = Object.entries(top).sort((a, b) => b[1] - a[1]).slice(0, 5);
  const max = rows.length ? rows[0][1] : 1;

  $('topTags').innerHTML = rows
    .map(
      ([tag, n]) =>
        '<div class="row">' +
        `<span style="width:52px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(tag)}</span>` +
        `<span class="bar"><i style="width:${Math.round((n / max) * 100)}%"></i></span>` +
        `<span class="num">${n}</span></div>`
    )
    .join('');
}

/** 标签分布的数据源由 `main.js` 在刷新时注入，免得 rail 也去调一次 IPC。 */
let topTagsSource = null;
export function setTagCounts(counts) {
  topTagsSource = counts;
}

/** 供每周回顾复用的统计快照。 */
export const statsSnapshot = () => cache;
