/**
 * Markdown 渲染。
 *
 * **渲染留在前端**，这是 Tauri 方案相对 GPUI 的一处实在便宜：浏览器白送排版、
 * 断行、中文避头尾（`line-break`）、字体回退，一行代码都不用写。Rust 侧只做索引
 * 与聚合，不碰渲染。
 *
 * 支持的语法（与原型一致）：粗体 / 斜体 / 删除线 / 高亮 / 行内码 / 代码块 /
 * 二~四级标题 / 引用 / 有序无序列表 / 待办 / 链接 / 分割线 / `#标签` / `[[引用]]`。
 *
 * ## 安全
 *
 * 正文是用户可控的，渲染结果直接进 `innerHTML`，所以 **`esc()` 必须在最前面**：
 * 先把整段转义，再往里插我们自己生成的标签。顺序反了就是一个 XSS。
 * 链接只放行 http/https（`isSafeLink`），其余 scheme 降级成纯文本。
 */

import { esc, REFRE, TAGRE, snippet, isSafeLink, reEsc } from './util.js';

/**
 * 行内渲染。
 *
 * @param {string} s 源文本
 * @param {(id:string)=>object|undefined} lookup 按 id 找片语，用于把引用渲染成摘要
 */
export function inline(s, lookup = () => undefined) {
  s = esc(s);

  // 行内码先摘出来占位，免得里面的 `*` `#` 被当成标记
  // 哨兵用 U+0001 / U+0002：它们不可能出现在用户正文里。写成转义序列而不是
  // 字面控制字符——后者在编辑器里是隐形的，读代码时根本看不出占位符长什么样。
  const codes = [];
  s = s.replace(/`([^`]+)`/g, (_, c) => {
    codes.push(c);
    return `\u0001${codes.length - 1}\u0001`;
  });

  // 引用：目标被删时渲染成 .dead 而不是消失——悄悄消失比留一个失效引用更糟
  s = s.replace(REFRE, (_, label, id) => {
    const target = lookup(id);
    if (!target) return `<span class="iref dead">${label || '已删除'}</span>`;
    // label 与 id 来自已转义的 s，不再重复转义；snippet 来自原始数据，必须转义
    return `<button class="iref" data-ref="${id}">${label || esc(snippet(target.text, 14))}</button>`;
  });

  // 链接：scheme 不安全就退回纯文本，不生成 <a>
  s = s.replace(/\[([^\]]+)\]\(([^\s)]+)\)/g, (whole, text, href) => {
    const url = href.replace(/&amp;/g, '&');
    if (!isSafeLink(url)) return text;
    return `<a href="${esc(url)}" data-ext="1" rel="noopener noreferrer">${text}</a>`;
  });

  s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  s = s.replace(/(^|[^*])\*([^*\n]+)\*(?!\*)/g, '$1<em>$2</em>');
  s = s.replace(/~~([^~]+)~~/g, '<del>$1</del>');
  s = s.replace(/==([^=]+)==/g, '<mark>$1</mark>');
  // 同理：tag 已在 esc(s) 里转义过，双重转义会让含 & 的标签显示成 &amp;
  s = s.replace(TAGRE, (_, pre, tag) => `${pre}<button class="itag" data-tag="${tag}">#${tag}</button>`);

  s = s.replace(/\u0001(\d+)\u0001/g, (_, i) => `<code>${codes[+i]}</code>`);
  return s;
}

/**
 * 块级渲染。
 *
 * @param {string} src 完整 Markdown 源
 * @param {(id:string)=>object|undefined} lookup
 */
export function render(src, lookup = () => undefined) {
  const fences = [];
  src = String(src).replace(/```([\s\S]*?)```/g, (_, c) => {
    fences.push(c.replace(/^\n/, '').replace(/\n$/, ''));
    return `\u0002${fences.length - 1}\u0002`;
  });

  let out = '';
  let list = null;
  let todoIndex = 0;
  let para = [];

  const flushP = () => {
    if (para.length) {
      out += `<p>${para.map((l) => inline(l, lookup)).join('<br>')}</p>`;
      para = [];
    }
  };
  const closeL = () => {
    if (list) {
      out += `</${list.split('.')[0]}>`;
      list = null;
    }
  };
  const openL = (tag, cls) => {
    const key = tag + (cls ? '.' + cls : '');
    if (list !== key) {
      closeL();
      out += `<${tag}${cls ? ` class="${cls}"` : ''}>`;
      list = key;
    }
  };

  for (const line of src.split('\n')) {
    const fence = line.trim().match(/^\u0002(\d+)\u0002$/);
    if (fence) {
      flushP(); closeL();
      out += `<pre><code>${esc(fences[+fence[1]])}</code></pre>`;
      continue;
    }
    if (!line.trim()) { flushP(); closeL(); continue; }

    let m;
    if ((m = line.match(/^(#{2,4})\s+(.*)$/))) {
      flushP(); closeL();
      const lv = m[1].length;
      out += `<h${lv}>${inline(m[2], lookup)}</h${lv}>`;
      continue;
    }
    if (/^\s*(---|\*\*\*|___)\s*$/.test(line)) { flushP(); closeL(); out += '<hr>'; continue; }
    if ((m = line.match(/^>\s?(.*)$/))) {
      flushP(); closeL();
      out += `<blockquote>${inline(m[1], lookup)}</blockquote>`;
      continue;
    }
    // 待办要排在无序列表前面——`- [ ] x` 也匹配无序列表的模式
    if ((m = line.match(/^\s*[-*+]\s+\[([ xX])\]\s+(.*)$/))) {
      flushP(); openL('ul', 'todo');
      const done = m[1] !== ' ';
      out += `<li><button class="ck${done ? ' done' : ''}" data-todo="${todoIndex++}" aria-label="切换待办"></button><span>${inline(m[2], lookup)}</span></li>`;
      continue;
    }
    if ((m = line.match(/^\s*[-*+]\s+(.*)$/))) {
      flushP(); openL('ul');
      out += `<li>${inline(m[1], lookup)}</li>`;
      continue;
    }
    if ((m = line.match(/^\s*\d+[.)]\s+(.*)$/))) {
      flushP(); openL('ol');
      out += `<li>${inline(m[1], lookup)}</li>`;
      continue;
    }
    closeL();
    para.push(line);
  }
  flushP(); closeL();
  return out;
}

/**
 * 勾选第 index 个待办，返回改写后的**源文本**（§2.1）。
 *
 * 待办是写作的副产品，不是任务系统——状态就存在正文里，没有独立的 todo 表，
 * 也就不会出现「正文与状态不一致」。
 */
export function toggleTodo(text, index) {
  let k = 0;
  return String(text)
    .split('\n')
    .map((line) => {
      const m = line.match(/^(\s*[-*+]\s+\[)([ xX])(\][\s\S]*)$/);
      if (!m) return line;
      if (k++ === index) return m[1] + (m[2] === ' ' ? 'x' : ' ') + m[3];
      return line;
    })
    .join('\n');
}

/** 未勾选的待办条数。每周回顾汇总用。 */
export function openTodos(text) {
  return String(text)
    .split('\n')
    .map((l) => l.match(/^\s*[-*+]\s+\[ \]\s+(.*)$/))
    .filter((m) => m && m[1].trim())
    .map((m) => m[1]);
}

/**
 * 回车续行：给定光标所在行，返回下一行该自动插入的前缀。
 *
 * 返回 `''` 表示当前是**空的列表项**，应当把这一行的标记清掉并跳出列表——
 * 这就是「回车两下退出列表」。返回 `null` 表示不续。
 */
export function continueList(line) {
  let m;
  if ((m = line.match(/^(\s*)([-*+]\s+)\[[ xX]\]\s*(.*)$/))) {
    return m[3].trim() ? `${m[1]}${m[2]}[ ] ` : '';
  }
  if ((m = line.match(/^(\s*)([-*+]\s+)(.*)$/))) {
    return m[3].trim() ? `${m[1]}${m[2]}` : '';
  }
  if ((m = line.match(/^(\s*)(\d+)([.)]\s+)(.*)$/))) {
    return m[4].trim() ? `${m[1]}${+m[2] + 1}${m[3]}` : '';
  }
  if ((m = line.match(/^(\s*)>\s?(.*)$/))) {
    return m[2].trim() ? `${m[1]}> ` : '';
  }
  return null;
}

/**
 * 搜索命中高亮。
 *
 * 走 TreeWalker 改文本节点，而不是对 HTML 串做正则替换——后者会把
 * `<button class="itag">` 里的属性也一起改掉。
 */
export function highlight(root, query) {
  if (!query) return;
  const re = new RegExp(reEsc(query), 'gi');
  const walk = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const hits = [];
  let n;
  while ((n = walk.nextNode())) {
    if (n.nodeValue && re.test(n.nodeValue)) { re.lastIndex = 0; hits.push(n); }
  }
  for (const node of hits) {
    const frag = document.createDocumentFragment();
    let last = 0, m;
    const r = new RegExp(re);
    while ((m = r.exec(node.nodeValue))) {
      frag.append(node.nodeValue.slice(last, m.index));
      const s = document.createElement('span');
      s.className = 'hit';
      s.textContent = m[0];
      frag.append(s);
      last = m.index + m[0].length;
      if (!m[0].length) r.lastIndex++;
    }
    frag.append(node.nodeValue.slice(last));
    node.replaceWith(frag);
  }
}
