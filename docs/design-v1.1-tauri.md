# 知言 · 桌面端设计文档（Tauri 方案）

**版本** v1.1 · 2026-08-05
**技术栈** Tauri 2.x + WebView2，Windows 10 1809+ / Windows 11
**状态** 完整施工文档，**与 v2.0（GPUI 方案）并列为备选**，待 P0 验证后择一

---

## 0. 两个方案的关系

本项目目前保留**两条并列的技术路线**，均为完整可施工的方案：

| | **本文（v1.1，Tauri + WebView2）** | **v2.0（Rust + GPUI）** |
|---|---|---|
| UI 层 | HTML/CSS/JS，原型代码可直接复用 | Rust 重写，原型降级为视觉规格 |
| 单机版工期 | **约 8 周** | 约 15 周 |
| 中文输入法 | **系统白送** | 需自实现，存亡级风险 |
| 中文排版（避头尾/混排间距） | **浏览器原生** | 需自实现 |
| 富文本编辑器 | contenteditable / textarea 白送 | 需自建（18 天） |
| 无障碍 | 白送 | 基本没有 |
| 安装包 | 22–26 MB | 45–70 MB |
| 常驻内存 | 80–120 MB | 60–90 MB |
| 冷启动 | 0.4–0.8 s | 0.1–0.3 s |
| 热键唤出 | < 80 ms | < 30 ms |
| 运行时依赖 | WebView2 运行时 | 无 |
| 渲染一致性 | 随系统 Edge 漂移 | 完全自控 |
| 开发迭代 | 热重载，秒级 | 编译等待，无热重载 |

**决策关卡（P0，5 天）**：验证 GPUI 的中文输入法、DPI/多屏、GPU 兼容性。

- P0 通过 → 可选 v2.0，换取更极致的性能与自控力
- P0 不通过 → 执行本文方案
- **工期压力大或团队无 Rust UI 经验 → 直接执行本文方案，不必做 P0**

数据模型（§7）、端到端加密（§8）、本地存储（§9）、云同步（§10）两份文档**完全一致**，无论选哪条路线都不需要重做。这是有意为之：把选型风险限制在 UI 层。

---

## 目录

1. [产品定位与设计原则](#1-产品定位与设计原则)
2. [功能规格](#2-功能规格)
3. [设计系统](#3-设计系统)
4. [技术架构](#4-技术架构)
5. [窗口模型与 Windows 集成](#5-窗口模型与-windows-集成)
6. [前端实现](#6-前端实现)
7. [数据模型](#7-数据模型)
8. [端到端加密](#8-端到端加密)
9. [本地存储](#9-本地存储)
10. [云同步](#10-云同步)
11. [打包、签名与更新](#11-打包签名与更新)
12. [安全与隐私](#12-安全与隐私)
13. [日志与可观测性](#13-日志与可观测性)
14. [测试策略](#14-测试策略)
15. [里程碑](#15-里程碑)
16. [待决策项](#16-待决策项)
17. [附录](#17-附录)

---

## 1. 产品定位与设计原则

### 1.1 是什么

知言是一款**卡片笔记**桌面应用。核心循环只有两步：**三秒内记下一个念头**，**在合适的时候让它自己回来**。

对标 flomo，差异在两处：桌面端的精致度与原生集成（全局速记、命令面板、时间脊），以及端到端加密。

### 1.2 三条设计原则

**① 界面后退，文字前进。**
应用 chrome 用小字号无衬线，用户写下的正文用宋体、大字号、宽行距。视觉重量的分配是有立场的：主角是用户的句子，不是控件。

**② 结构必须是内容的副产品，而不是前置作业。**
没有标题、没有文件夹、没有必填字段。标签靠正文里的 `#` 顺手产生，引用靠 `[[` 顺手产生，待办靠 `- [ ]` 顺手产生。任何要求"先归类再记录"的设计都会杀死记录本身。

**③ 写下的东西只有回头看见才算数。**
拾遗、每周回顾、时间脊、热力图不是统计功能，是产品的另一半。一个只能写不能回看的笔记应用，等价于一个漂亮的垃圾桶。

### 1.3 一处刻意的克制

**待办不做成独立 Todo 页面**，只在每周回顾汇总未勾选项。片语里的待办是**写作的副产品**，不是任务系统。做成独立清单会引诱用户把这个应用当待办工具用，那就偏离了定位。

---

## 2. 功能规格

### 2.1 记录

| 功能 | 说明 |
|---|---|
| 主编辑器 | 常驻主窗口顶部，接在时间脊起点（"此刻"节点） |
| 速记浮窗 | 全局热键唤起的无边框小窗，可拖动并记忆位置 |
| Markdown | 粗体、斜体、删除线、高亮、行内码、代码块、二~四级标题、引用、有序/无序列表、待办、链接、分割线 |
| 待办 | `- [ ]` 渲染为可点击复选框，勾选直接改写源文本 |
| 标签 | `#主题` 或 `#主题/子题`，输入时自动补全 |
| 行内引用 | `[[` 触发片语选择器，存为 `[[片段^uuid]]`，渲染为可点击小卡片 |
| 图片 | 工具栏插入或直接粘贴，点击放大 |
| 编辑器辅助 | 工具条、实时预览、回车自动续列表、空行跳出、Ctrl+B/I |

### 2.2 组织

| 功能 | 说明 |
|---|---|
| 标签树 | 两级嵌套、可折叠、可置顶、可重命名（同步更新所有正文）、可移除（保留片语） |
| 置顶片语 | 独立分组显示在时间线顶部，脊线为虚线 |
| 关联 | 手动选择器建立双向关联；行内引用自动产生反向链接 |
| 回收站 | 软删除、可恢复、可彻底删除（写墓碑） |
| 排序 | 最新在前 / 最早在前 |
| 搜索 | 本地全文，结果高亮 |

### 2.3 回看

| 功能 | 说明 |
|---|---|
| 时间脊 | 贯穿时间线的竖线，每天一个节点，**节点大小随当天字数变化** |
| 拾遗 | 随机漫步，可限定标签范围，可跳回原处 |
| 每周回顾 | 可翻周；条数/字数/活跃天/平均，七日柱状图，标签分布，**未勾选待办汇总**，值得再看的三条，整周导出 |
| 统计 | 半年热力图（点击筛选当天）、连续/最长连续/累计字数/日均、标签分布 |
| 每日目标 | 侧栏底部进度条 |

### 2.4 系统与账号

| 功能 | 说明 |
|---|---|
| 命令面板 | Ctrl+K，可执行命令、跳标签、搜片语 |
| 深浅外观 | 手动切换，跟随系统为可选项 |
| 分享成图 | canvas 现绘，跟随当前主题配色，存 PNG |
| 导入导出 | Markdown 导出、JSON 备份导入导出 |
| 账号 | 资料编辑、会员状态、同步开关、恢复码管理、登录设备、注销 |

---

## 3. 设计系统

### 3.1 字体分工（三重）

这是"精致"的主要载体，也是 Windows 移植中最容易失败的一环。

| 用途 | 字体 | 来源 | 说明 |
|---|---|---|---|
| 正文 | 思源宋体 SC Regular / SemiBold | **打包**（SIL OFL 1.1） | 用户写的字 |
| 界面 | MiSans Regular / Medium | **打包**（免费商用） | 按钮、标签、说明 |
| 数据 | Cascadia Mono | Win11 内置 | 日期、计数、字数，等宽对齐 |

**为什么必须打包**：不打包则中文一路回退到 **SimSun**。SimSun 是为 96dpi 点阵显示设计的老宋体，放进 15.5px、行高 1.85 的正文里会糊成一片——整个设计的立足点直接消失。这不是可选项。

**子集化**：全量思源宋体单字重约 20MB，四字重 80MB 不可接受。按 GB2312 常用字 + 常用标点 + ASCII 裁剪后每字重 3–4MB，**woff2 格式**四字重合计约 14MB。生僻字回退 SimSun，可接受。

> woff2 带 Brotli 压缩，比 GPUI 方案必须用的 OTF 小约 40%。这是本方案安装包更小的主要原因之一。

```css
@font-face {
  font-family: "Zhiyan Serif";
  src: url("../assets/fonts/SourceHanSerifSC-Regular.subset.woff2") format("woff2");
  font-weight: 400;
  font-display: block;   /* 非 swap：本地字体加载极快，block 可避免首屏 SimSun 跳变 */
}
:root {
  --serif: "Zhiyan Serif", Georgia, "SimSun", serif;
  --sans:  "Zhiyan Sans", "Segoe UI Variable Text", "Segoe UI", "Microsoft YaHei UI", sans-serif;
  --mono:  "Cascadia Mono", Consolas, monospace;
}
```

**ClearType 适配**：Windows 的 DirectWrite 渲染比 macOS 更瘦。正文行高 1.82 → **1.85**；加粗用 SemiBold(600) 而非 Bold(700)，避免中文粗体过重；**移除** `-webkit-font-smoothing: antialiased`（macOS 专用，Windows 上无效甚至有害）。

### 3.2 中文排版：本方案的一处白送优势

浏览器引擎原生处理这两件事，**无需任何代码**：

- **避头尾**：`，。、；：！？》」` 不出现在行首，`《「（` 不出现在行尾
- **中西文混排间距**：Chromium 的 `text-spacing` 相关行为

在 GPUI 方案里这两项需要自行实现排版前处理。中文正文里逗号跑到行首非常刺眼，会直接破坏"精致"的观感——本方案不必操心。

### 3.3 配色

纯白画布 + 描边分区，不靠底色深浅划分层次。

```css
:root {
  --paper:#FFFFFF;  --chrome:#FAFAF9;  --card:#FFFFFF;
  --card-2:#F5F6F3; --card-3:#EDEFEA;
  --ink:#1B1E19;    --ink-2:#565B52;   --ink-3:#9AA096;
  --line:#E2E5DF;   --line-soft:#EDEFEA;
  --accent:#2E5A49;      /* 松绿：主色 */
  --accent-soft:#CFDFD3; --accent-dim:#EAF1EB;
  --mark:#8A6A1F;        /* 赭金：只用于"今天"与"拾遗" */
  --danger:#9C4332;
  --h0:#F1F2EE; --h1:#D6E4D8; --h2:#A5C2AD; --h3:#5C8B70; --h4:#2E5A49;
}
html[data-theme="dark"] { /* 独立一套变量 */ }
```

侧栏与右栏用 `--chrome`（几乎察觉不到的暖白）与内容区区分。

**赭金的使用纪律**：只出现在两个地方——时间脊的"今天"节点、拾遗模块的时间标签。一旦扩散到第三处就失去标记意义。

### 3.4 时间脊（签名元素）

不用 GitHub 式绿方格——那是所有 flomo 仿品的标准答案。改为贯穿时间线左侧的一条竖脊：

```
此刻 ●━━━┓
NOW      ┃  [ 编辑器 ]
         ┃
         ┃  [ 筛选栏 ]
8月5日 ●━┫
周三     ┃  [ 卡片 ]
         ┃  [ 卡片 ]
8月4日 ○━┫
周二     ┃  [ 卡片 ]
```

- 节点直径 = `clamp(7, 6 + 当天字数/45, 15)` px
- 今天填赭金并带外环；字数 > 200 的日子填松绿实心；其余空心
- 置顶分组脊线为虚线（`repeating-linear-gradient`）

**编辑器必须接在脊上。** 原型早期版本中编辑器左边缘在 0、卡片在 78px，两条左边缘错开近 80px，视觉上"总感觉怪怪的"。正确解法不是把卡片拉回去，而是让编辑器成为脊的起点——整条时间线从"你正在写的这一刻"开始往回走。筛选栏同样对齐，左边缘是一条干净的直线。

**实现**：`.group` 与 `.spine` 共用 `padding-left:78px` 与 `::before` 竖线（`left:63px`），编辑器包在 `.group.now` 中。

热力图不在主界面重复出现，收进右栏"统计"页签。时间脊已经承担了"哪天写得多"，两个都摆是冗余。

### 3.5 Windows 视觉规范

| 项 | 规格 |
|---|---|
| 标题栏高度 | 32px，品牌居左，系统按钮居右 |
| 系统按钮 | 46×32，图标 10×10；关闭键 hover `#C42B1C`、按下 `#D14B3D`、图标转白 |
| 圆角 | 面板/卡片/弹层 8px，控件/按钮 4px（**不是** macOS 的 10–14px） |
| 导航选中态 | Win11 左侧强调条：3×16px 圆角竖条 + 浅底 |
| 通知 | 右下角，非居中 |
| 滚动条 | Win11 悬浮细条，hover 加深 |
| 开关 | Fluent 形态：关闭态描边 + 灰色小圆点，开启态填充 + 白点 |
| 焦点框 | 2px 实心 + 1px 反色描边（高对比度模式可见） |

原型 `zhiyan-windows.html` 已按此实现，可直接取值。

---

## 4. 技术架构

### 4.1 选型：Tauri 2

| 指标 | Tauri 2 (WebView2) | Electron 33 |
|---|---|---|
| 安装包（含字体） | 22–26 MB | 90–150 MB |
| 常驻内存（主窗+浮窗） | 80–120 MB | 250–350 MB |
| 冷启动 | 0.4–0.8 s | 1.2–2.5 s |
| 热键唤出（已驻留） | < 80 ms | 120–250 ms |

**决策依据**：知言常驻托盘、按热键即时唤出，**空闲内存与唤出延迟直接构成产品体验**。

**何时改选 Electron**：① 团队无 Rust 储备且工期紧；② 近期做 macOS 版并希望共用无边框窗口与热键代码；③ 依赖只有 Node 生态才有的原生模块。

### 4.2 WebView2 运行时

- 分发用 **Evergreen Bootstrapper**（约 2MB，缺失时联网安装）。不用 Fixed Version（+130MB 且需自行跟进安全更新）。
- 覆盖率：Win11 内置；Win10 自 2021 年随 Edge 推送，实测 > 97%。
- 安装器需检测运行时存在性（`HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F01...}`），缺失且离线时给出可操作提示，**不能启动后白屏**。
- 目标最低 **Chromium 111**（2023-03）。原型用到的 `color-mix()`、`backdrop-filter`、`::-webkit-scrollbar`、`aspect-ratio`、`:has()`、正则后行断言均需此版本以上。低于此版本在启动时提示升级 Edge。

### 4.3 前端不引入框架

现有原型是无依赖原生 JS，约 1700 行。**保持这个状态**，只做 ESM 模块拆分。理由：

- 应用复杂度集中在数据同步与系统集成，不在 UI 状态管理
- 无构建产物意味着 WebView2 兼容性问题可直接在 DevTools 里定位
- 体积与冷启动收益明显

构建用 **Vite**，只用其 ESM 打包与 dev server，不引入框架运行时。

### 4.4 分层

```
┌──────────────────── Tauri App ────────────────────┐
│  Rust 主进程（core）                                │
│  ├─ 窗口      main / quick                         │
│  ├─ 热键      global-shortcut + 冲突检测            │
│  ├─ 托盘      tray-icon + Jump List                │
│  ├─ 加密      Argon2id / HKDF / XChaCha20          │
│  ├─ 数据      SQLCipher (WAL) + blob 存储          │
│  ├─ 同步      outbox + 变更游标                     │
│  └─ 平台      windows-rs（前台/DPI/AUMID/Mica）     │
│         ▲                                          │
│         │ IPC (invoke / event) + zhiyan:// 协议    │
│         ▼                                          │
│  WebView2 渲染层（无框架 ESM）                      │
│  ├─ main window   时间脊/编辑器/侧栏/右栏           │
│  └─ quick window  速记浮窗                          │
└────────────────────────────────────────────────────┘
```

**铁律**：所有持久化只经 Rust 侧。前端不碰文件系统，加密、同步、迁移只有一处实现。

### 4.5 项目结构

```
zhiyan/
├─ src/                          # 前端
│  ├─ index.html                 # 主窗口
│  ├─ quick.html                 # 速记浮窗
│  ├─ styles/
│  │  ├─ tokens.css              # CSS 变量（明/暗）
│  │  ├─ base.css                # 重置 + 排版
│  │  ├─ chrome.css              # 标题栏 / 系统按钮
│  │  ├─ timeline.css            # 时间脊 / 卡片
│  │  └─ overlays.css            # 命令面板 / 弹层 / 菜单
│  ├─ js/
│  │  ├─ main.js                 # 主窗口入口
│  │  ├─ quick.js                # 浮窗入口
│  │  ├─ store.js                # 数据访问（唯一 invoke 出口）
│  │  ├─ markdown.js             # md() / inline() / 高亮
│  │  ├─ render.js               # 时间线 / 侧栏 / 统计渲染
│  │  ├─ editor.js               # 编辑器 / 补全 / 快捷键
│  │  ├─ commands.js             # 命令面板注册表
│  │  └─ platform.js             # 窗口控制 / 快捷键显示名
│  └─ assets/fonts/              # 子集化 woff2
├─ src-tauri/
│  ├─ src/
│  │  ├─ main.rs                 # 入口 + Builder 组装
│  │  ├─ windows.rs              # 窗口创建 / 定位 / 置前台
│  │  ├─ hotkey.rs               # 全局热键注册与冲突处理
│  │  ├─ tray.rs                 # 托盘 / Jump List
│  │  ├─ crypto/  keys.rs envelope.rs blob.rs
│  │  ├─ db/      mod.rs memo.rs blob.rs migrations/
│  │  ├─ sync/    outbox.rs pull.rs push.rs
│  │  └─ platform_win.rs         # windows-rs 调用
│  ├─ capabilities/default.json  # Tauri 2 权限
│  ├─ icons/
│  └─ tauri.conf.json
└─ scripts/subset-fonts.mjs
```

---

## 5. 窗口模型与 Windows 集成

### 5.1 主窗口

| 属性 | 值 |
|---|---|
| 尺寸 | 1180×760，最小 900×560（低于 900 隐藏右栏，低于 780 隐藏侧栏） |
| `decorations` | `false`（自绘标题栏） |
| 位置记忆 | 需持久化，含所在显示器；还原前校验该显示器仍存在 |
| 关闭行为 | 隐藏而非退出（设置项可改为真退出） |

### 5.2 自绘标题栏的两个必做项

**① 拖拽区域**：容器加 `data-tauri-drag-region`，交互元素必须排除，否则点击被吞。原型已按此实现。

**② Snap Layouts —— 最容易漏的一项。**

Win11 用户悬停最大化按钮 1 秒会期待弹出贴靠布局面板。自绘标题栏默认没有，用户会立刻察觉"这软件不对劲"。

```rust
// platform_win.rs
// 1. SetWindowSubclass 挂子类化过程
// 2. WM_NCHITTEST: 命中最大化按钮矩形 → 返回 HTMAXBUTTON
// 3. 返回 HTMAXBUTTON 后系统接管该区域鼠标消息，DOM click 不再触发，
//    必须自行处理 WM_NCLBUTTONDOWN / WM_NCLBUTTONUP 触发实际最大化
// 4. WM_NCMOUSELEAVE: 清除 hover 态并通知前端
```

按钮矩形由前端通过 `invoke("set_maxbutton_rect", {x,y,w,h})` 在布局与 DPI 变化时上报。

### 5.3 速记浮窗

| 属性 | 值 |
|---|---|
| 尺寸 | 460 × 自适应（96–360） |
| `skip_taskbar` / `always_on_top` / `transparent` | `true` |
| `resizable` | `false` |
| 生命周期 | **常驻但隐藏**（`hide()` 非 `close()`），保证唤出 < 80ms |

常驻代价约 30–40MB（一个 WebView 实例）。可提供设置项：隐藏 10 分钟后销毁、下次重建（唤出延迟升至 300–500ms）。

**三个必须处理的问题：**

**① 前台焦点抢占。** Windows 前台锁定机制会让非前台进程的 `SetForegroundWindow` 静默失败，只闪任务栏。热键触发时系统通常短暂授权，但**不保证**：

```rust
let fg = GetForegroundWindow();
let fg_tid = GetWindowThreadProcessId(fg, None);
let cur_tid = GetCurrentThreadId();
AttachThreadInput(cur_tid, fg_tid, true);
SetForegroundWindow(hwnd);
SetFocus(hwnd);
AttachThreadInput(cur_tid, fg_tid, false);
```

调用后前端还需 `element.focus()`，两层都要做。

**② 多显示器定位。** 必须出现在**鼠标当前所在的显示器**：`MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST)` 取工作区，居中偏上（工作区高度 20%）。记忆位置时存"相对某显示器的偏移"而非绝对坐标——否则拔掉外接屏后窗口跑到屏幕外。

**③ 混合 DPI。** Windows 允许 4K@150% 与 1080p@100% 并存。清单声明 `PerMonitorV2`（Tauri 默认已开），坐标换算必须区分物理/逻辑像素（`Monitor::scale_factor()`），否则拖到副屏偏移或模糊。

### 5.4 全局热键

**原型中的 `Ctrl+Shift+N` 必须换掉**：它在 Chrome/Edge 是"新建无痕窗口"，在资源管理器是"新建文件夹"。`RegisterHotKey` 全局抢占，注册即砸掉用户这两个习惯。

| 功能 | 键位 | 作用域 |
|---|---|---|
| 唤起/收起速记浮窗 | `Ctrl+Alt+Space` | 全局 |
| 显示/隐藏主窗口 | `Ctrl+Alt+Z` | 全局（默认关） |
| 命令面板 | `Ctrl+K` | 应用内 |
| 保存 | `Ctrl+Enter` | 应用内 |
| 本周回顾 | `Ctrl+Alt+R` | 应用内 |

**注册失败必须可见。** `ERROR_HOTKEY_ALREADY_REGISTERED` 说明被占用（常见占用方：搜狗输入法、QQ、PowerToys、Ditto）。静默失败会让用户以为软件坏了。

```rust
match app.global_shortcut().register(shortcut) {
    Ok(_)  => state.set_hotkey_ok(true),
    Err(e) => {
        state.set_hotkey_ok(false);
        // 托盘角标 + 设置页红字 + 首次失败弹一次通知
        app.emit("hotkey-conflict", HotkeyConflict { combo, reason: e.to_string() })?;
    }
}
```

设置页必须提供**改键 UI**：捕获按键的输入框，实时试注册，成功才保存。

**与输入法的交互**：中文输入法激活时，部分输入法会占用组合键。实测微软拼音默认占用 `Shift+Space` 而非 `Ctrl+Alt+Space`，冲突风险可接受，改键 UI 是兜底。

### 5.5 托盘、通知、自启、Mica

**托盘菜单**：速记 / 打开主窗口 / 本周回顾 / 立即同步（显示状态）/ 设置 / 退出。左键单击切换主窗口显隐，双击显示并聚焦。图标需 16/20/24/32/40/48px 多尺寸 ICO，明暗两套，监听 `WM_SETTINGCHANGE` 切换。

**Jump List**：任务栏右键的"速记 / 本周回顾 / 打开主窗口"，通过 `ICustomDestinationList`。成本很低但原生感强。

**Toast 通知**：**必须先注册 AppUserModelID**，否则通知根本不显示——最常见的踩坑点。

```rust
SetCurrentProcessExplicitAppUserModelID(w!("Zhiyan.Desktop"));
```

安装器创建的开始菜单快捷方式必须写入相同的 `System.AppUserModel.ID`。MSIX 打包时由清单自动提供。

**开机自启**：`tauri-plugin-autostart`（写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`）。注意用户可能在「设置→应用→启动」关闭，此时注册表项仍在但被 `StartupApproved` 禁用。设置页显示的必须是**真实生效状态**，否则会出现"开关是开的但没自启"。

**Mica / Acrylic**：Win11 22H2+ 通过 `DWMWA_SYSTEMBACKDROP_TYPE` 给浮窗设 Acrylic、主窗口可选 Mica：

```rust
let backdrop = DWMSBT_TRANSIENTWINDOW; // Acrylic
DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &backdrop as *const _ as _, 4);
```

启用后**应移除 CSS 的 `backdrop-filter`**，避免双重模糊。Win10 降级为不透明。

---

## 6. 前端实现

### 6.1 从原型到工程

原型 `zhiyan-windows.html` 是单文件，需拆为 ESM 模块（§4.5）。拆分原则：

- `store.js` 是**唯一**调用 `invoke()` 的模块，其余模块只与它对话
- `markdown.js` 保持纯函数，无 DOM 依赖，可单测
- `render.js` 负责 DOM 生成，不含业务判断
- 事件委托保留（原型已用），避免为上万条卡片挂监听

### 6.2 编辑器

本方案的最大红利：`<textarea>` 白送中文输入法、光标选区、撤销重做、剪贴板、字素簇边界。这些在 GPUI 方案里合计约 18 天工作量。

需要自己实现的仅剩：

| 能力 | 说明 |
|---|---|
| 自动续行 | 回车续列表/待办/有序号，空行跳出（原型已实现） |
| 补全弹层 | `#` 与 `[[` 触发，方向键选择，回车/Tab 插入，Esc 关闭（原型已实现） |
| 实时预览 | 切换 textarea / 渲染视图（原型已实现） |
| 工具条 | 包裹选区、行首插入（原型已实现） |

**组合串保护**：输入拼音过程中按下的键不应触发应用快捷键。监听 `compositionstart` / `compositionend`，组合期间屏蔽 `Ctrl+Enter` 等 Action。

> 这一条在 WebView 里只需几行；GPUI 方案里需要贯穿整个 IME 实现。

### 6.3 虚拟化

时间线可能有上万条。原型目前全量渲染，需改为虚拟滚动：

- 用 `IntersectionObserver` 或手动计算可视区
- 卡片高度不定，需维护高度缓存（渲染后测量并记录）
- 分页取数：`store.list({ cursor, limit: 50 })`

**性能目标**：万条数据滚动稳定 60fps。

### 6.4 动效

CSS transition 原生支持，原型已有的效果全部保留：卡片入场（8px 上浮 + 淡入 380ms）、弹层 pop、Toast 滑入、hover 过渡（120–200ms）、节点大小过渡。

`prefers-reduced-motion` 媒体查询原生支持，原型已实现。

---

## 7. 数据模型

### 7.1 记录主键：UUIDv4

**不用 ULID，也不用 UUIDv7。** 服务端必须知道记录主键才能做增量同步，而 ULID/UUIDv7 的前 48 位是明文毫秒时间戳——等于把每条记录的创建时刻直接送给服务端。E2EE 下这是不可接受的元数据泄漏。

用 **UUIDv4**（纯随机，16 字节）。本地排序靠 `created_at` 列 + 索引，功能上无损失，只是失去"主键字典序即时间序"的便利。

行内引用 `[[片段^uuid]]` 内嵌的也是 UUIDv4，用户看不到（渲染成引用小卡片）。

### 7.2 记录的规范形态

```json
{
  "v": 1,
  "text": "…完整 Markdown 正文，含 #标签 与 [[引用^uuid]]…",
  "createdAt": 1754380320000,
  "updatedAt": 1754380911000,
  "deletedAt": null,
  "pinned": false,
  "source": "quick",
  "tags": ["读书/认知", "产品"],
  "refs": ["9f2c…", "3a7b…"],
  "blobs": [{ "sha256": "a3f9…", "mime": "image/jpeg", "w": 1600, "h": 1200, "size": 184320 }],
  "pad": "………"
}
```

**标签、引用、附件元信息全部属于密文内容。** 这点容易忽略：若为了服务端按标签筛选而明文上传标签，等于送出用户的主题结构——「读书/心理学」「求职」这类标签本身就高度敏感。筛选一律本地做。

### 7.3 相对原型的三处关键修正

**① 附件外置。** 原型把图片存成 base64 塞进 JSON，2MB 图片膨胀成 2.7MB 文本，每次读写整包搬运。改为内容寻址，数据库只存 hash。

**② 硬删除改墓碑。** `purge()` 在 A 机彻底删除、B 机不知道，下次同步又推回来——"删不掉的笔记"是同步应用最经典的 bug。墓碑保留 90 天后由后台任务真删。

**③ 单键整包 JSON 改分表。** 原型用 `window.storage` 存一个大 JSON，改为 SQLCipher 分表 + 分页查询。

**迁移**：一次性脚本，建 `int → UUIDv4` 映射表，同时重写 `rel[]` 数组与正文中所有 `[[...^数字]]`。**趁数据量小尽早做。**

---

## 8. 端到端加密

### 8.1 威胁模型

**防得住**：拿到服务端库全量导出的人、拿到对象存储全部文件的人、服务端运维、偷走关机笔记本的人、中间人。以上均读不到任何正文。

**防不住（必须诚实告知）**：

- 已登录 Windows 会话下运行的恶意程序（免密启动的必然代价）
- 内存中的明文（WebView 渲染的是解密后的正文）
- 元数据：记录条数、每条大小、写入时刻——**写入时刻序列基本等价于热力图和作息**
- 用户主动导出的 Markdown（设计上就该是明文）

**明确的取舍**：**忘记密码 = 数据永久丢失**。服务端无任何恢复手段。因此恢复码是注册流程的强制环节。

### 8.2 密钥体系

```
用户密码
   │  Argon2id(salt=服务端随机盐, m=64MiB, t=3, p=4)
   ▼
MK  主密钥 (32B) ─────────────── 永不离开设备
   ├─ HKDF(MK,"auth") ─▶ AuthKey ─▶ 服务端（再 Argon2id 后落库）
   └─ HKDF(MK,"wrap") ─▶ KEK ──解开──▶ protected_dek
                                          ▼
DEK  数据密钥 (32B, 注册时随机)
   ├─ HKDF(DEK,"record")            ─▶ 加密每条片语
   ├─ HKDF(DEK,"blob", plain_hash)  ─▶ CEK，加密附件（收敛，见 8.5）
   ├─ HKDF(DEK,"index")             ─▶ DBKey，SQLCipher 本地库密钥
   └─ HKDF(DEK,"inbox")             ─▶ 解开 X25519 私钥（微信入口）
```

**为什么要 DEK 这一层**：直接用 MK 加密数据的话，**改密码就要全量重加密**。有了 DEK，改密码只需重新包一次 32 字节，数据一个字节不动。这是唯一理由，但足够充分。

**AuthKey 与 MK 分离**：HKDF 不可逆，服务端拿到 AuthKey 推不出 MK，解不开 `protected_dek`。服务端再对 AuthKey 做一次 Argon2id 后存储，防止库泄漏后被直接用于登录。

**Argon2id 参数**：`m=64MiB, t=3, p=4`，桌面端约 0.3–0.5 秒。**参数必须随 `protected_dek` 存服务端**，未来调参时老用户仍能用旧参数解开后静默升级。

**内存卫生**：密钥类型用 `Zeroize + ZeroizeOnDrop`，**一律用定长数组不用 `Vec<u8>`**——`Vec` 扩容会留下未清零的旧缓冲区副本。

### 8.3 加密信封

```
偏移   长度   内容
0      1      version = 0x01
1      1      alg     = 0x01  (XChaCha20-Poly1305)
2      24     nonce（随机）
26     N      ciphertext
26+N   16     tag
```

**为什么是 XChaCha20-Poly1305 而非 AES-GCM**：192 位 nonce 允许随机生成而不必担心碰撞。AES-GCM 的 96 位 nonce 在随机生成下有生日界约束，需要计数器管理，跨设备场景下计数器同步很麻烦。纯软件实现性能也好，不依赖 AES-NI。

**AAD 必须绑定记录身份**：

```rust
let aad = [&[version, alg][..], record_id.as_bytes()].concat();
```

不加 AAD 的话，攻击者可把 A 记录的密文整块换成 B 记录的——内容读不懂，但能做定向破坏或让记录错位。加了 AAD 换位后解密直接失败。

### 8.4 恢复码

```
protected_dek_pw       = Enc(HKDF(MK,"wrap"), DEK)
protected_dek_recovery = Enc(HKDF(RK,"wrap"), DEK)
```

`RK` 为 128 位随机数，编成 10 组 5 字符 Base32（Crockford 变体，去掉易混淆的 I/L/O/U）：

```
K7QF2  9XBTM  4RHVD  8NJCW  3PGZY
6SLKA  0EFTN  5MRQB  2HDVX  7WCJP
```

**注册流程必须包含**：① 显示 ② 提供"下载 txt"与"复制" ③ **要求回填其中随机两组以确认已保存**。

第三步不能省。只显示不校验的话，绝大多数用户会直接点下一步，然后在第一次忘记密码时永久丢失全部数据。这会增加注册摩擦，但 E2EE 下没有第二次机会。

**轮换**：用过一次的恢复码应作废并生成新的。设置页提供"重新生成恢复码"，需输入当前密码。

**不做**：安全问题找回、服务端托管密钥备份、"我们帮你保管一份副本"——那样 E2EE 就是假的。

### 8.5 附件加密与去重

**问题**：内容寻址去重依赖"相同明文 → 相同哈希"，但常规随机密钥/nonce 会让相同明文加密两次得到不同密文，去重失效。

**解法：收敛加密**

```
plain_hash = SHA256(明文)                            // 本地标识
CEK        = HKDF(DEK, info="blob" || plain_hash)    // 确定性派生
nonce      = HMAC-SHA256(CEK, "nonce")[0..24]        // 确定性
ciphertext = XChaCha20-Poly1305(CEK, nonce, 明文, aad=plain_hash)
cipher_id  = SHA256(ciphertext)                       // 服务端对象键
```

同一用户同一张图，`cipher_id` 恒定 → 上传前 `HEAD /blobs/{cipher_id}`，存在即跳过。

**为什么 CEK 要掺 DEK**：纯收敛加密下不同用户的相同文件密文相同，服务端可做"确认文件存在"攻击——拿一张已知图片算出密文哈希就能查出谁存了它。掺入 DEK 后跨用户去重失效，但跨用户去重本来也不该做。

**确定性 nonce 安全吗**：CEK 由明文唯一决定，(CEK, nonce, 明文) 三元组恒定，不存在同密钥不同明文复用 nonce，安全。

**缩略图也必须加密**（`info="thumb" || plain_hash`）。容易漏的一项——缩略图能直接看出原图内容，明文缓存等于加密白做。

**显示时不落明文到磁盘**：绝不能解密到临时文件再用 `file://` 加载。用 Tauri 自定义协议流式解密到内存：

```rust
app.register_uri_scheme_protocol("zhiyan", |ctx, req| {
    let hash = parse_hash(req.uri())?;
    let plain = blob::decrypt_to_memory(hash)?;   // 明文只在内存
    ResponseBuilder::new().mimetype(&mime).body(plain)
});
```

前端 `<img src="zhiyan://blob/a3f9…">`，CSP 中 `img-src` 限制到 `zhiyan:` 与 `data:`。

> 这是本方案相对 GPUI 方案额外需要的一层。GPUI 下解密字节可直接交给渲染器。

---

## 9. 本地存储

### 9.1 目录布局

```
%APPDATA%\Zhiyan\                  # ASCII 目录名，避免第三方工具处理中文路径出错
├─ zhiyan.db                       # SQLCipher（含 FTS 索引）
├─ zhiyan.db-wal / -shm
├─ blobs\a3\a3f9c2e1…              # 附件密文，按 plain_hash 前两位分桶
├─ cache\thumbs\a3\a3f9c2e1…       # 缩略图密文
├─ backups\2026-08-05T03-00.zbk    # 加密备份
└─ state.json                      # 非敏感：窗口位置、同步游标、UI 偏好
```

`state.json` 明文，**只能放不敏感内容**。窗口位置、主题、字号可以；最近搜索词、标签列表不行。

### 9.2 本地库必须整体加密

不加密的话，`memo` 表和 `memo_fts` 索引里躺着完整明文。笔记本被偷、磁盘被恢复、或备份软件把 `%APPDATA%` 同步到别处，加密就全白做了。

用 **SQLCipher**（`rusqlite` 的 `bundled-sqlcipher-vendored-openssl` feature）。它做页级透明加密：SQLite 引擎看到的是解密后的页，所以 **FTS5、索引、事务、WAL 全部照常工作，SQL 一行不用改**。这是它相对"应用层字段加密"的决定性优势——后者会让全文搜索和 `ORDER BY created_at` 直接失效。

### 9.3 打开数据库：必须用裸密钥模式

```rust
let conn = Connection::open(&db_path)?;
// 关键：x'...' = 直接提供 32 字节裸密钥，跳过 SQLCipher 自带的 PBKDF2
conn.pragma_update(None, "key", format!("x'{}'", hex::encode(db_key)))?;
conn.pragma_update(None, "cipher_page_size", 4096)?;
conn.pragma_update(None, "journal_mode", "WAL")?;
conn.pragma_update(None, "synchronous", "NORMAL")?;
```

**不要**用 `PRAGMA key = 'passphrase'`。那会触发 SQLCipher 内置的 25.6 万次 PBKDF2，每次打开连接都跑一遍——连接池场景下启动时间会劣化到秒级。`DBKey` 已是 Argon2id 派生的强密钥，无需再拉伸。

性能开销：页级 AES-256-CBC，查询慢 5%–15%，万级数据下感知不到。

### 9.4 免密启动

要求每次开应用输密码，对"按热键即时速记"的工具是灾难。所以：

```
DEK ──DPAPI(CryptProtectData, 用户作用域)──▶ 密文
                                             │
                            写入 Windows Credential Manager
                            （keyring crate，目标名 "Zhiyan/dek"）
```

DPAPI 用户作用域密钥绑定 Windows 登录凭据，离开这台机器的这个账户就解不开。

**安全边界必须写清楚**：同一 Windows 会话内运行的程序可以解出密钥。这是所有免密启动方案的共同代价（1Password、Bitwarden 的"记住我"同理）。

设置项：

- 「启动时要求输入密码」（默认关）
- 「空闲 N 分钟后锁定」（默认关，开启后清空内存密钥）
- 「设备丢失时远程吊销」——服务端标记 token 失效，但**已在本地的数据吊销不了**，UI 必须说明，不能让用户误以为能远程擦除

### 9.5 表结构

SQLCipher 已整库加密，本地表存**明文列**，FTS/索引/聚合全部可用。密文只在推送到服务端的那一刻生成。

```sql
CREATE TABLE memo (
  id          BLOB PRIMARY KEY,        -- UUIDv4，16 字节
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  deleted_at  INTEGER,                 -- 墓碑
  rev         INTEGER NOT NULL DEFAULT 1,
  text        TEXT NOT NULL,
  pinned      INTEGER NOT NULL DEFAULT 0,
  source      TEXT NOT NULL DEFAULT 'app',   -- app | quick | wechat | tg
  dirty       INTEGER NOT NULL DEFAULT 1,    -- outbox 标记
  server_seq  INTEGER
);
CREATE INDEX idx_memo_time  ON memo(created_at DESC) WHERE deleted_at IS NULL;
CREATE INDEX idx_memo_dirty ON memo(dirty) WHERE dirty = 1;

CREATE TABLE memo_tag (memo_id BLOB, tag TEXT, PRIMARY KEY(memo_id, tag));
CREATE INDEX idx_tag ON memo_tag(tag);

CREATE TABLE memo_ref (src BLOB, dst BLOB, kind TEXT, PRIMARY KEY(src, dst, kind));
CREATE INDEX idx_ref_dst ON memo_ref(dst);      -- 反向链接

CREATE TABLE blob (
  plain_hash BLOB PRIMARY KEY,   -- SHA256(明文)
  cipher_id  BLOB NOT NULL,      -- SHA256(密文)，服务端对象键
  mime TEXT, size INTEGER, width INTEGER, height INTEGER,
  refcount INTEGER NOT NULL DEFAULT 0,
  uploaded INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE memo_blob (memo_id BLOB, plain_hash BLOB, ord INTEGER, PRIMARY KEY(memo_id, ord));

CREATE VIRTUAL TABLE memo_fts USING fts5(
  text, content='memo', content_rowid='rowid', tokenize='trigram'
);

CREATE TABLE outbox (
  memo_id BLOB PRIMARY KEY, op TEXT NOT NULL, queued_at INTEGER NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0, last_error TEXT
);

CREATE TABLE kv (k TEXT PRIMARY KEY, v BLOB NOT NULL);
```

**中文分词**：`tokenize='trigram'`（SQLite 3.34+ 内置）。默认的 `unicode61` **不切分中文**，会把整句当一个 token，"认知"搜不到"注意力认知负荷"。trigram 索引膨胀约 3 倍，万级数据下无所谓。数据量或体验成为瓶颈时再考虑编译 `simple` 分词器扩展（支持拼音，但需自行编译 DLL）。

### 9.6 IPC 接口

前端所有数据访问收敛到 `store.js`，对应 Rust 命令：

```rust
#[tauri::command] async fn list_memos(filter: Filter, cursor: Option<Uuid>, limit: u32) -> Vec<Memo>;
#[tauri::command] async fn get_memo(id: Uuid) -> Option<Memo>;
#[tauri::command] async fn upsert_memo(input: MemoInput) -> Memo;  // 内部解析标签/引用，写三张表
#[tauri::command] async fn soft_delete(id: Uuid);
#[tauri::command] async fn restore(id: Uuid);
#[tauri::command] async fn purge(id: Uuid);                        // 写墓碑，非物理删
#[tauri::command] async fn search(q: String, limit: u32) -> Vec<Memo>;
#[tauri::command] async fn stats(range: Range) -> Stats;
#[tauri::command] async fn put_blob(bytes: Vec<u8>, mime: String) -> BlobMeta;
#[tauri::command] async fn export(fmt: ExportFmt) -> PathBuf;
#[tauri::command] async fn get_kv(k: String) -> Option<String>;
#[tauri::command] async fn set_kv(k: String, v: String);
```

**性能约束（本方案特有）**：

- `list_memos` **必须分页**（游标用 UUID + created_at）。IPC 走 JSON 序列化，万级数据整包传输会有明显卡顿。
- 热力图与统计**在 Rust 侧用 SQL 聚合算完再返回**，不要把原始数据搬到前端算。
- `put_blob` 传大图时走 `Vec<u8>`，注意 IPC 有大小上限；超过 10MB 的图片建议先在前端压缩。

> 这一整组约束是 IPC 边界的产物。GPUI 方案下不存在。

### 9.7 备份与导出

| 类型 | 加密 | 说明 |
|---|---|---|
| 本地自动备份 `.zbk` | ✅ DEK | 每日 + 启动检查，保留 30 日 + 12 月 |
| 云端 | ✅ | 同步本身即备份 |
| 导出 JSON | ⚠️ 可选 | "加密导出（需恢复码）" / "明文导出" |
| 导出 Markdown | ❌ 明文 | 本质如此，导出前明确提示 |

`.zbk` 格式：16 字节 magic + 版本，之后是信封格式密文，内容为 zstd 压缩的全量 JSON。

**关键提醒**：本地备份用 DEK 加密，而 DEK 依赖密码/恢复码。用户忘记密码且丢失恢复码时，本地备份同样打不开。因此"注销账号"与"重置密码"流程必须先引导导出**明文 Markdown**。

---

## 10. 云同步

### 10.1 服务端表

```sql
CREATE TABLE account (
  user_id UUID PRIMARY KEY, email TEXT UNIQUE NOT NULL,
  auth_hash TEXT NOT NULL,               -- Argon2id(AuthKey)
  kdf_salt BYTEA NOT NULL, kdf_params JSONB NOT NULL,
  protected_dek BYTEA NOT NULL, protected_dek_recovery BYTEA NOT NULL,
  inbox_pubkey BYTEA,                    -- X25519，见 10.5
  created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE record (
  user_id UUID NOT NULL, id BYTEA NOT NULL,   -- 客户端 UUIDv4
  seq BIGSERIAL,                              -- 单调游标
  ciphertext BYTEA NOT NULL,                  -- 服务端不可读
  device_id UUID NOT NULL,                    -- 回环过滤
  PRIMARY KEY (user_id, id)
);
CREATE INDEX idx_record_seq ON record(user_id, seq);

CREATE TABLE device (
  device_id UUID PRIMARY KEY, user_id UUID NOT NULL,
  name TEXT, platform TEXT, last_seen TIMESTAMPTZ, revoked BOOLEAN DEFAULT false
);
```

**服务端明文字段仅此而已**：`user_id`、随机 `id`、`seq`、密文长度、`device_id`。

注意 `updated_at` **不在服务端**——冲突判定完全在客户端做（解密后比对密文内的 `updatedAt`），服务端无需理解时间。

附件走对象存储（推荐 **Cloudflare R2**，出网流量免费，对图片多的笔记应用差别很大），key 为 `{user_id}/{cipher_id}`，上传用预签名 URL 避免密文过 API 服务器。

### 10.2 协议

```http
GET /v1/changes?since=88213&limit=200
→ { "changes":[{"id":"3f9a…","seq":88214,"device":"…","ciphertext":"base64…"}],
    "cursor":88214, "hasMore":false }

POST /v1/changes
  { "deviceId":"…", "changes":[{"id":"3f9a…","ciphertext":"base64…","baseSeq":88101}] }
→ { "accepted":["3f9a…"], "conflicts":[], "cursor":88220 }
```

`baseSeq` 是客户端上次见到的该记录 seq。服务端比对，不一致返回 conflicts，客户端先拉后合再重推。**服务端不需要看懂内容就能做这个并发控制。**

### 10.3 outbox

所有本地写入先落 SQLCipher 库并入 `outbox`，异步推送，失败指数退避。离线可用是天然的，不需额外处理。

触发时机：编辑后 debounce 2 秒、窗口失焦、应用启动、每 5 分钟兜底。

### 10.4 冲突与异常

**冲突**：按记录 LWW，比对解密后的 `updatedAt`。片语类笔记"写完就不动"，真并发概率极低。败方**保留为一条新的"冲突副本"，不静默丢弃**。

需留神的场景：待办勾选会重写整条 `text`。手机勾框、同时电脑改正文，LWW 会丢一边。v1 接受此损失。

**解密失败必须显式处理**，不能静默跳过。可能是 DEK 轮换后的旧记录，也可能是数据损坏。记入 `sync_error` 表并在 UI 暴露"N 条记录无法解密"。

**不建议上 CRDT**（Yjs / Automerge / Loro）。它解决的是同一段文本的并发字符级合并，代价是文档体积膨胀、库体积、且服务端无法直接查内容（E2EE 下服务端本来也查不了，但客户端解析成本仍在）。知言是短卡片 + 单人多端，收益覆盖不了成本。真要做协同编辑时再上。

### 10.5 微信 / Telegram 入口的折中

E2EE 与"服务端代收消息"天然冲突：微信把明文投递到你的服务器，服务器必须做点什么才能变成用户的记录。

**非对称收件箱**：注册时生成 X25519 密钥对，公钥明文存服务端，私钥用 DEK 加密后存服务端。微信消息到达时服务端用公钥做 sealed box（`crypto_box_seal`，无需私钥即可加密），写入 `inbox` 后**立即丢弃明文**。客户端下次同步时拉取、解密、转成正式记录。

**必须诚实告知**——设置页开启此入口时明示：

> 通过微信记录的内容，在加密前会短暂经过我们的服务器。若你希望所有内容都不经第三方，请关闭此入口。

服务端硬约束：不落盘、不进日志、不进 APM 采样、处理完立即 zeroize。

**或者干脆不做。** 若把"服务端从不接触明文"作为产品承诺，就该放弃这个入口。flomo 正因为有微信输入所以做不了 E2EE——这是真实的差异点。**这是产品定位问题，不是技术问题**，建议明确选一边而非模糊处理。

### 10.6 元数据泄漏与缓解

| 可见 | 可推断 |
|---|---|
| 记录条数 | 使用强度 |
| 每条密文长度 | 每条大致字数 |
| `seq` 递增时刻 | **几点在写 → 等价于热力图和作息** |
| 附件大小与数量 | 是否常存图片、大概尺寸 |
| 设备数与登录时间 | 有几台设备、何时用 |

**长度填充**：密文按桶对齐（256/512/1K/2K/4K/8K，之后按 8K 递增），明文加 `pad` 字段填充到目标长度，解密后丢弃。平均多传约 30%，纯文本笔记完全可接受。

**写入时刻**不做混淆。要遮就得随机延迟上传（数分钟到数小时），会破坏多端准实时体验，得不偿失。文档与隐私政策中如实写明。

---

## 11. 打包、签名与更新

### 11.1 安装器

主渠道用 **NSIS**（Tauri 内置支持）：

```json
"windows": {
  "nsis": {
    "installMode": "currentUser",
    "languages": ["SimpChinese", "English"],
    "displayLanguageSelector": false,
    "installerIcon": "icons/installer.ico"
  },
  "webviewInstallMode": { "type": "downloadBootstrapper" }
}
```

选 `currentUser` 而非 `perMachine`：免 UAC 提权，安装转化率明显更高；代价是多用户机器上每个用户各装一份。

### 11.2 代码签名——有真实成本

**SmartScreen 会拦截未签名安装包**，弹"Windows 已保护你的电脑"，对转化率杀伤极大。

| 方案 | 年成本 | SmartScreen |
|---|---|---|
| 不签名 | 0 | 一直拦截 |
| OV 证书 | ¥2000–4000 | 需累积下载量建立信誉，数周至数月 |
| EV 证书 | ¥4000–6000 | 签完即时通过 |
| Microsoft Store (MSIX) | 0（一次性注册费） | 商店身份自带信任 |

**注意**：2023 年 6 月起 OV/EV 私钥强制硬件令牌或云 HSM，CI 自动签名需配置云签名服务（Azure Trusted Signing、DigiCert KeyLocker），**不能再把 .pfx 放进 CI secrets**。

**推荐组合**：NSIS + OV 为主渠道，同时上架 Store 作为"安全下载"背书。Store 版还能拿到自动更新和一部分自然流量。

**MSIX 的限制**：全局热键与自启均可用（后者需声明 `startupTasks` 能力），但文件系统访问受限，`%APPDATA%` 会重定向到应用容器——数据迁移逻辑要兼容两种路径。Store 版也不能用自建 updater。

### 11.3 自动更新

用 `tauri-plugin-updater`，它有独立的 Ed25519 签名机制，与代码签名证书无关，**不额外花钱**。

- 更新源：静态 JSON 托管于 CDN（如 Cloudflare R2 + Workers）
- 策略：启动时检查 + 每 6 小时后台检查；下载完成后提示"重启以更新"，**不强制立即重启**
- 灰度：JSON 中带 `rollout` 百分比字段，客户端按设备 ID 哈希判定
- **必须做**：更新失败回滚。保留上一版本安装包，检测到新版本连续三次启动崩溃时提示回滚

> 相比 GPUI 方案需自建更新器，这是本方案的一处省力项。

---

## 12. 安全与隐私

### 12.1 Tauri 2 能力清单

Tauri 2 用 capability 文件做细粒度授权，遵循最小权限：

```json
{
  "identifier": "default",
  "windows": ["main", "quick"],
  "permissions": [
    "core:default",
    "core:window:allow-start-dragging",
    "core:window:allow-minimize",
    "core:window:allow-toggle-maximize",
    "core:window:allow-close",
    "global-shortcut:default",
    "tray:default",
    "notification:default",
    "autostart:default",
    "dialog:allow-save",
    "updater:default"
  ]
}
```

**不要**开启 `shell:allow-execute` 或宽泛的 `fs:allow-*`。文件读写全部走自定义命令，路径由 Rust 侧校验，杜绝前端传入任意路径。

### 12.2 CSP 与外链

正文渲染的是用户输入的 Markdown。前端 `inline()` 已对文本做 HTML 转义，但仍需 CSP 兜底：

```json
"security": {
  "csp": "default-src 'self'; img-src 'self' zhiyan: data:; style-src 'self' 'unsafe-inline'; script-src 'self'"
}
```

`[链接](url)` 生成的 `<a target="_blank">` **必须拦截**，改由 Rust 侧 `shell::open` 打开，且**只允许 http/https 协议**——否则 `file://` 或 `ms-msdt:` 之类的 scheme 会成为攻击面。

> 这一整类风险（CSP、XSS、scheme 注入）是 web 内容的产物。GPUI 方案下不存在。这是本方案相对 GPUI 多出来的安全维护成本。

### 12.3 其余

| 项 | 措施 |
|---|---|
| 图片解码 | 限制最大尺寸与解码内存，防解压炸弹 |
| 凭证 | token 存 Windows Credential Manager，不写数据库；MK 仅在内存，退出即 zeroize |
| DevTools | Release 构建禁用 |
| 依赖供应链 | `cargo-deny` + `npm audit` 检查许可证与已知漏洞 |

---

## 13. 日志与可观测性

E2EE 最容易从日志漏。硬性规则：

**① 任何日志不得包含 `memo.text`、标签名、附件字节。** 只允许出现记录 id、长度、错误码。

**② Rust 侧为正文类型手写 `Debug`**，杜绝 `{:?}` 误打全文：

```rust
impl fmt::Debug for Memo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Memo")
            .field("id", &self.id)
            .field("len", &self.text.len())
            .finish_non_exhaustive()
    }
}
```

**③ `tracing` 加脱敏 layer**，对字段名做白名单（只放行 `id`/`len`/`count`/`code`/`ms` 等），非白名单字段替换为 `<redacted>`。这比依赖每个调用点自觉更可靠。

**④ 前端 `console.log` 同样是泄漏面。** Release 构建用 Vite 的 `esbuild.drop` 移除全部 `console`，避免开发期的调试语句带着正文进入生产。

**⑤ 崩溃上报必须关闭 minidump 的堆内存采集**，否则明文正文随 dump 上传。只收堆栈 + 寄存器。

---

## 14. 测试策略

| 层次 | 工具 | 覆盖 |
|---|---|---|
| Rust 单元 | `cargo test` | UUID 迁移、标签树、统计聚合、周报 |
| 加密 | `cargo test` + RFC 向量 | Argon2id/HKDF/XChaCha20；**AAD 换位必须解密失败**；信封跨版本 |
| 数据层 | rusqlite 内存库 | 墓碑、引用级联、blob 引用计数、FTS 中文命中 |
| 前端逻辑 | **Vitest** | `md()` 渲染、待办勾选改写、补全触发正则、时间分组 |
| E2E | **WebDriver（tauri-driver）** | 启动、写入、热键唤出、托盘 |

> 前端有成熟测试生态（Vitest + WebDriver）是本方案相对 GPUI 的一处优势。GPUI 方案下 UI 层只能靠 `TestAppContext` 和自建截图比对。

**必须人工验证的矩阵**：

- Windows 10 22H2 / Windows 11 23H2 各一台
- 100% / 150% / 200% 缩放；**混合 DPI 双屏拖拽**
- 明暗主题切换、系统强调色变化
- 中文输入法（微软拼音 / 搜狗）下的热键与浮窗焦点
- 高对比度模式、屏幕阅读器（本方案有基本可用性）
- 旧版升级安装后的数据完整性
- **断网写入 → 联网后合并**的完整链路
- **WebView2 版本兼容**：在仅有较旧 Edge 的机器上验证降级提示

---

## 15. 里程碑

| 阶段 | 内容 | 工期 | 出口标准 |
|---|---|---|---|
| **M0** 骨架 | Tauri 跑起原型 HTML，验证 WebView2 CSS 兼容 | 2 天 | `color-mix`/`backdrop-filter`/滚动条正常 |
| **E0** 密钥 | Argon2id/HKDF/信封 + RFC 向量 + 跨版本解密测试 | 3 天 | 与 RFC 向量一致；AAD 换位解密失败 |
| **E1** 本地库 | SQLCipher + DPAPI/Credential Manager + 免密启动与锁定 | 4 天 | 冷启动 < 800ms；库文件外部工具打不开 |
| **M1** 外观 | 打包字体 + 自绘标题栏 + Snap Layouts + Win11 规范 | 5 天 | Windows 上视觉质量不低于原型 |
| **M2** 系统集成 | 全局热键（含冲突与改键 UI）+ 浮窗置前台 + 托盘 + Jump List + 自启 + Mica | 5 天 | 混合 DPI 双屏下浮窗定位与焦点 100% 可靠 |
| **M3** 数据层 | UUID 迁移 + 附件外置 + FTS + 虚拟滚动 + 本地备份 | 8 天 | 万级数据列表无掉帧，搜索 < 100ms |
| **E2** 账号 | 注册/登录/恢复码全流程（含回填校验） | 5 天 | 恢复码可完整找回数据 |
| **E3** 附件 | 收敛加密 + `zhiyan://` 流式解密 + 缩略图 | 4 天 | 磁盘上无任何明文图片 |
| **M4** 打包 | NSIS + 更新器 + 签名 + 崩溃上报 | 4 天 | 全新机器安装升级链路通过 |
| **M5/E4** 同步 | outbox + 变更游标 + 冲突提示 + 长度填充 | 15 天 | 双端离线编辑合并无丢失/无重复/无复活 |
| **E5** 密钥运维 | 改密码 / 恢复码轮换 / DEK 轮换 | 4 天 | 改密码不触发数据重传 |
| **E6** 审计 | 日志脱敏 + 崩溃配置 + 渗透自测 | 3 天 | 全量日志中检索不到任何正文片段 |

**M0 + E0 + E1 + M1–M4 约 8 周**可交付带本地加密的单机版。同步（M5/E4）作为独立阶段，建议单机版小范围验证后再启动。

**E0/E1 必须最先做**（M0 之后立刻），它们决定本地数据格式，越晚改迁移成本越高。

---

## 16. 待决策项

**① 路线选择：本方案 vs v2.0（GPUI）。** 见 §0。若不做 P0 验证则直接执行本方案。

**② 微信入口保不保。** 影响产品定位与隐私承诺措辞。见 §10.5。越晚决定返工越大。

**③ OPAQUE 还是当前 AuthKey 方案。** 当前方案下服务端在登录瞬间接触 AuthKey（虽推不出 MK）。OPAQUE 可做到零知识，但 Rust 生态成熟实现有限。建议 v1 用当前方案，协议版本号留升级空间。

**④ DEK 轮换的触发条件。** 轮换需全量重加密并上传，万级数据约数分钟。什么情况强制轮换（设备丢失？恢复码泄漏？）需产品侧定义。

**⑤ 附件是否计入免费额度。** 决定 blob 配额逻辑与账号面板"附件占用"那一栏的含义。

**⑥ Store 版是否作为主渠道。** 影响数据目录路径、更新机制、是否需为 MSIX 单独维护构建流水线。

**⑦ 公开分享单条。** E2EE 下需为每次分享生成独立密钥并把密钥放在 URL fragment（`#key=…`，不发给服务器）。可行但引入新的密钥管理，建议 v1 不做，只保留"分享成图片"这条本地渲染路径。

---

## 17. 附录

### 附录 A · 从原型到工程的改动清单

| 原型现状 | 目标 | 优先级 |
|---|---|---|
| `window.storage` 单键存整包 JSON | `invoke()` → SQLCipher 分表 | P0 |
| 自增整数 `id` | UUIDv4 | P0 |
| 图片 base64 存于 `imgs[]` | 收敛加密的内容寻址 blob | P0 |
| `purge()` 硬删除 | 墓碑 | P0 |
| 本地明文 | SQLCipher 整库加密 | P0 |
| 字体栈依赖系统 | 打包子集化 woff2 | P0 |
| `Ctrl+Shift+N` | `Ctrl+Alt+Space` | P0 |
| 无密钥体系 | 三层密钥 + 恢复码 | P0 |
| 单文件 1700 行 | ESM 模块拆分 | P1 |
| 全量渲染时间线 | 虚拟滚动 | P1 |
| 前端 JS 全量过滤搜索 | FTS5 trigram | P1 |
| 账号面板为静态 mock | 接真实鉴权 | P2 |
| 同步开关为装饰 | 五态显示 | P2 |

**同步状态的可见性值得单独强调**：同步类应用最伤信任的时刻，是用户不知道自己刚写的东西到底存没存上。侧栏底部应常驻"未上传 N 条"的入口，点击可查看具体条目。五态为：`已同步` / `同步中` / `N 条待上传` / `离线` / `冲突需处理`。

### 附录 B · 快捷键总表

| 快捷键 | 功能 | 作用域 |
|---|---|---|
| `Ctrl+Alt+Space` | 速记浮窗 | 全局 |
| `Ctrl+Alt+Z` | 显示/隐藏主窗口 | 全局（默认关） |
| `Ctrl+K` | 命令面板 | 应用内 |
| `Ctrl+N` | 聚焦主编辑器 | 应用内 |
| `Ctrl+Enter` | 保存 | 编辑器 |
| `Ctrl+B` / `Ctrl+I` | 粗体 / 斜体 | 编辑器 |
| `Ctrl+R` | 换一条拾遗 | 应用内 |
| `Ctrl+Alt+R` | 本周回顾 | 应用内 |
| `Ctrl+D` | 切换深浅外观 | 应用内 |
| `/` | 聚焦搜索 | 应用内 |
| `Esc` | 关闭弹层 / 收起浮窗 | — |
| `#` | 标签补全 | 编辑器 |
| `[[` | 片语引用补全 | 编辑器 |

**注意**：输入法组合串未上屏时（`compositionstart` 到 `compositionend` 之间）必须屏蔽全部应用快捷键，否则输入拼音过程中的按键会误触发命令。

### 附录 C · 术语表

| 术语 | 含义 |
|---|---|
| 片语 | 一条笔记的单位称呼（应用名"知言"，单位保留"片语"） |
| 时间脊 | 贯穿时间线左侧的竖线与日期节点，节点大小编码当天字数 |
| 拾遗 | 随机漫步式回顾 |
| 避头尾 | 中文排版规则：标点不出现在行首、特定标点不出现在行尾（本方案由浏览器原生处理） |
| MK / DEK / CEK | 主密钥 / 数据密钥 / 单附件密钥 |
| 收敛加密 | 密钥由内容哈希确定性派生，使相同明文产生相同密文以支持去重 |
| 墓碑 | 删除标记记录，代替物理删除以保证多端删除能收敛 |
| outbox | 本地待推送队列，保证离线写入不丢 |
| AAD | 附加认证数据，绑定密文与记录身份，防止密文换位 |
