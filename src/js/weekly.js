/**
 * 每周回顾。
 *
 * **待办只在这里汇总**——不做独立的 Todo 页面（§1.3）。片语里的待办是写作的
 * 副产品，不是任务系统；给它一个独立页面就等于把它变成任务系统。
 */

import { esc, snippet, countChars, weekRange, dayKey } from './util.js';
import { openTodos } from './markdown.js';
import { sheet } from './overlays.js';
import * as store from './store.js';

const fullDate = (t) => {
  const d = new Date(t);
  return `${d.getMonth() + 1}月${d.getDate()}日`;
};
const shortDate = (t) => {
  const d = new Date(t);
  return `${d.getMonth() + 1}/${d.getDate()}`;
};
const isoDate = (t) => {
  const d = new Date(t);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
};

/** 某一周的聚合结果。抽出来是为了能脱离 DOM 测。 */
export function aggregate(memos, from, to) {
  const list = memos
    .filter((m) => m.createdAt >= from && m.createdAt < to)
    .sort((a, b) => a.createdAt - b.createdAt);

  const perDay = [0, 0, 0, 0, 0, 0, 0];
  for (const m of list) perDay[(new Date(m.createdAt).getDay() + 6) % 7]++;

  const chars = list.reduce((s, m) => s + countChars(m.text), 0);

  const tags = {};
  for (const m of list) {
    for (const t of m.text.matchAll(/(?:^|[\s(（])#([^\s#，。、；：!？"'()（）[\]]+)/g)) {
      const parent = t[1].split('/')[0];
      tags[parent] = (tags[parent] || 0) + 1;
    }
  }

  const todos = [];
  for (const m of list) {
    for (const t of openTodos(m.text)) todos.push({ id: m.id, at: m.createdAt, text: t });
  }

  return {
    list,
    total: list.length,
    chars,
    perDay,
    activeDays: new Set(list.map((m) => dayKey(m.createdAt))).size,
    topTags: Object.entries(tags).sort((a, b) => b[1] - a[1]).slice(0, 5),
    todos,
    // 「值得再看」按字数取——长的那几条通常是想清楚了才写的
    picks: list.slice().sort((a, b) => countChars(b.text) - countChars(a.text)).slice(0, 3),
  };
}

export async function openWeek(offset = 0) {
  const [from, to] = weekRange(offset);
  // 回顾要的是这一周的全部，不是当前筛选下的——所以带上区间单独查一次。
  // 早先是「拉两千条回来自己筛」，那会被 list_memos 的单次上限静默截断
  const all = await store.listAll({ view: 'all', sort: 'old', from, to });
  const a = aggregate(all, from, to);

  const thisWeek = offset === 0;
  const todayIdx = (new Date().getDay() + 6) % 7;
  const maxBar = Math.max(1, ...a.perDay);
  const tmax = a.topTags.length ? a.topTags[0][1] : 1;

  sheet(
    '每周回顾',
    `<div class="wknav">
       <button class="nb" data-act="week" data-arg="${offset - 1}">‹</button>
       <span class="rng">${fullDate(from)} – ${fullDate(to - 864e5)}${thisWeek ? ' · 本周' : ''}</span>
       <button class="nb" data-act="week" data-arg="${offset + 1}"${offset >= 0 ? ' disabled' : ''}>›</button>
       <span class="sp"><button class="btn" data-act="week-export" data-arg="${offset}">导出这周</button></span>
     </div>
     <div class="wkgrid">
       ${cell(a.total, '片语')}
       ${cell(a.chars > 9999 ? (a.chars / 1000).toFixed(1) + 'k' : a.chars, '字数')}
       ${cell(`${a.activeDays}<small>/7</small>`, '活跃天')}
       ${cell(a.total ? (a.chars / a.total).toFixed(0) : 0, '平均每条')}
     </div>
     <div class="wkbars">${a.perDay
       .map(
         (n, i) =>
           `<div class="col"><span class="cn">${n || ''}</span>` +
           `<div class="bar${thisWeek && i === todayIdx ? ' hi' : ''}" style="height:${n ? Math.max(4, (n / maxBar) * 58) : 3}px"></div></div>`
       )
       .join('')}</div>
     <div class="wkdays">${['一', '二', '三', '四', '五', '六', '日']
       .map((d, i) => `<span class="${thisWeek && i === todayIdx ? 'td' : ''}">${d}</span>`)
       .join('')}</div>

     <div class="wksec">标签</div>
     ${
       a.topTags.length
         ? a.topTags
             .map(
               ([t, n]) =>
                 `<div class="wktag"><span style="width:56px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(t)}</span>` +
                 `<span class="bar"><i style="width:${Math.round((n / tmax) * 100)}%"></i></span><span class="num">${n}</span></div>`
             )
             .join('')
         : '<p class="wkempty">这周没有用到标签。</p>'
     }

     <div class="wksec">待办 · 还没勾掉</div>
     ${
       a.todos.length
         ? a.todos
             .map(
               (t) =>
                 `<button class="wkline" data-act="jump" data-arg="${esc(t.id)}"><span class="box"></span>` +
                 `<span class="t">${esc(t.text.replace(/[*`~=]/g, ''))}</span>` +
                 `<span class="d" style="margin-left:auto">${shortDate(t.at)}</span></button>`
             )
             .join('')
         : '<p class="wkempty">没有留下未完成的待办。</p>'
     }

     <div class="wksec">值得再看</div>
     ${
       a.picks.length
         ? a.picks
             .map(
               (m) =>
                 `<button class="wkline" data-act="jump" data-arg="${esc(m.id)}">` +
                 `<span class="d">${shortDate(m.createdAt)}</span>` +
                 `<span class="t">${esc(snippet(m.text, 40))}</span></button>`
             )
             .join('')
         : '<p class="wkempty">这周还没有写下什么。开一条也不迟。</p>'
     }`
  );
}

function cell(value, label) {
  return `<div class="wkcell"><div class="v">${value}</div><div class="k">${esc(label)}</div></div>`;
}

/** 导出这一周为 Markdown。**导出的是明文**，这是它的本分（§10.7）。 */
export async function exportWeek(offset) {
  const [from, to] = weekRange(offset);
  const all = await store.listAll({ view: 'all', sort: 'old', from, to });
  const a = aggregate(all, from, to);

  let out = `# 每周回顾 ${isoDate(from)} – ${isoDate(to - 864e5)}\n\n`;
  out += `- 片语 ${a.total} 条，合计 ${a.chars} 字\n`;
  out += `- 活跃 ${a.activeDays} 天\n\n---\n\n`;
  out += a.list
    .map((m) => {
      const d = new Date(m.createdAt);
      const time = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
      return `## ${isoDate(m.createdAt)} ${time}\n\n${m.text}\n`;
    })
    .join('\n---\n\n');

  return { name: `知言-每周回顾-${isoDate(from)}.md`, text: out };
}
