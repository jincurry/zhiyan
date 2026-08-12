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

/**
 * 本地时区相对 UTC 的偏移，**分钟**（东八区是 +480）。
 *
 * ## 为什么由前端传给 Rust，而不是 Rust 自己去问系统
 *
 * 「今天」「某一天」「热力图的每一格」都要先定下时区。这个口径必须与
 * `util.js` 的 `dayKey` 完全一致，而那份用的是 WebView 的本地时间。
 *
 * Rust 侧自己去问有两个问题：一是 `time` 的 `local-offset` 在多线程进程里读
 * 环境变量不安全（Unix 上直接返回错误）；二是就算读到了，也未必与 WebView
 * 认为的一致。不一致的表现是热力图整片错位一格，而且只在部分用户身上出现。
 *
 * 每次调用现算而不是缓存：用户可能改系统时区，也可能跨时区飞行。
 */
const tzOffset = () => -new Date().getTimezoneOffset();

// ── 片语 ────────────────────────────────────────────────────────

/**
 * 取一页片语。
 *
 * @param {object} filter `{ view, tag, query, sort }`
 * @param {string|null} cursor 上一页最后一条的 id；`null` 表示第一页
 * @param {number} limit
 */
export const listMemos = (filter, cursor = null, limit = PAGE) =>
  call('list_memos', 'listMemos', {
    filter: { ...filter, tzOffset: tzOffset() },
    cursor,
    limit,
  });

/**
 * 取满足条件的**全部**片语，内部自己翻页。
 *
 * `list_memos` 的单次上限是 500（Rust 侧钉死，见 §9.6），直接传一个大数字
 * 会被**静默截断**——那种 bug 的表现是「每周回顾少了几条」，没人会去查。
 * 要整段数据时走这里。
 *
 * `cap` 是安全阀，不是分页大小。真要取上万条的场景应该改用 Rust 侧的聚合命令，
 * 而不是把数据搬到前端来算。
 */
export async function listAll(filter, cap = 5000) {
  const out = [];
  let cursor = null;
  while (out.length < cap) {
    const page = await listMemos(filter, cursor, PAGE);
    if (!page.length) break;
    out.push(...page);
    cursor = page[page.length - 1].id;
    if (page.length < PAGE) break;
  }
  return out;
}

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
export const stats = (range = 'half-year') =>
  call('stats', 'stats', { range, tzOffset: tzOffset() });

/** 反向链接：哪些片语引用了这一条。 */
export const backlinks = (id, limit = 20) => call('backlinks', 'backlinks', { id, limit });

// ── 附件 ────────────────────────────────────────────────────────

/**
 * 存一张图。返回内容寻址的元信息，正文里只留 hash。
 *
 * 宽高由这边给——`<img>` 上直接读得到。Rust 侧为了两个整数把图片解码库拖进
 * 依赖树，换来的是安装包体积和一批解析器攻击面。
 */
export const putBlob = (bytes, mime, w = null, h = null) =>
  call('put_blob', 'putBlob', { bytes: [...bytes], mime, w, h });

/**
 * 附件的可显示地址。
 *
 * **不是 `blob:` 也不是 `data:`**：走 `zhiyan://` 自定义协议，密文在 Rust 侧
 * 读进来、在进程内解密、字节直接进 WebView，全程不落地。
 *
 * 曾经的做法是解密到临时文件再 `file://` 加载——那等于把明文写回磁盘，
 * 而且那个临时文件的生命周期没人管得住（§12.2）。
 */
export const blobUrl = (sha256) => (inTauri ? `zhiyan://blob/${sha256}` : '');
export const thumbUrl = (sha256) => (inTauri ? `zhiyan://thumb/${sha256}` : '');

// ── 导出与 kv ───────────────────────────────────────────────────

/**
 * 导出。`fmt` 为 `markdown` / `json` / `zbk`。返回落盘路径，用户取消时返回 `null`。
 *
 * **路径由用户在系统对话框里选，前端不传路径**（§12.1）：能传的话，
 * 一个 XSS 就能把正文写到任意位置。
 *
 * `markdown` 与 `json` 是**明文**的（§9.7），调用前必须让用户确认。
 */
export const exportTo = (fmt) => call('export', 'exportTo', { fmt, tzOffset: tzOffset() });

/** 墓碑与孤儿附件回收。启动时跑一次（§7.3 ②：墓碑保留 90 天）。 */
export const runGc = () => call('run_gc', 'runGc');

/**
 * kv：同步游标一类的东西。
 *
 * **不放敏感内容**——它对应 §9.1 里那份明文的 `state.json`。
 * 窗口位置、主题、字号可以；最近搜索词、标签列表不行。
 */
export const getKv = (k) => call('get_kv', 'getKv', { k });
export const setKv = (k, v) => call('set_kv', 'setKv', { k, v });

// ── 迁移（§7.3）─────────────────────────────────────────────────

/**
 * 从原型导出的整包 JSON 迁移过来。
 *
 * Rust 侧会做三件事：整数主键换成 UUIDv4、正文里的 `[[摘要^数字]]` 跟着改写、
 * base64 的图落成内容寻址的附件。少做任何一件数据就是坏的——尤其是第二件，
 * 漏了的话所有行内引用会**静默**指空。
 *
 * **只能在空库上跑**：这个操作不幂等，重跑一次等于把每条笔记复制一份。
 */
export const importLegacy = (json) => call('import_legacy', 'importLegacy', { json });

// ── 锁定（§9.4）─────────────────────────────────────────────────

/** 立刻锁定：清掉内存里的密钥并关库。空闲计时在前端，到点了调这条。 */
export const lockStore = () => call('lock_store', 'lockStore');
export const unlockStore = () => call('unlock_store', 'unlockStore');

/**
 * 本机停用：删掉凭据管理器里的密钥。
 *
 * **它删不掉已经落在本地的数据**——库还在，只是再也打不开。
 * 界面上必须说明白，不能让用户以为这是「远程擦除」。
 */
export const forgetDevice = () => call('forget_device', 'forgetDevice');

// ── 应用信息 ────────────────────────────────────────────────────

export const appInfo = () => call('app_info', 'appInfo');
