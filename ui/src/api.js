/**
 * IPC 边界。**这是前端里唯一允许调用 Rust 的地方。**
 *
 * 把它收成一个模块有两个实在的好处：
 *
 * 1. 换实现只改一处。阶段一它走内存 mock，阶段四换成真正的 `invoke`，
 *    上层一行不用动。
 * 2. 「哪些数据必须过 IPC」变得一目了然。散在各处调 `invoke` 的话，
 *    迟早会有人图省事把全量片语拉到前端再 `Array.filter`——那正是 v1.0 要
 *    避免的：JSON 序列化一万条片语的开销足以让界面卡住。
 *
 * ## 契约
 *
 * 所有返回值都是普通 JSON。片语的形状（`Memo`）在两端各有一份定义，
 * 靠 `test/api.test.js` 里的形状断言与 Rust 侧的序列化测试对齐。
 */

/** 跑在 Tauri 里还是普通浏览器里。 */
export const inTauri = typeof window !== 'undefined' && '__TAURI__' in window;

/**
 * 走 `withGlobalTauri` 暴露的全局对象，而不是 `import { invoke } from '@tauri-apps/api'`。
 *
 * 理由是前端刻意零依赖——没有打包器，裸模块说明符在浏览器里根本解析不了。
 * 代价是 `window.__TAURI__` 全局可见：真出了 XSS，攻击者能直接调 IPC。
 * 挡这一层的是 CSP（`script-src 'self'`，没有 `unsafe-inline`）与
 * capabilities 清单——能调的命令就那几条，且都不接受任意路径。
 */
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);

/**
 * 开发用的内存后端。
 *
 * **它不是产品的一部分**，只是让界面能在浏览器里独立开发与截图。
 * 它刻意保持「够用就好」：不做分页、不做 FTS、不做加密——真要那些行为，
 * 就该去跑真的后端，而不是把 mock 养成第二套实现。
 */
const mock = await (async () => {
  if (inTauri) return null;
  const { seed } = await import('./seed.js');
  let memos = seed();
  let trash = [];

  const nowId = () => crypto.randomUUID();
  const clone = (m) => JSON.parse(JSON.stringify(m));

  return {
    async list({ view = 'all', tag = null, query = '', sort = 'new' } = {}) {
      const { parseTags, isToday, dayKey } = await import('./util.js');
      if (view === 'trash') return trash.map(clone).sort((a, b) => b.deletedAt - a.deletedAt);
      let l = memos.filter((m) => {
        if (view === 'today' && !isToday(m.createdAt)) return false;
        if (view === 'untagged' && parseTags(m.text).length) return false;
        if (view === 'day' && dayKey(m.createdAt) !== tag) return false;
        if (tag && view !== 'day') {
          const tags = parseTags(m.text);
          if (!tags.some((t) => t === tag || t.startsWith(tag + '/'))) return false;
        }
        if (query && !m.text.toLowerCase().includes(query.toLowerCase())) return false;
        return true;
      });
      l.sort((a, b) => (sort === 'old' ? a.createdAt - b.createdAt : b.createdAt - a.createdAt));
      return l.map(clone);
    },
    async get(id) {
      return clone(memos.find((m) => m.id === id) ?? trash.find((m) => m.id === id) ?? null);
    },
    async upsert(input) {
      const now = Date.now();
      if (input.id) {
        const m = memos.find((x) => x.id === input.id);
        if (m) {
          Object.assign(m, { text: input.text, updatedAt: now, rev: m.rev + 1, edited: true });
          return clone(m);
        }
      }
      const m = {
        id: nowId(), createdAt: now, updatedAt: now, deletedAt: null, rev: 1,
        text: input.text, pinned: false, source: input.source ?? 'app',
        refs: input.refs ?? [], blobs: input.blobs ?? [], dirty: true,
      };
      memos.unshift(m);
      return clone(m);
    },
    async softDelete(id) {
      const i = memos.findIndex((m) => m.id === id);
      if (i < 0) return;
      const [m] = memos.splice(i, 1);
      m.deletedAt = Date.now();
      trash.unshift(m);
    },
    async restore(id) {
      const i = trash.findIndex((m) => m.id === id);
      if (i < 0) return;
      const [m] = trash.splice(i, 1);
      m.deletedAt = null;
      memos.unshift(m);
    },
    async purge(id) {
      trash = trash.filter((m) => m.id !== id);
    },
    async emptyTrash() {
      trash = [];
    },
    async setPinned(id, pinned) {
      const m = memos.find((x) => x.id === id);
      if (m) m.pinned = pinned;
    },
    async tagCounts() {
      const { parseTags } = await import('./util.js');
      const out = {};
      for (const m of memos) for (const t of parseTags(m.text)) out[t] = (out[t] || 0) + 1;
      return out;
    },
    async stats() {
      const { parseTags, dayKey, countChars, heatLevel } = await import('./util.js');
      const perDay = {};
      const charsPerDay = {};
      for (const m of memos) {
        const k = dayKey(m.createdAt);
        perDay[k] = (perDay[k] || 0) + 1;
        charsPerDay[k] = (charsPerDay[k] || 0) + countChars(m.text);
      }
      const days = Object.keys(perDay).sort((a, b) => new Date(a) - new Date(b));

      // 今天还没写不算断——早上打开应用就看到归零，是在惩罚用户还没开始写
      let current = 0;
      const cur = new Date();
      cur.setHours(0, 0, 0, 0);
      if (!perDay[dayKey(cur.getTime())]) cur.setDate(cur.getDate() - 1);
      while (perDay[dayKey(cur.getTime())]) { current++; cur.setDate(cur.getDate() - 1); }

      let longest = 0, run = 0, prev = null;
      for (const k of days) {
        const d = new Date(k);
        run = prev && (d - prev) / 864e5 <= 1.5 ? run + 1 : 1;
        longest = Math.max(longest, run);
        prev = d;
      }

      const totalChars = memos.reduce((a, m) => a + countChars(m.text), 0);
      const span = days.length ? Math.max(1, Math.round((Date.now() - new Date(days[0])) / 864e5)) : 1;
      const week = Date.now() - 7 * 864e5;

      // 热力图：最近 26 周，按日返回，档位由 Rust 侧算好
      const heat = [];
      const end = new Date();
      end.setHours(0, 0, 0, 0);
      end.setDate(end.getDate() + (6 - end.getDay()));
      for (let i = 181; i >= 0; i--) {
        const d = new Date(end);
        d.setDate(d.getDate() - i);
        const k = dayKey(d.getTime());
        const n = perDay[k] || 0;
        heat.push({ day: k, at: d.getTime(), count: n, level: heatLevel(n) });
      }

      return {
        total: memos.length,
        week: memos.filter((m) => m.createdAt > week).length,
        streak: current,
        longest,
        chars: totalChars,
        perDayAvg: memos.length / span,
        charsPerDay,
        heat,
      };
    },
    async counts() {
      const { parseTags, isToday } = await import('./util.js');
      return {
        all: memos.length,
        today: memos.filter((m) => isToday(m.createdAt)).length,
        untagged: memos.filter((m) => !parseTags(m.text).length).length,
        trash: trash.length,
      };
    },
    async appInfo() {
      return { version: '0.1.0-dev', platform: 'web', encryptedStore: false };
    },
  };
})();

/** 统一的调用出口：在 Tauri 里走 IPC，在浏览器里走 mock。 */
async function call(cmd, mockName, ...args) {
  if (inTauri) return invoke(cmd, args[0] ?? {});
  if (!mock || !mock[mockName]) throw new Error(`mock 缺少 ${mockName}`);
  return mock[mockName](...args);
}

// ── 片语 ────────────────────────────────────────────────────────

export const listMemos = (filter) => call('list_memos', 'list', filter);
export const getMemo = (id) => call('get_memo', 'get', id);
export const upsertMemo = (input) => call('upsert_memo', 'upsert', input);
export const softDeleteMemo = (id) => call('soft_delete_memo', 'softDelete', id);
export const restoreMemo = (id) => call('restore_memo', 'restore', id);
export const purgeMemo = (id) => call('purge_memo', 'purge', id);
export const emptyTrash = () => call('empty_trash', 'emptyTrash');
export const setPinned = (id, pinned) => call('set_pinned', 'setPinned', id, pinned);
export const viewCounts = () => call('view_counts', 'counts');

/**
 * 标签 → 片语数。侧栏的两级标签树从它构建。
 *
 * 由 Rust 侧 `GROUP BY` 出来，而不是前端遍历全量正文再 `parseTags`——
 * 后者要求前端持有全部片语，正是这道边界要挡住的事。
 */
export const tagCounts = () => call('tag_counts', 'tagCounts');

/** 统计与热力图。同样在 Rust 侧聚合完再过来。 */
export const stats = () => call('stats', 'stats');

// ── 应用 ────────────────────────────────────────────────────────

export const appInfo = () => call('app_info', 'appInfo');

// ── 窗口 ────────────────────────────────────────────────────────
//
// 自绘标题栏的三个按钮。浏览器里没有窗口可操作，静默忽略即可。

export const windowMinimize = () => (inTauri ? invoke('window_minimize') : null);
export const windowToggleMaximize = () => (inTauri ? invoke('window_toggle_maximize') : null);
export const windowHide = () => (inTauri ? invoke('window_hide') : null);
export const quickToggle = () => (inTauri ? invoke('quick_toggle') : null);

/**
 * 打开外部链接。
 *
 * **必须先过 `isSafeLink`**（调用方负责），这里再挡一道。两层都要有：
 * 渲染层挡的是「不生成 `<a>`」，这里挡的是「即使有人构造出点击事件也打不开」。
 */
export async function openExternal(url) {
  const { isSafeLink } = await import('./util.js');
  if (!isSafeLink(url)) {
    console.warn('拒绝打开非 http(s) 链接');
    return false;
  }
  if (inTauri) {
    await window.__TAURI__.opener.openUrl(url);
  } else {
    window.open(url, '_blank', 'noopener,noreferrer');
  }
  return true;
}
