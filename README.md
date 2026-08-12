# 知言 · 桌面端

卡片笔记应用。核心循环只有两步：**三秒内记下一个念头**，**在合适的时候让它自己回来**。

**Tauri 2 + WebView2**，Windows 10 1809+ / Windows 11。
施工依据是 [`docs/design-v1.1-tauri.md`](docs/design-v1.1-tauri.md)。
与并列备选的 GPUI 方案（[`docs/design-v2-gpui.md`](docs/design-v2-gpui.md)）的取舍见该文 §0。

## 布局（§4.5）

```
zhiyan/
├─ src/                     # 前端
│  ├─ index.html            # 主窗口
│  ├─ quick.html            # 速记浮窗
│  ├─ styles/               # tokens / base / chrome / timeline / overlays
│  ├─ js/                   # main / quick / store / markdown / render / editor / commands / platform
│  ├─ test/                 # node --test
│  └─ assets/fonts/         # 子集化 woff2（不入库）
├─ src-tauri/
│  ├─ src/                  # main / windows / error / state
│  ├─ crypto/               # §8 密钥体系（独立 crate，不依赖 Tauri）
│  ├─ db/                   # §9 SQLCipher（独立 crate）
│  ├─ capabilities/         # Tauri 2 权限
│  └─ tauri.conf.json
└─ scripts/subset-fonts.mjs
```

## 前后端怎么分（§4.4）

UI 跑在 WebView2 里，与 Rust 之间隔着一道 **IPC 边界**：

| 归 Rust | 归前端 |
|---|---|
| SQLCipher 存储、加密、同步 | DOM、样式、动效 |
| 统计聚合、FTS 检索 | Markdown 渲染 |
| 附件的加解密与落盘 | 文本输入（`<textarea>` 白送输入法） |

**铁律**：所有持久化只经 Rust 侧。前端不碰文件系统，加密、同步、迁移只有一处实现。

## 开发

```sh
npm install
npm run dev                    # Vite dev server，浏览器里就能开（走内存 mock）
npm test                       # 前端单测
cargo test -p zhiyan-crypto    # 加密层，秒级（不依赖 Tauri）
npm run tauri dev              # 完整应用
```

Linux 上跑需要 WebKitGTK：

```sh
sudo apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev \
                     libayatana-appindicator3-dev librsvg2-dev patchelf
```

字体（发布前必做，见 §3.1）：

```sh
ZHIYAN_FONT_SRC=/path/to/fonts npm run subset-fonts
```

Windows 发布构建：

```sh
npm run tauri build -- --features sqlcipher
```

**`--features sqlcipher` 不能省**：不开的话本地库是明文的，加密就全白做了（§9.2）。

## 几条不可动摇的约束

- **前端没有一个 `onclick`。** CSP 是 `script-src 'self'`（§12.2），内联处理器一律不执行。
  放开 `unsafe-inline` 等于把 XSS 的主要防线拆掉，而正文是用户可控的 Markdown。
- **`invoke()` 只出现在 `store.js` 与 `platform.js`。** 散着调迟早会有人把全量数据拉到前端。
- **`esc()` 必须在渲染的最前面**：先整段转义，再往里插我们自己生成的标签。顺序反了就是一个 XSS。
- **链接只放行 http/https**，渲染层、`platform.js`、capabilities 三道都挡。
- **`list_memos` 必须分页**（§9.6）。IPC 走 JSON 序列化，万级数据整包传输会明显卡顿。
- **组合串未上屏时屏蔽所有快捷键**（§6.2）。打拼音时的回车是选词，不是保存。
- **记录主键用 UUIDv4**，不用 ULID / UUIDv7：后两者前 48 位是明文时间戳。
- **正文类型手写 `Debug`**，错误对象也不带正文——它们会穿过 IPC 落进 WebView 日志。

上面几条里能被静态检查挡住的，CI 里都挡了。

## 分阶段交付

| 阶段 | 内容 | 状态 |
|---|---|---|
| 一 | Tauri 骨架 + 前端从原型落地 | ✅ |
| 二 | 密钥体系 / 信封 / 恢复码（§8） | ✅ `src-tauri/crypto` |
| 三 | SQLCipher / FTS5 / 墓碑 / blob（§9） | |
| 四 | IPC 命令层接线 + UUID 迁移 + `zhiyan://` 协议 | |
| 五 | 全局热键 / 托盘 / Snap Layouts / 前台焦点 / 打包（§5、§11） | |

与文档的逐条对照（含我做过的取舍与两处待你确认的）见
[`docs/tauri-decisions.md`](docs/tauri-decisions.md)。

## 许可

Apache-2.0
