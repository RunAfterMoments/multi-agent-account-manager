# 优化报告（2026-11-26）

基于 `docs/hardening-notes.md`（含第二轮复审 49 项）与 `docs/hardening-notes-*.md`
做的第二轮优化。共改 17 个文件，新增 431 行、删除 99 行。

协作方式：Google Gemini（flash-lite 模型）负责出每项的补丁设计，
小七负责审查、纠正、落地。Gemini 给出的 unified diff 多次出现行数错误、
虚构上下文、重复函数的问题，本次全部改由人工逐项精确编辑落盘，
未直接应用 Gemini 的 diff。

## 一、已修复

### Batch 1 — Cookie 导入与剪贴板（`src-tauri/src/platform.rs`）

- `write_cookie_clipboard`：写入成功后 60 秒自动清剪贴板。
  只在剪贴板内容仍等于本次写入的 Cookie 头时才清，不误删用户后来复制的内容。
- `push_validated_imported_cookie`：
  - Cookie 值含 `;` 或 `,` 时拒绝导入（之前只拦 `\r` / `\n` / NUL），
    堵住用分号注入第二个 Cookie 的口子；
  - `SameSite=None` 时强制 `secure = true`；
  - 父域 Cookie（如 Domain=hark.com 配 app.hark.com）只在父域至少两段、
    且入口/白名单 host 确实是其子域时才接受（`is_parent_domain` /
    `domain_matches`）。
- 新增测试 `parse_imported_cookies_rejects_semicolon_injection_and_forces_secure_for_samesite_none`。

### Batch 2 — 注入脚本与站点适配器

- `src-tauri/src/site_adapters.rs`（`NET_HOOK_INIT_JS`）：
  - 只在主站（muse.ai / *.muse.ai / hark.com / app.hark.com / *.hark.com，
    `initHost` / `isPrimaryHost`）且 URL 匹配聊天接口特征
    （`CHAT_API_PATTERNS` / `isChatApiUrl`）时才读响应体
    （`consumeStream`、错误片段）；其他站点的响应体不再经过这段逻辑。
  - EventSource / WebSocket 监听只挂在主站。
  - `window.__maamNet` 用 `Object.defineProperty` 密封
    （writable:false、configurable:false），页面脚本改不掉。
- `src-tauri/src/site_adapters.rs`（`merge_adapters`）：
  内置适配器的 `host_patterns` 不能再被用户覆盖放宽成 `"*"`，
  覆盖只能收紧。
  新增测试 `overrides_cannot_widen_builtin_host_patterns_to_wildcard`。
- `src-tauri/src/session_manager/scripts.rs`（`LOGIN_CAPTURE_INIT_JS`）：
  - `window.__maamAuthCheck` 同样密封。
  - 硬编码的 server action id `40e0249e…` 删除，改为
    `window.__maamSubActionId || ''`——扫不到 id 就不调未知 action，
    只用已缓存的额度。Android / iOS 探针脚本里的同一硬编码 id
    也已同步删除（见 Batch 3）。

### Batch 3 — Android（`AccountSessionPlugin.kt` / `AccountWebActivity.kt`）

- 更新安装：`downloadAndInstallApk` 要求 SHA-256 必填且为 64 位小写 hex，
  缺失或格式非法时删掉已下载文件、报进度错误、拒绝安装（`return@thread`），
  不复用旧文件、不调起安装器。
  缓存复用一律重算摘要，不再凭 URL 信任旧文件；
  下载后的摘要校验改为无条件（以前 `expectedSha != null` 才比对）。
- `openHttpConnection`：重定向白名单精确 host
  （github.com、api.github.com、objects.githubusercontent.com、
  release-assets.githubusercontent.com，与 Rust `updater.rs` 的
  `is_allowed_redirect_target` 一致），初始地址和每一跳都校验。
- `copyProfileCookie`：60 秒后自动清剪贴板（`Handler.postDelayed`），
  同样只在内容仍是本次 Cookie 时才清。
- `AccountWebActivity`：
  - 第三方 Cookie 默认关闭（`setAcceptThirdPartyCookies(wv, false)`）；
  - 混合内容一律禁止（`MIXED_CONTENT_NEVER_ALLOW`）；
  - `handleWebPermissionRequest` 一律 `deny()`，
    受保护媒体 ID（可追踪的设备标识）不再自动放行；
  - `startHttpDownload` 只允许 HTTPS（初始地址与重定向，禁止 https→http 降级），
    并加 512MB 上限（`contentLengthLong` 前置检查 +
    `copyStreamWithCheck` 新增 `maxBytes` 参数，逐块累计兜底）。

### Batch 3 — iOS（`AccountSessionPlugin.swift` / `AccountWebViewController.swift`）

- `copyProfileCookie`：剪贴板加 `.expirationDate` 60 秒（保留 `.localOnly`）；
  Cookie 路径校验与 macOS 对齐（只复制路径适用于入口地址的 Cookie）。
- `resolveTargetUrl`：不再从 Set-Cookie 头解析 `Domain=` 当目标 URL，
  一律用 Rust 侧已校验的入口 URL——头部自己的 Domain 不能给自己背书。
- `buildHttpConnectProxyConfig`：含 userinfo（`user:pass@host`）时拒绝；
  端口必须显式给出且在 1…65535 内，不再默认 80。
  解析失败时显式清空旧代理配置，并通过 `emitSessionFailed` 报错
  （"代理配置无效，已清除旧代理配置"），不再静默沿用。
- `javaScriptCanOpenWindowsAutomatically`：true → false。
- 陈旧会话不再自动 GET 登出（`https://muse.ai/api/logout` / hark 登录页）：
  探针误判会把已登录用户登出，改为只上报已登出事件，由用户手动处理。
- `decidePolicyFor`：`blob:` 无条件放行只限主框；
  子框只允许 https 或带 host 的 `blob:`，且 host 必须在白名单内，
  或是验证码 / widget 域名（hcaptcha.com、challenges.cloudflare.com、
  www.gstatic.com、www.google.com，`isSubframeAllowedHost`）；
  http 与其他 scheme 的子框一律拒绝。

### Batch 4 — Rust 杂项、前端、NSIS、CI

- 锁中毒不再直接 panic：11 处已审计的 `.lock().unwrap()` 全部改为
  `.lock().unwrap_or_else(|e| e.into_inner())`
  （`platform.rs`、`session_manager.rs`、`session_manager/group_chat.rs`、
  `session_manager/quota_sync.rs`、`session_manager/webview.rs`；
  `key_vault.rs:47` 原本就是这个写法）。
- Vault 单例（P1-7 后续项）：`SessionManager` 持有唯一的
  `ApiKeyVault` 实例（`session_manager.rs:225`，在 `new` 里初始化），
  `group_chat.rs` 的 upsert / 回滚 / 删除四处不再每次新建实例，
  keyring 可用性探测（一次 D-Bus 往返）只做一次。
- P0-1 前端接线补齐：`src/types.ts` 的 `ManagerSnapshot` 增加
  `persistBlocked` / `readOnly`；`src/render.ts` 的提示条在
  `persistBlocked` 时显示「继续使用空配置」「从备份恢复」按钮；
  `src/main.ts` 接到已有的 Rust 命令
  `continue_with_empty_config` / `restore_config_from_backup`；
  `src/styles.css` 增加 `.notice-actions` / `.notice-action-btn`。
- 审计日志轮转：`group_chat.rs` 的 `append_auto_approve_audit`
  在 `auto-approve-audit.log` 超过 1MB 时轮转到 `.log.1`（只保留一份），
  轮转失败不影响主流程。
- NSIS（`src-tauri/nsis/installer.nsi`）：
  - 编译期 guard：BUNDLEID 为空时 `!error`，避免静默装错位置；
  - PageReinstall 分支：旧安装路径下没有 `uninstall.exe` 或
    `${MAINBINARYNAME}.exe` 时视为无效记录，不执行旧卸载器，直接全新安装。
- CI（`.github/workflows/release.yml`）：21 个 `uses:` 全部按
  `docs/hardening-notes-ci.md` 的表 pin 到 commit SHA
  （checkout、setup-node、rust-toolchain、rust-cache、upload-artifact、
  setup-java、download-artifact），YAML 解析校验通过。

## 二、验证结果

| 检查 | 结果 |
| --- | --- |
| `rustfmt --check`（全部改动的 Rust 文件） | 通过 |
| 注入 JS 语法（`NET_HOOK_INIT_JS`、`LOGIN_CAPTURE_INIT_JS`，`node --check`） | 通过 |
| 硬编码 action id `40e0249e…` 全仓 grep | 无残留 |
| `npx tsc --noEmit` | 通过 |
| `npm run build`（tsc + vite） | 通过 |
| `npm run test:js` | 15/15 通过 |
| `cargo test` / `cargo check`（Rust 后端） | **未跑通，见下** |
| Kotlin / Swift 编译 | **本机无工具链，未编译，见下** |

### 未能验证的部分（诚实说明）

1. **Rust 后端编译/测试没跑起来。** 本机缺少 Tauri Linux 系统依赖
   （libwebkit2gtk-4.1-dev、libgtk-3-dev、libayatana-appindicator3-dev、
   librsvg2-dev、libsoup-3.0-dev），`cargo test` 在 `gdk-3.0` 的
   pkg-config 阶段失败。Rust 工具链本身已装好（cargo / rustc 1.100.0）。
   曾尝试 `apt-get install` 这些库，但安装需要用户在审批卡上确认，
   审批超时未执行——**本次没有擅自重试**。要完整验证 Rust 改动，
   请在有桌面依赖的机器（或批准 apt 安装后）跑
   `cargo test --lib` 与 `cargo clippy`。
   作为替代，本次做了 `rustfmt --check`（全部改动文件通过，
   rustfmt 会完整解析语法）、注入 JS 的 `node --check`、
   以及逐项 diff 人工复核。
2. **Kotlin / Swift 没有编译。** 本机无 Android SDK / Xcode，
   移动端改动只做了 diff 人工复核，未经编译器校验。
   建议在 `tauri android` / Xcode 环境里编译一次再发版。
3. **预先存在的 clippy 基线**（lib 约 19 项、test 约 20 项）未动，
   本次改动未引入新的 clippy 类别；全仓 `cargo fmt` 重排版也未做。

## 三、明确未做的项

- P1-8 增量写入（群消息改 jsonl / SQLite）：只有设计，未实现。
- NSIS 的 WiX `UninstallString` 强化、WebView2 bootstrapper
  哈希/签名：需要 embedBootstrapper 二进制或上游模板改动，未做。
- Android FileProvider 声明：待真机 / `tauri android init` 验证，未改。
- 时区覆盖脚本只限主框（P1）：需真机验证码测试，未做。
- iOS 下载功能缺口：功能缺口而非安全修复，未做。
- 审计报告里标注"待真机测试"的 5 项：本次未做真机测试。

## 五、复审修复（v4，2026-11-26 晚）

v3 压缩包经复审（综合 6.0/10，NO-GO）抓出 7 个问题，已逐项对照代码核实
并全部修复。10 个文件，+214/-40 行。

1. **iOS 代理无效时直连（高危）**：`AccountWebViewController.swift`
   的代理失败分支补 `dismiss + return`，不再无代理加载入口页。
   顺序：先 `emitSessionFailed`（sendEvent 要求 activeSpec 仍在），
   再 `invalidateAndClose(spec.switchSeq)`（否则 `deleteProfile`
   会因 `isUuidActive` 卡住），最后关闭。
2. **Cookie 导入残余注入**（`platform.rs`）：
   - `Path` 加校验：含 `;` 或控制字符直接丢弃该条；
   - `Domain` 加 DNS 字符校验（仅 `[a-z0-9.-]`、标签非空、首尾非 `./-`）；
   - `is_parent_domain` 改为显式白名单 `hark.com`、`muse.ai`，
     `Domain=workers.dev` 不再放行（白名单里有
     `meta-agents-apps.workers.dev` 也不得种整个公共后缀）；
   - 值放宽 `,`：`,` 是合法值字符，Chromium/Android/iOS 解析器只以 `;`
     分隔属性，桌面端还走 `set_value()`；测试同步更新，并新增
     Path/Domain 注入与 workers.dev 拒绝的断言。
3. **iOS blob 校验死代码**：新增 `blobInnerUrl`（剥掉 `"blob:"` 前缀再
   解析内层 URL——Foundation 解析 `blob:https://` 时 `url.host` 为 nil）。
   主框 `blob:https://` 仅内层 host 在白名单内才放行，
   `blob:null/` 保持放行；子框 blob 必须有内层 host 且在白名单/
   验证码域名内，`blob:null/` 子框拒绝。
4. **前端 persistBlocked 横幅**：恢复按钮移出可关闭的提示条，改成独立的
   不可关闭横幅（`persist-blocked-bar`），点 ✕ 不再让按钮消失；
   「继续使用空配置」加 `window.confirm` 二次确认；
   `store.rs` 的 `restore_config_from_backup` 改为先复制候选到同目录
   临时文件 `accounts.json.restore-tmp` 校验，通过才原子 rename 覆盖，
   全部失败时不再让坏候选覆盖 `store_path`。
5. **漏改的 2 处 `.lock().unwrap()`**：`session_manager.rs` 的
   `reveal_downloaded_file` 与 `handle_desktop_download_event`
   （FFI 回调内）全部改为 `unwrap_or_else(|e| e.into_inner())`；
   全仓 `src-tauri/src` 已无 `.lock().unwrap()`。
6. **`merge_adapters` 通配绕过**：所有 override 先去掉 `"*"`；
   覆盖内置 id 去掉后为空则保留内置 hostPatterns；新增 id 去掉后
   为空则整条丢弃。新增两个测试（纯通配新增被丢弃、混合通配只保留非通配）。
7. **Android**：
   - `decideMainFrameOverride` 对 http/https 先验 `hostMatchesAllowed`
     再谈下载拦截，聊天内容里的外站链接不再被当作下载放行；
     blob:/data: 保持现有流程；合法附件不受影响（CDN 域名已在白名单内）；
   - `DOWNLOAD_HINT_EXTENSIONS` 去掉 `"apk"`；
   - `importProfileCookies` 改用 `getOrCreateProfile`（已用 webkit 1.18.0
     的 sources jar 核实该 API 存在）：新账号从未打开过时 profile
     还不存在，之前 `getProfile` 返回 null 导致导入直接失败。

   补充说明：Gemini 本轮给出的方案有 3 处事实错误，已纠正未采用——
   代理分支先 invalidate 再 emit 会把事件屏蔽掉；`getOrCreateProfile`
   经 sources jar 核实是存在的（Gemini 称不存在）；Cookie/前端/
   merge_adapters 几处方案用了不存在的 API 与返回类型。
   第三方 Cookie 关闭后 Muse SSO / Turnstile / hCaptcha 登录仍需真机实测。

验证：`rustfmt --check` 通过；`npm run build` 通过；`npm run test:js`
15/15 通过。Rust `cargo test` 与 Kotlin/Swift 编译仍未跑通
（原因同第二节：本机缺系统依赖与移动端工具链），需走 CI 或有依赖的机器。

## 七、P0 攻坚 PR-A + PR-B（v5，2026-11-26 晚）

### PR-A — scripts.rs 注入（MAAM-H-02）

`src-tauri/src/session_manager/scripts.rs` 里 8 个 `build_*_js`
原来用链式 `.replace("__X__", &json)` 把 JSON 文本替换进 JS 模板：
先替换的值里若含有后一个占位符（如文件名含 `__CONTENT__`、
内容含引号/分号），会被后一次 replace 展开成可执行的 JS，
实测可逃出字符串（文件名 `a__CONTENT__b"; evil();//` 即可）。
全部改为 IIFE 实参传入：模板写成 `((adapter, fileName, ...) => {...})(json, ...)`，
值只以 JSON 编码出现在调用参数里，不再经过任何文本替换。
涉及的 8 个 builder：`build_new_chat_click_js`、`build_group_attach_file_js`、
`build_group_check_attached_js`、`build_hark_click_preview_card_js`、
`build_hark_read_expanded_js`、`build_group_send_js`、`build_group_poll_js`、
`build_group_sync_latest_js`（复审说 5 处，实际清点是 8 处）。
`scripts.rs` 里已无任何 `__X__` 占位符。

新增 4 个 Rust 单测（`scripts.rs` 末尾）：文件名/内容/提示词/
适配器选择器里带占位符与注入载荷时，输出必须原样包含各值的
JSON 编码。验证方式：`rustfmt --check` 通过；另写 node 验证脚本
（`gemini/verify-pr-a.py`）把 8 个模板抽出来，喂对抗性参数
（含占位符、引号、分号、`//`）在 DOM stub 下实跑，8/8 通过，
返回的都是正常 JSON，注入的 `alert()` 一次也没执行。
说明：这次是机械重写，我直接做的，没走 Gemini 出 diff
（它前两轮在精确代码上已出过 3 次事实错误），改用实跑验证兜底。

### PR-B — API Key 跨成员泄露 + 白名单（MAAM-H-06 前端 + MAAM-H-08）

- `src/main.ts`：切成员类型（`[data-ext-kind]`）与切预设
  （`[data-ext-preset-idx]`）时清空 `apiKeyDraft`。之前给 A 成员
  输的 Key 会留在草稿里，跟着保存进 B 成员。
- `src/modals.ts:653`：Key 输入框去掉 HTML `value="..."`。
  编辑打开时草稿本来就是空的（留空表示不修改），输入过程由
  `input` 事件同步草稿、不经过重新渲染，去掉后不影响输入；
  代价是保存失败重渲染后要重输 Key（可接受）。
- `src-tauri/src/providers.rs`：`EMBEDDED_WIDGET_HOSTS` 去掉
  `"firebaseapp.com"`（Firebase Hosting 公共后缀，任何人可托管
  任意内容，不能当可信 widget 域名）。代码与文档里无任何合法依赖。
- `src-tauri/src/session_manager/group_chat.rs`：绑定的 `thread_url`
  与适配器的 `new_chat_url` 跳转，从 `classify_top_level_url` 的
  `AllowHttps`（放行任意 allowed_host：CDN、facebook.com 等）
  收紧为 `is_verified_callback_origin`（只认本站 callback_origins：
  muse.ai、www.muse.ai、app.hark.com、hark.com）。
  `providers.rs` 的拒绝测试已扩展，覆盖 metaaiusercontent.com /
  facebook.com / appleid.apple.com。记录绑定时校验当前页的
  `classify_top_level_url` 不动。

验证：`rustfmt --check` 通过；`npm run build` 通过；
`npm run test:js` 15/15 通过。Rust `cargo test` 仍未跑通
（本机缺系统依赖，见第二节），Kotlin/Swift 不涉及本次改动。

待继续：PR-C（OpenAI API SSRF/重定向、auto_approve 熔断）、
PR-D（A2A/LocalCli 提权与隔离）、PR-E（updater 防降级、CI、Command 校验、Android 卡死）。

## 六、发版前真机手动测试清单

1. Android 更新：去掉 SHA-256 调用一次，应拒绝安装并删文件；
   篡改已下载 APK 后重进，应重新下载而非复用。
2. Android / iOS：复制 Cookie 后等 60 秒，剪贴板应清空；
   60 秒内自己复制别的内容，不应被清空。
3. Cookie 导入：值含 `;` 的导入应被拒绝；`SameSite=None` 的应强制 secure；
   Domain=hark.com 的 Cookie 在 app.hark.com 入口应可用，在无关域名应拒绝。
4. iOS：代理 URL 带 `user:pass@` 或缺端口时，应报错并清空旧代理；
   子框打开 hcaptcha / Cloudflare 挑战页应正常，子框打开其他外站应被拦。
5. iOS：陈旧会话只应提示已登出，不应自动跳登出页。
6. 前端：配置写盘被锁时，提示条应出现两个按钮，点「从备份恢复」应恢复，
   点「继续使用空配置」应继续。
7. 桌面端：`cargo test --lib` 与 `cargo clippy` 在有依赖的机器上跑通；
   NSIS 安装包在干净 Windows 虚拟机上装一次，确认重装分支不调旧卸载器。
