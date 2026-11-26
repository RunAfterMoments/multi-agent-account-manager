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

## 四、发版前真机手动测试清单

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
