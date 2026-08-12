# 与 v1.1 文档的对照

拿到 [`design-v1.1-tauri.md`](design-v1.1-tauri.md) 之前，阶段一是从 v2.0（GPUI）文档
对 v1.0 的描述反推着做的。这份记录逐条核对结果，**以 v1.1 为准**。

---

## 一、按文档改掉的（我之前推错了）

| 项 | 文档 | 我原来做的 | 现状 |
|---|---|---|---|
| 构建 | §4.3 **Vite**，只用 ESM 打包与 dev server，不引框架运行时 | 零依赖，无打包器 | 已改用 Vite 8 |
| `withGlobalTauri` | 有 Vite ⇒ 走 `@tauri-apps/api` 的 ESM 导入 | `true` + `window.__TAURI__` | 已改回 `false` |
| 目录 | §4.5 `src/` + `src-tauri/` | `ui/` + 5 个 crate 的工作区 | 已迁移 |
| CSS 拆分 | §4.5 tokens / base / chrome / timeline / overlays | 7 个文件、命名不同 | 已重切成 5 档 |
| IPC 出口 | §4.5 `store.js` | `api.js` | 已改名 |
| JS 模块 | §4.5 main / quick / store / markdown / render / editor / commands / platform | 13 个、命名不同 | 已对齐（另留 util / state / overlays / weekly / share / mock / seed 这些辅助模块） |
| 字体子集化 | §4.5 `scripts/subset-fonts.mjs` | `xtask` | 已改 |
| CSP | §12.2 `img-src 'self' zhiyan: data:` | 写成 `asset:` | 已改；`zhiyan://` 协议在阶段四接 |
| 命令签名 | §9.6 固定的 12 条 | 自造了一批名字 | 已对齐 |
| `list_memos` | §9.6 **必须分页**（cursor + limit） | 一次拉全 | 已改成键集分页，默认 50 |

## 二、按文档补上的（我之前漏了）

**① 输入法组合串保护（§6.2）。** 这是漏得最要紧的一条。

不做的话，打「shi」过程中的 `s`、打「ni」时的 `n` 都可能被当成快捷键前缀误触发；
更常见的是候选框还开着时按回车，本意是选词，却触发了保存。

监听 `compositionstart` / `compositionend`，组合期间屏蔽所有 Action。
`compositionend` 之后不能立刻解除——同一轮事件里紧跟着还有 `keyup`，部分输入法
还会补一个 `keydown`，所以用微任务把解除推后一拍。

**② Snap Layouts 的按钮矩形上报（§5.2 ②）。** 前端在布局与 DPI 变化时
`invoke("set_maxbutton_rect")`。Rust 侧的 `WM_NCHITTEST` 子类化在阶段五。

**③ 生产构建不带 mock。** 用 `import.meta.env.DEV` 而不是运行时判断，让 Vite
把 `mock.js` 与 `seed.js` 整个摇掉。只用运行时判断的话，那些假笔记会跟着进安装包。

## 三、我做了但文档没要求，保留的

**① 前端零 `onclick`，全部改成 `data-act` + 事件委托。**

文档 §12.2 定的 CSP 是 `script-src 'self'`（无 `unsafe-inline`），而原型满屏内联处理器——
两者不能并存。§6.1 只说「事件委托保留（原型已用）」，没点破内联处理器会失效。
我的改法是这个约束的必然结果，且 CI 里加了一条检查钉死它。

**② `purge_all` 与 `set_pinned` 两条命令。** 不在 §9.6 的清单里，理由写在
`store.js` 的函数注释上：前者拆成 N 次 `purge` 会变成 N 次 IPC 往返且出现删到
一半的中间态；后者走 `upsert_memo` 需要把整篇正文回传一遍，只为翻一个布尔值。

**③ 把 `tag_counts` / `view_counts` 折进 `stats`。** §9.6 只列了 `stats(range)`。
它们每次都一起刷新，拆开就是三次 IPC 往返换同一份数据。

## 四、文档要求但还没做的

| 项 | 文档 | 计划 |
|---|---|---|
| 虚拟滚动 | §6.3 万条 60fps | 附录 A 标 P1；分页已就位，虚拟化待做 |
| `zhiyan://` 协议 | §12.2 | 阶段四随附件一起 |
| 前台焦点抢占 | §5.3 ① `AttachThreadInput` | 阶段五 |
| Snap Layouts 子类化 | §5.2 ② | 阶段五 |
| 全局热键 + 冲突检测 + 改键 UI | §5.4 | 阶段五 |
| 托盘 / Jump List / AUMID / 自启 / Mica | §5.5 | 阶段五 |
| WebView2 运行时检测 | §4.2 | 阶段五（打包） |
| Chromium 111 下限检测 | §4.2 | 阶段五 |
| 账号面板接真实鉴权 | 附录 A P2 | 后续 |
| 同步五态显示 | 附录 A P2 | 后续 |

## 五、已确认的两处（2026-08-11）

**① crypto / db 拆成 `src-tauri/` 下的 path 子 crate。** 已按此实施。

目录仍符合 §4.5 的意图（加密与数据各成一块，都在 `src-tauri/` 下），
但两者可以脱离 Tauri 单独 `cargo test`。收益是实测出来的：
`zhiyan-crypto` 的依赖树里没有 tauri，改一行后全量重测 **560ms**；
挂在主 crate 上的话每次都要先编一遍 Tauri。

加密的正确性全靠测试兜底，把它绑在分钟级的编译上，人就不愿意频繁跑测试了。

```
src-tauri/
├─ src/            # main / windows / error / state
├─ crypto/         # §8：keys / envelope / recovery
└─ db/             # §9：待阶段三
```

**② §5.1 的隐藏断点是笔误。** 右栏断点按原型 CSS 取 **1120px**，
侧栏 780px。文档原文「最小 900×560（低于 900 隐藏右栏）」自相矛盾——
窗口不可能低于最小宽度，右栏就永远不会隐藏。

## 六、新发现的一处出入（需你确认）

**恢复码的熵与版式对不上。** §8.4 写「RK 为 **128 位**随机数，编成
**10 组 5 字符** Base32」，并给了 10 组的示例。但 10×5 = 50 个 Base32 字符
能装 250 bit，只放 128 bit 会空掉一多半。

现在按 **240 bit 熵 + 10 bit 校验位 = 250 bit** 实现：

- 版式与文档完全一致，用户要抄的仍是 50 个字符；
- 熵只多不少，满足「至少 128 位」的安全要求；
- 多出的 10 bit 做校验位，抄错一位有约 99.9% 的概率被当场挡下。

**若服务端或别的客户端按 128 bit 实现，这里要改。** 改动很小（换个
`RECOVERY_ENTROPY_LEN` 加填充位），但两端必须一致，否则恢复码互不认。
