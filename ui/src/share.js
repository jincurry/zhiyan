/**
 * 分享成图。
 *
 * 整张卡片在本地 canvas 上画完，**不经过任何服务器**。这是 E2EE 下唯一
 * 说得通的分享方式——「公开分享单条」需要给每次分享生成独立密钥并把密钥放进
 * URL fragment，v1 不做（§17 ⑧）。
 */

import { plain, parseTags } from './util.js';

const WIDTH = 480;
const PAD = 40;
const LINE_H = 30;
const DPR = 2;

/**
 * 把片语画进 canvas。
 *
 * 断行是手写的逐字测量——canvas 没有自动换行。中文在这里反而好办：
 * 每个字都能断，不像西文要考虑词边界。
 */
export function drawCard(canvas, memo, theme) {
  const ctx = canvas.getContext('2d');
  const body = plain(memo.text).split('\n').filter(Boolean);
  const tags = parseTags(memo.text);
  const maxW = WIDTH - PAD * 2;

  ctx.font = '17px Georgia, "Source Han Serif SC", "Songti SC", serif';

  const lines = [];
  for (const para of body) {
    let cur = '';
    for (const ch of para) {
      if (ctx.measureText(cur + ch).width > maxW) {
        lines.push(cur);
        cur = ch;
      } else {
        cur += ch;
      }
    }
    lines.push(cur);
    lines.push('');
  }
  while (lines.length && lines[lines.length - 1] === '') lines.pop();

  const height = PAD * 2 + lines.length * LINE_H + 56;
  canvas.width = WIDTH * DPR;
  canvas.height = height * DPR;
  canvas.style.width = WIDTH + 'px';
  canvas.style.height = height + 'px';
  ctx.scale(DPR, DPR);

  ctx.fillStyle = theme.card;
  ctx.fillRect(0, 0, WIDTH, height);
  // 左侧一道强调色，够了——分享图不该带品牌水印之外的任何装饰
  ctx.fillStyle = theme.accent;
  ctx.fillRect(0, 0, 3, height);

  ctx.fillStyle = theme.ink;
  ctx.font = '17px Georgia, "Source Han Serif SC", "Songti SC", serif';
  ctx.textBaseline = 'top';
  lines.forEach((l, i) => ctx.fillText(l, PAD, PAD + i * LINE_H));

  const y = PAD + lines.length * LINE_H + 16;
  ctx.strokeStyle = theme.lineSoft;
  ctx.beginPath();
  ctx.moveTo(PAD, y);
  ctx.lineTo(WIDTH - PAD, y);
  ctx.stroke();

  const d = new Date(memo.createdAt);
  const stamp = `${d.getFullYear()}.${String(d.getMonth() + 1).padStart(2, '0')}.${String(d.getDate()).padStart(2, '0')}  知言`;
  ctx.fillStyle = theme.ink3;
  ctx.font = '11px ui-monospace, Consolas, Menlo, monospace';
  ctx.fillText(stamp, PAD, y + 14);

  if (tags.length) {
    ctx.fillStyle = theme.accent;
    const s = '#' + tags.join('  #');
    ctx.fillText(s, WIDTH - PAD - ctx.measureText(s).width, y + 14);
  }
  return canvas;
}

/** 从当前 CSS 变量读配色，保证分享图与界面同一套主题。 */
export function themeFromCss() {
  const cs = getComputedStyle(document.documentElement);
  const get = (name) => cs.getPropertyValue(name).trim();
  return {
    card: get('--card'),
    ink: get('--ink'),
    ink3: get('--ink-3'),
    accent: get('--accent'),
    lineSoft: get('--line-soft'),
  };
}
