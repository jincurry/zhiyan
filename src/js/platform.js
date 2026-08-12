/**
 * 窗口控制与平台相关的显示细节（§4.5 / §5.2 / §5.3）。
 *
 * 与 `store.js` 分开是因为职责不同：那边是数据，这边是窗口与系统。
 * 两边都走 `invoke`，但混在一起的话，「哪些调用会写盘」就看不清了。
 */

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { openUrl } from '@tauri-apps/plugin-opener';
import { isSafeLink } from './util.js';

export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/**
 * 浏览器里没有窗口可操作，静默忽略即可。
 *
 * **一律返回 Promise**，哪怕浏览器分支没有异步的事要做。返回值时而是 Promise
 * 时而是裸值的 API，调用方 `.catch()` 一挂就是一个 `is not a function`——
 * 而且只在浏览器里挂，Tauri 里跑得好好的。
 */
const win = async (cmd, args) => (inTauri ? invoke(cmd, args) : null);

export const minimize = () => win('window_minimize');
export const toggleMaximize = () => win('window_toggle_maximize');

/** 关闭 = 隐藏而非退出（§5.1）。设置项可改为真退出。 */
export const hide = () => win('window_hide');

/** 唤起/收起速记浮窗。 */
export const quickToggle = () => win('quick_toggle');

/**
 * 上报最大化按钮的矩形，供 Snap Layouts 用（§5.2 ②）。
 *
 * **这是最容易漏的一项**：Win11 用户悬停最大化按钮 1 秒会期待弹出贴靠布局面板。
 * 自绘标题栏默认没有，用户会立刻察觉「这软件不对劲」。
 *
 * Rust 侧靠 `WM_NCHITTEST` 命中这个矩形后返回 `HTMAXBUTTON`。因为坐标要随
 * 布局与 DPI 变化，只能由前端上报——Rust 那边看不到 DOM。
 */
export function reportMaxButtonRect(el) {
  if (!inTauri || !el) return;
  const r = el.getBoundingClientRect();
  invoke('set_maxbutton_rect', {
    x: Math.round(r.left),
    y: Math.round(r.top),
    w: Math.round(r.width),
    h: Math.round(r.height),
  }).catch(() => {
    // 子类化还没接上（阶段五）时命令不存在，不该把控制台刷满
  });
}

/**
 * 打开外部链接。
 *
 * **只允许 http/https**（§12.2）。`file://` 或 `ms-msdt:` 之类的 scheme
 * 会成为攻击面。渲染层已经挡过一道不生成 `<a>`，这里挡的是「即使有人
 * 构造出点击事件也打不开」——两层都要有。
 */
export async function openExternal(url) {
  if (!isSafeLink(url)) {
    console.warn('拒绝打开非 http(s) 链接');
    return false;
  }
  if (inTauri) await openUrl(url);
  else window.open(url, '_blank', 'noopener,noreferrer');
  return true;
}

/**
 * 快捷键的显示名。
 *
 * Windows 上是 `Ctrl`，macOS 上是 `⌘`。目前只发 Windows，但把它收在一处，
 * 将来出 macOS 版时不用去十几个模板里找字符串。
 */
export function shortcutLabel(combo) {
  const mac = typeof navigator !== 'undefined' && /Mac/i.test(navigator.platform ?? '');
  if (!mac) return combo;
  return combo
    .replace(/Ctrl\+/g, '⌘')
    .replace(/Alt\+/g, '⌥')
    .replace(/Shift\+/g, '⇧');
}

// ── 全局热键（§5.4）─────────────────────────────────────────────

/** 当前的热键设置。浏览器里没有全局热键，返回空表。 */
export const hotkeyList = async () => (inTauri ? invoke('hotkey_list') : []);

/**
 * 改键。
 *
 * **Rust 侧先试注册、成功才保存**。反过来的话，用户填了一个被占用的键位、
 * 设置存下了、下次启动仍然不工作——而设置页显示的是「已设置」。
 */
export const hotkeyRebind = async (slot, combo, enabled) =>
  inTauri ? invoke('hotkey_rebind', { slot, combo, enabled }) : null;

/**
 * 把一个 `keydown` 事件转成 Rust 认得的组合键字符串。
 *
 * 只收「修饰键 + 一个主键」。纯修饰键（用户还在按 Ctrl，主键没落下）返回
 * `null`，让调用方继续等——不这么做的话，按 Ctrl 的一瞬间就被记成 `Ctrl+Control`。
 */
export function comboFromEvent(e) {
  const mods = [];
  if (e.ctrlKey) mods.push('Ctrl');
  if (e.altKey) mods.push('Alt');
  if (e.shiftKey) mods.push('Shift');
  if (e.metaKey) mods.push('Super');

  const key = e.key;
  if (['Control', 'Alt', 'Shift', 'Meta', 'OS'].includes(key)) return null;
  // 全局热键必须带修饰键。不带的话按一下字母就被全系统抢走了
  if (!mods.length) return null;

  // Rust 侧认的是 `Space` / `KeyA` / `Digit1` 这类 code 名，
  // 而 e.key 在切换输入法或按 Shift 时会变。用 e.code 稳得多
  const code = e.code;
  const main = code.startsWith('Key')
    ? code.slice(3)
    : code.startsWith('Digit')
      ? code.slice(5)
      : code;
  return [...mods, main].join('+');
}

// ── 自启（§5.5）─────────────────────────────────────────────────

/**
 * 自启状态。
 *
 * 返回 `{ configured, effective }` 两个值。不一致说明用户在
 * 「设置 → 应用 → 启动」里把它关掉了——注册表项还在，但被禁用了。
 * **界面上要显示 `effective`**，否则会出现「开关是开的但没自启」。
 */
export const autostartGet = async () =>
  inTauri ? invoke('autostart_get') : { configured: false, effective: false };
export const autostartSet = async (on) => (inTauri ? invoke('autostart_set', { on }) : null);

// ── 自动更新（§11.3）────────────────────────────────────────────

/**
 * 检查更新。
 *
 * 返回 `{ available, version, configured }`。**`configured: false` 要单独显示**：
 * 「已是最新」与「这个构建根本不会更新」对用户的含义完全不同——
 * 后者意味着他得自己去看有没有新版。
 */
export const checkUpdate = async () =>
  inTauri ? invoke('check_update') : { available: false, version: null, configured: false };

// ── 运行时自检（§4.2）───────────────────────────────────────────

/** WebView2 运行时版本与 Chromium 下限。 */
export const runtimeReport = async () =>
  inTauri ? invoke('runtime_report') : { chromiumMin: 111, chromiumOk: true, webview2: null };

/**
 * 本机 WebView 到底支不支持我们用到的特性。
 *
 * 与 Rust 侧读注册表**互为补充**：那边看的是「装了哪个版本」，这边看的是
 * 「这些特性真的能用吗」。装了新版但被组策略关掉某个特性的情况只有这边看得见。
 *
 * 查的这几项都是塌掉就没法用的：`color-mix()` 决定整套配色、`:has()` 决定
 * 侧栏与卡片的一批状态样式、后行断言在 Markdown 渲染里。
 */
export function featureGaps() {
  const gaps = [];
  const css = typeof CSS !== 'undefined' && CSS.supports;
  if (!css || !CSS.supports('color', 'color-mix(in srgb, red 50%, blue)')) gaps.push('color-mix()');
  if (!css || !CSS.supports('selector(:has(a))')) gaps.push(':has()');
  if (!css || !CSS.supports('aspect-ratio', '1')) gaps.push('aspect-ratio');
  if (!css || !CSS.supports('backdrop-filter', 'blur(2px)')) gaps.push('backdrop-filter');
  try {
    // 后行断言。语法不支持时是**解析期**错误，所以必须包在 eval 式的构造里
    new RegExp('(?<=a)b');
  } catch {
    gaps.push('正则后行断言');
  }
  return gaps;
}

// ── 后端事件 ────────────────────────────────────────────────────

/**
 * 订阅 Rust 侧发来的事件。
 *
 * 收在这个模块里而不是让各处直接 `import { listen }`：事件也是 IPC 的一半，
 * 散着订阅的话「后端会主动通知我什么」就没有一个地方能看全了。
 *
 * 浏览器里静默忽略——那些事件本来就只有 Tauri 里才有。
 */
export function onEvent(name, handler) {
  if (!inTauri) return () => {};
  let off = () => {};
  listen(name, (e) => handler(e.payload)).then((f) => {
    off = f;
  });
  return () => off();
}
