/**
 * DOM 渲染：时间线、侧栏、右栏。
 *
 * 按 §4.5 与 §6.1 的分工，这一层**只负责生成 DOM，不含业务判断**——
 * 要显示什么由 state 决定，怎么变由 main.js 决定，这里只管画。
 *
 * 三块合在一个模块里是因为它们共享同一份 state 快照与同一次重绘节奏：
 * 拆开会变成三处各自 reload 一次，多跑两趟 IPC。
 */

import {
  esc, dayKey, isToday, snippet, hhmm, countChars, nodeSize, WD, inlineRefs, agoLabel,
} from './util.js';
import { render as md, highlight } from './markdown.js';
import { state, byId } from './state.js';
import * as store from './store.js';

const $ = (id) => document.getElementById(id);


// ══════════ 时间线 ══════════

const SRC_LABEL = { quick: '速记', wechat: '微信', telegram: 'TG', import: '导入' };

/** 画整条时间线。 */
export function drawTimeline() {
  const tl = $('timeline');
  const list = state.memos;

  drawFilterbar(list.length);

  if (state.view === 'trash') {
    tl.innerHTML = list.length
      ? list.map(trashCard).join('')
      : empty('回收站是空的。', '删掉的片语会先放在这里，随时可以捞回来。');
    return;
  }

  if (!list.length) {
    const narrowed = state.query || state.tag || state.dayFilter;
    tl.innerHTML = empty(
      '这里还什么都没有。',
      narrowed ? '换个条件试试，或者把它写成一条新的片语。' : '在上面写第一句，Ctrl+Enter 存下。'
    );
    return;
  }

  let html = '';
  const pinned = list.filter((m) => m.pinned);
  const rest = list.filter((m) => !m.pinned);

  // 搜索或按天筛选时不单独分出置顶组——那时用户要的是「符合条件的全部」，
  // 把两条置顶提到最前反而打断了结果的连续性
  const separatePins = pinned.length && !state.query && !state.dayFilter;
  if (separatePins) {
    html +=
      '<section class="group pinned"><div class="gdate">置顶<span class="wd">PIN</span></div>' +
      '<div class="node" style="width:9px;height:9px"></div>' +
      pinned.map(card).join('') +
      '</section>';
  }

  const source = separatePins ? rest : list;
  const groups = new Map();
  for (const m of source) {
    const k = dayKey(m.createdAt);
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k).push(m);
  }

  for (const ms of groups.values()) {
    const d = new Date(ms[0].createdAt);
    const chars = ms.reduce((s, m) => s + countChars(m.text), 0);
    const size = nodeSize(chars);
    const today = isToday(ms[0].createdAt);
    // 节点大小编码当天字数——这是时间脊在信息密度上的全部意义
    const cls = today ? 'today' : chars > 200 ? 'fill' : '';
    html +=
      `<section class="group"><div class="gdate">${d.getMonth() + 1}月${d.getDate()}日` +
      `<span class="wd">${WD[d.getDay()]}</span></div>` +
      `<div class="node ${cls}" style="width:${size}px;height:${size}px" title="${chars} 字"></div>` +
      ms.map(card).join('') +
      '</section>';
  }

  tl.innerHTML = html;
  // 高亮必须在 innerHTML 之后、且只走文本节点，否则会改到属性里去
  for (const el of tl.querySelectorAll('.txt')) highlight(el, state.query);
}

function empty(a, b) {
  return `<div class="empty"><p>${esc(a)}</p><p>${esc(b)}</p></div>`;
}

/** 一张卡片。 */
function card(m) {
  const related = relations(m);
  const badge =
    m.source && m.source !== 'app'
      ? `<span class="src">${esc(SRC_LABEL[m.source] ?? m.source)}</span>`
      : '';

  return (
    `<article class="memo" data-id="${esc(m.id)}">` +
    `<div class="txt">${md(m.text, byId)}</div>` +
    images(m) +
    related +
    '<div class="mfoot">' +
    `<span class="mtime">${m.pinned ? '<span class="pinflag">◆</span>' : ''}${hhmm(m.createdAt)}` +
    badge +
    (m.edited ? '<span class="src">已编辑</span>' : '') +
    '</span>' +
    '<div class="mact">' +
    `<button class="${m.pinned ? 'on' : ''}" title="${m.pinned ? '取消置顶' : '置顶'}" data-act="memo-pin">◆</button>` +
    '<button title="分享成图" data-act="memo-share">' +
    '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M12 15V4M12 4L8 8M12 4l4 4"/><path d="M4 14v4a2 2 0 002 2h12a2 2 0 002-2v-4"/></svg></button>' +
    '<button title="复制 Markdown" data-act="memo-copy">' +
    '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V5a2 2 0 012-2h8"/></svg></button>' +
    '<button title="编辑" data-act="memo-edit">' +
    '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M4 20h4L19 9l-4-4L4 16z"/></svg></button>' +
    '<button class="dang" title="删除" data-act="memo-del">' +
    '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M5 7h14M9 7V5h6v2M7 7l1 13h8l1-13"/></svg></button>' +
    '</div></div></article>'
  );
}

/**
 * 关联与反向链接。
 *
 * 只在**当前快照**里找反向链接——完整的反向链接要回 Rust 侧查（阶段四的
 * `backlinks` 命令）。这里宁可少显示几条，也不把全量片语拉到前端来算。
 */
function relations(m) {
  const outgoing = [...new Set([...(m.refs ?? []), ...inlineRefs(m.text)])]
    .map(byId)
    .filter(Boolean);
  const incoming = state.memos.filter(
    (x) => x.id !== m.id && inlineRefs(x.text).includes(m.id) && !outgoing.some((o) => o.id === x.id)
  );

  const all = [
    ...outgoing.map((r) => [r, '关联']),
    ...incoming.map((r) => [r, '被引']),
  ];
  if (!all.length) return '';

  return (
    `<div class="rel"><span class="lab">${all.length} 条关联</span>` +
    all
      .map(
        ([r, kind]) =>
          `<a data-act="jump" data-arg="${esc(r.id)}"><span>${kind}</span><em>${esc(snippet(r.text, 26))}</em></a>`
      )
      .join('') +
    '</div>'
  );
}

/**
 * 附件缩略图。
 *
 * 阶段一里 `blobs` 还是空的。阶段四会把它换成 `asset:` 或自定义协议的 URL——
 * **不能写临时文件**，那等于把解密后的图片明文落盘，加密就白做了。
 */
function images(m) {
  if (!m.blobs?.length) return '';
  return (
    '<div class="mimg">' +
    m.blobs.map((b) => `<img src="${esc(b.url ?? '')}" alt="" data-act="lightbox" data-arg="${esc(b.url ?? '')}">`).join('') +
    '</div>'
  );
}

function trashCard(m) {
  const d = new Date(m.deletedAt);
  return (
    `<article class="memo" data-id="${esc(m.id)}" style="opacity:.82">` +
    `<div class="txt">${md(m.text, byId)}</div>` +
    `<div class="mfoot"><span class="mtime">删于 ${d.getMonth() + 1}/${d.getDate()} ${hhmm(m.deletedAt)}</span>` +
    '<div class="mact" style="opacity:1">' +
    '<button class="ghost" style="width:auto;padding:3px 9px;font-size:11.5px" data-act="memo-restore">恢复</button>' +
    '<button class="ghost dang" style="width:auto;padding:3px 9px;font-size:11.5px" data-act="memo-purge">彻底删除</button>' +
    '</div></div></article>'
  );
}

/** 筛选条与排序切换。 */
function drawFilterbar(n) {
  let chips = '';
  if (state.tag) {
    chips += `<span class="fx">#${esc(state.tag)}<button data-act="tag" data-arg="">✕</button></span>`;
  }
  if (state.dayFilter) {
    const [, mo, dd] = state.dayFilter.split('-');
    chips += `<span class="fx">${mo}月${dd}日<button data-act="day" data-arg="">✕</button></span>`;
  }
  if (state.query) {
    chips += `<span class="fx">“${esc(state.query)}”<button data-act="clear-query">✕</button></span>`;
  }

  const tail =
    state.view === 'trash'
      ? '<span class="sp"><button class="ghost" data-act="empty-trash">清空回收站</button></span>'
      : `<span class="sp"><button class="${state.sort === 'new' ? 'on' : ''}" data-act="sort" data-arg="new">最新</button>` +
        `<button class="${state.sort === 'old' ? 'on' : ''}" data-act="sort" data-arg="old">最早</button></span>`;

  $('filterbar').innerHTML = `${chips}<span>${n} 条</span>${tail}`;
}

// ══════════ 侧栏 ══════════

/** 标签计数缓存。写入后由 `refreshTags` 刷新，避免每次重绘都过一次 IPC。 */
let counts = {};

/** 标签计数从 state.stats 里取——它与统计同源，不再单独过一次 IPC。 */
export function refreshTags() {
  counts = state.stats?.tags ?? {};
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

// ══════════ 右栏 ══════════

let cache = null;

export function refreshStats() {
  cache = state.stats;
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
    `<div class="txt">${md(m.text, byId)}</div>`;

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
