/**
 * 弹层：sheet / 确认 / 菜单 / toast / lightbox。
 *
 * 全部走 `data-act` 委托，没有一个内联处理器（见 index.html 顶部的说明）。
 */

import { esc } from './util.js';

const $ = (id) => document.getElementById(id);

// ── Sheet ───────────────────────────────────────────────────────

/**
 * 打开一个 sheet。
 *
 * `html` 由调用方拼装，**调用方负责转义**——这个函数不能替它做，因为它分不清
 * 哪些是结构、哪些是数据。
 */
export function sheet(title, html) {
  $('shTitle').textContent = title;
  $('shBody').innerHTML = html;
  $('scrimPal').classList.remove('show');
  $('scrimSheet').classList.add('show');
}

/** 确认框。`onOk` 存在模块作用域里，不挂 `window`。 */
let pendingConfirm = null;

export function confirmSheet(title, message, onOk) {
  pendingConfirm = onOk;
  sheet(
    title,
    `<p style="font-size:13.5px;color:var(--ink-2);line-height:1.7;margin-bottom:16px">${esc(message)}</p>
     <div class="btnrow">
       <button class="btn solid" data-act="confirm-ok">确定</button>
       <button class="btn" data-act="close-all">取消</button>
     </div>`
  );
}

export function runConfirm() {
  const fn = pendingConfirm;
  pendingConfirm = null;
  closeAll();
  if (fn) fn();
}

export function closeAll() {
  $('scrimPal').classList.remove('show');
  $('scrimSheet').classList.remove('show');
  $('menu').classList.remove('show');
  $('lightbox').classList.remove('show');
}

// ── 上下文菜单 ──────────────────────────────────────────────────

/**
 * 在 (x, y) 处弹出菜单。
 *
 * `items` 形如 `[{ label, act, arg, danger }]`——**不接受 HTML**，
 * 这样菜单项文案里出现标签名（用户可控）也不会变成注入点。
 */
export function menu(x, y, items) {
  const el = $('menu');
  el.innerHTML = items
    .map(
      (it) =>
        `<button${it.danger ? ' class="dang"' : ''} data-act="${esc(it.act)}"${
          it.arg !== undefined ? ` data-arg="${esc(it.arg)}"` : ''
        }>${esc(it.label)}</button>`
    )
    .join('');
  el.style.left = Math.min(x, innerWidth - 170) + 'px';
  el.style.top = Math.min(y, innerHeight - items.length * 30 - 16) + 'px';
  el.classList.add('show');
}

export function closeMenu() {
  $('menu').classList.remove('show');
}

// ── Toast ───────────────────────────────────────────────────────

let toastTimer = null;
let toastAction = null;

/** 一句提示，可带一个撤销按钮。 */
export function toast(message, actionLabel, onAction) {
  const el = $('toast');
  el.textContent = '';

  const span = document.createElement('span');
  span.textContent = message;
  el.append(span);

  toastAction = onAction ?? null;
  if (actionLabel && onAction) {
    const b = document.createElement('button');
    b.textContent = actionLabel;
    b.dataset.act = 'toast-action';
    el.append(b);
  }

  el.classList.add('show');
  clearTimeout(toastTimer);
  // 带撤销的多留一会儿——1.7 秒不够看清「已移入回收站」再决定要不要撤销
  toastTimer = setTimeout(() => el.classList.remove('show'), actionLabel ? 4200 : 1700);
}

export function runToastAction() {
  const fn = toastAction;
  toastAction = null;
  $('toast').classList.remove('show');
  if (fn) fn();
}

// ── Lightbox ────────────────────────────────────────────────────

export function lightbox(src) {
  $('lbImg').src = src;
  $('lightbox').classList.add('show');
}
