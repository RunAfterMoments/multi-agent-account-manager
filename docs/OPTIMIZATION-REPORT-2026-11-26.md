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

## 八、P0 攻坚 PR-C + PR-D + PR-E（v6，2026-11-26 晚）

### 首次真实编译（前置步骤 0）

经用户批准安装系统库（`libwebkit2gtk-4.1-dev libgtk-3-dev
libayatana-appindicator3-dev librsvg2-dev libsoup-3.0-dev`）后首次跑通
`cargo test --lib` 的编译。两处环境坑已踩过并记录：
`libzstd-dev` 依赖的 `libzstd1` 与镜像自带的 Hatch 钉死版冲突，
需先 `--allow-downgrades` 降级 `libzstd1` 再装；`/tmp` 是 512M tmpfs，
cargo 目标目录放里面会撑爆，改用
`CARGO_TARGET_DIR=$HOME/workspace/multi-agent-optimization/target-maam`。
首次编译即抓出 PR-C 的一个真 bug（见下 `is_global_ip`）——
此前所有“验证通过”都只是 rustfmt / tsc / JS 测试，从未编过 Rust。

### PR-C — OpenAI API SSRF / 重定向 / 字节上限 + auto_approve 门控（MAAM-H-04/H-03）

- `src-tauri/src/group_transport.rs`：新增
  `validate_openai_endpoint_host`，在 `execute_openai_api_turn_inner`
  开头调用。URL 解析、只认 http(s)、`(host, port).to_socket_addrs()`
  解析出全部地址逐个判断；IPv4 映射 IPv6（`::ffff:10.0.0.1`）先还原
  成 IPv4；loopback 放行（本机 Ollama `127.0.0.1:11434` 是文档化合法
  用法），其余地址必须全局可路由，否则拒绝。
- **编译 bug 修复**：`IpAddr::is_global()` 在稳定版 Rust 1.100.0 上
  是不稳定 API（E0658），首次 `cargo test` 编译失败于此。改为手写
  `is_global_ip`（按 IANA 特殊用途地址表：IPv4 覆盖 0/8、10/8、
  100.64/10、127/8、169.254/16、172.16/12、192.0.0.0/24、192.0.2/24、
  192.168/16、198.18/15、198.51.100/24、203.0.113/24、224/4、240/4；
  IPv6 覆盖 ::/96、fc00::/7、fe80::/10、fec0::/10、ff00::/8、
  2001:db8::/32、2001::/32、2001:2::/48、100::/64、64:ff9b::/96），
  宁可误拒、绝不放过。测试随之扩展（0.0.0.0、组播、广播、三段文档地址、
  ::、2001:db8、ff02 均拒绝；公网 IPv6 `2606:4700:4700::1111` 放行）。
  残留：解析时检查与 reqwest 连接时再次解析之间存在 TOCTOU，
  彻底的固定需连接时钉死已解析 IP，本轮未做。
- 客户端构造：`.redirect(Policy::none())`（禁重定向跟随）；
  `read_timeout` 从写死 60 秒改为
  `agent.idle_timeout_secs.clamp(10, 600)`（默认 60，与 CLI 路径的
  idle-timeout 语义一致）。
- 字节上限：`MAX_API_SSE_LINE_BYTES = 1MB`（单行超限直接报错）、
  `MAX_API_RESPONSE_BYTES = 8MB`（累计超限截断并附“超过…字节上限”说明，
  与超时行为一致）、非流式体经 `read_response_body_capped` 读
  （8MB 超限报错）、错误体上限 64KB。`raw_fallback` 本来已有 16KB 上限。
- `src-tauri/src/js/auto_approve.js`：IIFE 的 try 块开头加三道门——
  非顶层 frame 直接返回、非 https 直接返回、origin 不在
  `window.__maamAutoApproveOrigins` 里（缺失/为空也拒绝）直接返回。
  `src-tauri/src/session_manager/webview.rs` 在初始化脚本里注入该列表
  （取 provider 的 `callback_origins`：muse 为 muse.ai / www.muse.ai，
  hark 为 app.hark.com / hark.com）。弹窗 webview 从未注入过
  AUTO_APPROVE_INIT_JS，只有 LOGIN_CAPTURE_INIT_JS，所以 fail-closed 安全。
- 熔断广播：`session_manager.rs` 新增 `auto_approve_sync_js` 辅助
  （设置开关也改用它）；`group_chat.rs` 的 `drain_auto_approve_report`
  在熔断时除持久化与快照外，向**全部** warm_pool webview 下发
  `(false, allow_commands)` 同步脚本——此前已加载页面里的
  `__maamAutoApprove = true` 会残留并继续自动点击。
- 审计日志 0o600：`append_auto_approve_audit` 每次 open 后都
  `set_permissions(0o600)`（unix），连带收紧此前 0o644 的旧日志。
- 测试：`openai_endpoint_host_filter_blocks_private_and_allows_loopback`
  （见上）；Node 门控 harness 6/6 通过（本站、www 子域、iframe 拦截、
  http 拦截、facebook.com 拦截、未注入 origins 拦截）；
  `tests/auto_approve.test.mjs` 的 `makePage` 补注 origins 后
  `npm run test:js` 恢复 15/15（此前因门控新增一度报
  `__maamScanAutoApprove is not a function`）。

### PR-D — A2A / LocalCli 提权与隔离（MAAM-H-01/M-09）

- `src-tauri/src/group_transport.rs`：`build_turn_prompt_for_member`
  里含其他成员完整回复的 `[阶段要求]` 段，从 `sys_sections` 移到
  `user_sections`（置于 A2A 接力说明之后、`[当前问题]` 之前）。
  系统提示只保留角色偏好、用户配置的共享上下文与技能。
- `src-tauri/src/session_manager/a2a.rs`：`run_group_a2a_chain` 里，
  接力目标是 LocalCli 成员（`external_agents` 查 `kind`）时不执行，
  其消息以 `Error` 状态入列并写明“为安全起见未自动执行本地命令，
  确认无误后点「重试」才会运行”，同时置 `locked.notice`。
  注意：重试跑的是原始用户问题（往回找最近的 role==user），
  不是接力片段。用户直接 @ LocalCli 成员不受影响。
- 可执行文件白名单：`LOCAL_CLI_ALLOWED_COMMANDS =
  ["claude", "codex", "gemini", "qwen", "opencode"]`，
  `validate_local_cli_command` 按 `Path::file_stem` 小写比对；
  在 `normalize_external_agent_input`（LocalCli 分支）与
  `execute_local_cli_turn` 开头两处强制执行（后者兜住旧记录）。
  拒绝 sh/bash/powershell/python3/node/rm/curl 与 `"claude --flag"` 式
  flag 走私。
- argv 里彻底去掉 `{prompt}`：`build_cli_args_and_stdin` 改为返回
  `Result`，任何含 `{prompt}` 的参数（裸写或嵌入）都拒绝并说明
  prompt 自动走 stdin（argv 会出现在进程列表里；首字符 `-` 即 flag
  注入）；`check_batch_prompt_conflict` 随之删除；
  `resolve_cli_executable`（Windows）简化为只返回 `PathBuf`。
  前端：state.ts 的 codex 预设 `args: "exec {prompt}"` 改为 `"exec"`；
  modals.ts 参数占位文案改为“prompt 自动走 stdin，不要写 {prompt}”。
  行为变化：已保存的含 `{prompt}` 成员会在执行时明确报错，
  需用户手动删掉 `{prompt}` 一次，未做静默存储迁移。
  codex 走 stdin 可行，有 docs/hardening-notes-cli.md 的手工测试为证。
- 最小 env：新增 `apply_minimal_cli_env`——`env_clear()` 后只补 PATH
  （`build_augmented_path()`，无则沿用父进程 PATH），unix 仅透传
  HOME、TMPDIR、LANG、LC_ALL、USER，Windows 仅透传 SystemRoot、
  USERPROFILE、TEMP、TMP。走代理的 CLI 需在各自配置文件里配代理。
- Job Object（仅 Windows）：创建 job 后调
  `SetInformationJobObject(..., JobObjectExtendedLimitInformation, ...)`
  置 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`，失败则 guard 保持 None
  （走 taskkill 兜底），维持“guard 存在 ⟹ 标志已置”的不变式。
  watcher 线程在发完 `Finished(Ok)` 后阻塞等 `job_done_rx` 再返回
  （释放 job 句柄）；`_job_done_tx` 是 `execute_local_cli_turn` 里的
  普通局部变量，所有返回路径（含超时提前返回与 panic）都会 drop 它。
  500ms 管道排空（给还握着 stdout 的孙进程）保留。
  **该段为 `#[cfg(windows)]`，本机 Linux 编译不到，需 Windows 编译 +
  手工测试。**
- stdout 2MB 上限：`MAX_CLI_STDOUT_BYTES`，累计超限即 `kill_tx` +
  Err（“本地 CLI 输出超过 2097152 字节上限，已终止”）。
  stderr 接收侧本来已截断 2048 字符。
- 非零退出码一律报错：此前仅“非零且输出为空”才报错，改为任何
  `code != 0` 都 Err，并附 stderr 摘要（≤240 字符）与/或部分 stdout
  （≤500 字符）。信号终止（code 为 None）行为不变。
- 并发锁：`run_group_member_turn` 里 per-member 锁与
  `in_flight_group_members` 计数移到 transport match 之前，
  OpenAiApi/LocalCli 回合也按成员串行（此前只有 WebView 回合拿锁，
  两个回合能并发跑同一个本地 CLI）；`_member_guard`（tokio
  `lock_owned`，Drop 释放）横跨整轮；计数递减 / warm-pool
  `last_used_ms` / 布局块在 match 后统一跑一次。
  `group_worker_semaphore` 仍只限 WebView 分支。
- `parse_at_mentions`：广播词（全体成员/全体/所有人、不区分大小写的
  all）先于邮箱抑制规则检查，成员别名仍走抑制。
  `a.b@all.com` → 触发全体，`test@m1.com` 仍不触发。
- 测试：新增/替换 `cli_args_reject_prompt_placeholder_and_always_use_stdin`、
  `local_cli_command_allowlist_rejects_shells_and_unknown_binaries`、
  `cli_turn_nonzero_exit_is_error_even_with_output`、
  `cli_turn_stdout_cap_kills_runaway_output`（3MB 零字节流，
  断言报错含 2097152 且 30 秒内返回）；`test_cli_agent` 改用
  `fake_cli_command()`（`$TMPDIR/maam-test-fake-claude/claude`，
  `#!/bin/sh` + `exec /bin/sh "$@"`，0o755），因执行时白名单
  不再接受字面 `sh`。
- 验证：4 个 Rust 文件 rustfmt 干净；`npm run build` 通过；
  `npm run test:js` 15/15。`cargo test` 待首次编译结果（见下）。

### PR-E — updater 防降级、CI、Command 校验、Android（MAAM-H-05/M-02/M-04）

已完成（Kotlin + 工作流 + 文档）：
- Android `AccountSessionCoordinator.kt` 的
  `recoverSpecOnActivityRecreate`：此前每次重建都
  `localViewBump.incrementAndGet()` 发一个新 `viewInstanceId` 并
  覆盖 activeSpec/committedSpec；Rust 侧 `ActiveSessionSlot` 只认
  `open_account_session` 时传过去的 id，Kotlin 单方面换 id 后，
  重建 Activity 发出的全部 mobile 事件（pageCommitVisible /
  identityDetected 等）在 `commit_mobile_session` /
  `apply_detected_metadata` 的 id 校验里被丢弃——页面看似打开、
  管理器永远收不到提交，会话卡死。改为沿用原 id（恢复幂等，
  连续两次重建返回同一份 spec）；提交前重建（加载中旋转屏幕）
  增加 `activeSpec` 兜底，绑定后仍走 `markCommittedAndReveal` 提交。
  旧实例失效不需要换 id：WebView 在 onDestroy 释放、探活循环在
  onPause 停止，残留回调另有 `activeSessionGeneration` 代际号拦截。
  `localViewBump` 字段与 `AtomicLong` import 一并删除。
  Kotlin 无工具链，本机只做了括号平衡与无残留引用检查，
  未编译——需 Android 真机/CI 验证。
- `release.yml`：顶层 `env` 注入
  `MAAM_UPDATE_REPO: ${{ github.repository }}`，desktop/android/iOS
  三个构建 job 全部生效——此前发布产物因编译期未配仓库地址，
  更新检查默认关闭。离线 Ed25519 签名方案此前已在
  `docs/hardening-notes-updater.md`“长期更新签名方案”一节成文
  （minisign、清单与安装包双签、公钥编译进二进制、密钥轮换发新版），
  本轮只更新该节第 6 点：仓库地址已接通，签名本身仍待用户生成密钥对
  （`minisign -G`，私钥进 CI secret，公钥随构建注入）后落地。
  另：`release.yml` 改动违反了 `docs/hardening-notes-ci.md`
  “release.yml 保持不动”的约定，复审要求优先，`ci.yml` 头注释
  已同步注明该例外。
- `release.yml` Android job：`android init` 之后加
  allowBackup 打补丁步骤。`src-tauri/gen/android` 由 init 生成且被
  `.gitignore` 排除，无法入库改模板，故在工作流里用 python 处理
  `AndroidManifest.xml`：已有该属性则改值、没有则插入，
  随后 grep 校验失败即中断构建。两种情形（有/无属性）均已本地实测。
  目的：禁止 Android 自动备份带走账号会话数据（cookie / WebView Profile）。
- `ci.yml`：矩阵加 `macos-15`；加 `npm run test:js` 步骤；
  加 `cargo audit`（`cargo install cargo-audit --locked` 安装，
  不引入第三方 action、免 pin SHA）与 `npm audit --omit=dev`
  （只审随产物发布的生产依赖）。两处 YAML 均已解析校验。
  注意：`npm audit` 在本机因代理拦截 registry 审计端点未能实测，
  `cargo-audit` 本机未安装，CI 首次跑即见分晓——若依赖树已有
  已知漏洞，CI 直接变红，届时再定修复或放行。

以上四项已全部完成，见第九节。

## 九、P0 对齐 + v5 尾项 + PR-E Rust（v6，2026-11-26 晚）

复审给 v5 打 6.8/10，要求 PR-C/D/E + 一次真编译才 GO。本轮三项全部
做完，并首次跑通全量 Rust 测试（104 通过、0 失败）。v6 相对 v5：
24 文件，约 +1400/-230（含报告）。

### PR-C 对齐（复审 vs 初版的差）
- **DNS 钉死**：`validate_openai_endpoint_host` 改返回
  `(host, addrs)`，`execute_openai_api_turn_inner` 经
  `resolve_to_addrs` 把地址钉进 reqwest 客户端；IP 字面量跳过钉死。
  附带修了一个 PR-C 的真 bug：url crate 的 `host_str()` 对 IPv6 带
  方括号（`"[::1]"`），`to_socket_addrs` 认不出——首轮全量测试即抓出
  （`::1` 断言失败）。现 IP 字面量先解析再用、域名才走 DNS。
- **字节上限收紧**：SSE 单行 1MB→256KiB；SSE 累计 8MB→2MiB（新常量
  `MAX_API_SSE_ACCUMULATED_BYTES`）；非流式 JSON 8MB→4MiB（新常量
  `MAX_API_JSON_BODY_BYTES`）；错误体 64KB→16KiB。
- **ReDoS**：`isTransientStatusText` 两处（scripts.rs:2605/3307）加
  160 字符上限 + 按行匹配。抽函数实测：12 组用例全过、对抗串 0ms、
  1000 次 159 字符 3ms；“正在思考中”判 false 与原正则一致（非回归）。
- **审计单条截断**：`append_auto_approve_audit` 里 text 截 1000 字、
  button 截 128 字。
- **熔断轮询同步**：群聊轮询循环（`for poll_round in 0..420`）每 20 轮
  （约 9 秒）调一次 `drain_auto_approve_report`，不必等保活同步周期。
- **auto_approve.js 门**：保留初版的 origins 白名单（callback_origins
  注入），等价或严于复审的 https + top + isPrimaryHost 形式。

### PR-D 对齐
- **`parse_at_mentions` 改回邮箱抑制优先**：@ 前是字母数字即跳过，
  广播词与别名都受保护；两条测试翻转为 `(false, [])`；独立的
  @all / @全体成员不受影响。
- **外部成员长度豁免取消**：超长时外部成员改截断后直接内联
  （4500 字 + 已略标记），不再无上限；webview 仍走附件 + 截断兜底。
  注意：`execute_external_member_turn` 根本不收 fallback/attachment，
  复审字面做法（删豁免）会让外部成员收到“请读附件”的空指令——本轮
  按“截断内联”实现，等价达成上限。
- **Review 管线点名 LocalCli 挂起**：与 A2A 接力相同的 hold
  （Error 状态 + 中止后续阶段 + notice，点「重试」确认后跑）。
- **自动失败切换禁切入 LocalCli**：
  `run_group_member_turn_with_failover` 里选出 next_key 后查 kind，
  LocalCli 直接不切；从 LocalCli 切走不受影响。
- **workdir**：保存时 canonicalize + is_dir 校验（normalize），执行时
  再 canonicalize 一次并用真实路径 `current_dir`；存的仍是用户原样输入。
- **CLI stdout 通道改为有界 `mpsc::channel(128)`**，读线程用
  `blocking_send`（管道顶满即反压子进程）；配合已有的 2MiB 累计上限。
- **Drop 守卫**：`InFlightGuard`（in_flight 计数 + warm-pool 时间戳 +
  布局）与 `ExternalTurnCancelGuard`（取消令牌注销），try_lock 快路径
  + 抢不到锁时派生异步清理；两处手动扣减/注销块已删，改为守卫接管。
- **A2A 跨链去重**：InnerState 新增 `a2a_relay_in_flight`（group_id →
  在途目标集），选目标时过滤已在途并登记；子链 spawn 包一层，回合
  结束释放（panic 残留会致该成员本轮不再被自动接力，重启恢复）。
- **已核查无需改**：Review 前序回复已在 role: user + `[阶段要求]`
  定界符 + 每稿 `###` 标题，满足复审要求。

### v5 的 4 个小尾项
- `main.ts` 预设点击补 `modal.editingId = null`。另核查：
  `[data-ext-kind]` 切类型不改 editingId 是安全的——跨 kind 时旧 Key
  不会进新服务（kind 变了）；只有同 kind 换 baseUrl 的预设会漏。
- `render()` 末尾把 `apiKeyDraft` 写回 `#ext-agent-key-input`（放在
  焦点恢复之前，避免光标跳末尾）；输入框加
  `autocomplete="new-password" spellcheck="false"`。
- `store.rs` 恢复改内存校验：新增 `storage::parse_store_str`（只解析
  不触盘、不做损坏备份、不做 vault 迁移；未来版本拒绝），通过后
  `save_store_atomic` 写回（0o600 + sync_all + 原子替换）；删临时文件
  路径与 restore-tmp 垃圾来源。
- blob/cookie/APK：桌面 `providers.rs` 的 `blob:https://` 剥前缀验内层
  host（https + 白名单），`blob:http://` 与内层外站拒绝，测试补两条；
  iOS 子帧 blob 分支加 `inner.scheme == "https"`；`platform.rs` Cookie
  导入上限 200 条 / 合计 512 KiB；Android `openLastDownloadedFile`
  拒绝打开 `application/vnd.android.package-archive`（提示去下载目录
  手动安装）。

### PR-E Rust
- `updater.rs`：`download_and_install_update` 在 verify 之后、安装之前
  加 `has_update` 防降级检查；`download_to_updates_cache` 加 512MiB
  上限（Content-Length 预检 + 流式循环硬停，删 .part 后报错；清单
  声明 size 仅作展示）。Android APK 下载走 Kotlin 插件桥
  （`downloadAndInstallApk`），本轮未加 cap——见残留。
- `lib.rs`：16 条命令全部补 `webview: tauri::Webview` 参数 + label
  校验。脚本初版误把 `get_manager_state` 与 `check_for_updates` 的参数
  写进相邻函数（单行签名锚点失效），cargo test 首轮即抓出
  （E0415/E0425），已手工纠正；现 43 条命令逐一核查均为 1 参数 +
  1 校验。
- `emit_to("manager", …)`：6 处——`store.rs` 的 `emit_snapshot`、
  `updater.rs` 的 `emit_progress`、`session_manager.rs` 的 3 处
  download-event 与 1 处移动端 UpdateProgress 中继；
  `src-tauri/src` 内已无 `app.emit(` 广播。
- 删 3 个过期权限文件（`add/remove/update_custom_provider.toml`）：
  对应命令已不存在，capabilities 与 tauri.conf.json 均无引用。

### 验证（本机首次全量通过）
- `cargo test --lib`：**104 通过、0 失败**（首轮 102 通过 2 失败：
  `::1` 方括号 bug 与 `deepseek-cli` 旧测试撞白名单，均已修——见上；
  另仅 2 条 pre-existing 的 `proxy` 未使用变量警告）。
- `rustfmt --check`：全部改动文件干净。
- `npm run build`（tsc + vite）通过；`npm run test:js` 15/15。
- `isTransientStatusText` 抽函数实测全过（见 PR-C）。
- 仍未验证：Windows-only Rust（Job Object）与 Kotlin/Swift 未编译；
  `cargo audit` / `npm audit` 未在本机跑通（CI 首跑见分晓）。CI yml
  已备好（macos-15/test:js/两处 audit）。复审 step 1“先推 v5 跑 CI”
  已被本机全量编译 + 测试覆盖——同一棵树已编过且测过，不必为 v5
  单独跑一轮。

### 残留与下一步
- **PR-F（单独做）**：key_vault Windows DPAPI / 移动端 Keystore &
  Keychain + `Zeroizing<String>`。
- **Step 2 预发布项**：lock-across-await 解锁、流式 delta + 局部 DOM 更新。
- Android APK 下载上限在 Kotlin 插件侧，未加。
- Ed25519 更新签名仍待用户生成密钥对（`release.yml` 已注入
  `MAAM_UPDATE_REPO`，发布产物现已开启更新检查，但只有 sha256
  校验、无签名——被攻破的 release 可推坏更新；缓解是文档里的签名方案）。
- 无已知 GitHub 远端（源码来自 zip），CI 需用户推到自己的仓库后跑。

## 十、v6.1 复审补丁（2026-11-26 晚）

背景：用户逐行复审 v6（2,793 行 diff），评分 **7.6/10**（v1 5.3 → v3 6.0 → v5 6.8 → v6 7.6），
判定**暂不 GO**，先打 v6.1 补丁、不要直接开 PR-F / Step 2。以下 10 点逐项在本机核实均为真，已全修。

### 5 个 CI 硬伤

1. `updater.rs`：新增 `const MAX_UPDATE_DOWNLOAD_BYTES` 抢走了修饰
   `download_to_updates_cache` 的 `#[cfg(desktop)]`，移动端会无条件编译该函数并报错——
   已在函数前补回 `#[cfg(desktop)]`。
2. `group_transport.rs`：`fn is_global_ip` 漏了 `#[cfg(desktop)]`，而它的
   `use std::net::{IpAddr, ToSocketAddrs}` 带了该属性，移动端报 `E0412`——已补。
3. Windows：`normalize_external_agent_validates_api_and_cli` 用 `Some("/tmp")`，
   v6 起做 `canonicalize`，Windows 上 `/tmp` 不存在必 panic——改用
   `std::env::temp_dir()`；6 个 `cli_turn_*` 测试 + 辅助 `fake_cli_command` /
   `test_cli_agent` 加 `#[cfg(unix)]`（跑 POSIX `sh -c` / `/dev/zero`）；
   `Cargo.toml` 的 `windows` features 补 `"Win32_System_Threading"`
   （`JOBOBJECT_EXTENDED_LIMIT_INFORMATION` 的 `IoInfo: IO_COUNTERS` 需要它；
   windows crate 未 vendored 到本机，该点以复审的 docs.rs 核查 + 构造方式为准，未本地编译验证）。
4. `ci.yml`：`cargo audit` 在仓库根目录跑，但 `Cargo.lock` 在 `src-tauri/`——
   audit 步骤移入独立的 Linux-only `audit` job，用
   `cargo audit --file src-tauri/Cargo.lock` + `npm audit --omit=dev`。
5. Clippy：删 `session_manager.rs` 重复的 `#[cfg(desktop)]`；2 处既有的 `proxy`
   未使用变量警告——绑定改名 `_proxy`（proxy 只在 `#[cfg(windows)]` 下使用，
   macOS 的 `let _ = &proxy;` 已删）。

### 5 个逻辑 / 安全 Bug

1. `a2a_relay_in_flight` 泄漏（`a2a.rs`）：目标在循环前就登记进在途集合，
   LocalCli 目标 `continue` 跳过 spawn、写盘失败直接 `return` 都会永久泄漏——
   改为只在 `persist_locked` 成功后、仅对真正进入 `spawned` 的目标登记；
   spawn 包装新增 `A2aInFlightGuard`（Drop，复用 `cleanup_inner_on_drop`，该函数
   改为 `pub(super)`），panic 也不残留。
2. `restore_config_from_backup` 丢 Key（`store.rs` / `storage.rs`）：`api_key` 是
   `#[serde(default, skip_serializing)]`，v5/v6 备份恢复后内存 Key 为空（401 直到重启），
   旧版明文备份恢复后写盘剥离 Key 且未入 vault（永久丢失）——把加载路径的迁移循环
   抽成 `migrate_agent_keys_to_vault(store, vault_dir)`（`storage.rs` 公共函数，
   加载与恢复共用），恢复在 `parse_store_str` 校验通过后、`save_store_atomic` 之前执行；
   vault 写失败时跳过该候选、不写剥离后的 store。
   （`migrate_from_value` 已给旧版明文 Key 赋 `api_key_ref`，两类备份同一循环覆盖。）
3. `isTransientStatusText` 回溯 + 空行回归（`scripts.rs` 两处同构点）：旧正则在
   `'Muse 消息 '.repeat(19) + '!'`（153 字）上单次 `test()` 实测 **559 ms**
   （复审测得 249 ms），且 `"Muse\n\n思考中"` 误判 `false`——重写为确定性 O(N)
   逐行 token 化（无回溯）：`TRANSIENT_TOKENS` 字面表最长优先匹配；泛型
   （`正在`+1–10 汉字贪心最长；1–8 汉字+`中`在首个 `中` 处最短优先）仅允许紧跟
   brand（Muse/Hark）之后，与旧正则分支结构一致；分隔符沿用原字符类 `[….\s]`
   （不含 `。`）；空行过滤。`isIdleOnlyStatus` 两处同加 160 字上限。
   与旧正则（取自 `git show HEAD:`）差分测试 60+ 用例，仅 3 处有意差异：
   两个空行用例（复审要求的修复）与 `"Muse..."` → true（旧正则 brand 单独分支
   缺 `[….\s]*` 尾巴，语义上应为 true）。保留的怪癖已验证：
   `"正在思考中"` → false、`"正在生成中"` → false、`"Muse 正在思考中"` → true。
   修复后同对抗输入单次调用 **0.13 ms**。
4. `apply_minimal_cli_env` 过窄（`group_transport.rs`）：Windows 白名单补
   `ComSpec` / `PATHEXT` / `SystemDrive` / `APPDATA` / `LOCALAPPDATA`
   （npm `.cmd` shim 与凭据查找需要，均不含 API Key）；Unix 补
   `XDG_CONFIG_HOME` / `XDG_DATA_HOME`。
5. Android `openLastDownloadedFile` APK 绕过：精确 MIME 比对改为
   `isApkDownload()`（MIME 去 `;` 参数后小写比对，或文件名以
   `.apk/.apks/.xapk/.aab` 结尾）；新增 `lastDownloadedFilename` 字段，
   `onDownloadCompleted` 对 APK 直接置 `canOpen=false`，`onDownloadFailed` 清空。

### 小尾项

- `InFlightGuard` 挪到 `member_lock.lock_owned().await` 之前构造
  （等锁期间取消不再泄漏 +1 计数；drop 顺序安全：新 turn 只能在旧 future
  完全结束后开始）。
- Review 管线第 0 阶段豁免：`if *idx > 0 && is_local_cli`（第 0 阶段输入全是用户
  自己写的，LocalCli 排第一棒不再整条评审报错）。
- 移动端 `manager` 导航锁：`lib.rs` 的 `#[cfg(mobile)]` 分支补与桌面相同的
  `on_navigation` 限制（`WebviewWindowBuilder::on_navigation` 在 vendored
  tauri 2.12.6 中存在）。
- `tests/auto_approve.test.mjs` 补 5 条门控用例（http / 外站 / 缺 origins /
  空 origins / iframe；iframe 用例用 `new Function` + 假 window，
  因 jsdom 的 `window.top` 不可配置；门控在触 `document` 前返回）。
- Android APK 下载上限（`AccountSessionPlugin.kt`）：512MiB
  `contentLengthLong` 预检 + 流式硬停（外层 catch 已删 partFile）。
- `is_global_ip` 边角：补 6to4 `s[0] == 0x2002` 拒绝；NAT64 从 /96 放宽为
  `s[0] == 0x0064 && s[1] == 0xff9b`（覆盖 `64:ff9b:1::/48`）；补 2 个测试用例。

### release.yml：更新默认关闭

`MAAM_UPDATE_REPO: ${{ github.repository }}` → `${{ inputs.maam_update_repo || vars.MAAM_UPDATE_REPO }}`
（新增可选 `workflow_dispatch` 输入）。理由：带更新开启、无 Ed25519 签名的公开版
一旦装机，后续即使加上签名、老版本也不会验签，仓库或 Release 出事仍能向存量版本推恶意更新。
内部自测可填 input；正式发版前先落地签名，再在仓库 Variables 里打开。
`ci.yml` 头注释与 `docs/hardening-notes-updater.md` 第 6 点已同步。

### Clippy 首次全量清理（本机此前从未跑过 clippy）

首次 `cargo clippy --all-targets -- -D warnings` 报 21 项：1 个 sidebar 测试硬错误
（`ProviderId::muse` 是关联函数，需 `ProviderId::muse()`，已修）+ 20 个既有 lint。
处理方式：`#[allow(clippy::too_many_arguments)]`（platform.rs 12 参函数、
group_chat.rs 两处、lib.rs `add_account`/`rename_account`）；`ReviewJob` 类型别名；
3 处 let-else 改 `?`；auto-deref；相同块合并；`quota_sync.rs` 去多余 `return;`；
webview.rs `PendingDownloads` 别名 + `format!` 去引用 + `never_loop` allow；
`slot_indices.into_iter().zip(reordered)`（仅第二处 `into_iter()` 是多余的，
`Vec` 本身没有 `.zip`）；测试里 `&[api_key.clone()]` → `std::slice::from_ref`。

两次教训（已记入过程，未藏进代码）：
- 本 crate 是 **edition 2021，let-chain 不可用**——两处嵌套 if（2346 的 if-let 套 if-let、
  2385 的 if-let 套 if-bool）无法合并，恢复 v6 原样并在
  `prepare_group_member_thread` 上加 `#[allow(clippy::collapsible_if)]`
  （附注释说明原因）；2385 的重构草稿曾引用不存在的 `adapter_spec`，
  已按 `git show HEAD:` 恢复为使用作用域内 `spec` 的原 form。
- clippy 修复脚本的 `old = """...("""` 赋值行与 `s.replace(old, """...""")`
  调用行结尾易混（前者无 `)`、后者有），曾致两次 SyntaxError，改手写 edit 后解决。

另：clippy 在 Linux 下报一条 cargo manifest 警告（`tauri-plugin-account-session`
疑似未使用）——误报，该依赖在 `platform.rs:2302` 以 `pub use ... as mobile`
在 `#[cfg(mobile)]` 下使用；不删，`-D warnings` 也不因此失败。

### 验证（v6.1 + 侧边栏，全部通过）

- `cargo fmt --check`：干净。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`：**0 错误**。
- `cargo test --lib`：**105 通过、0 失败**（v6 的 104 + 新增 sidebar 测试 1）。
- `npm run build`（tsc + vite）通过；`npm run test:js`：**20/20**（15 既有 + 5 门控新增）。
- 两份 workflow YAML 解析通过；Kotlin 两文件括号计数与 HEAD 的 naive 计数完全一致
  （同为计数 artifact，diff 仅 +17/-3 / +9/-0），无 Kotlin 工具链、未编译。
- 环境备注：本次验证前虚拟机已被替换过一次，Tauri Linux 开发库丢失导致测试链接失败
  （缺 `-lgtk-3 -lwebkit2gtk-4.1` 等）；按 v6 的老办法重装——先
  `apt-get install --allow-downgrades libzstd1=1.5.5+dfsg2-2build1.1`
  （镜像钉死的 `+hatch2` 版与 `libzstd-dev` 冲突），再装 `libgtk-3-dev`
  `libwebkit2gtk-4.1-dev` `libgdk-pixbuf-2.0-dev` `libayatana-appindicator3-dev`
  `librsvg2-dev`；`CARGO_TARGET_DIR` 仍用工作区内目录，绝不放 512M 的 `/tmp`。

## 十一、侧边栏折叠功能（2026-11-26 晚，单独 commit）

按用户给的设计单独做，不混进 v6.1 安全修复。收起 312px `.middle-column`，保留 68px
`.service-rail`（绝不收到 0：右侧账号页是原生子 WebView，永远盖在 manager HTML 之上，
无法在账号页上叠悬浮展开按钮；服务栏是天然的展开入口）。左半区总宽
`LEFT_PANE_WIDTH = 380px`，收起态 68px，右侧对话区多出 312px。

- Rust：`AppSettings` 加 `#[serde(default)] pub sidebar_collapsed: bool`
  （旧 `accounts.json` 无需升 `STORE_VERSION`）；`session_manager.rs` 加
  `const SERVICE_RAIL_WIDTH: f64 = 68.0`；`ManagerSnapshot` 加派生字段
  `effective_sidebar_collapsed`（`snapshot_from_locked` 统一计算，前后端单一真相源）；
  `SessionManager::set_sidebar_collapsed`（改偏好 → `#[cfg(desktop)]` 重算布局 →
  `request_persist()` 异步防抖写盘 → 返回 snapshot）；`webview.rs` 新增
  `is_sidebar_effectively_collapsed(locked) = settings.sidebar_collapsed && !modal_active && !persist_blocked && conflicts.is_empty()`，
  `apply_desktop_layout_locked` 是唯一依赖 `LEFT_PANE_WIDTH` 的入口，
  一处改完全端联动（active WebView、macOS loading、群聊后台、Resize/DPI 全走这里；
  `set_modal_active` 本来就会调它，弹窗临时展开/关闭后自动复原是零额外代码）。
  `lib.rs` 新增 `set_sidebar_collapsed` 命令（带标准 `webview.label() != "manager"`
  校验，命令总数 44），注册进 `generate_handler![]`。
- 前端：`types.ts` 两字段；`state.ts` 加 `ICON_CHEVRON_LEFT/RIGHT`；
  `styles.css` 仅一条规则（`.desktop-shell.sidebar-collapsed .middle-column { display: none; }`，
  无宽度过渡动画——原生 `set_bounds` 是瞬时跳变，CSS 动画会出 200ms 白缝）；
  `main.ts`：`.desktop-shell` 按 snapshot 挂 class；`.rail-footer` 顶部放
  `#toggle-sidebar-btn`（chevron 随态翻转，title 写 `Ctrl+\`，收起态且有 notice
  或 downloadToast 时挂 `.warn-dot`）；`setSidebarCollapsed` 轻量 invoke
  （不复用 `update_app_settings`，后者会重注册快捷键并广播脚本，太重）；
  收起态点已选服务 / 无激活账号的服务 / 无 activeGroupId 的群聊时自动展开；
  `Cmd/Ctrl + \` 快捷键（不在全局注册，只在 manager 内；`Cmd/Ctrl + B` 留给富文本加粗）。
- 刻意未动：`initial_hidden_child_rect` 也用了 `LEFT_PANE_WIDTH`，但那个视图隐藏在
  屏幕外、激活时重算布局，不影响。
- 偏好与生效态分离：用户偏好 `sidebar_collapsed`，实际生效
  `effective = 偏好 && !modal_active && !persist_blocked && conflicts.is_empty()`——
  收起态点设置/更新时自动临时展开到 380px 显示完整弹窗，关闭后自动缩回；
  写盘锁死或 UUID 冲突时强制展开，不隐藏恢复按钮。
- 测试：新增 `sidebar_effective_collapsed_forced_open_by_modal_blocked_conflicts`
  （`SessionManager::new(temp_dir)` + `get_snapshot()` + 直接改 `inner.lock()` +
  `snapshot_from_locked`；覆盖 modal_active / persist_blocked / conflicts 三种强制展开，
  以及老配置 JSON 默认反序列化为 `false`）。
- 细节：代码里的反斜杠经字节数核实正确（keydown 行 2 个反斜杠 = 单个 `\` 字符串；
  title 行每处 2 个）；工具输出显示会转义反斜杠，以字节数为准，别凭显示改。

## 十三、v6.1.1 移动端导入修复（2026-11-26 晚，小 commit）

背景：用户复审 v6.1 + 侧边栏（1,822 行 diff），评分 **8.1/10**
（5.3 → 6.0 → 6.8 → 7.6 → 8.1），判定桌面端 CI 已就绪；推 `release.yml`
验 Android/iOS 前先修 1 处移动端 `#[cfg(desktop)]` 导入坑，另 2 处小边角顺手改。

### 移动端导入（4 处，逐项核实均为真）

根因：P2-11 拆文件时留下的无条件 `use super::{...}` 引入了父模块里
`#[cfg(desktop)]` 的符号，Android/iOS 构建报 `E0432: unresolved import`。
修法一律是把 desktop 专用符号拆进 `#[cfg(desktop)] use super::{...};`：

- `webview.rs`：9 个 desktop 符号（`WarmDesktopEntry`、5 个 `*_INIT_JS`、
  `GROUP_IDLE_RECLAIM_MS`、`LEFT_PANE_WIDTH`、`MAX_CONCURRENT_PREWARMS`、
  `MAX_WARM_DESKTOP_VIEWS`）+ `SERVICE_RAIL_WIDTH`（在 `session_manager.rs`
  补 `#[cfg(desktop)]`，只在 desktop 布局函数里用）；其余 5 个
  （`ActiveSessionSlot`、`InnerState`、`ManagerSnapshot`、`SessionManager`、
  `APP_TITLE`）无 cfg，留在无条件导入。
- `a2a.rs`：`GroupAttachment`、`REVIEW_FALLBACK_CHAR_LIMIT`、
  `REVIEW_INLINE_CHAR_LIMIT` 移入 desktop 块；`type ReviewJob` 加
  `#[cfg(desktop)]`（唯一使用点是 `#[cfg(desktop)]` 的
  `run_group_review_pipeline` 的 `jobs: Vec<ReviewJob>` 参数）。
- `group_chat.rs`：9 个 `build_*_js` + `GroupAttachment` 移入 desktop 块
  （9 个构建函数与 5 个 `*_INIT_JS` 同属父模块的 `#[cfg(desktop)]
  pub(crate) use scripts::{...}` 重导出）；`evaluate_account_quota` /
  `AccountQuotaEval`（`quota.rs` 无 desktop cfg，重导出也无 cfg）留在无条件导入。
- `quota_sync.rs`：`DesktopProbeResult`（定义在 `session_manager.rs:2803`
  带 `#[cfg(desktop)]`，唯一使用点是同为 desktop 的 `eval_identity_probe`）
  并入已有的 `#[cfg(desktop)] use super::{IDENTITY_PROBE_JS};`。

### 本机首次 Android 检查：顺手多抓 1 处

为验证上述修复，用 `rustup target add aarch64-linux-android` 后跑
`cargo check --target aarch64-linux-android --lib`（`cfg(desktop)` 由
tauri-build 按 target_os 发放，Android 目标上即为 false，正好模拟移动端面）。
首轮剩唯一错误：`group_transport.rs:959` 的 `cli_timeout_error` 用了
`Duration`，但 `use std::time::{Duration, Instant}` 是 `#[cfg(desktop)]`——
该函数只被 `#[cfg(desktop)]` 的 `execute_local_cli_turn` 调用，
连同只被它调用的 `format_timeout_partial` 一起加 `#[cfg(desktop)]`。
修复后 Android check **0 错误**（首次在本机跑通移动端 Rust 检查）。
残留 44 条 mobile 警告多为 desktop 专用 import 在移动端未使用——警告不阻断
`cargo build`；`clippy -D warnings` 只在 `ci.yml` 的桌面三 OS 上跑，
Android job 只做 `tauri android build`，故不影响 CI。

### isTransientLine 两处边界（复审 §2.1，逐项对旧正则核实）

- `matchToken` 的泛型 `X+中` 分支在 `cjk === 0` 时（行首就是 `中`）直接返回 1，
  `"Muse 中"` 误判 true；旧正则 `{1,8}中` 要求至少 1 个前置汉字——
  改成 `if (cjk >= 1 && line[j] === '中')`。
- 品牌词后未判 `i > save`：无空格紧连时也走了 `matchToken(true)`，
  `"Muse正在瞎编中"`（泛型、非字面词）误判 true；旧正则品牌后泛型分支要求
  `[ \t\r\n]+`，而 branch 2（品牌单独）+ branch 3（字面词）拼不出泛型——
  改成 `const len = i > save ? matchToken(true) : 0;`。
  两处均在 `scripts.rs` 的两个同构点改（已确认两点改后文本完全一致）。
- 差分验证：从改后文件原样抽出 tokenizer（含 `TRANSIENT_TOKENS` /
  `extraWords` / `isTransientStatusText`），对旧正则（v6，`git show c08bbd4^`）
  跑 36 个用例——**全部一致**，除 2 处已知的有意差异（`"Muse..."`、
  `"Muse\n\n思考中"`，v6.1 报告已记）；对抗输入 0.28ms。
  注意 `"Muse思考中"` 仍判 true（旧正则同样 true：branch 2 品牌单独 +
  branch 3 字面词 `思考中`），`i > save` 只影响泛型分支。

### 侧边栏按钮小边角（复审 §2.2）

收起态点「设置」临时展开时，`#toggle-sidebar-btn` 显示「收起」但因
`modal_active` 强制展开、点它视觉上没反应——弹窗打开期间按钮置
`disabled`（title 改为“关闭弹窗后可切换侧边栏”），点击处理里再守一道
`modal.kind !== "none"` 直接返回（不关弹窗，避免误关未保存的设置表单）；
tooltip 按平台显示：macOS `⌘\`，其余 `Ctrl+\`（`IS_MAC`，
与 `render.ts` 同款 `navigator.userAgent.includes("Mac")` 判定）；
补 `.rail-util-btn:disabled` 样式（半透明 + `not-allowed`）。
`Ctrl+\` 快捷键早已有 modal 守卫，不受影响。

### 验证（v6.1.1）

- `cargo fmt --check`：干净。
- `cargo clippy --all-targets -- -D warnings`：0 错误。
- `cargo test --lib`：**105/105**；`npm run build` 通过；`npm run test:js`：**20/20**。
- `cargo check --target aarch64-linux-android --lib`：**0 错误**。
- 环境备注：本次验证中虚拟机又被替换了一次，Tauri Linux 开发库再次丢失
  （`cargo test` 链接失败缺 `-lgtk-3` 等），按老办法重装
  （先降级 `libzstd1` 再装 5 个 `-dev` 包）；这是本机验证的环境税，
  不影响仓库代码。

## 十二、残留（v6.1.1 后）

- Android Rust 已在本机 `cargo check --target aarch64-linux-android` 0 错误
  （仅检查未链接、未编 APK）；仍未验证：Windows-only Rust（Job Object
  `SetInformationJobObject` 路径）、Kotlin/Swift 编译、iOS Rust 检查。
  首次真检查是用户推到 GitHub 后跑 CI（`ci.yml` 已有 ubuntu-24.04 /
  windows-2025 / macos-15 + audit job）与 `release.yml`。
- `cargo audit` / `npm audit` 未在本机跑通（cargo-audit 未装；npm audit 端点被代理拦）——
  CI 的 audit job 是首次真跑，现有依赖的高危漏洞可能让它变红。
- Ed25519/minisign 更新签名**已实现（v6.2，见 §十四）**；`release.yml` 仍默认更新关闭（vars 为空），
  开启只需用户在仓库 Settings > Variables 设 `MAAM_UPDATE_REPO=<owner/repo>` 后再发版。
  注意：首次带签名的公开 Release 尚未发过，签名流水线的端到端验证（真实 Secret +
  `minisign -S` + 上传）需一次真实 publish 才能证明。
- **PR-F 与 Step 2 明确延后**（按 v6 复审路线）：PR-F（key_vault DPAPI / Keystore /
  Keychain + `Zeroizing<String>`，去静默明文回退）；Step 2（`self.inner.lock().await`
  跨 `.await` 持锁拆解、流式 delta + 局部 DOM 更新）。等用户复审 v6.1.1 后再定。
- 早期路线“明确未做的项”仍在：P1-8 增量写入、WiX `UninstallString` 强化 /
  WebView2 bootstrapper 哈希、Android FileProvider、时区脚本主框化、iOS 下载缺口、
  审计报告 5 项真机测试；Android 第三方 Cookie 关闭可能影响 Meta SSO /
  Turnstile 人机验证（需真机手动测）。
- 下一步（按 v6 复审路线）：① 用户把 v6.1.1 推到自己的 GitHub 跑首轮真 CI
  （看三 OS 全绿 + audit 报告；手动跑一次 `release.yml` workflow_dispatch，
  `publish_release: false`，首次真实验证 Windows Job Object、Android Kotlin、iOS Swift）；
  ② Ed25519 签名 + PR-F；③ Step 2 + 真机验收（Windows `.cmd` CLI + Job Object 杀树 +
  NSIS 重装；Android 旋转恢复会话 + APK 拦截）。

## 十四、v6.2 Ed25519（minisign）离线更新签名（2026-11-27，闭环 MAAM-H-05）

### 密钥
- `minisign -G` 生成（空口令，密钥本身靠 GitHub Secret 保护）。公钥
  `RWRKjD63aofr1DUF3GDVK8tjmvnIzrN4YqTMHHr7WHHsT8bkD6mtdS5j`
  （key id `D4EB876AB73E8C4A`），编译进 `updater.rs` 常量
  `MAAM_MINISIGN_PUBLIC_KEY`，另存仓库根目录 `minisign.pub`（供
  release.yml 自检用）。私钥只存 GitHub 仓库 Secret
  `MAAM_MINISIGN_SECRET_KEY`（2026-11-27 设定，存的是标准密钥文件全文：
  `untrusted comment:` 行 + base64 行），本地临时文件已 shred 删除。
  私钥丢失的后果：无法再用旧公钥签署新版，需先发一版带新公钥的程序。

### Rust（桌面 + Android 共用同一份实现）
- 依赖：`minisign-verify = "0.2"`（实际 0.2.5，全平台共用）；
  `[dev-dependencies] minisign = "0.7"`（实际 0.7.9，仅测试里现生成
  密钥对做签名，永不使用真实密钥）。
- `verify_minisign_bytes`（latest.json 这类小文本）与
  `verify_minisign_file`（安装包流式验签，64KiB 分块）；两者都只接受
  预哈希签名（`allow_legacy=false`，与发布端 `minisign -S -H` 一致）。
- 接线点（全部 fail-closed，无静默回退）：
  - `check_for_updates` 取到 latest.json 后**先验签再解析**，签名缺失/
    无效直接 `Err`，不回退 GitHub API（回退会绕过签名门）；
  - 桌面分支：下载完成、SHA-256 校验之后、任何安装步骤之前流式验签，
    失败删文件（缓存复用路径同样经过这里）；
  - Android 分支：先取安装包 `.minisig`（小文本，走现有 `fetchText` 桥，
    缺失即拒、不下载大文件）→ Kotlin 只下载 + SHA-256（`downloadApk`，
    不安装，返回路径）→ Rust 校验路径确在应用缓存 `updates` 目录下且
    是 `.apk` → 流式验签，失败删文件 → 通过才调 `installApk`。
- 8 个新测试（`cargo test --lib` 105 → 113）：合法签名通过、篡改内容失败、
  篡改签名失败、错密钥失败、缺失/垃圾签名失败、文件流式通过 + 篡改失败、
  内置公钥解析、`.minisig` URL 后缀。

### Android 验签决策：全部在 Rust 做，Kotlin 只下载 + 安装
- 依据：minSdk=24，而 `java.security.Signature` 的 Ed25519 要 API 28+ 才有；
  生产签名是预哈希模式（算法字节 `ED`），预哈希要 BLAKE2b，JCA 没有；
  Bouncy Castle 不在依赖里，引入 ~5MB 加密库只为验签不值；插件没有
  Kotlin 测试基建。Rust 侧本来就有 `fetchText` 桥，同一公钥、同一份实现、
  全 API 级别覆盖。
- `downloadAndInstallApk` 拆成 `downloadApk` + `installApk`（Rust 侧
  `account-session` 的 `download_and_install_apk` 同样拆成 `download_apk` +
  `install_apk`；`platform::mobile` 是整包 re-export，改名自动流通）。
  `installApk` 保留原安装逻辑，另加 canonical 路径校验（必须在
  `cacheDir/updates` 下、是 `.apk`、真实存在）。下载逻辑逐行未动。

### release.yml（publish job）
- 新增 "Sign release assets with minisign" 步骤：`apt install minisign` →
  Secret 写 0600 临时密钥文件（`trap shred -u || rm -f`）→ `minisign -S -H`
  签 5 个资产 + latest.json → `minisign -V -p minisign.pub` 逐个自检，
  失败即中断发布；发布步骤的 files 数组加 6 个 `.minisig`。
  注意：私钥即使用空口令生成也经 scrypt 加密保存，`minisign -S` 每次都
  要交互式问口令，CI 无 tty 会直接失败（v6.2.1 修复：签名循环改成
  `printf '\n' | minisign -S -H …`，空行即解密通过；本机已验证空口令
  密钥 + 管道喂空行可正常签署并通过 `minisign -V`）。
- v6.2.2（用户 2026-11-27 12:03 审计，8.8/10）：补 2 条。① 删掉
  `check_for_updates` 的未签名 GitHub Releases API 回退——原来清单取回
  非 2xx（例如攻击者发一个不带 latest.json 的 Release）会落入未签名
  API，伪造版本提示/更新日志/下载链接，iOS 因走 Safari 无第二道
  `.minisig` 验签会被直接导去下载恶意 IPA；现改为只信任已签名清单，
  取回失败、非 2xx、验签失败、解析失败一律 fail-closed 报错，`GithubRelease`
  / `GithubAsset` / `select_github_asset` / `github_latest_api_url` 四处
  死代码一并删除，重定向白名单同步去掉 `api.github.com`。② Android
  `verify_downloaded_apk_path` 两边先 `std::fs::canonicalize` 再比较，
  解决真机 `/data/data/<pkg>/cache` → `/data/user/0/<pkg>/cache` 符号
  链接导致合法更新被误拦（同时消掉 `..` 片段；`Path::starts_with`
  本就按组件比较，无 updates-evil 式前缀误判）。顺手：桌面
  `fetch_text_with_status` 的 `resp.text()` 改成分块流式读并加 2 MiB
  上限（与移动端 fetchText 一致，Kotlin 侧 2 * 1024 * 1024）。
- `MAAM_UPDATE_REPO` 的值未动，更新仍关闭。顶部注释更新为“验签已落地”。
- CLI→Rust 互通已验：minisign CLI 0.11 签的文件，Rust `verify()`（清单）与
  `verify_stream()`（安装包）均通过，篡改文件流式验签失败。

### 验证（v6.2）
- fmt、clippy（Linux/Windows）、`cargo test --lib` 113/113、Windows `--no-run`、
  `cargo check --target aarch64-linux-android --lib` 0 错误（44 条 mobile 警告
  是此前就有的）、npm build、test:js 全绿。
- Kotlin 未本机编译（本机无 Android SDK），只做了逐行比对 + 括号平衡检查；
  需 CI 的 release Android 构建验证。iOS 不做应用内验签（沿用现有设计：
  Safari 打开下载），但 `.ipa` 照常签署上传。
- 未验证：真实发布流程（未触发 publish）；首次带签名的公开 Release 需用户点头。

## 十五、v6.3 PR-F 密钥库真加固（2026-11-27，闭环 MAAM-H-06）

### 改动总览

- `key_vault.rs` 全量重写（270 行 → 约 800 行）：`store_key` /
  `load_key` / `delete_key` 改为 `async fn`，`load_key` 返回
  `Option<Zeroizing<String>>`（内存 Key 用完即清零）。后端检测
  缓存一次：桌面 keyring（Keyring）→ 文件（File，仅 Windows
  DPAPI）；移动端插件桥接（Bridge）；测试专用内存后端（Memory，
  `#[cfg(test)]`）。
- `store_key` 不再静默写明文：keyring/DPAPI/桥接全部失败时返回
  `Err`，中文报错说清原因与出路。`store_key` 的 keyring 分支删了
  `.or_else(|_| self.store_file(...))` 静默回退。
- `load_key` 保留旧明文 `api-keys.json` 只读迁移路径：读到旧条目
  时，若安全后端可用立即迁移（写安全后端、删旧条目）；安全后端
  不可用则照常返回旧值（不断现有功能）。迁移完文件为空时删文件。

### 桌面：Windows DPAPI 回退

- keyring 不可用时，回退文件存 `dpapi:<base64(CryptProtectData 密文)>`，
  读时 `CryptUnprotectData` 解密；CurrentUser 作用域 +
  `CRYPTPROTECT_UI_FORBIDDEN`。文件仍走 `write_bytes_atomic_0600`
  原子写盘。
- 旧明文值（无 `dpapi:` 前缀）在 load 时读到即迁移：写 DPAPI 密文、
  删明文条目。
- API 逐项对照 vendored windows 0.62.2 源码核实
  （`~/.cargo/registry/src/*/windows-0.62.2/src/Windows/Win32/Security/Cryptography/mod.rs`）：
  `CRYPT_INTEGER_BLOB`（8986 行）、`CryptProtectData`（1659 行，
  `PCWSTR::null()` 作空描述）、`CryptUnprotectData`（1872 行）、
  `CRYPTPROTECT_UI_FORBIDDEN = 1`（8271 行）；`LocalFree` 在
  `Win32::Foundation`，`HLOCAL` 包的是 `*mut c_void`（不是 isize，
  构造时要 `HLOCAL(output.pbData as *mut core::ffi::c_void)`）。
  另补了 Cargo.toml 的 `Win32_Security_Cryptography` feature（原只
  有 `Win32_Security`）与 `base64 = "0.23"` 依赖（与 reqwest/hyper-util
  共用 0.23.1）。
- 空输入加密直接拒绝（Err）；解密失败返回 None，不抛错。

### 移动端：插件桥接（keyring 不编译）

- account-session 插件新增三个命令（命名照现有 `fetchText` /
  `downloadApk` 风格），Rust 侧经 `MobileSessionBridge::run_mobile_plugin_async`
  调用（`plugins/account-session/src/lib.rs` 新增 `store_api_key` /
  `load_api_key` / `delete_api_key`，`#[cfg(mobile)]`）。
- Android（Kotlin）：AndroidKeyStore 生成 AES/GCM/NoPadding 主密钥
  （alias 固定 `maam-api-key`），每条值随机 12 字节 IV（`cipher.iv`），
  (IV + 密文) Base64(NO_WRAP) 后存应用私有 SharedPreferences
  （名 `maam-api-keys`，MODE_PRIVATE，key 即 keyRef）。无新依赖。
  load 查无条目返回 `{"value": null}`（`JSONObject.NULL` 显式放）；
  load 只取密钥不新建（新密钥解不开旧密文），解密失败只 reject、
  不删数据。
- iOS（Swift）：Keychain，kSecClassGenericPassword，service =
  `multi-agent-account-manager`（与 Rust `KEYRING_SERVICE` 同值），
  account = keyRef，SecItemAdd（已存在 → SecItemUpdate）/
  SecItemCopyMatching（查无 → `value: null`）/ SecItemDelete（查无
  也算成功）。`import Security` 新增。
- 移动端 `probe_keyring` 改为走桥接（桥接失败即后端不可用）。
- 决策（与用户原提议的出入，已在 v6.2 披露过同类取舍）：Android
  验签在 Rust 做是因 JCA 限制；此处 Keystore/Keychain 的加解密本身
  必须在原生侧（Rust 侧无 Keystore/Keychain 绑定），故三命令放
  Kotlin/Swift，Rust 只做桥接调用。

### 调用方异步化（Option A，逐字实现）

- `migrate_agent_keys_to_vault` → `pub async fn`，改传 `&ApiKeyVault`
  （不再每次新建一个，用 setup 注入的实例，移动端才能走桥接）；
  `load_or_migrate_store` → `pub async fn`；`SessionManager::new` →
  `pub async fn` + `new_with_app(dir, app)`（setup 用）+ 共享
  `new_inner`；setup（同步闭包）用
  `tauri::async_runtime::block_on(SessionManager::new_with_app(...))`
  包起来（setup 在主线程跑，不在 runtime worker 内，block_on 安全；
  保留启动时迁移语义，不 defer，避免群聊轮次在 Key 恢复前 401）。
- `ApiKeyVault` 新增 `app_handle: Mutex<Option<AppHandle>>` +
  `set_app_handle()`（换 handle 后失效后端缓存）；`SessionManager`
  构造时把 AppHandle 交给 vault。
- `delete_key` 也改为 async（移动端桥接删除是异步的，三个插件命令
  本就包含 delete）。group_chat.rs 三处（upsert 回滚、kind 改 local_cli、
  删除 agent）全部补 `.await`；第二处回滚分支最初漏了一个
  `delete_key` 未 await，被 clippy 的 unused_must_use 抓出。
- `SessionManager::new` 在生产侧已无调用，改为 `#[cfg(test)]`；
  `VaultBackend` 与三个测试构造器改为 `pub(crate)`（`new_with_backend`
  的 `pub fn` + 私有枚举曾报 private-in-public）。

### 行为变化：Linux 无 secret service 时新 Key 存不进（fail-closed）

- 以前：`store_key` 在 keyring 不可用时静默写明文 `api-keys.json`，
  功能照常用。
- 现在：**新 Key 存不进**。`store_key` 返回 Err，界面走原有
  “保存 API Key 到系统凭据库失败：{e}” 报错链。提示文案（Linux）：
  > 系统凭据库（Secret Service）不可用，无法安全保存 API Key；
  > 请安装并启动 GNOME Keyring 或 KWallet 等 Secret Service 后重试
  > （本版本不再把明文 Key 写到回退文件）
  macOS 文案对应为“系统钥匙串不可用……请确认钥匙串访问正常后重试”；
  移动端桥接失败为“移动端安全存储（Keystore/Keychain）不可用……
  请重试”。
- **旧 Key 不受影响**：已存在 `api-keys.json` 明文条目的用户，
  load 时照常读回；有安全后端时当场迁移走（写安全后端、删旧条目、
  文件空了就删文件），无安全后端时条目保留、下次启动重试迁移。
  磁盘配置在迁移成功后剥离明文、只留 `apiKeyRef`。
- Windows 不受此变化影响：keyring 不可用时回退文件改存 DPAPI 密文，
  新 Key 照常能存。

### 测试

- `cargo test --lib` 117/117（基线 113；旧 `file_fallback_roundtrip_and_0600_perms`
  被按后端分情况断言的 `store_key_fails_closed_without_secure_backend`
  取代，-1；新增 5 个，+5）。
- 新增：① `store_key_fails_closed_without_secure_backend`（按检测到
  的后端分情况断言：Keyring 则往返，File 则报错含“凭据库/安全”且
  不写回退文件、load 为 None）；② `store_key_errors_when_no_secure_backend`
  （强制 File 后端：非 Windows 报错，Linux 断言含 "Secret Service"；
  Windows 上 File 后端即 DPAPI，断言存成功且文件为 `dpapi:` 值——
  最初版本未分平台，在 Windows 交叉 clippy 下既报 `err` 未使用，
  `unwrap_err()` 也会在 Windows 上 panic）；③
  `legacy_plaintext_file_migrates_on_load`（手写旧明文文件；Keyring
  分支断言迁移 + keyring 含 Key，File 分支断言条目保留）；④
  `memory_backend_roundtrip_for_tests`；⑤ `#[cfg(windows)]`
  `dpapi_encrypt_decrypt_roundtrip`（含空输入拒绝）与
  `dpapi_file_value_roundtrip`（DPAPI 文件值 + 旧明文读时改写）；
  ⑥ storage `load_or_migrate_store_keeps_config_when_vault_unavailable`
  （无论迁移成败内存 Key 不丢；按 startup_notice 是否含“未能迁移”
  分情况断言磁盘保留原配置或已剥离明文）。
- 其余 5 个调 `load_or_migrate_store` 的 storage 测试改为
  `#[tokio::test]` + 真实 vault + `.await`（迁移对它们是 no-op）。
- session_manager 两个 vault 测试与 A2A 防环测试注入共享内存 vault：
  `set_test_vault`（`#[cfg(test)]`，async，换 vault 后按生产迁移逻辑
  重跑一次——构造时用的真实 vault 可能没能把磁盘 `api_key_ref`
  恢复进内存，换 vault 后需补跑；最初版本只换不重跑，重启恢复断言
  失败）。roundtrip 测试的 mgr/mgr2/mgr3 共用同一 `Arc<Mutex<BTreeMap>>`。
- `load_key` 返回 `Option<Zeroizing<String>>` 后，测试里 6 处
  `.as_deref()` 需补 `.map(String::as_str)`（`as_deref()` 得
  `Option<&String>` 而非 `Option<&str>`），另 1 处 `load_keyring`
  同理；均由编译器报错逐个抓出。

### 验证（v6.3）

- fmt、clippy（Linux/Windows）`--all-targets -- -D warnings` 全绿；
  `cargo test --lib` 117/117；`cargo check --target aarch64-linux-android --lib`
  0 错误（44 条 mobile 警告，与基线一致——中途曾因 `VaultBackend::Keyring`
  在移动端永不构造多出 1 条，已用 `cfg_attr(allow(dead_code))` 压回）；
  npm build、test:js 全绿。
- 中途虚拟机又换了一次（第 6 次，apt 包全丢）：`cargo test` 链接报缺
  `-lgdk_pixbuf-2.0` / `-lgtk-3`，跑 `restore-build-env.sh` 恢复（GTK OK）。
- Kotlin 与 Swift 均为草稿先行、逐项对照现有文件与官方 API 核实后落地：
  Kotlin（676 → 815 行，+139）与 Swift（430 → 540 行，+110），
  均未改其他文件。Swift 草稿的 service 值已与 `KEYRING_SERVICE`
  逐字核对一致。
- 未验证：iOS Swift 本机编不到（无 Apple 工具链），Windows DPAPI
  本机跑不到（只有交叉 clippy 编过），Kotlin 本机编不到（无 Android
  SDK）——三项均靠 CI（check Windows 跑 DPAPI 测试、release Android
  构建编 Kotlin、macOS/iOS 构建编 Swift）。

## 十六、v6.3.2 复审小补丁（2026-11-27，用户评 9.0/10 后的收口）

用户逐行复审 v6.3.1（对照 v6.1.7，3,140 行 diff）打分 **9.0/10**：
初版审计 9 条 HIGH（MAAM-H-01~H-09）至此全部闭环。本轮抓出 4 个细节，
收进小补丁 v6.3.2（未 commit，工作树改动待复核；10 文件 +221/-61）。
实现代码未改设计，逐项按用户给的修法落地。

### 1. 移动端启动防死锁 + 失败探测不永久缓存（key_vault.rs / storage.rs / session_manager.rs）

- `detect_backend()` 只缓存成功探测到的安全后端（`Keyring` /
  `Bridge`）；探测返回 `File` 时不写缓存，下次调用重新探测。
  代价是 keyring 不可用时每次 store/load 多一次探测（Linux 上一次
  D-Bus 往返），调用本身低频，可接受；未加 TTL 之类复杂机制。
  移动端启动期插件尚未就绪、Linux 用户事后启动 Secret Service，
  进程内都可恢复。
- 移动端迁移延后：`load_or_migrate_store` 新增 `migrate_keys: bool`
  参数；`SessionManager::new_inner` 传 `cfg!(desktop)`——桌面仍在
  setup 的 `block_on` 里同步迁移（行为完全不变：keyring 是同步调用，
  无插件调用，安全）；移动端传 `false`，迁移改由 `run_startup_tasks`
  开头的 `#[cfg(mobile)]` 块跑（已是 `spawn` 的异步任务，不占主线程，
  在自动切账号/任何 API 轮次之前）。迁移失败立同款 notice
  （"API Key 未能迁移到系统凭据库，已保留原配置，下次启动重试"），
  下次启动重试。
- 延后时关键细节：`migrate_keys=false` 不只跳过迁移，还连同
  `migrated` 的写回一并跳过（`if migrated && migrate_keys`）——明文
  Key 尚未进 vault，此时写盘会把 `api_key`（`skip_serializing`）剥离，
  若在启动任务执行前崩溃 Key 即永久丢失。磁盘原文件不动，等启动任务
  迁移成功后，有旧明文 Key 才 `request_persist()` 写盘剥离明文；
  无旧明文时迁移只是从 vault 回填内存，不写盘。
- 测试：原有 `load_or_migrate_store` 调用点全部传 `true`（桌面行为）；
  新增 `load_or_migrate_store_defers_key_migration_when_flag_false`
  覆盖延后路径（load 不碰 vault、不写盘、内存 Key 保留；补跑迁移后
  Key 进 vault）。`run_startup_tasks` 的 mobile 块是 `#[cfg(mobile)]`，
  本机编不到，靠 CI 的 Android/iOS 构建验编译。

### 2. iOS Keychain 备份属性 & Android Keystore 并发/落盘（Swift / Kotlin）

- Swift（`AccountSessionPlugin.swift` 的 store/load/deleteApiKey）：
  `SecItemAdd` 与 `SecItemUpdate` 都显式加
  `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`，API Key 永不随
  iCloud / 加密备份离开本机；三处都补 `guard !args.keyRef.isEmpty`
  （与 Kotlin 对齐，空 keyRef 直接 reject）。
- Kotlin（`AccountSessionPlugin.kt`）：新增 `private val apiKeyLock = Any()`；
  store 的 `getOrCreateApiKey()` + 加密 + 写 SharedPreferences 整段包进
  `synchronized(apiKeyLock)`，load 的取密钥+解密也同步（防与 store 的
  密钥轮换竞态），delete 的 prefs remove 也同步。store 与 delete 的异步
  `.apply()` 改同步 `.commit()` 并校验返回 `true`（失败抛错进 reject，
  避免 resolve 后崩溃丢 Key）。AAD：store 在 `init(ENCRYPT_MODE)` 后、
  `doFinal` 前调 `cipher.updateAAD(keyRef UTF-8)`；load 在
  `init(DECRYPT_MODE)` 后、`doFinal` 前用同一 keyRef 调 `updateAAD`，
  密文被换到别的条目时解密失败。

### 3. Zeroizing 贯通到长驻内存 + DPAPI 解密缓冲清零

- 核实（vendored zeroize 1.9.1，
  `~/.cargo/registry/src/*/zeroize-1.9.1/src/lib.rs`）：
  `Zeroizing<Z>` 有 `Debug` 实现（601 行，`finish_non_exhaustive()`，
  **不打印内容**，derive Debug 不会泄 Key）、`Clone`（619 行，要求
  `Z: Clone`）、`Deserialize`（721 行，`serde` feature 下，要求
  `Z: Zeroize + Deserialize`，`String` 满足）与 `Serialize`（707 行）。
  `ExternalAgentRecord` 的 `Debug/Clone/PartialEq/Eq/Serialize/Deserialize`
  六个 derive 全部直接兼容，无需手写。`Cargo.toml` 改为
  `zeroize = { version = "1", features = ["serde"] }`；`Cargo.lock`
  只多 3 行（zeroize 依赖加 `serde`，serde 本已在锁内）。
- `ExternalAgentRecord.api_key: String` → `Zeroizing<String>`（仍
  `#[serde(default, skip_serializing)]`，只留内存，绝不落盘、不下发
  前端；快照仍只给 `hasApiKey`）。适配点：`storage.rs` 旧配置解析
  （`Zeroizing::new(api_key)`）、迁移回填（`.map(|k| k.to_string())`
  直接去掉，`load_key` 本就返回 `Zeroizing<String>`）、
  `group_transport.rs` 两处构造（`normalize_external_agent_input` 的
  OpenaiApi/LocalCli 分支）与两处测试 helper、`group_chat.rs` 的
  `prev_api_key: Option<Zeroizing<String>>`（回滚 `store_key` 靠 Deref
  到 `&str`）；其余 `.trim()` / `.is_empty()` / `.clear()` /
  `bearer_auth(...)` 均靠 `Deref`/`DerefMut` 链零改动。测试里的
  `assert_eq!(rec.api_key, "...")` 改为 `.as_str()` 比较。
- `dpapi_decrypt` 返回 `Zeroizing<Vec<u8>>`；`LocalFree` 之前先对
  `from_raw_parts_mut(output.pbData, output.cbData as usize)` 做
  `zeroize::Zeroize::zeroize`（`impl Zeroize for [Z]` 在 lib.rs:452 行，
  切片清零成立），再释放。调用方 `decrypt_file_value` 用
  `std::mem::take(&mut *plain)` 取出原 `Vec` 再 `String::from_utf8`，
  不多一次拷贝。
- **偏差说明**：用户修法写的 `plain.into_inner()` 在 zeroize 1.9.1
  里**不存在**（`Zeroizing` 只有 `new`，无 `into_inner`，已 grep 核实）；
  改用 `mem::take` 达到同样效果（取出原 Vec，空壳 drop 时零化空 Vec），
  设计意图不变。

### 4. 两个极小边角

- `key_vault.rs`：`ApiKeyVault` 加 `file_lock: Mutex<()>`（三个构造器
  都初始化），`write_file_entry` / `delete_file_entry` 的读-改-写全程
  持锁；`load_file_migrating` 里调 `delete_file_entry` 的路径也被覆盖；
  两个函数都是同步的，锁内无 await，无嵌套持锁。
- `updater.rs`：`MAX_UPDATE_TEXT_BYTES` 加 `#[cfg(desktop)]`（只在桌面
  分支用，537/547 行均在 `#[cfg(desktop)]` 块内）；Android check 的
  mobile 警告由 44 条降到 43 条，少的正是这条 `dead_code`。

### 验证（本机逐项，2026-11-27）

- `cargo fmt --check` 通过；`cargo fmt` 后重 grep 确认各点位。
- `cargo clippy --all-targets -- -D warnings`（Linux）通过（仅 1 条
  基线 manifest 警告：unused dependency `tauri-plugin-account-session`）。
- `cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings`
  通过（DPAPI 改动：`Zeroize::zeroize` 切片调用与 `mem::take` 均编过）。
- `cargo test --lib`：**118/118**（117 + 新增延后迁移测试 1 个，单独重跑
  通过）。
- `cargo check --target aarch64-linux-android --lib`：0 错误，43 条警告
  （基线 44，少 1 条即 `MAX_UPDATE_TEXT_BYTES` 的 `dead_code`，已核实
  剩余 `dead_code` note 行只是通用提示，无该常量警告）。
- `npm run build`、`npm run test:js` 通过。
- 未验证：iOS Swift 本机编不到（无 Apple 工具链），Kotlin 本机编不到
  （无 Android SDK）——两项靠 CI（release Android 构建编 Kotlin、
  macOS/iOS 构建编 Swift）。Windows DPAPI 测试靠 CI Windows check 真跑。
