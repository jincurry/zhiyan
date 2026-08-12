# 发布（§11）

这份文档记的是**只有发布时才做、做错了又很难发现**的那些步骤。

---

## 一、构建

```sh
npm ci
ZHIYAN_FONT_SRC=/path/to/fonts npm run subset-fonts   # §3.1，不做的话中文用回落字体
node scripts/gen-icons.mjs                            # 缺 icon.ico 会直接构建失败
npm run tauri build -- --features sqlcipher
```

**`--features sqlcipher` 不能省。** 不开的话 `PRAGMA key` 会被普通 SQLite
静默忽略，库照开、数据照写，只是全是明文（§9.2）。这件事在界面上能看见：
设置页的「本地库」一行会显示「未加密」——那是运行时探测出来的，不是猜的。

发布前请在真机上开一次设置页确认它显示「已加密」。

---

## 二、WebView2 运行时（§4.2）

配的是 **Evergreen Bootstrapper**（`webviewInstallMode: downloadBootstrapper`，
约 2MB，缺失时联网安装）。不用 Fixed Version：+130MB，而且安全更新要自己跟。

覆盖率：Win11 内置；Win10 自 2021 年随 Edge 推送，实测 > 97%。

安装器会检测运行时存在性。**离线且缺失时必须给出可操作提示，不能启动后白屏**——
白屏是最难报告的故障，用户只会说「打不开」。

应用内还有第二道：设置页读注册表里的运行时版本，同时前端做特性探测
（`color-mix()` / `:has()` / `aspect-ratio` / `backdrop-filter` / 正则后行断言）。
两道互补——前者知道装了哪个版本，后者知道那些特性真的能不能用。

---

## 三、代码签名（§11.2）——有真实成本

**SmartScreen 会拦截未签名安装包**，弹「Windows 已保护你的电脑」。
对转化率的杀伤极大，而且用户看到那个弹窗后多半就不装了。

| 方案 | 年成本 | SmartScreen |
|---|---|---|
| 不签名 | 0 | 一直拦截 |
| OV 证书 | ¥2000–4000 | 需累积下载量建立信誉，数周至数月 |
| EV 证书 | ¥4000–6000 | 签完即时通过 |
| Microsoft Store (MSIX) | 0（一次性注册费） | 商店身份自带信任 |

**2023 年 6 月起 OV/EV 私钥强制硬件令牌或云 HSM。**
CI 自动签名要配云签名服务（Azure Trusted Signing、DigiCert KeyLocker）——
**不能再把 `.pfx` 放进 CI secrets**，那条路已经走不通了。

推荐组合：NSIS + OV 为主渠道，同时上架 Store 作为「安全下载」背书。

### MSIX 的两个限制

- `%APPDATA%` 会**重定向到应用容器**。§9.1 那套目录布局要兼容两种路径，
  否则商店版与官网版的数据互相看不见。
- Store 版不能用自建 updater。

---

## 四、自动更新（§11.3）

`tauri-plugin-updater` 有独立的 Ed25519 签名机制，与代码签名证书无关，
**不额外花钱**。

### 生成密钥

```sh
npm run tauri signer generate -- -w ~/.tauri/zhiyan.key
```

- 公钥写进 `tauri.conf.json` 的 `plugins.updater.pubkey`
- 私钥**绝不入库**。CI 里走 `TAURI_SIGNING_PRIVATE_KEY` 与
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 两个 secret

### 仓库里的 `plugins.updater` 为什么是空的

```json
"plugins": { "updater": { "endpoints": [], "pubkey": "" } }
```

两个都不能省，也都不能填假的：

- **整段省掉应用起不来。** 插件初始化时读不到配置会直接 panic——
  这一条是跑起来才发现的，`cargo build` 一点问题都没有。
- **填占位公钥比空着更糟。** 它看起来像配好了，实际谁都验不过，
  而失败信息是「签名不匹配」，排查方向会完全跑偏。

空的 `endpoints` 是明确的「没配」（`check_update` 会返回 `configured: false`，
界面显示「此构建不检查更新」而不是「已是最新」）；空的 `pubkey` 验什么都失败，
是 fail closed。

发布时把这一段替换掉：

```json
"plugins": {
  "updater": {
    "endpoints": ["https://<你的 CDN>/zhiyan/{{target}}/{{arch}}/{{current_version}}"],
    "pubkey": "<tauri signer generate 出来的公钥>"
  }
}
```

### 策略

- 启动时检查 + 每 6 小时后台检查
- 下载完成后提示「重启以更新」，**不强制立即重启**——正在写东西的人被强制
  重启会丢草稿，而更新本身没那么急
- 灰度：JSON 里带 `rollout` 百分比，客户端按设备 ID 哈希判定
- **必须做**：更新失败回滚。保留上一版安装包，检测到新版本连续三次启动崩溃时提示回滚

---

## 五、安装器写快捷方式的 AppUserModelID（§5.5）

Toast 通知**必须先注册 AUMID**，否则根本不显示——这是这一块最常见的踩坑点。

进程侧已经在 `platform_win::init_process` 里调了
`SetCurrentProcessExplicitAppUserModelID("Zhiyan.Desktop")`。

**安装器创建的开始菜单快捷方式必须写入相同的 `System.AppUserModel.ID`。**
两边不一致的表现是：通知在开发时（直接跑 exe）正常，装完之后就没了。

MSIX 打包时由清单自动提供，不用手写。

---

## 六、发布前的自查

- [ ] 设置页「本地库」显示**已加密**
- [ ] 字体子集已生成（否则中文走回落字体，行高和字重都不对）
- [ ] 全局热键在装了搜狗输入法的机器上试一次；被占用时应弹一次通知并在设置页标红
- [ ] 悬停最大化按钮 1 秒，Win11 上应弹出贴靠布局面板
- [ ] 按热键从别的应用切过来，浮窗应**抢到焦点并且光标在输入框里**（§5.3 ①）
- [ ] 拔掉外接屏后重启，主窗口应回到主屏而不是消失（§5.1）
- [ ] 装完之后从开始菜单启动，发一条通知确认能弹出来（AUMID 对不对）
- [ ] `%APPDATA%\Zhiyan\` 下没有 `dev-dek.key`——有的话说明这个构建是 debug 的
