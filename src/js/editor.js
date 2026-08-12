/**
 * 编辑器。
 *
 * 用的是 `<textarea>`。这一行就是 Tauri 方案相对 GPUI 最大的一处便宜：
 * **中文输入法整个不用管**——组合串、候选窗定位、上屏替换、退格，
 * 全是操作系统与 WebView 协作完成的。GPUI 那边这是要 5 天 P0 验证、
 * 且可能否决整个选型的头号风险。
 */

import { countChars, esc, snippet } from './util.js';
import { render, continueList } from './markdown.js';
import { state } from './state.js';
import { putBlob, blobUrl } from './store.js';

const $ = (id) => document.getElementById(id);

/** 补全弹层当前的模式与候选，模块内可见即可，不挂 window。 */
let suggestMode = null;

// ── 输入法组合串保护（§6.2）──────────────────────────────────────

/**
 * 是否正在输入法组合中（拼音还没上屏）。
 *
 * **组合期间必须屏蔽所有应用快捷键**。不做的话，打「shi」的过程中那个 `s`、
 * 打「ni」时的 `n`，都可能被当成 `Ctrl+N` 之类的组合键前缀误触发；更常见的是
 * 候选框还开着时按回车，本意是选词，却触发了保存。
 *
 * 这一条在 WebView 里只是几行监听；GPUI 方案里要贯穿整个 IME 实现。
 */
let composing = false;

export const isComposing = () => composing;

/**
 * 给一个 textarea 装上组合串保护。
 *
 * `compositionend` 之后不能立刻解除——同一轮事件里紧跟着还有 `keyup`，
 * 部分输入法还会补一个 `keydown`。用一个微任务把解除推后，避开这一拍。
 */
export function guardComposition(ta) {
  ta.addEventListener('compositionstart', () => {
    composing = true;
  });
  ta.addEventListener('compositionend', () => {
    queueMicrotask(() => {
      composing = false;
    });
  });
}

// ── 文本操作 ────────────────────────────────────────────────────

export function insertText(t, ta = $('input')) {
  const s = ta.selectionStart;
  const e = ta.selectionEnd;
  ta.value = ta.value.slice(0, s) + t + ta.value.slice(e);
  ta.selectionStart = ta.selectionEnd = s + t.length;
  ta.focus();
  refresh();
}

/** 用 `mark` 包住选区；没有选区就插入占位词并选中它。 */
export function wrap(mark, ta = $('input')) {
  const s = ta.selectionStart;
  const e = ta.selectionEnd;
  const sel = ta.value.slice(s, e) || '文字';
  ta.value = ta.value.slice(0, s) + mark + sel + mark + ta.value.slice(e);
  ta.focus();
  ta.selectionStart = s + mark.length;
  ta.selectionEnd = s + mark.length + sel.length;
  refresh();
}

/** 在光标所在行的行首插入前缀。 */
export function prefixLine(prefix, ta = $('input')) {
  const pos = ta.selectionStart;
  const start = ta.value.lastIndexOf('\n', pos - 1) + 1;
  ta.value = ta.value.slice(0, start) + prefix + ta.value.slice(start);
  ta.focus();
  ta.selectionStart = ta.selectionEnd = pos + prefix.length;
  refresh();
}

/** 把光标所在行整行换成 `text`。回车跳出列表时用。 */
function replaceLine(text, ta) {
  const pos = ta.selectionStart;
  const start = ta.value.lastIndexOf('\n', pos - 1) + 1;
  ta.value = ta.value.slice(0, start) + text + ta.value.slice(pos);
  ta.selectionStart = ta.selectionEnd = start + text.length;
  refresh();
}

// ── 刷新 ────────────────────────────────────────────────────────

export function refresh() {
  const ta = $('input');
  // 自适应高度：先归零再按 scrollHeight 撑开，否则删字时不会缩回去
  ta.style.height = 'auto';
  ta.style.height = Math.max(78, ta.scrollHeight) + 'px';

  $('count').textContent = countChars(ta.value) + ' 字';
  $('saveBtn').disabled = !ta.value.trim() && !state.drafts.length;

  const pv = $('preview');
  if (!pv.hidden) {
    pv.innerHTML = render(ta.value) || '<p style="color:var(--ink-3)">预览为空。</p>';
  }
}

export function togglePreview() {
  const pv = $('preview');
  const ta = $('input');
  const on = pv.hidden;
  pv.hidden = !on;
  ta.hidden = on;
  $('pvBtn').classList.toggle('on', on);
  if (on) pv.innerHTML = render(ta.value) || '<p style="color:var(--ink-3)">预览为空。</p>';
  else ta.focus();
}

// ── 编辑已有片语 ────────────────────────────────────────────────

export function startEdit(memo) {
  state.editId = memo.id;
  // 附件要一起带进草稿。不带的话保存时 blobs 是空的，图会被静默丢掉——
  // 而用户只是改了个错别字
  state.drafts = [...(memo.blobs ?? [])];
  drawThumbs();
  $('input').value = memo.text;
  $('composer').classList.add('editing');
  $('cancelBtn').hidden = false;
  $('saveTxt').textContent = '更新';
  $('main').scrollTo({ top: 0 });
  $('input').focus();
  refresh();
}

export function cancelEdit() {
  state.editId = null;
  state.drafts = [];
  drawThumbs();
  $('input').value = '';
  $('composer').classList.remove('editing');
  $('cancelBtn').hidden = true;
  $('saveTxt').textContent = '存下';
  refresh();
}

/** 存完之后把编辑器恢复到干净状态。 */
export function clearComposer() {
  state.editId = null;
  state.drafts = [];
  drawThumbs();
  $('input').value = '';
  $('composer').classList.remove('editing');
  $('cancelBtn').hidden = true;
  $('saveTxt').textContent = '存下';
  if (!$('preview').hidden) togglePreview();
  hideSuggest();
  refresh();
}

// ── 补全弹层 ────────────────────────────────────────────────────

/**
 * `#` 与 `[[` 触发补全。
 *
 * @param ta 目标 textarea（主编辑器或速记浮窗共用这一套）
 * @param box 弹层元素
 * @param ctx `{ tags, memos }`
 */
export function updateSuggest(ta, box, ctx) {
  const before = ta.value.slice(0, ta.selectionStart);
  let m;

  if ((m = before.match(/\[\[([^[\]\n]*)$/))) {
    const q = m[1].trim().toLowerCase();
    const hits = ctx.memos
      .filter((x) => x.id !== state.editId && (!q || x.text.toLowerCase().includes(q)))
      .slice(0, 7);
    if (!hits.length) return hideSuggest(box);
    suggestMode = 'ref';
    box.innerHTML = hits
      .map((x, i) => {
        const d = new Date(x.createdAt);
        return (
          `<button class="${i === 0 ? 'hl' : ''}" data-ref="${esc(x.id)}" data-label="${esc(refLabel(x))}">` +
          `<span class="sugref"><span>${d.getMonth() + 1}/${d.getDate()}</span>` +
          `<em>${esc(snippet(x.text, 30))}</em></span></button>`
        );
      })
      .join('');
  } else if ((m = before.match(/#([^\s#]*)$/))) {
    const hits = ctx.tags.filter((t) => t.includes(m[1])).slice(0, 7);
    if (!hits.length) return hideSuggest(box);
    suggestMode = 'tag';
    box.innerHTML = hits
      .map((t, i) => `<button class="${i === 0 ? 'hl' : ''}" data-tag="${esc(t)}">#${esc(t)}</button>`)
      .join('');
  } else {
    return hideSuggest(box);
  }

  box.style.left = '16px';
  box.style.top = ta.offsetTop + ta.offsetHeight - 8 + 'px';
  box.style.maxWidth = ta.offsetWidth - 32 + 'px';
  box.classList.add('show');
}

/** 引用的摘要不能含 `[` `]` `^` —— 它们会破坏 `[[摘要^id]]` 的解析。 */
function refLabel(m) {
  return snippet(m.text, 14).replace(/[[\]^\n|]/g, '');
}

export function hideSuggest(box = $('suggest')) {
  box.classList.remove('show');
  suggestMode = null;
}

export function applySuggest(ta, box, btn) {
  const pos = ta.selectionStart;
  const before = ta.value.slice(0, pos);
  const after = ta.value.slice(pos);

  const [re, ins] =
    suggestMode === 'tag'
      ? [/#([^\s#]*)$/, `#${btn.dataset.tag} `]
      : [/\[\[([^[\]\n]*)$/, `[[${btn.dataset.label}^${btn.dataset.ref}]] `];

  const replaced = before.replace(re, ins);
  ta.value = replaced + after;
  ta.selectionStart = ta.selectionEnd = replaced.length;
  hideSuggest(box);
  ta.focus();
}

/**
 * 补全弹层的键盘处理。返回 true 表示事件已被消费。
 *
 * **必须排在其余快捷键之前**：弹层开着时的回车是「选中候选」，不是「保存」。
 */
export function suggestKey(e, ta, box) {
  // 组合中不接管方向键与回车——那时它们属于输入法的候选框
  if (composing) return false;
  if (!box.classList.contains('show')) return false;
  const items = [...box.querySelectorAll('button')];
  if (!items.length) return false;
  const i = items.findIndex((b) => b.classList.contains('hl'));

  const move = (to) => {
    items[Math.max(i, 0)]?.classList.remove('hl');
    items[to].classList.add('hl');
  };

  if (e.key === 'ArrowDown') { e.preventDefault(); move(Math.min(i + 1, items.length - 1)); return true; }
  if (e.key === 'ArrowUp') { e.preventDefault(); move(Math.max(i - 1, 0)); return true; }
  if (e.key === 'Enter' || e.key === 'Tab') { e.preventDefault(); applySuggest(ta, box, items[Math.max(i, 0)]); return true; }
  if (e.key === 'Escape') { e.preventDefault(); hideSuggest(box); return true; }
  return false;
}

// ── 回车续行 ────────────────────────────────────────────────────

/** 处理编辑器里的回车。返回 true 表示已消费。 */
export function handleEnter(e, ta) {
  if (composing) return false;
  const before = ta.value.slice(0, ta.selectionStart);
  const line = before.slice(before.lastIndexOf('\n') + 1);
  const next = continueList(line);
  if (next === null) return false;

  e.preventDefault();
  if (next === '') {
    // 空的列表项：清掉标记跳出列表，而不是再续一个空项
    replaceLine(line.match(/^(\s*)/)[1], ta);
  } else {
    insertText('\n' + next, ta);
  }
  return true;
}

// ── 附件（§7.3 ①、§8.5）────────────────────────────────────────

/**
 * 允许的图片类型。与 Rust 侧 `put_blob` 的白名单一致。
 *
 * **不放 `image/svg+xml`**：SVG 里能写脚本，走 `zhiyan://` 出去就是一个 XSS。
 */
const IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/avif'];

/** IPC 的单次载荷上限（§9.6 给的建议值）。 */
const MAX_BLOB_BYTES = 10 * 1024 * 1024;

/**
 * 读出图片的像素尺寸。
 *
 * 在这一侧读而不是让 Rust 解码：为了两个整数把图片解码库拖进 Rust 的依赖树，
 * 换来的是安装包体积和一批解析器攻击面。浏览器本来就要解这张图。
 */
async function dimensions(blob) {
  try {
    const bmp = await createImageBitmap(blob);
    const wh = { w: bmp.width, h: bmp.height };
    bmp.close();
    return wh;
  } catch {
    return { w: null, h: null }; // 尺寸只影响排版，读不到不该挡住保存
  }
}

/**
 * 收下一批文件，落成内容寻址的附件。
 *
 * 返回成功入库的数量。**不抛错**：贴了五张图坏了一张时，另外四张该照常进去。
 */
export async function ingestFiles(files, onProgress) {
  let ok = 0;
  for (const file of files) {
    if (!IMAGE_TYPES.includes(file.type)) {
      onProgress?.(`不支持的图片格式：${file.type || '未知'}`);
      continue;
    }
    if (file.size > MAX_BLOB_BYTES) {
      onProgress?.(`图片超过 ${MAX_BLOB_BYTES / 1048576}MB，请先压缩`);
      continue;
    }
    try {
      const { w, h } = await dimensions(file);
      const bytes = new Uint8Array(await file.arrayBuffer());
      state.drafts.push(await putBlob(bytes, file.type, w, h));
      ok++;
    } catch {
      onProgress?.('这张图没能存下');
    }
  }
  if (ok) drawThumbs();
  refresh();
  return ok;
}

/**
 * 给编辑器装上粘贴与拖放。
 *
 * 图片**在这一步就落盘**，不是等到保存时才传——用户贴完图往往还要写一段话，
 * 那几秒足够把加密和写盘做完。代价是撤销时会留下没人引用的附件，
 * 由 `run_gc` 回收（refcount 为 0 的会被清掉）。
 */
export function attachImageIngest(ta, onProgress) {
  ta.addEventListener('paste', (e) => {
    const files = [...(e.clipboardData?.files ?? [])];
    if (!files.length) return; // 纯文本粘贴走默认行为
    e.preventDefault();
    ingestFiles(files, onProgress);
  });

  // 拖放要挡在整个编辑器上：拖到 textarea 边上一点就落空，很难对准
  const box = $('composer');
  for (const type of ['dragover', 'dragenter']) {
    box.addEventListener(type, (e) => {
      if (e.dataTransfer?.types?.includes('Files')) {
        e.preventDefault();
        box.classList.add('dropping');
      }
    });
  }
  for (const type of ['dragleave', 'drop']) {
    box.addEventListener(type, () => box.classList.remove('dropping'));
  }
  box.addEventListener('drop', (e) => {
    const files = [...(e.dataTransfer?.files ?? [])];
    if (!files.length) return;
    e.preventDefault();
    ingestFiles(files, onProgress);
  });
}

/** 画编辑器下方的缩略图条。 */
export function drawThumbs() {
  const box = $('thumbs');
  box.innerHTML = state.drafts
    .map(
      (b, i) =>
        `<figure><img src="${esc(blobUrl(b.sha256))}" alt="">` +
        `<button data-act="draft-remove" data-arg="${i}" title="移除">×</button></figure>`
    )
    .join('');
}

/** 移除一张待存的图。只是解除引用，字节由 `run_gc` 回收。 */
export function removeDraft(i) {
  state.drafts.splice(i, 1);
  drawThumbs();
  refresh();
}
