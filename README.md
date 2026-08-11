# 知言 · 桌面端

卡片笔记应用。核心循环只有两步：**三秒内记下一个念头**，**在合适的时候让它自己回来**。

**Rust + Tauri 2 + WebView2**，Windows 10 1809+ / Windows 11。

规格来自 [`docs/design-v2-gpui.md`](docs/design-v2-gpui.md) —— 注意那份是 v2.0（GPUI）文档，
本仓库走的是它所取代的 **v1.0（Tauri）路线**。两者的差异与我自己补的取舍记在
[`docs/tauri-decisions.md`](docs/tauri-decisions.md)。

## 布局

```
zhiyan/
├─ crates/
│  ├─ zhiyan-app/     # Tauri 应用壳 + IPC 命令层（含 tauri.conf.json / capabilities）
│  ├─ zhiyan-core/    # 纯逻辑：标签 / 引用 / 统计聚合
│  ├─ zhiyan-crypto/  # 加密原语
│  ├─ zhiyan-store/   # SQLCipher
│  └─ zhiyan-sync/    # 同步
├─ ui/                # 前端：零依赖的 HTML/CSS/JS，没有打包器
│  ├─ index.html
│  ├─ styles/         # 从原型精确切分而来
│  ├─ src/            # 模块化的 JS
│  └─ test/           # node --test
└─ xtask/             # 打包、字体子集化、签名
```

## 前后端怎么分

Tauri 方案里 UI 跑在 WebView 里，前后端之间隔着一道 **IPC 边界**。这道边界决定了什么放哪边：

| 归 Rust | 归前端 |
|---|---|
| SQLCipher 存储、加密、同步 | DOM、样式、动效 |
| 统计聚合、FTS 检索 | Markdown 渲染 |
| 附件的加解密与落盘 | 文本输入（`<textarea>` 白送输入法） |

**聚合必须在 Rust 侧做完再过来**，不能把全量数据搬到前端再 `Array.filter`——
序列化一万条片语的开销足以让界面卡住。

## 开发

前端零依赖，可以脱离 Tauri 单独开发（`api.js` 会自动退回内存 mock）：

```sh
cd ui && python3 -m http.server 5173     # 然后开 http://localhost:5173
cd ui && node --test "test/**/*.test.js"
```

整个应用：

```sh
cargo test --workspace
cargo tauri dev          # 需要 cargo-tauri：cargo install tauri-cli --version '^2'
```

Linux 上跑需要 WebKitGTK：

```sh
sudo apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev \
                     libayatana-appindicator3-dev librsvg2-dev patchelf
```

Windows 发布构建：

```sh
cargo tauri build --features sqlcipher
```

**`--features sqlcipher` 不能省**：不开的话本地库是明文的，端到端加密就白做了（§10.2）。

## 几条不可动摇的约束

- **前端没有一个 `onclick`。** CSP 是 `script-src 'self'`，内联处理器一律不执行。
  放开 `unsafe-inline` 等于把 XSS 的主要防线拆掉，而正文是用户可控的 Markdown。
- **`esc()` 必须在渲染的最前面**：先整段转义，再往里插我们自己生成的标签。顺序反了就是一个 XSS。
- **链接只放行 http/https**，渲染层与打开时各挡一道。
- **`api.js` 是前端唯一能调 Rust 的地方**，散着调 `invoke` 迟早会有人把全量数据拉到前端。
- **正文类型必须手写 `Debug`**，错误对象也不许带正文——它们会穿过 IPC 落进 WebView 日志。
- **记录主键用 UUIDv4**，不用 ULID / UUIDv7：后两者前 48 位是明文时间戳。
- **`ui/fonts/` 里的 woff2 不入库**，由 `xtask` 从上游字体子集化生成。

## 分阶段交付

| 阶段 | 内容 | 状态 |
|---|---|---|
| 一 | Tauri 骨架 + 前端从原型落地 | 进行中 |
| 二 | `zhiyan-crypto`：密钥体系 / 信封 / 恢复码 | |
| 三 | `zhiyan-core` + `zhiyan-store`：SQLCipher / FTS5 / 墓碑 / blob | |
| 四 | IPC 命令层 + 前端接线 + UUID 迁移 | |
| 五 | 系统集成：全局热键 / 托盘 / Snap Layouts / 速记浮窗 | |

## 许可

Apache-2.0
