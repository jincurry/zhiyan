import { defineConfig } from 'vite';
import { resolve } from 'node:path';

/**
 * §4.3：**不引入框架**，Vite 只用其 ESM 打包与 dev server。
 *
 * 两个入口 = 两个窗口。Vite 会按入口切 chunk，速记浮窗因此不会把主窗口的
 * 时间线、统计、每周回顾一起加载进来——浮窗要的是 80ms 内唤出，多加载
 * 十几 KB 没必要。
 */
export default defineConfig({
  root: 'src',
  publicDir: 'assets',
  // Tauri 的 CLI 会读 tauri.conf.json 的 devUrl，端口固定住免得两边对不上
  server: { port: 5173, strictPort: true },
  build: {
    outDir: '../dist',
    emptyOutDir: true,
    // WebView2 的下限是 Chromium 111（§4.2）
    target: 'chrome111',
    // 发布构建不留 sourcemap：它会把正文相关的逻辑连同注释一起带进安装包
    sourcemap: false,
    rollupOptions: {
      input: {
        main: resolve(import.meta.dirname, 'src/index.html'),
        quick: resolve(import.meta.dirname, 'src/quick.html'),
      },
    },
  },
  // 前端不该有任何环境变量注入的余地
  envPrefix: [],
});
