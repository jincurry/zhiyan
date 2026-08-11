/**
 * 时间线：时间脊 + 卡片列表。
 *
 * 脊是 CSS 画的（`.group::before` 一条 1px 竖线 + `.node` 一个圆），不是 canvas
 * 也不是 SVG——这正是 Tauri 方案省事的地方，GPUI 那边同一件事要写一个自定义 Element。
 */

import { esc, dayKey, isToday, snippet, hhmm, countChars, nodeSize, WD, inlineRefs } from './util.js';
import { render, highlight } from './markdown.js';
import { state, byId } from './state.js';

const $ = (id) => document.getElementById(id);

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
    `<div class="txt">${render(m.text, byId)}</div>` +
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
    `<div class="txt">${render(m.text, byId)}</div>` +
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
