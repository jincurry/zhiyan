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
| Jump List | §5.5 | 见下方第十节，**已明确不做**并写清了理由 |
| 代码签名 | §11.2 | 有真实成本，要先定证书方案。步骤在 `docs/release.md` |
| 更新源与 Ed25519 密钥 | §11.3 | 同上；仓库里 `plugins.updater` 是空的，界面如实说「未配置」 |
| 更新失败回滚 | §11.3「必须做」 | 需要真实更新链路才验证得了 |
| 账号面板接真实鉴权 | 附录 A P2 | 后续；DEK 的落地位置已就位，换成从 `protected_dek` 解出来即可 |
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
├─ src/            # main / commands / protocol / state / vault / windows / hotkeys / tray / platform_win
├─ crypto/         # §8：keys / envelope / recovery
└─ db/             # §9：schema / memo / stats / blobstore / backup / legacy / parse
```

**② §5.1 的隐藏断点是笔误。** 右栏断点按原型 CSS 取 **1120px**，
侧栏 780px。文档原文「最小 900×560（低于 900 隐藏右栏）」自相矛盾——
窗口不可能低于最小宽度，右栏就永远不会隐藏。

## 六、阶段三对 §9.5 的三处增补

表结构基本照抄 §9.5，下面三样是文档没写但缺了就不成立的。

**① `memo.purged_at` 列。** §9.5 只给了 `deleted_at`，但 §7.3 ② 要求
「墓碑保留 90 天后由后台任务真删」——只有一个 `deleted_at` 时，
「在回收站里、还能恢复」与「已彻底删除、只剩墓碑」这两种状态分不开。

不加列的硬凑法是「正文为空即墓碑」，但那样用户写了一条空笔记再删掉，
它就永远从回收站里消失了。

做成时间戳而不是布尔值，是因为 90 天的计时器要的正是**彻底删除的时刻**，
不是进回收站的时刻。

**② FTS5 的三个同步触发器。** §9.5 把 `memo_fts` 声明成
`content='memo'`（外部内容表），这种模式下 FTS5 **不会**自己跟着基表走，
官方文档要求配一组 `AFTER INSERT/DELETE/UPDATE` 触发器。

漏了的表现是：新写的笔记搜不到、改过的笔记按旧文还搜得到。两个都很难在
开发期撞见——刚写完的那条正显示在列表里，没人会去搜它。

**③ 几个 §9.5 没列的索引**：回收站列表（`deleted_at DESC`）、墓碑回收
（`purged_at`）、附件反查（`memo_blob.plain_hash`）、孤儿附件（`refcount=0`）。
都是有对应查询才加的。

### 另外两处「文档没说，但只能这么办」

**`Filter` 里没有 `today` / `day` 两个视图，只有一段毫秒区间。**
它们都要「本地时区的今天是哪一段」，而时区是运行环境状态。埋进存储层等于
让每个查询都隐式依赖系统时钟与 TZ 设置，「跨年那天的热力图」这种边界情形
就没法写测试了。所以由 IPC 层算好区间传进来。统计同理，`Clock` 显式传入。

**`upsert_memo` / `soft_delete` 这些写操作都要调用方传 `now_ms`。**
同上，另外它让「补录一条昨天的笔记」和导入历史数据成为可能。

## 七、标签解析有两份实现，用对照表钉住

前端 `util.js` 要解析标签才能做高亮，Rust 侧要解析才能写 `memo_tag` 表。
两份实现无法合并（一份跑在 WebView 里、一份跑在写事务里），只能保证一致。

分叉的表现很难查：

- 界面上高亮着的标签，点侧栏筛不出来；
- 时间脊上每天的字数加起来，对不上统计面板的总数。

做法是把用例表放在仓库根的 `tests/fixtures/parse-parity.json`，
`src/test/parse-parity.test.js` 与 `src-tauri/db/tests/parity.rs` **各跑一遍同一张表**。
期望值以前端为准（用户看到的是那一份），由 `scripts/gen-parse-fixtures.mjs` 生成。

这不是多余的谨慎：这张表第一次跑起来就抓到了一处真分歧——
`用 C#写的` 的字数，前端算 4、Rust 算 5。前端的 `plain()` 最后会无差别地
删掉所有 `#`，我照着标签规则写的那版只删标签里的。为此把 `plain()` 改成了
与 JS 一样的五趟顺序替换（趟与趟之间有依赖：`>` 先被删掉，`> - 列表项`
那一行才认得出列表符号）。

## 八、阶段四的四处判断

**① 时区由前端传给 Rust，Rust 不自己去问系统。**

「今天」「某一天」「热力图的每一格」都要先定下时区，而这个口径必须与
`util.js` 的 `dayKey` 完全一致——那份用的是 WebView 的本地时间。

Rust 侧自己去问有两个问题：`time` 的 `local-offset` 在多线程进程里读环境变量
不安全（Unix 上直接返回错误）；就算读到了，也未必与 WebView 认为的一致。
不一致的表现是热力图整片错位一格，而且只在部分用户身上出现。

所以 `store.js` 每次调用都带上 `tzOffset`（分钟），存储层的 `Filter` 只收
一段毫秒区间。副作用是存储层完全不依赖系统时钟，「跨年那天」这类边界情形
能写测试了。

**② `zhiyan://` 的响应必须带 `Cache-Control: no-store`。**

容易漏的一条：WebView2 会把普通响应缓存到磁盘上——那正是我们刚刚解密出来的
明文。§12.2 只说了「不能解密到临时文件」，缓存是同一件事的另一条路径。

同时把 MIME 从库里查而不是从字节嗅探，并加 `X-Content-Type-Options: nosniff`；
`image/svg+xml` **不在白名单里**，SVG 里能写脚本。

CSP 里除了 `zhiyan:` 还写上了 `http://zhiyan.localhost`：Windows 上自定义协议
会被改写成那个形式。

**③ 免密启动的回退分调试与发布两套。**

凭据管理器在容器和不少 Linux 桌面上没有可用后端。调试构建回退到应用目录下的
明文密钥文件（否则界面没法开发），**发布构建是硬错误**——把密钥明文写在数据库
旁边等于 §8 白做，偷走 `%APPDATA%` 的人连钥匙一起拿走了。宁可开不起来，
也不能悄悄降级。

**④ 开库失败不阻止应用启动。**

密钥取不到时起一个未解锁的空壳，`app_info` 如实报 `unlocked: false`。
让进程直接退出的话，用户看到的是一个闪一下就没了的窗口——那种失败最难报告。

## 九、阶段五：Windows 那一层

**① 交叉编译检查抓到四处 API 错。**

`platform_win.rs` 在 Linux 上整个是空的（`#![cfg(windows)]`），所以它**编译不到**——
打错一个模块路径要到发布构建才发现。装了 mingw 之后
`cargo check --target x86_64-pc-windows-gnu` 当场抓到：
`AttachThreadInput` 在 `System::Threading` 而不是 `UI::Input::KeyboardAndMouse`、
`ScreenToClient` 在 `Graphics::Gdi`、`RegOpenKeyExW` 第三个参数不是 `Option`、
`LocalFree` 不收 `Option`。最后那条在阶段四就写下了，一直没被编译过。

CI 的 Windows job 因此从「只 build」加成了 clippy + test + build 三条。

**② 图标必须有 `icon.ico`，缺了 Windows 构建直接失败。**

这件事在 Linux 上完全看不出来——`cargo build` 一点问题都没有。
写了 `scripts/gen-icons.mjs`（纯 Node 手写 PNG 与 ICO 编码，不引图形库），
CI 里比对生成结果，改了图案忘了提交会红。

托盘另出明暗两套：通知区底色跟着任务栏走，一套颜色总有一边糊在背景里。

**③ `plugins.updater` 不能省也不能填假的。**

整段省掉的话插件初始化直接 panic——**应用起不来，而 `cargo build` 一点问题都没有**，
是跑起来才发现的。填一个占位公钥更糟：看起来像配好了，实际谁都验不过，
失败信息还是「签名不匹配」，排查方向会完全跑偏。

现在留的是 `endpoints: []` + `pubkey: ""`：前者是明确的「没配」，
后者验什么都失败（fail closed）。设置页显示「此构建未配置更新源，需要手动下载新版」，
而不是「已是最新」——两句话对用户的含义完全不同。

**④ 自启要读 `StartupApproved`，不能只看注册表项在不在。**

§5.5 点名了这个坑：用户在「设置 → 应用 → 启动」里关掉之后，
`HKCU\...\Run` 下的项**仍然在**。只读它的话设置页显示「已开启」而实际不自启，
用户重装、反复开关都解决不了——因为开关本来就是开的。

**⑤ 托盘左键：可见但没聚焦时是「拿到前面来」，不是「收起去」。**

只看 `is_visible` 的话，点托盘会把一个被别的窗口压住的知言直接藏掉，
而用户的意思几乎总是相反的。

## 十、Jump List 明确不做

§5.5 说它「成本很低但原生感强」。**成本不低。**

`ICustomDestinationList` 是一套 COM 接口，要 `CoInitializeEx`、
`IObjectCollection`、给每个条目建 `IShellLinkW` 并写
`System.AppUserModel.ID` 属性；还要一条命令行启动路径（`--quick` / `--week`）
和与之配套的单实例转发。

真正的问题是**它在这里一行都验证不了**：不像子类化和 DPAPI 那样至少能靠
交叉编译保证签名正确，Jump List 的失败模式是「装完之后右键任务栏什么都没有」，
只有真机能看见。写一段没验证过的 COM 然后说它能用，比暂时不做更糟。

需要的前置条件（AUMID、单实例、托盘菜单）都已经就位，补它是一次独立的小改动。

## 十一、恢复码与备份密钥（2026-08-12 已确认）

**恢复码的熵与版式对不上。** §8.4 写「RK 为 **128 位**随机数，编成
**10 组 5 字符** Base32」，并给了 10 组的示例。但 10×5 = 50 个 Base32 字符
能装 250 bit，只放 128 bit 会空掉一多半。

现在按 **240 bit 熵 + 10 bit 校验位 = 250 bit** 实现：

- 版式与文档完全一致，用户要抄的仍是 50 个字符；
- 熵只多不少，满足「至少 128 位」的安全要求；
- 多出的 10 bit 做校验位，抄错一位有约 99.9% 的概率被当场挡下。

**已确认按 240 bit 实现**（2026-08-12）。服务端与其他客户端要按这个来：
240 bit 熵 + 10 bit 校验位，编成 10 组 5 个 Crockford Base32 字符。

**备份密钥：`HKDF(DEK,"backup")`，已确认。** §9.7 说 `.zbk`「用 DEK 加密」，
但 §8.2 的派生清单里没有备份这一档。直接拿 DEK 当对称密钥用，等于让备份文件与
将来任何也「用 DEK」的东西共用一把钥匙。多派生一层的代价是一次 HKDF，
换的是密钥不复用。info 串定为 `zhiyan/v1/backup`，服务端将来要读 `.zbk` 时对齐它。
