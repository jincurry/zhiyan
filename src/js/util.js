/**
 * 通用工具。
 *
 * 这一层刻意不碰 DOM，也不碰 IPC——它能在 `node --test` 里直接跑。
 */

export const WD = ['周日', '周一', '周二', '周三', '周四', '周五', '周六'];

/** HTML 转义。**任何进入 innerHTML 的用户内容都必须过它。** */
export const esc = (s) =>
  String(s).replace(/[&<>"']/g, (c) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
  })[c]);

/** 本地日期键 `YYYY-M-D`。用本地时区——跨零点写的笔记要归到用户眼里的那一天。 */
export const dayKey = (t) => {
  const d = new Date(t);
  return `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
};

export const isToday = (t) => dayKey(t) === dayKey(Date.now());

/** 标签：`#主题[/子题]`，两级。 */
export const TAGRE = /(^|[\s(（])#([^\s#，。、；：!？"'()（）[\]]+)/g;

/** 行内引用：`[[摘要^id]]`。 */
export const REFRE = /\[\[([^\]^\n]*)\^([0-9a-fA-F-]+)\]\]/g;

export function parseTags(s) {
  const out = [];
  const re = new RegExp(TAGRE);
  let m;
  while ((m = re.exec(s))) out.push(m[2]);
  return [...new Set(out)];
}

export function inlineRefs(s) {
  const out = [];
  const re = new RegExp(REFRE);
  let m;
  while ((m = re.exec(s))) out.push(m[2]);
  return [...new Set(out)];
}

/** 去掉 Markdown 标记，留纯文本。摘要、字数、分享成图都用它。 */
export const plain = (s) =>
  String(s)
    .replace(REFRE, '「$1」')
    .replace(TAGRE, '$1')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/[*~=`>#]/g, '')
    .replace(/^\s*[-+]\s+/gm, '')
    .trim();

/**
 * 字数。
 *
 * 中文按字、英文按词——纯按字符数会让一段英文的计数虚高，用它驱动时间脊节点
 * 大小时会让「写了几句英文的那天」显得格外重。
 */
export function countChars(s) {
  const t = plain(s);
  let n = 0, inWord = false;
  for (const ch of t) {
    if (/[0-9A-Za-z]/.test(ch)) {
      if (!inWord) { n++; inWord = true; }
    } else {
      inWord = false;
      if (!/\s/.test(ch)) n++;
    }
  }
  return n;
}

/** 摘要，截到 n 个字符。 */
export const snippet = (s, n) => {
  const p = plain(s).replace(/\s+/g, ' ').trim();
  return [...p].length > n ? [...p].slice(0, n).join('') + '…' : p;
};

/** 正则元字符转义。用户输入拼进正则前必须过它。 */
export const reEsc = (s) => String(s).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** `HH:MM` */
export const hhmm = (t) => {
  const d = new Date(t);
  return String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0');
};

/** `M月D日` */
export const mdCn = (t) => {
  const d = new Date(t);
  return `${d.getMonth() + 1}月${d.getDate()}日`;
};

/** 相对时间，用于拾遗的时间标签。 */
export function agoLabel(t, now = Date.now()) {
  const days = Math.round((now - t) / 864e5);
  if (days <= 0) return '今天';
  if (days === 1) return '昨天';
  if (days < 30) return `${days} 天前`;
  if (days < 365) return `${Math.round(days / 30)} 个月前`;
  return `${Math.round(days / 365)} 年前`;
}

/**
 * 时间脊节点直径：`clamp(7, 6 + 当天字数/45, 15)` px。
 *
 * 节点大小编码当天字数——这是时间脊在信息密度上的全部意义。
 */
export const nodeSize = (chars) => Math.max(7, Math.min(15, 6 + Math.round(chars / 45)));

/** 热力档位。阈值固定而非按当期最大值归一，否则低产期看起来和高产期一样满。 */
export const heatLevel = (n) => (n === 0 ? 0 : n === 1 ? 1 : n === 2 ? 2 : n <= 4 ? 3 : 4);

/** 某周的 [起, 止)，周一为起点。`off` 为 0 表示本周，-1 上周。 */
export function weekRange(off = 0, now = Date.now()) {
  const d = new Date(now);
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() - ((d.getDay() + 6) % 7) + off * 7);
  const e = new Date(d);
  e.setDate(e.getDate() + 7);
  return [d.getTime(), e.getTime()];
}

/**
 * 链接是否安全。**只允许 http/https**。
 *
 * `file://`、`ms-msdt:` 这类 scheme 交给系统打开等于本地代码执行的入口。
 */
export function isSafeLink(url) {
  try {
    const u = new URL(String(url).trim());
    return u.protocol === 'http:' || u.protocol === 'https:';
  } catch {
    return false;
  }
}
