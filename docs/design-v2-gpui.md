# 知言 · 桌面端设计文档

**版本** v2.0 · 2026-08-05
**技术栈** Rust + GPUI（原生 GPU 渲染），Windows 10 1809+ / Windows 11
**状态** 设计定稿，待 P0 技术验证

> 本文取代 v1.0（Tauri + WebView2 方案）。UI 技术栈整体更换，**§3–§5、§10–§14 为重写**；数据模型、加密、同步（§6–§9）内容基本延续，但因去掉 IPC 边界而有简化。
>
> **原型 HTML（`zhiyan-windows.html`）的角色随之改变**：它不再是待移植的代码，而是**视觉与交互的规格说明**。实现时对照它取尺寸、配色、层级、动效时长，但代码零复用。

---

## 目录

1. [产品定位与设计原则](#1-产品定位与设计原则)
2. [功能规格](#2-功能规格)
3. [技术选型：为什么是 GPUI，以及它的代价](#3-技术选型为什么是-gpui以及它的代价)
4. [架构与代码组织](#4-架构与代码组织)
5. [UI 实现方案](#5-ui-实现方案)
6. [文本输入与输入法（最高风险项）](#6-文本输入与输入法最高风险项)
7. [窗口与 Windows 集成](#7-窗口与-windows-集成)
8. [数据模型](#8-数据模型)
9. [端到端加密](#9-端到端加密)
10. [本地存储](#10-本地存储)
11. [云同步](#11-云同步)
12. [打包、签名与更新](#12-打包签名与更新)
13. [安全与隐私](#13-安全与隐私)
14. [日志与可观测性](#14-日志与可观测性)
15. [测试策略](#15-测试策略)
16. [里程碑](#16-里程碑)
17. [待决策项](#17-待决策项)
18. [附录](#18-附录)

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
拾遗、每周回顾、时间脊、热力图不是统计功能，是产品的另一半。

### 1.3 一处刻意的克制

**待办不做成独立 Todo 页面**，只在每周回顾汇总未勾选项。片语里的待办是写作的副产品，不是任务系统。

---

## 2. 功能规格

（与 v1.0 一致，此处保留完整清单以便对照工作量）

### 2.1 记录

| 功能 | 说明 | GPUI 实现难度 |
|---|---|---|
| 主编辑器 | 常驻主窗口顶部，接在时间脊起点 | **高**（见 §6） |
| 速记浮窗 | 全局热键唤起的无边框小窗，可拖动记忆位置 | 中 |
| Markdown | 粗体/斜体/删除线/高亮/行内码/代码块/二~四级标题/引用/列表/待办/链接/分割线 | **高**（自绘） |
| 待办 | `- [ ]` 渲染为可点击复选框，勾选改写源文本 | 中 |
| 标签 | `#主题[/子题]`，输入时补全 | 中 |
| 行内引用 | `[[` 触发选择器，存 `[[片段^uuid]]`，渲染为可点击小卡片 | 中 |
| 图片 | 插入或粘贴，点击放大 | 低 |
| 编辑器辅助 | 工具条、实时预览、回车续列表、Ctrl+B/I | 中 |

### 2.2 组织

标签树（两级、可折叠/置顶/重命名/移除）、置顶片语、双向关联与反向链接、回收站、排序、本地全文搜索与高亮。

### 2.3 回看

时间脊（节点大小随当天字数变化）、拾遗（随机漫步）、每周回顾（翻周/统计/柱状图/待办汇总/导出）、统计页（半年热力图、连续天数、标签分布）、每日目标。

### 2.4 系统与账号

命令面板、深浅外观、分享成图、导入导出、账号面板（资料/会员/同步/恢复码/设备/注销）。

---

## 3. 技术选型：为什么是 GPUI，以及它的代价

### 3.1 对比

| 指标 | **Rust + GPUI** | Tauri 2 + WebView2 | Electron |
|---|---|---|---|
| 安装包（含字体） | 45–70 MB | 22–26 MB | 90–150 MB |
| 常驻内存（主窗+浮窗） | 60–90 MB | 80–120 MB | 250–350 MB |
| 冷启动 | 0.1–0.3 s | 0.4–0.8 s | 1.2–2.5 s |
| 热键唤出（已驻留） | < 30 ms | < 80 ms | 120–250 ms |
| 时间线滚动 | GPU 合成，稳定 120fps | 依赖 WebView 合成 | 同左 |
| 运行时依赖 | **无** | WebView2 运行时 | 无（自带） |
| 渲染一致性 | 完全自控 | 随系统 Edge 漂移 | 自控 |
| 语言 | 单一 Rust | Rust + JS | JS |
| IPC / 序列化 | **无** | 有 | 有 |
| UI 开发效率 | 低（编译周期 + 无热重载） | 高 | 高 |
| 富文本编辑 | **需自建** | contenteditable 白送 | 同左 |
| 中文输入法 | **需自实现** | 白送 | 白送 |
| 无障碍（屏幕阅读器） | 弱 | 白送 | 白送 |

### 3.2 选 GPUI 的真实收益

**① 去掉 IPC 边界是结构性简化。** v1.0 里 `list_memos` 必须分页、统计必须在 Rust 侧聚合，都是为了绕开 JSON 序列化开销。GPUI 下数据结构直接传引用，这一整类约束消失。§10.6 的"IPC 接口"退化成普通模块函数。

**② 安全面显著缩小。** 没有 web 内容就没有 CSP、没有 XSS、没有 `target="_blank"` 的 scheme 注入、不需要自定义协议来避免图片明文落盘——解密后的字节直接交给渲染器。§13 因此大幅简化。

**③ 时间脊这类自绘元素反而更容易。** 在 CSS 里要用 `::before` + 绝对定位拼出来的竖线与节点，在 GPUI 里就是一个自定义 `Element` 里几个 `paint_quad`，且能做到像素级精确、随 DPI 无损缩放。

**④ 无运行时依赖。** 不用再处理 WebView2 Bootstrapper、不用检测运行时缺失、不用担心 Edge 版本低于 Chromium 111 导致 `color-mix()` 失效。

### 3.3 必须接受的代价

**① 整个 UI 层从零重写。** 原型 1700 行 JS + 900 行 CSS 全部作废，改为约 6000–9000 行 Rust。这是最大的一笔成本。

**② 中文输入法要自己接。** 见 §6。这是本方案的头号风险。

**③ 富文本编辑器要自己写。** 带 Markdown 实时渲染、`#` 与 `[[` 补全弹层、可点击待办、行内标签着色的编辑器，本质是一个小型编辑器。Zed 用了数万行实现它的 editor。我们的需求简化很多（单卡片、无多光标、无 LSP），但仍是 3–4 周的量。

**④ 无障碍基本没有。** GPUI 目前对屏幕阅读器的支持有限。若产品需要满足无障碍要求，这条会是硬阻塞。

**⑤ 开发迭代变慢。** 没有热重载，改一个间距要等一次增量编译（GPUI 项目通常 3–15 秒）。UI 微调的成本比 CSS 高一个量级。

**⑥ GPUI 本身的成熟度。** 它主要为 Zed 自身需求演进，API 有 churn，独立应用的文档与示例稀少，Windows 后端相对 macOS 后端验证得少。

### 3.4 依赖获取方式需要先确认

GPUI 长期以来是通过 git 依赖从 Zed 仓库引入的：

```toml
gpui = { git = "https://github.com/zed-industries/zed", rev = "<pin 到具体 commit>" }
```

**P0 阶段必须先确认三件事**（这些在近期变化较快，以实际验证为准）：

1. 当前是否已有独立发布的 `gpui` crate 版本；若仍是 git 依赖，**必须 pin 到具体 commit**，不能用分支——否则上游一次重构就会让构建断掉。
2. Windows 后端的当前状态与已知缺陷清单（尤其 IME、多显示器 DPI、透明窗口）。
3. 许可证条款（GPUI 随 Zed 仓库为 Apache-2.0，但需确认所引入的具体 crate 与其传递依赖）。

**如果第 2 条的验证结果不理想，应回退到 Tauri 方案（v1.0 仍有效）。** 建议把 P0 做成一个可以否决整个技术选型的关卡，而不是既定事实。

---

## 4. 架构与代码组织

### 4.1 分层

```
┌──────────────── 知言（单进程，纯 Rust）────────────────┐
│                                                        │
│  ui/            GPUI 视图层                             │
│  ├─ shell       窗口骨架 / 自绘标题栏 / 三栏布局          │
│  ├─ timeline    时间脊（自定义 Element）/ 卡片列表        │
│  ├─ editor      文本输入 / IME / Markdown 实时渲染       │
│  ├─ sidebar     标签树 / 导航 / 目标进度                 │
│  ├─ rail        拾遗 / 统计 / 热力图                     │
│  ├─ overlays    命令面板 / 弹层 / 菜单 / Toast           │
│  └─ quick       速记浮窗（独立 Window，同进程）           │
│                        ▲                               │
│                        │ 直接调用 + gpui 事件            │
│                        ▼                               │
│  core/          领域逻辑（不依赖 gpui，可独立测试）        │
│  ├─ model       Memo / Tag / Ref / Blob                │
│  ├─ markdown    pulldown-cmark → 富文本片段             │
│  ├─ stats       热力图 / 连续天数 / 周报聚合              │
│  └─ search      查询构造                                │
│                        ▲                               │
│                        ▼                               │
│  store/         SQLCipher + blob 存储                   │
│  crypto/        Argon2id / HKDF / XChaCha20 / 信封      │
│  sync/          outbox / 拉取 / 推送 / 冲突             │
│  platform/      windows-rs：热键 / 托盘 / DPAPI / Mica  │
└────────────────────────────────────────────────────────┘
```

**关键约束：`core/` 不得依赖 `gpui`。** 领域逻辑（Markdown 解析、统计聚合、标签树构建）保持纯函数，可以在 `cargo test` 里脱离 GUI 直接测。这是把"UI 难测"的问题隔离开的唯一办法。

### 4.2 Cargo 工作区

```
zhiyan/
├─ Cargo.toml                  # workspace
├─ crates/
│  ├─ zhiyan/                  # bin，组装 + main
│  ├─ zhiyan-ui/               # 依赖 gpui
│  ├─ zhiyan-core/             # 纯逻辑，无 gpui
│  ├─ zhiyan-store/            # rusqlite/sqlcipher
│  ├─ zhiyan-crypto/           # 加密原语
│  ├─ zhiyan-sync/             # 同步
│  └─ zhiyan-platform/         # windows-rs 封装
├─ assets/
│  ├─ fonts/                   # 子集化 OTF
│  └─ icons/
└─ xtask/                      # 打包、字体子集化、签名
```

用 `xtask` 模式而不是脚本语言：打包流程也是 Rust，跨环境一致。

### 4.3 主要依赖

| 用途 | crate | 备注 |
|---|---|---|
| UI | `gpui` | git pin，见 §3.4 |
| Markdown 解析 | `pulldown-cmark` | 只用解析，渲染自己做 |
| 数据库 | `rusqlite` + `bundled-sqlcipher-vendored-openssl` | |
| 加密 | `argon2` `hkdf` `chacha20poly1305` `sha2` `hmac` `zeroize` | |
| 全局热键 | `global-hotkey` 或直接 `RegisterHotKey` | 见 §7.3 |
| 托盘 | `tray-icon` | |
| 凭据 | `keyring` | Windows Credential Manager |
| 平台 API | `windows` (windows-rs) | Mica / DPAPI / AUMID / Jump List |
| 异步 | `tokio`（仅 sync 与 IO） | UI 线程不阻塞 |
| 序列化 | `serde` + `serde_json` | 仅用于加密载荷与导出 |
| 压缩 | `zstd` | 备份 |
| 日志 | `tracing` + 自定义脱敏 layer | 见 §14 |

**注意**：GPUI 有自己的执行器（`BackgroundExecutor` / `ForegroundExecutor`）。数据库与网络 IO 走 `cx.background_spawn()`，结果通过 `cx.update()` 回到 UI 线程。不要在 GPUI 应用里另起一套 tokio runtime 管理 UI 相关任务，两套调度器混用是常见的死锁来源。

---

## 5. UI 实现方案

### 5.1 设计令牌

CSS 变量改为 Rust 常量结构体，明暗两套：

```rust
pub struct Theme {
    pub paper: Hsla, pub chrome: Hsla, pub card: Hsla,
    pub card_2: Hsla, pub card_3: Hsla,
    pub ink: Hsla, pub ink_2: Hsla, pub ink_3: Hsla,
    pub line: Hsla, pub line_soft: Hsla,
    pub accent: Hsla, pub accent_soft: Hsla, pub accent_dim: Hsla,
    pub mark: Hsla, pub danger: Hsla,
    pub heat: [Hsla; 5],
}

impl Theme {
    pub fn light() -> Self {
        Self {
            paper: rgb(0xFFFFFF).into(),   chrome: rgb(0xFAFAF9).into(),
            card:  rgb(0xFFFFFF).into(),   card_2: rgb(0xF5F6F3).into(),
            card_3: rgb(0xEDEFEA).into(),
            ink:   rgb(0x1B1E19).into(),   ink_2: rgb(0x565B52).into(),
            ink_3: rgb(0x9AA096).into(),
            line:  rgb(0xE2E5DF).into(),   line_soft: rgb(0xEDEFEA).into(),
            accent: rgb(0x2E5A49).into(),  accent_soft: rgb(0xCFDFD3).into(),
            accent_dim: rgb(0xEAF1EB).into(),
            mark:  rgb(0x8A6A1F).into(),   danger: rgb(0x9C4332).into(),
            heat: [rgb(0xF1F2EE).into(), rgb(0xD6E4D8).into(), rgb(0xA5C2AD).into(),
                   rgb(0x5C8B70).into(), rgb(0x2E5A49).into()],
        }
    }
}
```

主题存为 GPUI 的 `Global`，通过 `cx.global::<Theme>()` 取用。切换主题后 `cx.refresh()` 全窗重绘。

**赭金（`mark`）的使用纪律不变**：只出现在时间脊的"今天"节点与拾遗模块的时间标签。扩散到第三处就失去标记意义。

### 5.2 元素写法

GPUI 提供类 Tailwind 的链式样式 API：

```rust
impl Render for MemoCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.global::<Theme>();
        div()
            .flex().flex_col()
            .px(px(17.)).pt(px(14.)).pb(px(10.))
            .mb(px(10.))
            .bg(t.card)
            .border_1().border_color(t.line)
            .rounded(px(8.))
            .hover(|s| s.shadow_sm().border_color(t.line))
            .child(self.render_body(cx))
            .children(self.render_images(cx))
            .children(self.render_relations(cx))
            .child(self.render_footer(cx))
    }
}
```

> **API 稳定性提示**：GPUI 的 `Render` 签名在不同版本间变动过（`ViewContext<Self>` → `Window` + `Context<Self>`）。上面按较新形态书写，实现前以 pin 的 commit 为准，并在 `zhiyan-ui` 内做一层薄封装隔离，避免上游改签名时全项目改动。

**尺寸从原型 HTML 取值**，不要重新设计：卡片圆角 8px、内边距 17/14/10、卡片间距 10px、正文 15.5px/行高 1.85、脊线 x=63px、卡片左边缘 x=78px。

### 5.3 时间脊：自定义 Element

这是 GPUI 相对 CSS 的优势项。实现一个 `TimeSpine` 元素，接收当天分组数据，自行绘制竖线与节点：

```rust
pub struct SpineNode { pub y: Pixels, pub diameter: Pixels, pub kind: NodeKind }
pub enum NodeKind { Now, Today, Filled, Hollow, Pinned }

impl Element for TimeSpine {
    type RequestLayoutState = ();
    type PrepaintState = Vec<SpineNode>;

    fn paint(&mut self, _id: Option<&GlobalElementId>, bounds: Bounds<Pixels>,
             _: &mut Self::RequestLayoutState, nodes: &mut Self::PrepaintState,
             window: &mut Window, cx: &mut App) {
        let t = cx.global::<Theme>();
        let x = bounds.origin.x + px(63.);

        // 竖线；置顶分组画虚线（按 3px 实 / 3px 空 分段绘制）
        window.paint_quad(fill(
            Bounds::new(point(x, bounds.origin.y), size(px(1.), bounds.size.height)),
            t.line,
        ));

        for n in nodes {
            let r = n.diameter / 2.;
            let b = Bounds::new(point(x - r, n.y - r), size(n.diameter, n.diameter));
            match n.kind {
                NodeKind::Now | NodeKind::Today =>
                    window.paint_quad(fill(b, t.mark).corner_radii(r)),
                NodeKind::Filled =>
                    window.paint_quad(fill(b, t.accent).corner_radii(r)),
                _ => window.paint_quad(
                    quad(b, r, t.card, px(1.5), t.accent, BorderStyle::Solid)),
            }
        }
    }
}
```

节点直径公式沿用原型：`clamp(7, 6 + 当天字数/45, 15)` px。

**编辑器必须接在脊上**——这是原型里修掉的一个真实问题：早期编辑器左边缘在 0、卡片在 78px，两条边缘错开近 80px，视觉上不对劲。正确解法不是把卡片拉回去，而是让编辑器成为脊的起点（"此刻 / NOW"节点），整条时间线从"你正在写的这一刻"往回走。

### 5.4 虚拟化列表

时间线可能有上万条。**不能用 `uniform_list`**——卡片高度随内容变化。用 GPUI 的变高列表（`list` / `ListState`，Zed 的对话面板即用此实现）：

```rust
let state = ListState::new(item_count, ListAlignment::Top, px(600.), move |ix, win, cx| {
    this.update(cx, |this, cx| this.render_row(ix, win, cx)).into_any_element()
});
```

**行高缓存**：Markdown 渲染成本不低，滚动时反复排版会掉帧。为每条片语缓存"渲染后的富文本片段 + 测得高度"，仅在文本、字号、窗口宽度变化时失效。

### 5.5 Markdown 渲染

流程：`pulldown-cmark` 解析 → 转成自有的 `Block`/`Inline` 中间表示 → 映射为 GPUI 元素。

**行内样式**用 `StyledText` + `TextRun` 分段（粗体、斜体、删除线、高亮背景、行内码、标签着色、引用小卡片、搜索命中高亮）。

**块级**各自成元素：标题、段落、引用块（左侧 2px 强调线）、有序/无序列表、待办列表（可点击方块）、代码块（等宽 + 背景）、分割线。

**必须自己处理的三件事**（CSS 白送、GPUI 不送）：

| 事项 | 做法 |
|---|---|
| 中英文混排的行内间距 | 中西文交界处插入约 0.15em 视觉间隙（"盘古之白"），在构造 `TextRun` 时按字符类别切分 |
| 中文断行规则 | 标点不能出现在行首（`，。、；：！？》」`），不能在行尾的（`《「（`）。GPUI 的换行按字符宽度断，需在排版前做一遍标点挤压/避头尾处理 |
| 可点击的行内元素 | 标签、行内引用、链接需要命中测试。用 `InteractiveText` 绑定 range→回调，或把它们拆成独立的行内元素参与 flex 布局 |

避头尾这一条容易被忽略，但中文正文里逗号跑到行首非常刺眼，直接破坏"精致"的观感。

### 5.6 动效

GPUI 的 `Animation` 表达力弱于 CSS transition。按重要性取舍：

| 原型效果 | GPUI 处理 |
|---|---|
| 新卡片入场（8px 上浮 + 淡入，380ms） | **保留**，用 `.with_animation()` |
| 弹层 pop（200ms 缩放 + 位移） | **保留** |
| Toast 滑入 | **保留** |
| hover 背景/边框过渡（120–200ms） | **降级为即时切换**。GPUI 无 hover 过渡原语，逐帧插值不值当 |
| 节点大小过渡 | 去掉。数据变化时直接重绘 |

`prefers-reduced-motion` 对应读取系统"显示动画效果"设置（`SPI_GETCLIENTAREAANIMATION`），关闭时跳过全部动画。

### 5.7 字体

| 用途 | 字体 | 来源 |
|---|---|---|
| 正文 | 思源宋体 SC Regular / SemiBold | **内嵌**（SIL OFL 1.1） |
| 界面 | MiSans Regular / Medium | **内嵌**（免费商用） |
| 数据 | Cascadia Mono | 系统（Win11 内置），回退 Consolas |

**为什么必须内嵌**：不内嵌则中文回退到 **SimSun**——为 96dpi 点阵设计的老宋体，放进 15.5px/行高 1.85 的正文会糊成一片，整个设计的立足点消失。这不是可选项。

**与 v1.0 的差异：格式从 woff2 改为 OTF/TTF。** GPUI 的字体系统走 DirectWrite 自定义字体集合，不支持 woff2。子集化输出 `.otf`，体积比 woff2 大（无 Brotli 压缩），四字重合计约 22–28 MB（v1.0 的 woff2 方案约 14 MB）。这是安装包从 26MB 涨到 45–70MB 的主要原因之一。

```rust
// 启动时注册内嵌字体
cx.text_system().add_fonts(vec![
    Cow::Borrowed(include_bytes!("../assets/fonts/SourceHanSerifSC-Regular.subset.otf")),
    Cow::Borrowed(include_bytes!("../assets/fonts/SourceHanSerifSC-SemiBold.subset.otf")),
    Cow::Borrowed(include_bytes!("../assets/fonts/MiSans-Regular.subset.otf")),
    Cow::Borrowed(include_bytes!("../assets/fonts/MiSans-Medium.subset.otf")),
])?;
```

**字符集**：GB2312 常用字 + 常用标点 + ASCII + 应用内固定文案用字。生僻字回退系统字体，可接受。

**回退链必须显式配置**：`TextStyle.font_family` 指定主字体，`font_fallbacks` 依次为界面黑体、系统中文字体。GPUI 的字体回退不像浏览器那样自动，缺字时可能直接渲染成豆腐块。

**ClearType 适配**：行高从原型的 1.82 调到 **1.85**；加粗用 SemiBold(600) 而非 Bold(700)，避免中文粗体过重。

---

## 6. 文本输入与输入法（最高风险项）

### 6.1 问题

在 WebView 里，`<textarea>` 的中文输入是操作系统与浏览器协作完成的，开发者不用碰。在 GPUI 里，**输入法交互必须由应用自己实现**：拼音输入过程中的未上屏文本（composition / marked text）、候选窗的定位、上屏时的替换、退格与光标在组合串中的行为，全部要处理。

对一个中文优先的笔记应用，输入法出问题等于产品不可用。**这是本方案的头号风险，必须在 P0 阶段验证，验证不通过就回退 Tauri。**

### 6.2 需要实现的接口

GPUI 通过 `PlatformInputHandler`（在 Windows 后端对应 `WM_IME_*` 系列消息）把输入法事件交给应用。需要实现的方法大致为：

| 方法 | 职责 |
|---|---|
| `selected_text_range` | 当前选区，用于输入法定位 |
| `marked_text_range` | 当前未上屏的组合串范围 |
| `text_for_range` | 供输入法读取上下文（部分输入法用于联想） |
| `replace_text_in_range` | 上屏：用最终文本替换范围 |
| `replace_and_mark_text_in_range` | 组合中：替换并标记为未上屏 |
| `unmark_text` | 取消组合 |
| `bounds_for_range` | **候选窗定位**——返回组合串的屏幕矩形 |

`bounds_for_range` 最容易做错。返回错误的坐标会让候选窗飘到屏幕角落或压住正在输入的文字。在多显示器 + 混合 DPI 下尤其要留意坐标系换算。

### 6.3 必须验证的输入法矩阵

**P0 验收清单**（任一项不通过即视为选型风险暴露）：

- 微软拼音：连续输入、翻页选词、`Shift` 中英切换、组合中按退格
- 搜狗拼音：同上，另测其悬浮候选窗
- 微软五笔 / 双拼
- 组合串未上屏时切换窗口（浮窗 ↔ 主窗），组合应正确取消而非丢字
- 组合串未上屏时按 `Ctrl+Enter`，不应触发保存
- 候选窗在 150% 缩放的副屏上的定位精度
- 在编辑器滚动过程中输入，候选窗需跟随

### 6.4 编辑器本身

除了 IME，还要自己实现的编辑能力：

| 能力 | 说明 |
|---|---|
| 光标与选区 | 字素簇（grapheme cluster）边界移动，不能按字节。中文、emoji、组合字符 |
| 鼠标交互 | 点击定位、拖拽选择、双击选词（中文分词边界）、三击选行 |
| 键盘 | 方向键/Home/End/Ctrl+方向键（按词）/Shift+组合选择 |
| 撤销重做 | 按输入间隔合并操作，`Ctrl+Z` / `Ctrl+Shift+Z` |
| 剪贴板 | 文本 + 图片粘贴（图片走 `put_blob`） |
| 自动续行 | 回车续列表/待办/有序号，空行跳出 |
| 补全弹层 | `#` 与 `[[` 触发，方向键选择，回车/Tab 插入，Esc 关闭 |
| 实时渲染 | 编辑区显示带样式的文本（加粗即显示为粗体），而非纯源码 |

**关于"实时渲染"的取舍建议**：让编辑中的文本直接显示样式（所见即所得）会极大增加光标定位与选区计算的复杂度——样式化文本的视觉位置与源码字符索引不再一一对应。

**建议 v1 采用较简方案**：编辑态显示**纯源码 + 轻量着色**（标签、引用、标记符号着色，但不改变字号/字重/隐藏标记），保存后在卡片中完整渲染。原型里的"预览"按钮保留，作为编辑时查看渲染效果的手段。这个取舍能省下大量复杂度，且符合许多 Markdown 工具的做法。

**若坚持所见即所得**，工期需在 §16 的基础上再加 3–4 周。

---

## 7. 窗口与 Windows 集成

### 7.1 主窗口

```rust
cx.open_window(WindowOptions {
    titlebar: None,                                   // 自绘
    window_bounds: Some(WindowBounds::Windowed(restored_bounds)),
    window_min_size: Some(size(px(900.), px(560.))),
    window_background: WindowBackgroundAppearance::Opaque,
    ..Default::default()
}, |window, cx| cx.new(|cx| Shell::new(window, cx)))?;
```

- 尺寸 1180×760，最小 900×560（低于 900 隐藏右栏，低于 780 隐藏侧栏）
- 位置记忆需**持久化所在显示器**，还原前校验该显示器是否仍存在
- 关闭行为：隐藏而非退出（设置项可改为真退出）

### 7.2 自绘标题栏与 Snap Layouts

标题栏 32px，品牌居左，系统按钮居右（46×32，图标 10×10，关闭键 hover `#C42B1C`、按下 `#D14B3D`、图标转白）。

**拖拽区域**：GPUI 需在标题栏空白区域处理拖动（发送 `WM_NCLBUTTONDOWN` with `HTCAPTION`，或使用 GPUI 提供的窗口拖动 API）。

**Snap Layouts —— 最容易漏的一项。** Win11 用户悬停最大化按钮 1 秒会期待弹出贴靠布局面板。自绘标题栏默认没有，用户会立刻察觉"这软件不对劲"。

```
1. SetWindowSubclass 挂子类化过程
2. WM_NCHITTEST：命中最大化按钮矩形 → 返回 HTMAXBUTTON
3. 返回 HTMAXBUTTON 后系统接管该区域鼠标消息，
   应用层的 click 事件不再触发 → 必须自行处理
   WM_NCLBUTTONDOWN / WM_NCLBUTTONUP 来触发实际最大化
4. WM_NCMOUSELEAVE：清除 hover 态
```

按钮矩形由 UI 层在布局与 DPI 变化时上报给 `platform` 模块。

> 与 v1.0 的差异：Tauri 下需要跨 IPC 上报矩形，GPUI 下是同进程直接写共享状态，简单一些；但子类化 hook 的本体逻辑完全一样。

### 7.3 全局热键

**原型中的 `Ctrl+Shift+N` 必须换掉**：它在 Chrome/Edge 是"新建无痕窗口"，在资源管理器是"新建文件夹"。`RegisterHotKey` 全局抢占，注册即砸掉用户这两个习惯。

| 功能 | 键位 | 作用域 |
|---|---|---|
| 唤起/收起速记浮窗 | `Ctrl+Alt+Space` | 全局 |
| 显示/隐藏主窗口 | `Ctrl+Alt+Z` | 全局（默认关） |
| 命令面板 | `Ctrl+K` | 应用内 |
| 保存 | `Ctrl+Enter` | 应用内 |
| 本周回顾 | `Ctrl+Alt+R` | 应用内 |

**应用内快捷键**用 GPUI 的 Action + KeyBinding 体系（`cx.bind_keys`），**全局热键**用 `RegisterHotKey`，两者是不同机制，不要混。

**注册失败必须可见。** `ERROR_HOTKEY_ALREADY_REGISTERED` 说明被占用（常见占用方：搜狗输入法、QQ、PowerToys、Ditto）。静默失败会让用户以为软件坏了 —— 需在托盘图标加角标、设置页红字提示、首次失败弹一次通知，并提供**改键 UI**（捕获按键的输入框，实时试注册，成功才保存）。

**GPUI 特有注意**：`RegisterHotKey` 的 `WM_HOTKEY` 消息投递到注册它的线程的消息队列。必须在 GPUI 的主线程（拥有窗口消息循环的线程）注册，并把消息桥接到 GPUI 的事件系统（通过 `cx.update()` 或专用 channel）。在后台线程注册会收不到消息。

### 7.4 速记浮窗

```rust
cx.open_window(WindowOptions {
    titlebar: None,
    kind: WindowKind::PopUp,               // 不进任务栏
    is_movable: true,
    window_background: WindowBackgroundAppearance::Blurred,
    window_bounds: Some(WindowBounds::Windowed(bounds_on_cursor_monitor())),
    ..Default::default()
}, ...)?;
```

- 460 × 自适应（96–360）
- **常驻但隐藏**（`hide()` 非 `close()`），保证唤出 < 30ms。GPUI 窗口本身很轻，常驻代价远小于 WebView（约 5–10MB vs 30–40MB），因此不必像 v1.0 那样提供"隐藏 10 分钟后销毁"的选项
- `always_on_top`

**三个必须处理的问题：**

**① 前台焦点抢占。** Windows 前台锁定机制会让非前台进程的 `SetForegroundWindow` 静默失败，只闪任务栏：

```rust
let fg = GetForegroundWindow();
let fg_tid = GetWindowThreadProcessId(fg, None);
let cur_tid = GetCurrentThreadId();
AttachThreadInput(cur_tid, fg_tid, true);
SetForegroundWindow(hwnd);
SetFocus(hwnd);
AttachThreadInput(cur_tid, fg_tid, false);
```

之后还需在 GPUI 侧 `window.focus(&editor_focus_handle)`。两层都要做。

**② 多显示器定位。** 必须出现在**鼠标当前所在的显示器**：`MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST)` 取工作区，居中偏上（工作区高度 20%）。记忆位置时存"相对某显示器的偏移"而非绝对坐标——否则拔掉外接屏后窗口跑到屏幕外。

**③ 混合 DPI。** 4K@150% 与 1080p@100% 并存时，坐标换算必须区分物理/逻辑像素。GPUI 内部用逻辑像素（`Pixels`），与 windows-rs 交互时需按 `scale_factor` 换算。**这是 GPUI + windows-rs 混用最容易出错的接缝**，建议在 `platform` 模块内封装 `PhysicalPoint`/`LogicalPoint` 两个类型，用类型系统防止混淆。

### 7.5 托盘、通知、自启、Mica

**托盘**（`tray-icon` crate）：速记 / 打开主窗口 / 本周回顾 / 立即同步（显示状态）/ 设置 / 退出。左键切换主窗显隐，双击显示并聚焦。图标需 16/20/24/32/40/48px 多尺寸 ICO，明暗两套，监听 `WM_SETTINGCHANGE` 切换。

**Jump List**：任务栏右键的"速记 / 本周回顾 / 打开主窗口"，通过 `ICustomDestinationList`。

**Toast 通知**：**必须先注册 AppUserModelID**，否则通知根本不显示——最常见的踩坑点。

```rust
SetCurrentProcessExplicitAppUserModelID(w!("Zhiyan.Desktop"));
```

安装器创建的快捷方式必须写入相同的 `System.AppUserModel.ID`。

**开机自启**：直接写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`。注意用户可能在「设置→应用→启动」关闭，此时注册表项仍在但被 `StartupApproved` 禁用。设置页显示的必须是**真实生效状态**。

**Mica / Acrylic**：Win11 22H2+ 通过 `DWMWA_SYSTEMBACKDROP_TYPE` 设置。与 GPUI 的 `WindowBackgroundAppearance` 配合时需确认二者不冲突（GPUI 可能已通过该 API 实现 `Blurred`）——P0 需验证。Win10 降级为不透明。

---

## 8. 数据模型

### 8.1 记录主键：UUIDv4

**不用 ULID，也不用 UUIDv7。** 服务端必须知道记录主键才能做增量同步，而 ULID/UUIDv7 的前 48 位是明文毫秒时间戳——等于把每条记录的创建时刻直接送给服务端。E2EE 下这是不可接受的元数据泄漏。

用 **UUIDv4**（纯随机，16 字节）。本地排序靠 `created_at` 列 + 索引，功能无损失。

### 8.2 领域类型

```rust
#[derive(Clone, Debug)]
pub struct Memo {
    pub id: Uuid,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    pub rev: u32,
    pub text: String,               // 完整 Markdown 源
    pub pinned: bool,
    pub source: Source,             // App | Quick | WeChat | Telegram
    pub tags: Vec<String>,          // 派生自 text，冗余存储供索引
    pub refs: Vec<Uuid>,            // 手动关联 + 行内引用合并
    pub blobs: Vec<BlobRef>,
    pub dirty: bool,
    pub server_seq: Option<i64>,
}
```

**注意**：`Debug` 需手动实现以避免打印正文，见 §14。

**加密载荷**（上传时序列化）：

```json
{
  "v": 1, "text": "…", "createdAt": …, "updatedAt": …, "deletedAt": null,
  "pinned": false, "source": "quick",
  "tags": ["读书/认知"], "refs": ["9f2c…"],
  "blobs": [{"sha256":"a3f9…","mime":"image/jpeg","w":1600,"h":1200,"size":184320}],
  "pad": "………"
}
```

**标签、引用、附件元信息全部属于密文内容。** 若为服务端筛选而明文上传标签，等于送出用户的主题结构——「读书/心理学」「求职」这类标签本身就高度敏感。筛选一律本地做。

### 8.3 相对原型的三处关键修正

**① 附件外置。** 原型把图片存成 base64 塞进 JSON，2MB 图片膨胀成 2.7MB 文本。改为内容寻址，数据库只存 hash。

**② 硬删除改墓碑。** `purge()` 在 A 机彻底删除、B 机不知道，下次同步又推回来——"删不掉的笔记"是同步应用最经典的 bug。墓碑保留 90 天后由后台任务真删。

**③ 单键整包 JSON 改分表。** 原型用 `window.storage` 存一个大 JSON，改为 SQLCipher 分表 + 分页查询。

**迁移**：一次性脚本，建 `int → UUIDv4` 映射表，同时重写 `rel[]` 与正文中所有 `[[...^数字]]`。趁数据量小尽早做。

---

## 9. 端到端加密

（内容与 v1.0 一致，此处为完整保留）

### 9.1 威胁模型

**防得住**：拿到服务端库全量导出的人、拿到对象存储全部文件的人、服务端运维、偷走关机笔记本的人、中间人。以上均读不到任何正文。

**防不住（必须诚实告知）**：
- 已登录 Windows 会话下运行的恶意程序（免密启动的必然代价）
- 内存中的明文
- 元数据：记录条数、每条大小、写入时刻——**写入时刻序列基本等价于热力图和作息**
- 用户主动导出的 Markdown（设计上就该是明文）

**明确的取舍**：**忘记密码 = 数据永久丢失**。因此恢复码是注册流程的强制环节。

### 9.2 密钥体系

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
   ├─ HKDF(DEK,"blob", plain_hash)  ─▶ CEK，加密附件（收敛）
   ├─ HKDF(DEK,"index")             ─▶ DBKey，SQLCipher 本地库密钥
   └─ HKDF(DEK,"inbox")             ─▶ 解开 X25519 私钥
```

**为什么要 DEK 这一层**：直接用 MK 加密数据的话，**改密码就要全量重加密**。有了 DEK，改密码只需重新包一次 32 字节。这是唯一理由，但足够充分。

**AuthKey 与 MK 分离**：HKDF 不可逆，服务端拿到 AuthKey 推不出 MK。服务端再对 AuthKey 做一次 Argon2id 后存储。

**内存卫生**：密钥类型用 `Zeroize + ZeroizeOnDrop`，**一律用定长数组不用 `Vec<u8>`**——`Vec` 扩容会留下未清零的旧缓冲区副本。

> **GPUI 下的额外注意**：明文正文会进入文本排版缓存与 GPU 纹理图集。进程内存 dump 仍可读到，与 WebView 方案风险相当。锁定时需清空渲染缓存并触发重绘。

### 9.3 加密信封

```
偏移   长度   内容
0      1      version = 0x01
1      1      alg     = 0x01  (XChaCha20-Poly1305)
2      24     nonce（随机）
26     N      ciphertext
26+N   16     tag
```

**为什么是 XChaCha20-Poly1305 而非 AES-GCM**：192 位 nonce 允许随机生成而不必担心碰撞；AES-GCM 的 96 位 nonce 需计数器管理，跨设备同步麻烦。纯软件实现性能也好。

**AAD 必须绑定记录身份**：`aad = version || alg || record_id`。不加的话攻击者可把 A 的密文整块换成 B 的——内容读不懂，但能做定向破坏或让记录错位。

### 9.4 恢复码

```
protected_dek_pw       = Enc(HKDF(MK,"wrap"), DEK)
protected_dek_recovery = Enc(HKDF(RK,"wrap"), DEK)
```

128 位随机数编成 10 组 5 字符 Base32（Crockford 变体，去掉 I/L/O/U）。

**注册流程必须包含**：① 显示 ② 下载 txt / 复制 ③ **要求回填其中随机两组以确认已保存**。

第三步不能省。只显示不校验的话，绝大多数用户会直接点下一步，然后在第一次忘记密码时永久丢失全部数据。

**不做**：安全问题找回、服务端托管密钥备份、"我们帮你保管一份副本"。

### 9.5 附件加密与去重

```
plain_hash = SHA256(明文)
CEK        = HKDF(DEK, info="blob" || plain_hash)     // 确定性
nonce      = HMAC-SHA256(CEK, "nonce")[0..24]         // 确定性
ciphertext = XChaCha20-Poly1305(CEK, nonce, 明文, aad=plain_hash)
cipher_id  = SHA256(ciphertext)                        // 服务端对象键
```

同一用户同一张图 `cipher_id` 恒定 → 上传前 `HEAD /blobs/{cipher_id}`，存在即跳过。

**为什么 CEK 要掺 DEK**：纯收敛加密下不同用户的相同文件密文相同，服务端可做"确认文件存在"攻击。掺入 DEK 后跨用户去重失效，但跨用户去重本来也不该做。

**缩略图也必须加密**（`info="thumb" || plain_hash`）。容易漏的一项——明文缩略图等于加密白做。

> **GPUI 下的简化**：v1.0 需要注册 `zhiyan://` 自定义协议，把解密字节喂给 WebView 以避免明文落盘。GPUI 下直接把解密后的 `Vec<u8>` 交给 `ImageSource`，**明文从不离开进程内存**，这一整套协议层可以删掉。

---

## 10. 本地存储

### 10.1 目录布局

```
%APPDATA%\Zhiyan\                  # ASCII 目录名
├─ zhiyan.db                       # SQLCipher（含 FTS 索引）
├─ zhiyan.db-wal / -shm
├─ blobs\a3\a3f9c2e1…              # 附件密文，按 plain_hash 前两位分桶
├─ cache\thumbs\a3\a3f9c2e1…       # 缩略图密文
├─ backups\2026-08-05T03-00.zbk    # 加密备份
└─ state.json                      # 非敏感：窗口位置、同步游标、UI 偏好
```

`state.json` 明文，**只能放不敏感内容**。窗口位置、主题、字号可以；最近搜索词、标签列表不行。

### 10.2 本地库必须整体加密

不加密的话，`memo` 表和 `memo_fts` 索引里躺着完整明文。笔记本被偷、磁盘被恢复、或备份软件把 `%APPDATA%` 同步到别处，加密就全白做了。

用 **SQLCipher**。页级透明加密，SQLite 引擎看到解密后的页，所以 **FTS5、索引、事务、WAL 全部照常工作，SQL 一行不用改**。这是它相对"应用层字段加密"的决定性优势——后者会让全文搜索和 `ORDER BY created_at` 直接失效。

### 10.3 打开数据库：必须用裸密钥模式

```rust
let conn = Connection::open(&db_path)?;
// 关键：x'...' = 直接提供 32 字节裸密钥，跳过 SQLCipher 自带的 PBKDF2
conn.pragma_update(None, "key", format!("x'{}'", hex::encode(db_key)))?;
conn.pragma_update(None, "cipher_page_size", 4096)?;
conn.pragma_update(None, "journal_mode", "WAL")?;
conn.pragma_update(None, "synchronous", "NORMAL")?;
```

**不要**用 `PRAGMA key = 'passphrase'`。那会触发内置的 25.6 万次 PBKDF2，每次打开连接都跑一遍——连接池场景下启动时间劣化到秒级。`DBKey` 已是 Argon2id 派生的强密钥，无需再拉伸。

性能开销：页级 AES-256-CBC，查询慢 5%–15%，万级数据下感知不到。

### 10.4 免密启动

```
DEK ──DPAPI(CryptProtectData, 用户作用域)──▶ 密文
                                             │
                            写入 Windows Credential Manager
                            （keyring crate，目标名 "Zhiyan/dek"）
```

DPAPI 用户作用域密钥绑定 Windows 登录凭据，离开这台机器的这个账户就解不开。

**安全边界必须写清楚**：同一 Windows 会话内运行的程序可以解出密钥。这是所有免密启动方案的共同代价（1Password、Bitwarden 的"记住我"同理）。

设置项：「启动时要求输入密码」（默认关）、「空闲 N 分钟后锁定」（默认关）、「设备丢失时远程吊销」（服务端标记 token 失效，但**已在本地的数据吊销不了**，UI 必须说明）。

### 10.5 表结构

SQLCipher 已整库加密，本地表存**明文列**，FTS/索引/聚合全部可用。密文只在推送时生成。

```sql
CREATE TABLE memo (
  id          BLOB PRIMARY KEY,        -- UUIDv4，16 字节
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  deleted_at  INTEGER,                 -- 墓碑
  rev         INTEGER NOT NULL DEFAULT 1,
  text        TEXT NOT NULL,
  pinned      INTEGER NOT NULL DEFAULT 0,
  source      TEXT NOT NULL DEFAULT 'app',
  dirty       INTEGER NOT NULL DEFAULT 1,
  server_seq  INTEGER
);
CREATE INDEX idx_memo_time  ON memo(created_at DESC) WHERE deleted_at IS NULL;
CREATE INDEX idx_memo_dirty ON memo(dirty) WHERE dirty = 1;

CREATE TABLE memo_tag (memo_id BLOB, tag TEXT, PRIMARY KEY(memo_id, tag));
CREATE INDEX idx_tag ON memo_tag(tag);

CREATE TABLE memo_ref (src BLOB, dst BLOB, kind TEXT, PRIMARY KEY(src, dst, kind));
CREATE INDEX idx_ref_dst ON memo_ref(dst);      -- 反向链接

CREATE TABLE blob (
  plain_hash BLOB PRIMARY KEY,
  cipher_id  BLOB NOT NULL,
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

**中文分词**：`tokenize='trigram'`（SQLite 3.34+ 内置）。默认的 `unicode61` **不切分中文**，"认知"搜不到"注意力认知负荷"。trigram 索引膨胀约 3 倍，万级数据下无所谓。

### 10.6 数据访问接口

**与 v1.0 的主要差异：没有 IPC，这些是普通 Rust 函数。**

```rust
impl Store {
    pub fn list(&self, filter: &Filter, cursor: Option<Uuid>, limit: u32) -> Result<Vec<Memo>>;
    pub fn get(&self, id: Uuid) -> Result<Option<Memo>>;
    pub fn upsert(&self, input: MemoInput) -> Result<Memo>;   // 内部解析标签/引用，写三张表
    pub fn soft_delete(&self, id: Uuid) -> Result<()>;
    pub fn restore(&self, id: Uuid) -> Result<()>;
    pub fn purge(&self, id: Uuid) -> Result<()>;              // 写墓碑
    pub fn search(&self, q: &str, limit: u32) -> Result<Vec<Memo>>;
    pub fn stats(&self, range: Range) -> Result<Stats>;
    pub fn put_blob(&self, bytes: &[u8], mime: &str) -> Result<BlobMeta>;
    pub fn export(&self, fmt: ExportFmt) -> Result<PathBuf>;
}
```

**v1.0 里"必须分页、统计必须在 Rust 侧聚合、不要把原始数据搬到前端"这一整组约束消失了**——没有序列化边界，传引用即可。分页仍然保留，但理由从"IPC 开销"变成"虚拟化列表的取数窗口"。

**线程模型**：所有 `Store` 调用走 `cx.background_spawn()`，结果通过 `cx.update()` 回 UI 线程。不要在 GPUI 主线程上做数据库 IO——SQLCipher 的解密开销足以在大查询时掉帧。

### 10.7 备份与导出

| 类型 | 加密 | 说明 |
|---|---|---|
| 本地自动备份 `.zbk` | ✅ DEK | 每日 + 启动检查，保留 30 日 + 12 月 |
| 云端 | ✅ | 同步本身即备份 |
| 导出 JSON | ⚠️ 可选 | "加密导出（需恢复码）" / "明文导出" |
| 导出 Markdown | ❌ 明文 | 本质如此，导出前明确提示 |

**关键提醒**：本地备份用 DEK 加密，而 DEK 依赖密码/恢复码。用户忘记密码且丢失恢复码时，本地备份同样打不开。因此"注销账号"与"重置密码"流程必须先引导导出**明文 Markdown**。

---

## 11. 云同步

（与 v1.0 一致）

### 11.1 服务端表

```sql
CREATE TABLE account (
  user_id UUID PRIMARY KEY, email TEXT UNIQUE NOT NULL,
  auth_hash TEXT NOT NULL, kdf_salt BYTEA NOT NULL, kdf_params JSONB NOT NULL,
  protected_dek BYTEA NOT NULL, protected_dek_recovery BYTEA NOT NULL,
  inbox_pubkey BYTEA, created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE record (
  user_id UUID NOT NULL, id BYTEA NOT NULL, seq BIGSERIAL,
  ciphertext BYTEA NOT NULL, device_id UUID NOT NULL,
  PRIMARY KEY (user_id, id)
);
CREATE INDEX idx_record_seq ON record(user_id, seq);

CREATE TABLE device (
  device_id UUID PRIMARY KEY, user_id UUID NOT NULL,
  name TEXT, platform TEXT, last_seen TIMESTAMPTZ, revoked BOOLEAN DEFAULT false
);
```

**服务端明文字段仅此而已**：`user_id`、随机 `id`、`seq`、密文长度、`device_id`。`updated_at` **不在服务端**——冲突判定完全在客户端做。

附件走对象存储（推荐 Cloudflare R2，出网流量免费），key 为 `{user_id}/{cipher_id}`，上传用预签名 URL。

### 11.2 协议

```http
GET /v1/changes?since=88213&limit=200
→ { "changes":[{"id":"3f9a…","seq":88214,"device":"…","ciphertext":"base64…"}],
    "cursor":88214, "hasMore":false }

POST /v1/changes
  { "deviceId":"…", "changes":[{"id":"3f9a…","ciphertext":"base64…","baseSeq":88101}] }
→ { "accepted":["3f9a…"], "conflicts":[], "cursor":88220 }
```

`baseSeq` 用于并发控制，服务端不需要看懂内容就能做。

### 11.3 outbox

本地写入先落库并入 `outbox`，异步推送，失败指数退避。离线可用是天然的。触发：编辑后 debounce 2 秒、窗口失焦、启动、每 5 分钟兜底。

### 11.4 冲突与异常

按记录 LWW，比对解密后的 `updatedAt`。败方**保留为一条新的"冲突副本"，不静默丢弃**。

需留神：待办勾选会重写整条 `text`，手机勾框 + 电脑改正文时 LWW 会丢一边。v1 接受此损失。

**解密失败必须显式处理**，不能静默跳过。记入 `sync_error` 表并在 UI 暴露"N 条记录无法解密"。

### 11.5 微信 / Telegram 入口的折中

E2EE 与"服务端代收消息"天然冲突：微信把明文投递到你的服务器。

**非对称收件箱**：注册时生成 X25519 密钥对，公钥明文存服务端，私钥用 DEK 加密后存服务端。消息到达时服务端用公钥做 sealed box，写入 `inbox` 后**立即丢弃明文**。客户端同步时拉取、解密、转成正式记录。

**必须诚实告知**：

> 通过微信记录的内容，在加密前会短暂经过我们的服务器。若你希望所有内容都不经第三方，请关闭此入口。

**或者干脆不做。** 若把"服务端从不接触明文"作为产品承诺，就该放弃这个入口。flomo 正因为有微信输入所以做不了 E2EE——这是真实的差异点。**这是产品定位问题，不是技术问题**，建议明确选一边。

### 11.6 元数据泄漏与缓解

| 可见 | 可推断 |
|---|---|
| 记录条数 | 使用强度 |
| 每条密文长度 | 每条大致字数 |
| `seq` 递增时刻 | **几点在写 → 等价于热力图和作息** |
| 附件大小与数量 | 是否常存图片 |

**长度填充**：密文按桶对齐（256/512/1K/2K/4K/8K，之后按 8K 递增），明文加 `pad` 字段。平均多传约 30%，纯文本笔记完全可接受。

**写入时刻**不做混淆——要遮就得随机延迟上传，会破坏多端准实时体验。隐私政策中如实写明。

---

## 12. 打包、签名与更新

### 12.1 构建

```
cargo build --release --target x86_64-pc-windows-msvc
```

**必须用 MSVC 工具链**，不用 GNU：GPUI 的 Windows 后端依赖 DirectX / DirectWrite 的 COM 接口，MSVC 链接更可靠。

`Cargo.toml` release profile：

```toml
[profile.release]
opt-level = 3
lto = "thin"        # "fat" 编译时间翻倍，收益有限
codegen-units = 1
strip = "debuginfo" # 但保留 PDB 供崩溃符号化
panic = "abort"
```

**PDB 必须单独归档**（不随安装包分发），用于崩溃堆栈符号化。

### 12.2 安装器

**与 v1.0 的差异：没有 Tauri bundler**。选项：

| 方案 | 说明 |
|---|---|
| `cargo-packager` | 社区维护，支持 NSIS/WiX/MSIX，配置类似 Tauri bundler，**推荐** |
| 直接写 NSIS 脚本 | 控制力最强，维护成本中等 |
| WiX (MSI) | 企业分发友好，学习曲线陡 |

安装模式选 **per-user**（免 UAC 提权，转化率明显更高），安装到 `%LOCALAPPDATA%\Programs\Zhiyan\`。

**不再需要**：WebView2 Bootstrapper、运行时检测逻辑。这是 GPUI 方案的一处实际简化。

### 12.3 代码签名

**SmartScreen 会拦截未签名安装包**，弹"Windows 已保护你的电脑"，对转化率杀伤极大。

| 方案 | 年成本 | SmartScreen |
|---|---|---|
| 不签名 | 0 | 一直拦截 |
| OV 证书 | ¥2000–4000 | 需累积下载量建立信誉，数周至数月 |
| EV 证书 | ¥4000–6000 | 签完即时通过 |
| Microsoft Store (MSIX) | 0（一次性注册费） | 商店身份自带信任 |

**注意**：2023 年 6 月起 OV/EV 私钥强制硬件令牌或云 HSM，CI 自动签名需配置云签名服务（Azure Trusted Signing、DigiCert KeyLocker），**不能再把 .pfx 放进 CI secrets**。

**推荐组合**：NSIS + OV 为主渠道，同时上架 Store 作为"安全下载"背书。

**MSIX 的限制**：全局热键与自启均可用（后者需声明 `startupTasks`），但 `%APPDATA%` 会重定向到应用容器——数据路径逻辑要兼容两种情况。

### 12.4 自动更新

**与 v1.0 的差异：没有 `tauri-plugin-updater`，需自建。**

方案：`self_update` crate，或自行实现（Zed 的做法）：

1. 启动时 + 每 6 小时拉取 CDN 上的 `latest.json`（含版本号、下载地址、Ed25519 签名）
2. 验签后后台下载安装包到临时目录
3. 提示"重启以更新"，**不强制立即重启**
4. 用户确认后启动安装器并退出主进程

**必须做的两件事**：

- **签名验证**：更新包必须用应用内置公钥验签，否则更新通道就是后门。这与代码签名证书是两回事，不花钱。
- **失败回滚**：保留上一版本安装包；检测到新版本连续三次启动崩溃时，提示回滚。

**灰度**：`latest.json` 带 `rollout` 百分比，客户端按设备 ID 哈希判定。

---

## 13. 安全与隐私

**相对 v1.0 大幅简化**——没有 web 内容，以下整类风险消失：

- ~~CSP 配置~~
- ~~XSS / HTML 注入~~
- ~~`<a target="_blank">` 的 scheme 注入（`file://`、`ms-msdt:`）~~
- ~~自定义协议以避免图片明文落盘~~
- ~~WebView DevTools 需在 release 禁用~~
- ~~Tauri capability 权限清单~~

**仍然存在的**：

| 项 | 措施 |
|---|---|
| Markdown 链接 | 点击前校验 scheme，**只允许 http/https**，用 `ShellExecuteW` 打开；其余 scheme 拒绝并提示 |
| 图片解码 | 用户提供的图片走 `image` crate 解码，需限制最大尺寸与解码内存，防解压炸弹 |
| 剪贴板 | 粘贴的图片同样限制大小 |
| 凭证 | token 存 Credential Manager，不写数据库；MK 仅在内存，退出即 zeroize |
| 依赖供应链 | `cargo-deny` 检查许可证与已知漏洞；GPUI 为 git 依赖，需 pin commit 并定期审查 diff |
| 反调试 | 不做。徒增复杂度，挡不住有能力的攻击者 |

---

## 14. 日志与可观测性

E2EE 最容易从日志漏。硬性规则：

**① 任何日志不得包含 `text`、标签名、附件字节。** 只允许记录 id、长度、错误码。

**② 为正文类型手写 `Debug`**，杜绝 `{:?}` 误打全文：

```rust
impl fmt::Debug for Memo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Memo")
            .field("id", &self.id)
            .field("len", &self.text.len())
            .field("tags", &self.tags.len())
            .finish_non_exhaustive()
    }
}
```

**③ `tracing` 加一层脱敏 layer**，对字段名做白名单（只放行 `id`/`len`/`count`/`code`/`ms` 等），非白名单字段一律替换为 `<redacted>`。这比依赖每个调用点自觉更可靠。

**④ 崩溃上报必须关闭 minidump 的堆内存采集**，否则明文正文随 dump 上传。只收堆栈 + 寄存器。

**⑤ GPUI 特有**：渲染层的 debug overlay、元素树 dump 等调试功能在 release 构建中编译移除（`#[cfg(debug_assertions)]`），避免通过快捷键意外暴露内容。

---

## 15. 测试策略

**相对 v1.0 的最大变化：UI 测试变难。** 没有 DOM，没有 WebDriver，没有前端测试框架。应对策略是把逻辑尽量挤出 UI 层。

| 层次 | 工具 | 覆盖 |
|---|---|---|
| 领域逻辑 | `cargo test`（`zhiyan-core`，**不依赖 gpui**） | Markdown 解析、标签树、统计聚合、周报、避头尾规则 |
| 加密 | `cargo test` + RFC 向量 | Argon2id/HKDF/XChaCha20；**AAD 换位必须解密失败**；信封跨版本 |
| 数据层 | rusqlite 内存库 | 墓碑、引用级联、blob 引用计数、FTS 中文命中、UUID 迁移 |
| UI 单元 | `gpui::TestAppContext` | 视图状态转换、按键 action 分发、焦点流转 |
| 视觉回归 | 自建截图对比 | 关键界面在 100%/150%/200% 下截图，与基线比对 |
| E2E | 手工 | 见下 |

**视觉回归值得投入**：GPUI 下改一处样式常量可能影响多处，没有 CSS 那样的作用域隔离。建立十来张基准截图（明暗 × 三档缩放 × 主界面/浮窗/命令面板），在 CI 里比对像素差异。

**必须人工验证的矩阵**：

- **输入法矩阵**（§6.3 全部条目）—— 每个里程碑都要跑一遍，不是只在 P0
- Windows 10 22H2 / Windows 11 23H2 各一台
- 100% / 150% / 200% 缩放；**混合 DPI 双屏拖拽**
- 明暗主题切换、系统强调色变化
- 高对比度模式
- **GPU 兼容性**：集显（Intel UHD）、独显（NVIDIA/AMD）、远程桌面（无硬件加速）、虚拟机。GPUI 依赖 GPU 渲染，这一项在 WebView 方案下不需要测，现在必须测
- 旧版升级后的数据完整性
- 断网写入 → 联网后合并

---

## 16. 里程碑

**总工期相对 v1.0 增加约 7 周**，全部增量来自 UI 层重写与编辑器自建。

| 阶段 | 内容 | 工期 | 出口标准 |
|---|---|---|---|
| **P0 选型验证** | GPUI 依赖形态确认；**中文输入法可行性验证**；DPI/多屏/透明窗；GPU 兼容性抽测 | **5 天** | §6.3 输入法清单全部通过。**不通过则回退 v1.0 的 Tauri 方案** |
| **E0 密钥** | Argon2id/HKDF/信封 + RFC 向量 + 跨版本解密测试 | 3 天 | 与 RFC 向量一致；AAD 换位解密失败 |
| **E1 本地库** | SQLCipher + DPAPI/Credential Manager + 免密启动与锁定 | 4 天 | 冷启动 < 300ms；库文件外部工具打不开 |
| **U1 骨架** | 窗口 + 自绘标题栏 + Snap Layouts + 三栏布局 + 主题令牌 + 字体注册 | **8 天** | 静态界面与原型截图一致；三档缩放正常 |
| **U2 时间脊与卡片** | 自定义 Element + 变高虚拟化列表 + Markdown 渲染（含避头尾、盘古之白） | **12 天** | 万条数据滚动稳定 60fps+；渲染结果与原型一致 |
| **U3 编辑器** | 文本输入 + IME + 光标选区 + 撤销 + 补全弹层 + 自动续行 | **18 天** | 输入法矩阵全通过；编辑千字文档无卡顿 |
| **U4 侧栏与右栏** | 标签树 + 导航 + 拾遗 + 统计 + 热力图 + 每周回顾 | **10 天** | 交互与原型一致 |
| **U5 弹层与浮窗** | 命令面板 + 设置/账号面板 + 菜单 + Toast + 速记浮窗 | **10 天** | 浮窗唤出 < 30ms |
| **M3 数据层** | UUID 迁移 + 附件外置 + FTS + 本地备份 + 导入导出 | 8 天 | 万级数据搜索 < 100ms |
| **M2 系统集成** | 全局热键（含冲突与改键 UI）+ 浮窗置前台 + 托盘 + Jump List + 自启 + Mica | 6 天 | 混合 DPI 双屏下浮窗定位与焦点 100% 可靠 |
| **E2 账号** | 注册/登录/恢复码全流程（含回填校验） | 5 天 | 恢复码可完整找回数据 |
| **E3 附件** | 收敛加密 + 内存解码 + 缩略图 | 4 天 | 磁盘上无任何明文图片 |
| **M4 打包** | cargo-packager + 自建更新器 + 签名 + 崩溃上报 | 6 天 | 全新机器安装升级链路通过 |
| **M5/E4 同步** | outbox + 变更游标 + 冲突提示 + 长度填充 | 15 天 | 双端离线编辑合并无丢失/无重复/无复活 |
| **E5 密钥运维** | 改密码 / 恢复码轮换 / DEK 轮换 | 4 天 | 改密码不触发数据重传 |
| **E6 审计** | 日志脱敏 + 崩溃配置 + 渗透自测 | 3 天 | 全量日志中检索不到任何正文片段 |

**关键路径**：`P0 → E0 → E1 → U1 → U3`。U3（编辑器）是最长的单项，且依赖 P0 的输入法结论，建议 P0 一结束就启动 U3 的技术预研，与 U1/U2 并行。

**单机版（不含同步）约 15 周**（v1.0 为 8 周）。同步作为独立阶段。

**P0 是可以否决整个选型的关卡。** 5 天的验证成本，换的是不在第 10 周才发现输入法过不去。这个投入必须花。

---

## 17. 待决策项

**① P0 输入法验证结果。** 这是所有其他决策的前提。不通过则回退 v1.0。

**② 编辑器是否做所见即所得。** §6.4 建议 v1 用"源码 + 轻量着色"，保存后完整渲染。若坚持 WYSIWYG，U3 需 +3–4 周。

**③ 微信入口保不保。** 影响产品定位与隐私承诺措辞。见 §11.5。越晚决定返工越大。

**④ 无障碍是否为硬需求。** GPUI 对屏幕阅读器支持有限。若为硬需求，本方案不成立。

**⑤ OPAQUE 还是当前 AuthKey 方案。** 当前方案下服务端在登录瞬间接触 AuthKey（推不出 MK）。建议 v1 用当前方案，协议版本号留升级空间。

**⑥ DEK 轮换的触发条件。** 需产品侧定义（设备丢失？恢复码泄漏？）。

**⑦ Store 版是否作为主渠道。** 影响数据目录路径、更新机制、构建流水线。

**⑧ 公开分享单条。** E2EE 下需为每次分享生成独立密钥并把密钥放在 URL fragment（不发给服务器）。建议 v1 不做，只保留"分享成图片"这条本地渲染路径。

---

## 18. 附录

### 附录 A · 从原型到工程的改动清单

| 原型现状 | 目标 | 优先级 |
|---|---|---|
| HTML/CSS/JS 界面 | **GPUI 重写**（原型降级为视觉规格） | P0 |
| `<textarea>` 白送输入法 | 自实现 `PlatformInputHandler` | P0 |
| `window.storage` 单键整包 JSON | SQLCipher 分表 | P0 |
| 自增整数 `id` | UUIDv4 | P0 |
| 图片 base64 存于 `imgs[]` | 收敛加密的内容寻址 blob | P0 |
| `purge()` 硬删除 | 墓碑 | P0 |
| 本地明文 | SQLCipher 整库加密 | P0 |
| 字体栈依赖系统 | 内嵌子集化 OTF | P0 |
| `Ctrl+Shift+N` | `Ctrl+Alt+Space` | P0 |
| 无密钥体系 | 三层密钥 + 恢复码 | P0 |
| 前端 JS 全量过滤搜索 | FTS5 trigram | P1 |
| CSS hover 过渡 | 即时切换（GPUI 无过渡原语） | P1 |
| 账号面板为静态 mock | 接真实鉴权 | P2 |
| 同步开关为装饰 | 五态显示 | P2 |

**同步状态的可见性值得单独强调**：同步类应用最伤信任的时刻，是用户不知道自己刚写的东西到底存没存上。侧栏底部应常驻"未上传 N 条"的入口。五态为：`已同步` / `同步中` / `N 条待上传` / `离线` / `冲突需处理`。

### 附录 B · 快捷键总表

| 快捷键 | 功能 | 作用域 | 实现机制 |
|---|---|---|---|
| `Ctrl+Alt+Space` | 速记浮窗 | 全局 | `RegisterHotKey` |
| `Ctrl+Alt+Z` | 显示/隐藏主窗口 | 全局（默认关） | `RegisterHotKey` |
| `Ctrl+K` | 命令面板 | 应用内 | GPUI Action |
| `Ctrl+N` | 聚焦主编辑器 | 应用内 | GPUI Action |
| `Ctrl+Enter` | 保存 | 编辑器 | GPUI Action |
| `Ctrl+B` / `Ctrl+I` | 粗体 / 斜体 | 编辑器 | GPUI Action |
| `Ctrl+R` | 换一条拾遗 | 应用内 | GPUI Action |
| `Ctrl+Alt+R` | 本周回顾 | 应用内 | GPUI Action |
| `Ctrl+D` | 切换深浅外观 | 应用内 | GPUI Action |
| `/` | 聚焦搜索 | 应用内 | GPUI Action |
| `Esc` | 关闭弹层 / 收起浮窗 | — | GPUI Action |
| `#` | 标签补全 | 编辑器 | 编辑器内部 |
| `[[` | 片语引用补全 | 编辑器 | 编辑器内部 |

**注意**：编辑器聚焦时，`Ctrl+Enter` 等 Action 需通过 GPUI 的 focus 上下文分发，避免与文本输入冲突。**组合串未上屏时必须屏蔽所有 Action**，否则输入拼音过程中的按键会误触发命令。

### 附录 C · 术语表

| 术语 | 含义 |
|---|---|
| 片语 | 一条笔记的单位称呼（应用名"知言"，单位保留"片语"） |
| 时间脊 | 贯穿时间线左侧的竖线与日期节点，节点大小编码当天字数 |
| 拾遗 | 随机漫步式回顾 |
| GPUI | Zed 开发的 Rust GPU 加速 UI 框架 |
| Element | GPUI 的可绘制单元；自定义 Element 需实现 request_layout/prepaint/paint |
| marked text | 输入法未上屏的组合串 |
| 避头尾 | 中文排版规则：标点不出现在行首、特定标点不出现在行尾 |
| 盘古之白 | 中西文之间的视觉间隙 |
| MK / DEK / CEK | 主密钥 / 数据密钥 / 单附件密钥 |
| 收敛加密 | 密钥由内容哈希确定性派生，使相同明文产生相同密文以支持去重 |
| 墓碑 | 删除标记记录，代替物理删除以保证多端删除能收敛 |
| outbox | 本地待推送队列，保证离线写入不丢 |
| AAD | 附加认证数据，绑定密文与记录身份，防止密文换位 |
