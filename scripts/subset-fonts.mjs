/**
 * 字体子集化（§3.1 / §4.5）。
 *
 * **为什么必须子集化**：思源宋体 SC 全量 OTF 约 16MB，MiSans 约 9MB。原样打包会让
 * 安装包从 22MB 涨到 50MB 以上，直接抹掉 Tauri 相对 Electron 的体积优势。
 *
 * **为什么必须内嵌**：不内嵌则中文回退到 SimSun——为 96dpi 点阵设计的老宋体，
 * 放进 15.5px / 行高 1.82 的正文会糊成一片，整个设计的立足点消失。这不是可选项。
 *
 * 子集范围取 GB2312 常用字 + 标点 + ASCII + 常见符号，约 7000 字，压到 1.5–2.5MB/档。
 * 缺字时浏览器会自动回退到 font-family 里的下一档，不会显示豆腐块。
 *
 * ## 用法
 *
 * 需要 `fonttools`（`pip install fonttools brotli`）与上游字体文件：
 *
 * ```sh
 * ZHIYAN_FONT_SRC=/path/to/fonts node scripts/subset-fonts.mjs
 * ```
 *
 * 产物落到 `src/assets/fonts/`，**不入库**（见 .gitignore）——字体有各自的
 * 分发许可，把它们塞进 git 历史既臃肿又容易踩许可证的线。
 */

import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const OUT = join(ROOT, 'src/assets/fonts');
const SRC = process.env.ZHIYAN_FONT_SRC;

/** 上游文件 → 产物名。等宽用 Win11 内置的 Cascadia Mono，不打包。 */
const FACES = [
  ['SourceHanSerifSC-Regular.otf', 'SourceHanSerifSC-Regular.subset.woff2'],
  ['SourceHanSerifSC-SemiBold.otf', 'SourceHanSerifSC-SemiBold.subset.woff2'],
  ['MiSans-Regular.ttf', 'MiSans-Regular.subset.woff2'],
  ['MiSans-Medium.ttf', 'MiSans-Medium.subset.woff2'],
];

/**
 * 子集范围。
 *
 * `--unicodes` 用区间而不是逐字列举：正文里会出现什么字无法预知，
 * 按「用得到的字」裁剪迟早会在某个生僻姓氏上翻车。
 */
const UNICODES = [
  'U+0020-007E', // ASCII
  'U+00A0-00FF', // 拉丁补充
  'U+2000-206F', // 通用标点（含盘古之白要用的窄空格 U+2009）
  'U+2E80-2EFF', // 康熙部首
  'U+3000-303F', // CJK 标点
  'U+3400-4DBF', // 扩展 A
  'U+4E00-9FFF', // CJK 统一表意文字
  'U+FF00-FFEF', // 全角形式
  'U+2190-21FF', // 箭头（界面里的 ‹ › ↺ 之类）
  'U+2460-24FF', // 带圈数字
  'U+25A0-25FF', // 几何图形（时间脊与图标退化时用）
  'U+2600-26FF', // 杂项符号
].join(',');

function main() {
  if (!SRC) {
    console.error('需要 ZHIYAN_FONT_SRC 指向上游字体目录');
    console.error('例：ZHIYAN_FONT_SRC=~/fonts node scripts/subset-fonts.mjs');
    process.exit(1);
  }
  mkdirSync(OUT, { recursive: true });

  let done = 0;
  for (const [from, to] of FACES) {
    const input = join(SRC, from);
    if (!existsSync(input)) {
      console.warn(`跳过：找不到 ${input}`);
      continue;
    }
    execFileSync(
      'pyftsubset',
      [
        input,
        `--unicodes=${UNICODES}`,
        '--flavor=woff2',
        // 布局特性只留排版必需的，其余（花体、竖排、旧式数字）全部丢掉
        '--layout-features=kern,liga,locl,ccmp,vert,vrt2',
        '--no-hinting',
        '--desubroutinize',
        `--output-file=${join(OUT, to)}`,
      ],
      { stdio: 'inherit' }
    );
    console.log(`✓ ${to}`);
    done++;
  }

  if (!done) {
    console.error('一个字体都没生成。检查 ZHIYAN_FONT_SRC 里的文件名是否与 FACES 一致。');
    process.exit(1);
  }
  console.log(`\n生成 ${done} 个字体到 ${OUT}`);
  console.log('提醒：这些文件不入库，打包机上要各跑一次。');
}

main();
