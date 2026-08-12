/**
 * 浏览器里的内存后端。
 *
 * **不是产品的一部分**——只为让界面能脱离 Tauri 独立开发与截图。跑在 Tauri 里时
 * `store.js` 根本不会加载它。
 *
 * 刻意保持「够用就好」：不加密、不落盘、FTS 用 `includes` 顶。真要那些行为就去跑
 * 真后端，而不是把 mock 养成第二套实现——两套实现迟早会在某个边角上分叉，
 * 到时候你分不清是后端的 bug 还是 mock 的。
 *
 * 唯一认真对待的是**返回值的形状**：它必须与 Rust 侧的序列化结果一致，
 * 否则接线时才发现对不上。
 */

import { parseTags, isToday, dayKey, countChars, heatLevel } from './util.js';
import { seed } from './seed.js';

export async function createMock() {
  let memos = seed();
  let trash = [];
  const kv = new Map();

  const clone = (m) => (m ? JSON.parse(JSON.stringify(m)) : null);
  const find = (id) => memos.find((m) => m.id === id);

  function filtered({ view = 'all', tag = null, query = '', sort = 'new', from = null, to = null } = {}) {
    if (view === 'trash') return trash.slice().sort((a, b) => b.deletedAt - a.deletedAt);

    const list = memos.filter((m) => {
      if (view === 'today' && !isToday(m.createdAt)) return false;
      if (view === 'untagged' && parseTags(m.text).length) return false;
      if (view === 'day' && dayKey(m.createdAt) !== tag) return false;
      if (tag && view !== 'day') {
        const tags = parseTags(m.text);
        if (!tags.some((t) => t === tag || t.startsWith(tag + '/'))) return false;
      }
      if (query && !m.text.toLowerCase().includes(query.toLowerCase())) return false;
      // 显式区间：每周回顾用它取某一周
      if (from !== null && m.createdAt < from) return false;
      if (to !== null && m.createdAt >= to) return false;
      return true;
    });

    // 置顶永远在最前，其余按时间。与 Rust 侧的 ORDER BY 保持一致。
    return list.sort((a, b) => {
      if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
      return sort === 'old' ? a.createdAt - b.createdAt : b.createdAt - a.createdAt;
    });
  }

  return {
    /** 键集分页：游标是上一页最后一条的 id。 */
    async listMemos({ filter, cursor, limit }) {
      const list = filtered(filter);
      const start = cursor ? list.findIndex((m) => m.id === cursor) + 1 : 0;
      return list.slice(start, start + limit).map(clone);
    },

    async getMemo({ id }) {
      return clone(find(id) ?? trash.find((m) => m.id === id));
    },

    async upsertMemo({ input }) {
      const now = Date.now();
      const existing = input.id && find(input.id);
      if (existing) {
        Object.assign(existing, {
          text: input.text ?? existing.text,
          pinned: input.pinned ?? existing.pinned,
          updatedAt: now,
          rev: existing.rev + 1,
          edited: true,
          dirty: true,
        });
        return clone(existing);
      }
      const m = {
        id: crypto.randomUUID(),
        createdAt: now,
        updatedAt: now,
        deletedAt: null,
        rev: 1,
        text: input.text ?? '',
        pinned: false,
        source: input.source ?? 'app',
        tags: parseTags(input.text ?? ''),
        refs: input.refs ?? [],
        blobs: input.blobs ?? [],
        dirty: true,
      };
      memos.unshift(m);
      return clone(m);
    },

    async softDelete({ id }) {
      const i = memos.findIndex((m) => m.id === id);
      if (i < 0) return;
      const [m] = memos.splice(i, 1);
      m.deletedAt = Date.now();
      trash.unshift(m);
    },

    async restore({ id }) {
      const i = trash.findIndex((m) => m.id === id);
      if (i < 0) return;
      const [m] = trash.splice(i, 1);
      m.deletedAt = null;
      memos.unshift(m);
    },

    // 墓碑：正文清空但行还在。物理删掉的话，别的设备下次同步会把它推回来。
    async purge({ id }) {
      const m = trash.find((x) => x.id === id);
      if (m) {
        m.text = '';
        m.tags = [];
        m.tombstone = true;
      }
      trash = trash.filter((x) => x.id !== id);
    },

    async purgeAll() {
      trash = [];
    },

    async setPinned({ id, pinned }) {
      const m = find(id);
      if (m) {
        m.pinned = pinned;
        m.updatedAt = Date.now();
        m.dirty = true;
      }
    },

    async search({ q, limit }) {
      return filtered({ query: q }).slice(0, limit).map(clone);
    },

    /** 统计、热力图、标签分布、视图计数，一次给全。 */
    async stats() {
      const perDay = {};
      for (const m of memos) {
        const k = dayKey(m.createdAt);
        perDay[k] = (perDay[k] || 0) + 1;
      }
      const days = Object.keys(perDay).sort((a, b) => new Date(a) - new Date(b));

      // 今天还没写不算断——早上打开应用就看到归零，是在惩罚用户还没开始写
      let streak = 0;
      const cur = new Date();
      cur.setHours(0, 0, 0, 0);
      if (!perDay[dayKey(cur.getTime())]) cur.setDate(cur.getDate() - 1);
      while (perDay[dayKey(cur.getTime())]) {
        streak++;
        cur.setDate(cur.getDate() - 1);
      }

      let longest = 0;
      let run = 0;
      let prev = null;
      for (const k of days) {
        const d = new Date(k);
        run = prev && (d - prev) / 864e5 <= 1.5 ? run + 1 : 1;
        longest = Math.max(longest, run);
        prev = d;
      }

      // 半年热力图，每格一天，档位在这一侧算好
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

      const tags = {};
      for (const m of memos) for (const t of parseTags(m.text)) tags[t] = (tags[t] || 0) + 1;

      const chars = memos.reduce((a, m) => a + countChars(m.text), 0);
      const span = days.length
        ? Math.max(1, Math.round((Date.now() - new Date(days[0])) / 864e5))
        : 1;

      return {
        total: memos.length,
        week: memos.filter((m) => m.createdAt > Date.now() - 7 * 864e5).length,
        streak,
        longest,
        chars,
        perDayAvg: memos.length / span,
        heat,
        tags,
        counts: {
          all: memos.length,
          today: memos.filter((m) => isToday(m.createdAt)).length,
          untagged: memos.filter((m) => !parseTags(m.text).length).length,
          trash: trash.length,
        },
      };
    },

    async backlinks({ id, limit }) {
      return memos
        .filter((m) => m.id !== id && m.text.includes(`^${id}]]`))
        .slice(0, limit)
        .map(clone);
    },

    async putBlob() {
      throw new Error('mock 不支持附件：它需要真实的内容寻址存储');
    },

    async exportTo() {
      throw new Error('mock 不支持导出：落盘只能由 Rust 侧做');
    },

    async importLegacy() {
      throw new Error('mock 不支持迁移：它要写真实的库');
    },

    async runGc() {
      return { tombstones: 0, blobs: 0 };
    },

    // 浏览器里没有密钥，锁定是个空操作。做成 no-op 而不是抛错，
    // 是为了让「空闲锁定」这类逻辑在浏览器里也能走完整条路径
    async lockStore() {},
    async unlockStore() {},
    async forgetDevice() {},

    async getKv({ k }) {
      return kv.get(k) ?? null;
    },
    async setKv({ k, v }) {
      kv.set(k, v);
    },

    async appInfo() {
      return { version: '0.1.0-dev', platform: 'web', encryptedStore: false, unlocked: true };
    },
  };
}
