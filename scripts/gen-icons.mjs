/**
 * 生成全部图标。
 *
 * ```sh
 * node scripts/gen-icons.mjs
 * ```
 *
 * ## 为什么要有这个脚本
 *
 * Windows 构建**必须**有 `icons/icon.ico`，缺了 `tauri-build` 直接失败——
 * 而这件事在 Linux 上开发时完全看不出来。托盘还要 16/20/24/32/40/48 六个尺寸
 * 的多分辨率 ICO（§5.5）：只放一个 32px 的话，任务栏在 150% 缩放下会拿它
 * 硬拉到 40px，糊得很明显。
 *
 * 不引图形库：PNG 用 zlib 手写编码，ICO 就是个 PNG 容器。为了几个图标拖进
 * 一整套原生依赖不划算，而且那类包在 CI 上装起来最容易出岔子。
 *
 * ## 图案
 *
 * 时间脊（§3.4）：一条竖线，几个大小不一的节点。节点大小编码当天字数——
 * 这是时间脊在信息密度上的全部意义，也是这个应用最像它自己的地方。
 * 用几何图形而不是「知」字，是因为纯 Node 里没有字体光栅化。
 */

import { deflateSync } from 'node:zlib';
import { writeFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const OUT = fileURLToPath(new URL('../src-tauri/icons/', import.meta.url));

/** 主色。与 `tokens.css` 的 `--accent` 一致。 */
const ACCENT = [0x2e, 0x5a, 0x49];
const PAPER = [0xf7, 0xf5, 0xef];

/** 超采样倍数。图标很小，锯齿在 16px 上格外扎眼。 */
const SS = 4;

/** 画一张 `size×size` 的 RGBA 位图。 */
function draw(size, { fg = PAPER, bg = ACCENT, transparentBg = false } = {}) {
  const n = size * SS;
  const buf = new Float64Array(n * n * 4);

  const put = (x, y, [r, g, b], a = 1) => {
    if (x < 0 || y < 0 || x >= n || y >= n) return;
    const i = (y * n + x) * 4;
    buf[i] = buf[i] * (1 - a) + r * a;
    buf[i + 1] = buf[i + 1] * (1 - a) + g * a;
    buf[i + 2] = buf[i + 2] * (1 - a) + b * a;
    buf[i + 3] = Math.max(buf[i + 3], a * 255);
  };

  const disc = (cx, cy, r, color) => {
    for (let y = Math.floor(cy - r); y <= Math.ceil(cy + r); y++) {
      for (let x = Math.floor(cx - r); x <= Math.ceil(cx + r); x++) {
        if ((x - cx) ** 2 + (y - cy) ** 2 <= r * r) put(x, y, color);
      }
    }
  };

  const roundRect = (x0, y0, w, h, r, color) => {
    for (let y = y0; y < y0 + h; y++) {
      for (let x = x0; x < x0 + w; x++) {
        const dx = Math.max(x0 + r - x, 0, x - (x0 + w - 1 - r));
        const dy = Math.max(y0 + r - y, 0, y - (y0 + h - 1 - r));
        if (dx * dx + dy * dy <= r * r) put(x, y, color);
      }
    }
  };

  if (!transparentBg) {
    // Windows 的图标习惯是留一点内边距，贴边的方块在任务栏里显得比别人大
    const pad = Math.round(n * 0.06);
    roundRect(pad, pad, n - pad * 2, n - pad * 2, Math.round(n * 0.22), bg);
  }

  // 时间脊：竖线偏左 38%，四个节点，大小不一
  const spine = Math.round(n * 0.38);
  const lineW = Math.max(1, Math.round(n * 0.035));
  const top = Math.round(n * 0.2);
  const bottom = Math.round(n * 0.8);
  for (let y = top; y < bottom; y++) {
    for (let x = spine - (lineW >> 1); x < spine - (lineW >> 1) + lineW; x++) {
      put(x, y, fg, 0.5);
    }
  }

  // 半径按 nodeSize 的意思来：编码当天字数，所以刻意不等大
  const nodes = [
    [0.24, 0.055],
    [0.44, 0.095],
    [0.63, 0.07],
    [0.79, 0.115],
  ];
  for (const [t, r] of nodes) {
    disc(spine, Math.round(n * t), Math.round(n * r), fg);
  }

  // 右侧三道横线，像卡片上的文字
  const lines = [
    [0.3, 0.34],
    [0.5, 0.42],
    [0.7, 0.28],
  ];
  const barH = Math.max(1, Math.round(n * 0.05));
  for (const [t, w] of lines) {
    const y0 = Math.round(n * t) - (barH >> 1);
    roundRect(
      spine + Math.round(n * 0.13),
      y0,
      Math.round(n * w),
      barH,
      barH / 2,
      fg
    );
  }

  // 降采样：SS×SS 一格取平均
  const out = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let r = 0, g = 0, b = 0, a = 0;
      for (let dy = 0; dy < SS; dy++) {
        for (let dx = 0; dx < SS; dx++) {
          const i = ((y * SS + dy) * n + x * SS + dx) * 4;
          r += buf[i]; g += buf[i + 1]; b += buf[i + 2]; a += buf[i + 3];
        }
      }
      const c = SS * SS;
      const o = (y * size + x) * 4;
      out[o] = Math.round(r / c);
      out[o + 1] = Math.round(g / c);
      out[o + 2] = Math.round(b / c);
      out[o + 3] = Math.round(a / c);
    }
  }
  return out;
}

// ── PNG ─────────────────────────────────────────────────────────

function crc32(buf) {
  let c = ~0;
  for (const byte of buf) {
    c ^= byte;
    for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1));
  }
  return ~c >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function png(rgba, size) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // 位深
  ihdr[9] = 6; // RGBA
  // 每行前面那个 0 是过滤器类型。漏了它整张图会错位成斜的
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0;
    rgba.copy(raw, y * (size * 4 + 1) + 1, y * size * 4, (y + 1) * size * 4);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

// ── ICO ─────────────────────────────────────────────────────────

/**
 * ICO 就是一张目录表加若干张图。Vista 以后每一张可以直接是 PNG，
 * 不必转成上下颠倒的 BMP。
 */
function ico(entries) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2); // 1 = 图标
  header.writeUInt16LE(entries.length, 4);

  const dir = Buffer.alloc(16 * entries.length);
  let offset = header.length + dir.length;
  entries.forEach((e, i) => {
    const o = i * 16;
    dir[o] = e.size >= 256 ? 0 : e.size; // 256 在这里写 0
    dir[o + 1] = e.size >= 256 ? 0 : e.size;
    dir[o + 2] = 0; // 调色板
    dir[o + 3] = 0;
    dir.writeUInt16LE(1, o + 4); // 色彩平面
    dir.writeUInt16LE(32, o + 6); // 位深
    dir.writeUInt32LE(e.data.length, o + 8);
    dir.writeUInt32LE(offset, o + 12);
    offset += e.data.length;
  });

  return Buffer.concat([header, dir, ...entries.map((e) => e.data)]);
}

// ── 产出 ────────────────────────────────────────────────────────

mkdirSync(OUT, { recursive: true });

const write = (name, buf) => {
  writeFileSync(OUT + name, buf);
  console.log(`  ${name}  ${buf.length} B`);
};

for (const [name, size] of [
  ['32x32.png', 32],
  ['128x128.png', 128],
  ['128x128@2x.png', 256],
  ['icon.png', 512],
]) {
  write(name, png(draw(size), size));
}

// §5.5：托盘要 16/20/24/32/40/48 六个尺寸。少了的话 150% 缩放下会被硬拉糊
const ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 256];
write(
  'icon.ico',
  ico(ICO_SIZES.map((size) => ({ size, data: png(draw(size), size) })))
);

// 安装器图标：NSIS 只认 ICO，尺寸要求宽松些
write(
  'installer.ico',
  ico([32, 48, 64, 128, 256].map((size) => ({ size, data: png(draw(size), size) })))
);

// 托盘的明暗两套（§5.5）。通知区的底色跟着任务栏主题走，
// 一套颜色总有一边糊在背景里
write(
  'tray-light.ico',
  ico(
    [16, 20, 24, 32, 40, 48].map((size) => ({
      size,
      data: png(draw(size, { fg: ACCENT, transparentBg: true }), size),
    }))
  )
);
write(
  'tray-dark.ico',
  ico(
    [16, 20, 24, 32, 40, 48].map((size) => ({
      size,
      data: png(draw(size, { fg: PAPER, transparentBg: true }), size),
    }))
  )
);

console.log('图标已生成。');
