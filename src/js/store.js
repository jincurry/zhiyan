/**
 * 数据访问。**这是前端里唯一调用 `invoke()` 的模块**（§4.5 / §6.1）。
 *
 * 铁律：所有持久化只经 Rust 侧（§4.4）。前端不碰文件系统，加密、同步、迁移
 * 只有一处实现。
 *
 * ## 命令与 §9.6 一一对应
 *
 * `list_memos` / `get_memo` / `upsert_memo` / `soft_delete` / `restore` / `purge` /
 * `search` / `stats` / `put_blob` / `export` / `get_kv` / `set_kv`。
 *
 * 两处**在文档清单之外**的补充，理由写在各自的函数上：`purge_all`、`set_pinned`。
 *
 * ## 性能约束（§9.6，本方案特有）
 *
 * - `list_memos` **必须分页**。IPC 走 JSON 序列化，万级数据整包传输会明显卡顿。
 * - 统计与热力图**在 Rust 侧用 SQL 聚合算完再返回**，不要把原始数据搬到前端算。
 * - `put_blob` 超过 10MB 的图片建议先在前端压缩。
 *
 * 这一整组约束是 IPC 边界的产物，GPUI 方案下不存在。
 */

import { invoke } from '@tauri-apps/api/core';

/** 跑在 Tauri 里还是普通浏览器里。浏览器下退回内存 mock，便于脱离后端开发界面。 */
export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** 一页的默认条数。§6.3 给的建议值。 */
export const PAGE = 50;

/**
 * 开发用的内存后端。
 *
 * 用 `import.meta.env.DEV` 而不是运行时判断 `inTauri`：前者是 Vite 的编译期
 * 常量，生产构建里整个分支连同 `mock.js`、`seed.js` 会被摇掉。
 * 只用运行时判断的话，那些假数据会跟着进安装包——既白占体积，
 * 也让「这条笔记哪来的」变成一个不该存在的问题。
 */
const mock =
  import.meta.env.DEV && !inTauri ? await (await import('./mock.js')).createMock() : null;

async function call(cmd, mockName, args = {}) {
  if (inTauri) return invoke(cmd, args);
  if (!mock?.[mockName]) throw new Error(`mock 缺少 ${mockName}`);
  return mock[mockName](args);
}

// ── 片语 ────────────────────────────────────────────────────────

/**
 * 取一页片语。
 *
 * @param {object} filter `{ view, tag, query, sort }`
 * @param {string|null} cursor 上一页最后一条的 id；`null` 表示第一页
 * @param {number} limit
 */
export const listMemos = (filter, cursor = null, limit = PAGE) =>
  call('list_memos', 'listMemos', { filter, cursor, limit });

export const getMemo = (id) => call('get_memo', 'getMemo', { id });

/** 新建或更新。**Rust 侧内部解析标签与引用并写三张表**，前端不预先算。 */
export const upsertMemo = (input) => call('upsert_memo', 'upsertMemo', { input });

export const softDelete = (id) => call('soft_delete', 'softDelete', { id });
export const restore = (id) => call('restore', 'restore', { id });

/** 彻底删除 = **写墓碑，不是物理删**（§7.3 ②）。 */
export const purge = (id) => call('purge', 'purge', { id });

/**
 * 清空回收站。**不在 §9.6 的清单里**，是我加的。
 *
 * 替代方案是前端遍历回收站逐条 `purge`，那会变成 N 次 IPC 往返；
 * 而「清空」在语义上本来就是一次事务，拆成 N 次还会出现删到一半失败的中间态。
 */
export const purgeAll = () => call('purge_all', 'purgeAll');

/**
 * 置顶开关。**不在 §9.6 的清单里**，是我加的。
 *
 * 走 `upsert_memo` 也能做到，但那需要前端把整条正文回传一遍——
 * 为翻一个布尔值搬运整篇文字，在 IPC 边界上是明显的浪费。
 */
export const setPinned = (id, pinned) => call('set_pinned', 'setPinned', { id, pinned });

/** 全文搜索。走 FTS5 trigram（附录 A 的 P1 项）。 */
export const search = (q, limit = PAGE) => call('search', 'search', { q, limit });

// ── 聚合 ────────────────────────────────────────────────────────

/**
 * 统计、热力图、标签分布、视图计数。
 *
 * 全部在 Rust 侧用 SQL 聚合完再返回。合成一个命令而不是拆四个，是因为
 * 它们每次都一起刷新——拆开就是四次 IPC 往返换同一份数据。
 */
export const stats = (range = 'half-year') => call('stats', 'stats', { range });

// ── 附件 ────────────────────────────────────────────────────────

/** 存一张图。返回内容寻址的元信息，正文里只留 hash。 */
export const putBlob = (bytes, mime) => call('put_blob', 'putBlob', { bytes: [...bytes], mime });

// ── 导出与 kv ───────────────────────────────────────────────────

/** 导出。`fmt` 为 `markdown` / `json` / `json-encrypted`。返回落盘路径。 */
export const exportTo = (fmt) => call('export', 'exportTo', { fmt });

/**
 * kv：同步游标一类的东西。
 *
 * **不放敏感内容**——它对应 §9.1 里那份明文的 `state.json`。
 * 窗口位置、主题、字号可以；最近搜索词、标签列表不行。
 */
export const getKv = (k) => call('get_kv', 'getKv', { k });
export const setKv = (k, v) => call('set_kv', 'setKv', { k, v });

// ── 应用信息 ────────────────────────────────────────────────────

export const appInfo = () => call('app_info', 'appInfo');
