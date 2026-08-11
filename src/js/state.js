/**
 * 应用状态。
 *
 * 前端只持有**当前视图需要的那一份快照**，不做本地全量副本——筛选、搜索、聚合
 * 一律回 Rust 侧重查。这条边界是故意画在这里的：一旦前端开始维护「全部片语」的
 * 数组，下一步必然是在它上面 `Array.filter`，而那正是 v1.0 要避免的。
 */

import * as store from './store.js';

/** 可以明文落盘的偏好（§10.1）。 */
const PREF_KEY = 'zhiyan:prefs:v1';

/**
 * **能放这里的只有不敏感内容。**
 *
 * 窗口位置、主题、字号可以；最近搜索词、标签列表不行——localStorage 在
 * WebView 里是明文的，把标签名写进去等于把用户的主题结构泄漏到磁盘上。
 */
const PERSISTED = ['theme', 'fontSize', 'goal', 'sort', 'tab'];

export const state = {
  /** 当前视图的片语快照。 */
  memos: [],
  /** 四个固定视图的计数，由 Rust 侧聚合。 */
  counts: { all: 0, today: 0, untagged: 0, trash: 0 },
  /** 统计、热力图、标签分布。同样由 Rust 侧 SQL 聚合完再过来。 */
  stats: null,
  /** 当前筛选下是否已经取到底。 */
  exhausted: false,

  // ── 筛选条件 ──
  view: 'all',
  tag: null,
  query: '',
  dayFilter: null,
  sort: 'new',

  // ── 编辑器 ──
  /** 非 null 表示正在编辑已有片语。 */
  editId: null,
  /** 待插入的图片（data URL）。阶段四改成 blob 句柄。 */
  drafts: [],

  // ── 右栏 ──
  tab: 'recall',
  recallId: null,
  reviewed: 0,
  reviewDay: '',
  scope: 'all',

  // ── 偏好 ──
  theme: 'light',
  fontSize: 15.5,
  goal: 3,

  /**
   * 标签的置顶与折叠。
   *
   * **暂存在内存里**：标签名是敏感的（「求职」「心理学」这类标签本身就说明问题），
   * 不能进明文的 localStorage。阶段四挪进加密库的 kv 表。
   */
  tagMeta: { pinned: [], closed: [] },
};

/** 从 localStorage 读回不敏感偏好。 */
export function loadPrefs() {
  try {
    const raw = localStorage.getItem(PREF_KEY);
    if (!raw) return;
    const saved = JSON.parse(raw);
    for (const k of PERSISTED) {
      if (saved[k] !== undefined) state[k] = saved[k];
    }
  } catch {
    // 存储被禁或内容损坏都不该让应用起不来，用默认值继续
  }
}

let prefTimer = null;
export function savePrefs() {
  clearTimeout(prefTimer);
  prefTimer = setTimeout(() => {
    try {
      const out = {};
      for (const k of PERSISTED) out[k] = state[k];
      localStorage.setItem(PREF_KEY, JSON.stringify(out));
    } catch {
      // 同上
    }
  }, 300);
}

/** 当前筛选条件，喂给 `store.listMemos`。 */
export function filter() {
  return {
    view: state.dayFilter ? 'day' : state.view,
    tag: state.dayFilter ?? state.tag,
    query: state.query,
    sort: state.sort,
  };
}

/**
 * 重新取数。
 *
 * 每次筛选条件变化、每次写入之后都要调它。看起来「多查了一次」，但这是让
 * 前端不持有全量副本的代价，也是它不会与真实数据不一致的原因。
 */
export async function reload() {
  // 一次取第一页 + 一次取聚合。两条命令并发发出去，不必串行等。
  const [memos, aggregate] = await Promise.all([
    store.listMemos(filter(), null, store.PAGE),
    store.stats(),
  ]);
  state.memos = memos;
  state.stats = aggregate;
  state.counts = aggregate.counts;
  // 不足一页说明后面没有了；有了它才知道要不要继续翻
  state.exhausted = memos.length < store.PAGE;
}

/**
 * 续取下一页。
 *
 * §9.6 要求 `list_memos` 必须分页——IPC 走 JSON 序列化，万级数据整包传输会
 * 明显卡顿。所以列表是「一页一页要」的，不是一次拉全。
 */
export async function loadMore() {
  if (state.exhausted || !state.memos.length) return false;
  const cursor = state.memos[state.memos.length - 1].id;
  const page = await store.listMemos(filter(), cursor, store.PAGE);
  state.memos = [...state.memos, ...page];
  state.exhausted = page.length < store.PAGE;
  return page.length > 0;
}

/** 按 id 找当前快照里的片语。渲染引用时用。 */
export const byId = (id) => state.memos.find((m) => m.id === id);

/** 重置筛选条件到「全部」。 */
export function resetFilter() {
  state.view = 'all';
  state.tag = null;
  state.dayFilter = null;
}
