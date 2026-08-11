/**
 * 窗口控制与平台相关的显示细节（§4.5 / §5.2 / §5.3）。
 *
 * 与 `store.js` 分开是因为职责不同：那边是数据，这边是窗口与系统。
 * 两边都走 `invoke`，但混在一起的话，「哪些调用会写盘」就看不清了。
 */

import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { isSafeLink } from './util.js';

export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** 浏览器里没有窗口可操作，静默忽略即可。 */
const win = (cmd, args) => (inTauri ? invoke(cmd, args) : null);

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
