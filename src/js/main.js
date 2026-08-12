/**
 * 装配层：把各模块接起来，并承载**全部事件委托**。
 *
 * 这里是唯一注册 DOM 监听的地方。原型用的是满屏 `onclick`，在
 * `script-src 'self'` 的 CSP 下一律不执行——所有交互改走 `data-act`，
 * 由 `ACTIONS` 表分发。
 *
 * 这么做除了满足 CSP，还有一个附带好处：能做什么一目了然，就是这张表。
 */

import * as store from './store.js';
import * as platform from './platform.js';
import { state, loadPrefs, savePrefs, reload, byId, resetFilter } from './state.js';
import { esc } from './util.js';
import { toggleTodo } from './markdown.js';
import {
  drawTimeline, drawSidebar, refreshTags, allTags, tagCountSnapshot,
  refreshStats, drawStats, setTab, rollRecall, setTagCounts,
} from './render.js';
import * as ed from './editor.js';
import * as ov from './overlays.js';
import * as palette from './commands.js';
import { openWeek, exportWeek } from './weekly.js';
import { drawCard, themeFromCss } from './share.js';

const $ = (id) => document.getElementById(id);

// ── 重绘 ────────────────────────────────────────────────────────

/** 取数 + 全量重绘。写入与筛选变化之后调它。 */
async function refresh() {
  await reload();
  refreshTags();
  refreshStats();
  setTagCounts(tagCountSnapshot());
  draw();
}

/** 只重绘，不取数。纯 UI 状态（折叠、排序高亮）变化时用。 */
function draw() {
  document.documentElement.style.setProperty('--fs', state.fontSize + 'px');
  drawTimeline();
  drawSidebar();
  drawStats();
}

function scrollTop() {
  $('main').scrollTo({ top: 0 });
}

// ── 导航 ────────────────────────────────────────────────────────

async function setView(view) {
  state.view = view;
  state.tag = null;
  state.dayFilter = null;
  await refresh();
  scrollTop();
}

async function pickTag(tag) {
  state.tag = state.tag === tag || !tag ? null : tag;
  state.view = 'all';
  state.dayFilter = null;
  await refresh();
  scrollTop();
}

async function setDay(day) {
  state.dayFilter = day || null;
  state.view = 'all';
  state.tag = null;
  await refresh();
  scrollTop();
}

/** 跳到某条片语并闪一下。找不到（被筛掉了）就先清筛选再跳。 */
async function jumpTo(id) {
  if (!byId(id)) {
    resetFilter();
    state.query = '';
    $('q').value = '';
    await refresh();
  }
  requestAnimationFrame(() => {
    const el = document.querySelector(`.memo[data-id="${CSS.escape(id)}"]`);
    if (!el) return;
    el.scrollIntoView({ block: 'center' });
    el.classList.add('flash');
    setTimeout(() => el.classList.remove('flash'), 1600);
  });
}

// ── 片语操作 ────────────────────────────────────────────────────

async function save() {
  const text = $('input').value.trim();
  if (!text && !state.drafts.length) return;

  const editing = state.editId;
  await store.upsertMemo({ id: editing, text, source: 'app', blobs: state.drafts });
  ed.clearComposer();
  await refresh();
  ov.toast(editing ? '已更新' : '已存下');
  if (!editing) scrollTop();
}

/**
 * 选图。
 *
 * 用 `<input type="file">` 而不是 Tauri 的文件对话框：这样**不需要任何
 * 文件系统权限**——浏览器把选中的字节直接给我们，路径从头到尾没出现过。
 * 开 `fs:allow-*` 才是把攻击面拉开的做法（§12.1）。
 */
function pickImage() {
  const input = document.createElement('input');
  input.type = 'file';
  input.accept = 'image/png,image/jpeg,image/gif,image/webp,image/avif';
  input.multiple = true;
  input.addEventListener('change', () => ed.ingestFiles([...input.files], (m) => ov.toast(m)));
  input.click();
}

async function deleteMemo(id) {
  await store.softDelete(id);
  if (state.recallId === id) state.recallId = null;
  await refresh();
  ov.toast('已移入回收站', '撤销', async () => {
    await store.restore(id);
    await refresh();
  });
}

async function toggleTodoAt(id, index) {
  const memo = byId(id);
  if (!memo) return;
  // 勾选**改写源文本**——待办的状态就存在正文里，没有第二份真相
  await store.upsertMemo({ id, text: toggleTodo(memo.text, index) });
  await refresh();
}

async function copyMemo(id) {
  const memo = byId(id);
  if (!memo) return;
  try {
    await navigator.clipboard.writeText(memo.text);
    ov.toast('已复制 Markdown');
  } catch {
    ov.toast('复制失败');
  }
}

function shareMemo(id) {
  const memo = byId(id);
  if (!memo) return;
  ov.sheet(
    '分享成图',
    '<div class="sharecard"><canvas id="shareCv"></canvas></div>' +
      '<div class="btnrow" style="margin-top:12px">' +
      `<button class="btn solid" data-act="share-save" data-arg="${esc(id)}">保存图片</button>` +
      `<button class="btn" data-act="memo-copy-id" data-arg="${esc(id)}">复制 Markdown</button></div>`
  );
  drawCard($('shareCv'), memo, themeFromCss());
}

function saveShare(id) {
  const cv = $('shareCv');
  if (!cv) return;
  download(`知言-${Date.now()}.png`, cv.toDataURL('image/png'), true);
  ov.toast('已保存图片');
}

/** 下载。Tauri 里 `<a download>` 由 WebView 处理，行为与浏览器一致。 */
function download(name, content, isDataUrl = false) {
  const a = document.createElement('a');
  a.download = name;
  if (isDataUrl) {
    a.href = content;
  } else {
    const blob = new Blob([content], { type: 'text/markdown;charset=utf-8' });
    a.href = URL.createObjectURL(blob);
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  }
  a.click();
}

/**
 * 全量导出。
 *
 * **走 Rust 侧的 `export` 命令**，不在这边拼。早先是把全部片语拉过来自己拼
 * Markdown，有两个问题：一是 `list_memos` 有单次上限，超了会被静默截断；
 * 二是同一份格式在 Rust 与 JS 各有一版，迟早会分叉。
 *
 * 路径由用户在系统对话框里选，前端不传路径（§12.1）。
 */
async function exportMarkdown() {
  // 明文这件事必须先说，不能导完了才提
  ov.confirmSheet(
    '导出 Markdown',
    '导出的文件是明文的，任何人拿到都能直接读。放进网盘或聊天工具之前请想一下。',
    async () => {
      try {
        const path = await store.exportTo('markdown');
        ov.toast(path ? '已导出（明文）' : '已取消');
      } catch {
        ov.toast('导出失败');
      }
    }
  );
}

// ── 主题 ────────────────────────────────────────────────────────

function toggleTheme() {
  const root = document.documentElement;
  state.theme = root.dataset.theme === 'dark' ? 'light' : 'dark';
  root.dataset.theme = state.theme;
  savePrefs();
}

// ── 标签菜单 ────────────────────────────────────────────────────

function tagMenu(tag, ev) {
  const pinned = state.tagMeta.pinned.includes(tag);
  ov.menu(ev.clientX, ev.clientY, [
    { label: pinned ? '取消置顶' : '置顶标签', act: 'tag-pin', arg: tag },
    { label: '设为回顾范围', act: 'tag-scope', arg: tag },
  ]);
}

// ── 动作表 ──────────────────────────────────────────────────────

/**
 * `data-act` → 处理函数。
 *
 * 第二个参数是 `data-arg`（永远是字符串），第三个是原始事件。
 * **只有出现在这张表里的名字才会被执行**——这也是为什么它比 `onclick` 安全：
 * 攻击者即使能往 DOM 里塞属性，也只能触发这几件事，且参数是字符串不是代码。
 */
const ACTIONS = {
  // 标题栏
  'win-min': () => platform.minimize(),
  'win-max': () => platform.toggleMaximize(),
  'win-close': () => platform.hide(),
  theme: toggleTheme,
  palette: () => palette.openPalette(),
  settings: () => openSettings(),
  'acct-menu': (_, __, ev) =>
    ov.menu(ev.clientX - 100, ev.clientY + 10, [
      { label: '偏好设置…', act: 'settings' },
      { label: '导出 Markdown', act: 'export-md' },
    ]),

  // 导航
  view: (arg) => setView(arg),
  tag: (arg) => pickTag(arg),
  day: (arg) => setDay(arg),
  sort: (arg) => { state.sort = arg; savePrefs(); refresh(); },
  'clear-query': () => { state.query = ''; $('q').value = ''; refresh(); },
  jump: (arg) => { ov.closeAll(); jumpTo(arg); },
  'tag-fold': (arg, _, ev) => {
    ev.stopPropagation();
    const closed = state.tagMeta.closed;
    state.tagMeta.closed = closed.includes(arg) ? closed.filter((x) => x !== arg) : [...closed, arg];
    draw();
  },
  'tag-menu': (arg, _, ev) => { ev.stopPropagation(); tagMenu(arg, ev); },
  'tag-pin': (arg) => {
    const pinned = state.tagMeta.pinned;
    state.tagMeta.pinned = pinned.includes(arg) ? pinned.filter((x) => x !== arg) : [...pinned, arg];
    ov.closeMenu();
    draw();
  },
  'tag-scope': (arg) => {
    state.scope = arg;
    ov.closeMenu();
    setTab('recall');
    rollRecall();
    ov.toast('回顾范围：#' + arg);
  },

  // 编辑器
  wrap: (arg) => ed.wrap(arg),
  prefix: (arg) => ed.prefixLine(arg),
  insert: (arg) => ed.insertText(arg),
  preview: () => ed.togglePreview(),
  save,
  'cancel-edit': () => ed.cancelEdit(),
  'pick-image': () => pickImage(),
  'draft-remove': (arg) => ed.removeDraft(Number(arg)),

  // 卡片
  'memo-pin': async (_, el) => {
    const id = memoIdOf(el);
    const memo = byId(id);
    if (!memo) return;
    await store.setPinned(id, !memo.pinned);
    await refresh();
    ov.toast(memo.pinned ? '已取消置顶' : '已置顶');
  },
  'memo-edit': (_, el) => {
    const memo = byId(memoIdOf(el));
    if (memo) ed.startEdit(memo);
  },
  'memo-del': (_, el) => deleteMemo(memoIdOf(el)),
  'memo-copy': (_, el) => copyMemo(memoIdOf(el)),
  'memo-copy-id': (arg) => copyMemo(arg),
  'memo-share': (_, el) => shareMemo(memoIdOf(el)),
  'share-save': (arg) => saveShare(arg),
  'memo-restore': async (_, el) => {
    await store.restore(memoIdOf(el));
    await refresh();
    ov.toast('已恢复');
  },
  'memo-purge': (_, el) => {
    const id = memoIdOf(el);
    ov.confirmSheet('彻底删除', '这一条会被永久删除，无法恢复。', async () => {
      await store.purge(id);
      await refresh();
      ov.toast('已彻底删除');
    });
  },
  'empty-trash': () => {
    if (!state.counts.trash) return;
    ov.confirmSheet('清空回收站', `将永久删除 ${state.counts.trash} 条片语，无法恢复。`, async () => {
      await store.purgeAll();
      await refresh();
      ov.toast('回收站已清空');
    });
  },

  // 右栏
  tab: (arg) => { state.tab = arg; savePrefs(); setTab(arg); },
  'recall-roll': () => rollRecall(),
  'recall-jump': () => state.recallId && jumpTo(state.recallId),
  week: (arg) => openWeek(Number(arg)),
  'week-export': async (arg) => {
    const { name, text } = await exportWeek(Number(arg));
    download(name, text);
    ov.toast('已导出本周回顾（明文）');
  },

  // 弹层
  'close-all': () => ov.closeAll(),
  'close-lightbox': () => ov.closeAll(),
  'confirm-ok': () => ov.runConfirm(),
  'toast-action': () => ov.runToastAction(),
  lightbox: (arg) => ov.lightbox(arg),
  'palette-run': (arg) => { ov.closeAll(); palette.runRow(Number(arg)); },
  'export-md': () => { ov.closeMenu(); exportMarkdown(); },
};

const memoIdOf = (el) => el.closest('.memo')?.dataset.id;

function openSettings() {
  ov.closeMenu();
  ov.sheet(
    '偏好设置',
    `<div class="field"><label>每日目标</label>
       <input type="number" id="setGoal" min="1" max="50" value="${state.goal}"></div>
     <div class="field"><label>正文字号</label>
       <input type="range" id="setFs" min="13" max="20" step="0.5" value="${state.fontSize}"></div>
     <p class="hint" style="margin-top:12px">
       本地库加密状态与同步设置要等后面的阶段接上。这里显示的都是真实生效的值——
       宁可少显示一项，也不显示一个假的「已加密」。</p>`
  );
  $('setGoal').addEventListener('change', (e) => {
    state.goal = Math.max(1, Number(e.target.value) || 1);
    savePrefs();
    draw();
  });
  $('setFs').addEventListener('input', (e) => {
    state.fontSize = Number(e.target.value);
    savePrefs();
    document.documentElement.style.setProperty('--fs', state.fontSize + 'px');
  });
}

// ── 事件委托 ────────────────────────────────────────────────────

document.addEventListener('click', (e) => {
  // 遮罩：点空白处关闭
  const scrim = e.target.closest('[data-scrim]');
  if (scrim && e.target === scrim) { ov.closeAll(); return; }

  // 正文里的可点击元素，它们由 markdown.js 生成，没有 data-act
  const tag = e.target.closest('.itag');
  if (tag) { pickTag(tag.dataset.tag); return; }

  const ref = e.target.closest('.iref[data-ref]');
  if (ref) { jumpTo(ref.dataset.ref); return; }

  const link = e.target.closest('a[data-ext]');
  if (link) {
    e.preventDefault();
    // platform.openExternal 内部还会再校验一次 scheme——两层都要有
    platform.openExternal(link.href);
    return;
  }

  const ck = e.target.closest('.ck[data-todo]');
  if (ck) {
    const id = memoIdOf(ck);
    if (id) toggleTodoAt(id, Number(ck.dataset.todo));
    return;
  }

  const el = e.target.closest('[data-act]');
  if (!el) {
    // 点别处关掉上下文菜单
    if (!e.target.closest('#menu')) ov.closeMenu();
    return;
  }
  const fn = ACTIONS[el.dataset.act];
  if (fn) fn(el.dataset.arg ?? '', el, e);
});

document.addEventListener('mouseover', (e) => {
  const item = e.target.closest('.pitem[data-arg]');
  if (item) palette.highlightRow(Number(item.dataset.arg));
});

// ── 键盘 ────────────────────────────────────────────────────────

ed.guardComposition($('input'));
// 粘贴与拖放直接落盘成附件（§7.3 ①）
ed.attachImageIngest($('input'), (msg) => ov.toast(msg));

$('input').addEventListener('input', () => {
  ed.refresh();
  ed.updateSuggest($('input'), $('suggest'), { tags: allTags(), memos: state.memos });
});

$('input').addEventListener('keydown', (e) => {
  // 组合串未上屏时不响应任何快捷键（§6.2）——那些按键属于输入法
  if (e.isComposing || ed.isComposing()) return;

  // 补全弹层优先：开着时的回车是「选中候选」，不是「保存」
  if (ed.suggestKey(e, $('input'), $('suggest'))) return;

  const mod = e.metaKey || e.ctrlKey;
  if (mod && e.key === 'Enter') { e.preventDefault(); save(); return; }
  if (mod && e.key.toLowerCase() === 'b') { e.preventDefault(); ed.wrap('**'); return; }
  if (mod && e.key.toLowerCase() === 'i') { e.preventDefault(); ed.wrap('*'); return; }
  if (e.key === 'Enter' && !mod) { ed.handleEnter(e, $('input')); return; }
  if (e.key === 'Escape' && state.editId) { ed.cancelEdit(); }
});

$('q').addEventListener('input', (e) => {
  state.query = e.target.value.trim();
  refresh();
});

$('pq').addEventListener('input', (e) => palette.fill(e.target.value));

$('pq').addEventListener('keydown', (e) => {
  const n = palette.rowCount();
  if (e.key === 'ArrowDown') { e.preventDefault(); palette.highlightRow(Math.min(palette.currentIndex() + 1, n - 1)); }
  else if (e.key === 'ArrowUp') { e.preventDefault(); palette.highlightRow(Math.max(palette.currentIndex() - 1, 0)); }
  else if (e.key === 'Enter') {
    e.preventDefault();
    if (!n) {
      // 没有匹配时，把输入的内容直接变成一条新片语的草稿
      const v = e.target.value.trim();
      ov.closeAll();
      $('input').value = v;
      $('input').focus();
      ed.refresh();
    } else {
      ov.closeAll();
      palette.runRow(palette.currentIndex());
    }
  }
});

document.addEventListener('keydown', (e) => {
  // 同上：打拼音的过程中不该触发命令面板、切换主题这些
  if (e.isComposing || ed.isComposing()) return;

  const mod = e.metaKey || e.ctrlKey;
  const typing = ['INPUT', 'TEXTAREA'].includes(document.activeElement?.tagName);

  if (e.ctrlKey && e.altKey && (e.code === 'Space' || e.key === ' ')) { e.preventDefault(); platform.quickToggle(); }
  else if (e.ctrlKey && e.altKey && e.key.toLowerCase() === 'r') { e.preventDefault(); openWeek(0); }
  else if (mod && e.key.toLowerCase() === 'k') { e.preventDefault(); palette.openPalette(); }
  else if (mod && e.key.toLowerCase() === 'd') { e.preventDefault(); toggleTheme(); }
  else if (mod && e.key.toLowerCase() === 'r') { e.preventDefault(); setTab('recall'); rollRecall(); }
  else if (mod && e.key.toLowerCase() === 'n') { e.preventDefault(); $('input').focus(); scrollTop(); }
  else if (e.key === 'Escape') ov.closeAll();
  else if (e.key === '/' && !typing) { e.preventDefault(); $('q').focus(); }
});

// ── 启动 ────────────────────────────────────────────────────────

palette.initPalette({
  tags: allTags,
  commands: [
    { label: '写新片语', icon: '✎', kbd: 'Ctrl+N', run: () => { $('input').focus(); scrollTop(); } },
    { label: '速记浮窗', icon: '▭', kbd: 'Ctrl+Alt+Space', run: () => platform.quickToggle() },
    { label: '本周回顾', icon: '▦', kbd: 'Ctrl+Alt+R', run: () => openWeek(0) },
    { label: '上周回顾', icon: '▦', run: () => openWeek(-1) },
    { label: '随机拾遗', icon: '↺', kbd: 'Ctrl+R', run: () => { setTab('recall'); rollRecall(); } },
    { label: '切换深浅外观', icon: '◐', kbd: 'Ctrl+D', run: toggleTheme },
    { label: '只看今日', icon: '◷', run: () => setView('today') },
    { label: '查看全部', icon: '▤', run: () => setView('all') },
    { label: '未归类的片语', icon: '◇', run: () => setView('untagged') },
    { label: '打开回收站', icon: '⌫', run: () => setView('trash') },
    { label: '统计与热力图', icon: '▩', run: () => setTab('stats') },
    { label: '导出 Markdown', icon: '↓', run: exportMarkdown },
    { label: '偏好设置', icon: '⚙', run: openSettings },
  ],
});
palette.bindNavigation({ pickTag, jumpTo });

/**
 * 上报最大化按钮矩形，供 Snap Layouts 用（§5.2 ②）。
 *
 * 布局与 DPI 变化时都要重报——按钮位置会随窗口宽度和缩放变。
 */
function reportMaxButton() {
  platform.reportMaxButtonRect($('maxBtn'));
}
addEventListener('resize', reportMaxButton);

(async () => {
  loadPrefs();
  document.documentElement.dataset.theme = state.theme;
  setTab(state.tab);

  try {
    const info = await store.appInfo();
    console.info(`知言 ${info.version} · ${info.platform} · 本地库加密：${info.encryptedStore}`);
    // 未解锁时列表会全是 locked 错误。现在只是提示一句，解锁引导界面在阶段五
    if (info.unlocked === false) ov.toast('本地库没能打开，改动不会被保存');
  } catch {
    // 拿不到就算了，不该因为一条信息性命令挡住启动
  }

  await refresh();
  rollRecall();
  ed.refresh();
  reportMaxButton();

  // 墓碑与孤儿附件的回收（§7.3 ②：墓碑保留 90 天）。
  // 放在首屏画完之后：它可能要删几百个文件，抢在前面会让启动看起来很慢
  setTimeout(() => {
    store.runGc().catch(() => {}); // 回收失败不该打扰用户，下次启动会再试
  }, 3000);
})();
