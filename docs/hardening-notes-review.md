# 第二轮审查报告（只审查，不改代码）

- 基线：v0.1.10，提交 `0416f41`，分支 `hardening/second-round-review`
- 工作目录：`~/workspace/maam-wt-review`
- 审查范围：任务书第 4 节 7 项。行号以基线为准，函数名为准。
- 方法：静态阅读全部目标文件；对 `platform.rs` 的 Cookie 域名过滤与 `;` 注入点跑了单元测试验证（见末尾「验证命令」）；其余需真机/安装包验证的项标注「待实测」，不猜结论。
- 严重程度：P0 可直接利用 / 丢数据；P1 需特定条件；P2 纵深防御 / 建议。

## 结论总览

| # | 区域 | P0 | P1 | P2 | 待实测 |
|---|------|----|----|----|--------|
| 1 | Android | 1 | 2 | 7 | 2 |
| 2 | iOS | 0 | 1 | 6 | 1 |
| 3 | platform.rs | 0 | 1 | 6 | 1 |
| 4 | site_adapters / 注入 JS | 0 | 2 | 4 | 0 |
| 5 | installer.nsi | 0 | 0 | 5 | 1 |
| 6 | session_manager 并发 | 0 | 0 | 3 | 0 |
| 7 | README / latest.json | 0 | 0 | 2 | 0 |
| 合计 | | **1** | **6** | **33** | **5** |

最值得优先修的 3 项（见各节明细）：
1. **Android APK 更新不校验 sha256（P0）**——`sha256` 为空时直接安装，且缓存复用不校验；与 P0-2 同根，但 Android 分支此前未被覆盖。
2. **桌面端 NET_HOOK 在主框登录页读取响应体（P1）**——`NET_HOOK_INIT_JS` 无任何 host 门禁，主框可导航到 `facebook.com` / `appleid.apple.com`，钩子会在这些页面读 POST 响应体。
3. **页面 JS 可伪造 `window.__maam*` 全局变量（P1）**——身份、额度、群聊回复都信任页面可写的全局变量，Rust 侧不做二次校验。

---

## 1. Android

文件：`src-tauri/plugins/account-session/android/src/main/java/com/ldjx/accountmanager/AccountWebActivity.kt`（2550 行）、`AccountSessionPlugin.kt`（595 行）、`AccountSessionCoordinator.kt`（294 行）、`RecentAccountPages.kt`（128 行）、`AndroidManifest.xml`（17 行）。

### P0 / `AccountSessionPlugin.kt:downloadAndInstallApk` / APK 安装前不校验 sha256

- 依据：
  - `sha256` 参数是 `String?` 可空。`expectedSha` 为空时，全新下载只计算摘要不比对（`if (expectedSha != null && actualSha != expectedSha)` 才拒绝），随后直接调起安装器。
  - 缓存复用分支（`canReuse`）在 `expectedSha == null` 时只凭 `lastDownloadedUrl == args.url` 就信任 `cacheDir/updates/` 下的旧文件，不做任何校验。
  - 上游：`updater.rs:841` 的 `extract_sha256_digest` 在 `latest.json` 缺少/损坏 `sha256` 时返回 `None`，于是空值一路透传到 Kotlin。`check_for_updates` 的 GitHub API 兜底分支同样可能产出 `None`。
  - 这与 P0-2「sha256 必填」的要求直接冲突，且 Android 分支在任务书中未被点名，属于新发现的同根漏洞。
- 建议：与 P0-2 一起修——Rust 侧 `sha256` 缺失直接拒绝（覆盖 Android 分支）；Kotlin 侧把 `sha256` 改为必填，`expectedSha == null` 时拒绝安装并删除已下载文件；复用分支一律重算摘要。

### P1 / `AccountSessionPlugin.kt:openHttpConnection`、`fetchText` / 更新重定向不限主机

- 依据：手动重定向循环只校验 `https` scheme，不校验主机。初始 URL 虽经 Rust 侧 `validate_download_url`（仅限 GitHub 主机），但任一跳都可跳到任意 `https` 主机；`fetchText`（用于拉 `latest.json`）同样如此。结合上一条（sha256 可空），攻击链为：清单被重定向到攻击者 JSON → `downloadUrl` 指向攻击者 APK → 无 sha 校验直接安装。利用条件高（需控制清单 URL 的重定向目标，即攻破 GitHub 侧或 TLS），故为 P1 而非 P0。
- 建议：与 P0-2 第 4 条同款修复——Kotlin 侧实现白名单重定向（`github.com`、`objects.githubusercontent.com`、`release-assets.githubusercontent.com`），每跳校验。

### P1 / `AndroidManifest.xml` + 两处 `FileProvider.getUriForFile` / FileProvider 未声明（待实测）

- 依据：仓库内没有任何 `<provider android:name="androidx.core.content.FileProvider">` 声明，也没有任何 `res/xml/file_paths.xml`；插件的 `AndroidManifest.xml` 只声明了一个 `exported="false"` 的 Activity；`src-tauri/gen/android` 不存在于仓库。两处调用点（`AccountSessionPlugin.kt:495` APK 安装、`AccountWebActivity.kt:2391` 旧版下载打开）都用 authority `"${packageName}.fileprovider"`。若 Tauri 生成的工程也没有声明该 provider，两处调用都会抛 `IllegalArgumentException`，应用内更新安装在 Android 上直接失败。
- 待实测：跑一次 `tauri android init` / 实际打包，检查 `src-tauri/gen/android/app/src/main/AndroidManifest.xml` 与 `res/xml/file_paths.xml`；或在真机上走一遍应用内更新。若缺失，在插件 manifest 里补 `<provider … android:exported="false" android:grantUriPermissions="true">` + 限定 `cache-path` 的 `file_paths.xml`（只暴露 `cacheDir/updates`，不要 `external-path`）。

### P2 / `AccountWebActivity.kt:1839-1842` / 混合内容与第三方 Cookie

- 依据：`mixedContentMode = MIXED_CONTENT_COMPATIBILITY_MODE`（允许 http 子资源，http 图片/媒体请求会把同域 Cookie 以明文发出）；`cm.setAcceptThirdPartyCookies(wv, true)`（允许第三方 Cookie，增加跨站追踪面）。
- 建议：`mixedContentMode = MIXED_CONTENT_NEVER_ALLOW`；第三方 Cookie 默认关（登录流程如需再放开到指定域名）。

### P2 / `AccountWebActivity.kt:1999-2080` / 下载走 http 且无大小上限

- 依据：`handleDownloadRequest` 接受 `http://` 下载链接；`startHttpDownload` 的重定向循环只禁非 http(s) scheme，允许 https→http 降级（`CookieManager.getCookie(http…)` 不返回 Secure Cookie，但非 Secure Cookie 会以明文发出）；且 HTTP 下载全程无大小上限（与 blob 下载的 256MB 上限 `MAX_BLOB_DOWNLOAD_BYTES` 不对称），恶意页面可触发超大下载占满设备存储；下载无需用户确认（`DownloadListener` 自动开始）。
- 建议：下载只允许 https（或至少禁止 https→http 降级）；HTTP 下载加大小上限与下载前确认；文件名已做清洗（`sanitizeDownloadFilename` 会剥 `/ \` 与控制字符、去首 `.`，`.apk` 仍允许——属有意为之，用户仍需手动点安装，见备注）。

### P2 / `AccountWebActivity.kt:1874`、`AndroidBlobDownloadBridge` / JS Bridge 只靠随机 token 隔离

- 依据：`addJavascriptInterface(blobBridge, "__maamAndroidDownload")` 对主框和弹窗都注册；`beginBlobDownload/appendBlobChunk/finishBlobDownload` 凭 `pendingBlobTokens` 里的 UUID token 认领会话（token 由原生生成、经 `evaluateJavascript` 的 IIFE 传入，页面 JS 无法直接读到），文件名经 `sanitizeDownloadFilename` 清洗，大小上限 256MB。设计基本成立。
- 建议（纵深）：token 应一次有效且 `finish/fail` 后立即失效（现状 `pendingBlobTokens.remove` 在 begin 时已做，`activeBlobSessions` 在 finish 时 remove，基本满足）；另注意 `hintName` 取自页面可写的 `window.__maamLastDownload`，已清洗，仅作文件名，无进一步风险。无直接漏洞，记为 P2 备注。

### P2 / `AccountWebActivity.kt:1879-1891` / 权限请求自动放行 EME

- 依据：`handleWebPermissionRequest` 对 `RESOURCE_PROTECTED_MEDIA_ID` 直接 `grant`，其余拒绝。Protected Media ID 是可用于追踪的设备标识，DRM 场景下无需用户确认即放行。
- 建议：首次遇到时弹确认，或仅对 muse/hark 主域放行并记录。

### P2 / `AccountWebActivity.kt` / 弹窗与主框的输入监听

- 依据：`QUOTA_HOOK_INIT_SCRIPT` 的 fetch/WebSocket 钩子有 `isPrimaryHost` 门禁，不在弹窗装；但 `IDENTITY_PROBE_SCRIPT` 的 `input/change` 监听在弹窗也注册，且探针循环每 4.5 秒对弹窗执行 `evaluateJavascript`，只取 `typed`（Apple ID / Facebook 登录页 typed 的邮箱会被记为 `capturedIdentityHint`，用作身份回退）。属登录捕获的有意设计，但应在文档中明确：应用会读取第三方 IdP 登录页的输入内容。
- 建议：文档注明；或将弹窗探针限制为仅在 `isExternalLoginHost` 时取 `typed`，其余字段不读（现状已是只取 `typed`，保持即可）。

### P2 / `AccountSessionPlugin.kt:copyProfileCookie` / 剪贴板无清理时机

- 依据：复制 Cookie 后无任何定时清理；仅在 API 33+ 置 `ClipDescription.EXTRA_IS_SENSITIVE`。系统行为未在代码中保证。
- 建议：复制后起一个 60 秒定时器，用空内容覆盖剪贴板（需处理用户已复制走他物的情况：只在剪贴板内容仍是本次 Cookie 时才清）；或在下次打开账号时清理。详见第 3 节（全平台同问题）。

### P2 / `AccountSessionPlugin.kt:46` / `proxyUrl` 是死字段

- 依据：`OpenSessionArgs.proxyUrl` 传到 Kotlin 后从未被使用；Rust 侧 `SUPPORTS_ACCOUNT_PROXY` 在 Android 上为 `false`，`parse_account_proxy` 会直接拒绝保存代理，故 Android 上该字段恒为空。无安全影响，仅是死代码。
- 建议：删除 Kotlin 侧字段，或在注释中说明 Android 不支持账号代理（与 `SUPPORTS_ACCOUNT_PROXY` 对齐）。

### 待实测 / 账号删除与 Profile 删除的竞态

- 依据：`deleteProfile` 在视图仍活动时拒绝（`isUuidActive`），Rust 侧先把 key 记入 `pending_cleanup`、下次启动重试（`session_manager.rs:5399-5400, 5625-5626, 4262 retry_pending_cleanups`），并先调 `RecentAccountPages.discardProfile` 销毁已停放的 WebView；`delete_account` / `clear_account_data` 在移动端不像桌面端那样先 sleep 120/150ms 等视图销毁（`session_manager.rs:5436-5443` 的 sleep 只在 `closed_desktop_view` 时）。Android 上视图未销毁时 `ProfileStore.deleteProfile` 返回 false → 走重试，数据最终能清；但「删除后立即重启是否残留 Cookie」未实测。
- 待实测条件：真机上登录账号 → 删除账号（视图仍在前台时）→ 立即重启应用 → 检查该 uuid 的 Profile 是否仍存在、Cookie 是否可读。

### 已确认好的地方（Android）

- 插件 manifest 只有一个 Activity，`exported="false"`，无 intent-filter；权限只有 `INTERNET` / `ACCESS_NETWORK_STATE` / `REQUEST_INSTALL_PACKAGES`（更新安装必需）/ `WRITE_EXTERNAL_STORAGE maxSdkVersion=28`。
- 未声明 `usesCleartextTraffic` / `networkSecurityConfig`：targetSdk≥28 时系统默认禁明文；主框导航 `hostMatchesAllowed` 强制 https（`AccountSessionCoordinator.kt:hostMatchesAllowed`）；`fetchText` 与更新下载强制 https 初始 URL。但注意代码层 `http://` 下载仍可走明文（见上 P2）。
- `allowFileAccess=false`、`allowContentAccess=false`；无 `onReceivedSslError` 改写（默认拒绝）；无 `setWebContentsDebuggingEnabled`；`shouldOverrideUrlLoading` 拦截非 http(s)/about/blob/data scheme。
- Profile 创建/删除基本对称：`setProfile(uuid)` 隐式创建，`deleteProfile` + `pending_cleanup` 重启重试；冷启动时校验 Profile 存在（`openAccountSession` 的 `isColdStart` 检查与 `session_manager.rs:7599` 的 Rust 侧检查双重把关）；`RecentAccountPages` 停放的页面在删除前先销毁；`onPageCommitVisible` 校验 profile 名与 allowedHosts。
- `hostMatchesAllowed` 用 `host == clean || host.endsWith("." + clean)`，`evilmuse.ai` 不会匹配 `muse.ai`；`allowedHosts == ["*"]` 时放行全部——仅当 provider 配置如此，内置 muse/hark 不含 `*`。
- 下载重定向发 Cookie 前有 `isSameSiteOrAllowedHost` 检查（同站或同属一个 allowedHost 才带 Cookie 与 Referer）。

---

## 2. iOS

文件：`src-tauri/plugins/account-session/ios/Sources/AccountSessionPlugin.swift`（428 行）、`AccountWebViewController.swift`（1005 行）。

### P1 / `AccountWebViewController.swift:decidePolicyFor` / 子框可加载任意 https/http 主机（待实测其一为删除竞态）

- 依据：`decidePolicyFor navigationAction` 对非主框（`!isMainFrame`）只校验 scheme 属于 http/https/about/blob/data，**不校验 host**。主框受 `hostMatchesAllowed` 限制，但任意 iframe/subframe 可加载任意 https（乃至 http）页面，与主框共享同一个 `WKWebsiteDataStore`（同一账号的 Cookie jar）。同时 `blob:https://` 与 `blob:null/` 主框导航也被无条件放行。
- 建议：子框导航也校验 host 白名单（至少禁 http，第三方 iframe 仅放行 `EMBEDDED_WIDGET_HOSTS` 一类的验证码/ widget 域名）；`blob:` 导航校验内层 origin。
- 注：弹窗（`createWebViewWith`）复用主框配置的 data store，隔离未破；`javaScriptCanOpenWindowsAutomatically = true`（Android 侧为 false）放宽了弹窗，建议与 Android 对齐为 false。

### P2 / `AccountSessionPlugin.swift:copyProfileCookie` / 剪贴板无过期时间

- 依据：`UIPasteboard.general.setItems(…, options: [.localOnly: true])`——`localOnly` 阻止了跨设备同步，好；但未设 `expirationDate`，Cookie 会一直在剪贴板里，直到被其他内容覆盖。另：域名过滤只校验 host（`entryHost == domain || entryHost.hasSuffix("." + domain)`），不校验 path，且未排除公共后缀（`Domain=.ai` 形式的 Cookie 若存在于 store 中也会被复制；浏览器一般不会签发公共后缀 Cookie，且 Rust 侧导入已拦，故为备注级）。
- 建议：`setItems` 时加 `.expirationDate: Date(timeIntervalSinceNow: 60)`；或复制后 60 秒由应用主动清空（仅当剪贴板仍是本次内容时）。

### P2 / `AccountSessionPlugin.swift:importProfileCookies` / 域名校验完全依赖 Rust 侧

- 依据：`resolveTargetUrl` 从 Set-Cookie 头里取第一个 `Domain=` 自行拼出 `https://<domain>/` 作为 `HTTPCookie.cookies(withResponseHeaderFields:for:)` 的目标 URL，等于是用头部自己的 Domain 为自己背书。若头部值含 `;` 注入（见第 3 节 P2），第一个 `Domain=` 可能是注入值，Cookie 会被种到注入的域下。正常流程中 Rust 侧 `parse_imported_cookies` 已过滤域名，所以此条是纵深防御问题；且该插件命令不直接暴露给前端（见第 4 节末备注），只经 Rust 调用。
- 建议：iOS 侧不再从头部解析 Domain，改用传入的 `url`（entry URL）作为目标 URL；或解析后二次校验 Domain 属于 allowedHosts。

### P2 / `AccountSessionPlugin.swift:buildHttpConnectProxyConfig` / 凭据静默丢弃、缺端口默认 80

- 依据：只解析 scheme/host/port；`http://user:pass@host:port` 的 userinfo 被静默丢弃（代理无认证会连不上，用户看到的只是「代理不可用」）；无端口时默认 80。Rust 侧 `parse_account_proxy` 已拒绝 userinfo 与无端口地址，故正常流程到不了这里。
- 建议：解析失败/含 userinfo 时返回 nil 并让调用方报错（现状 `bindSessionSpec` 里 `buildHttpConnectProxyConfig` 返回 nil 时静默保留旧的 `proxyConfigurations`，见下条）。

### P2 / `AccountWebViewController.swift:397-402` / 代理配置失败时静默保留旧配置

- 依据：`proxyRaw` 非空但解析失败时，既不清空也不报错，data store 沿用上一次的代理配置；只有 `proxyRaw` 为空时才清空。
- 建议：解析失败时显式 `dataStore.proxyConfigurations = []` 并向 Rust 报错。

### P2 / `AccountWebViewController.swift` / 无下载实现

- 依据：全文件无 `WKDownloadDelegate` / `decidePolicyFor navigationResponse`。WKWebView 默认对 `Content-Disposition: attachment` 的导航无可见处理，iOS 端点下载链接基本无反应（与 Android 的完整下载实现不对称）。
- 建议：功能缺口，非安全问题；如补实现，应复用 Android 的同款校验（https-only、host 校验、大小上限、落盘前确认）。

### P2 / `AccountWebViewController.swift:697-700` /  stale 会话自动 GET 登出

- 依据：连续两次探针判 loggedOut 且 `staleSession` 为真时，自动 `load(https://muse.ai/api/logout)`（hark 则跳登录页）。这是用 GET 触发的状态变更，且依赖探针判断；若探针误判（如页面结构变化导致 composer 检测失败），会把已登录用户登出。
- 建议：改为仅提示用户手动处理，或在触发前用 `__maamVerifySession` 再确认一次。

### 待实测 / 删除账号时 `remove(forIdentifier:)` 与存活 WebView 的竞态

- 依据：`deleteProfile` 只查 `isUuidActive`（`activeSpec`，Rust 侧 `hide_foreground_slot` 调 `closeActiveSession` 后即清零），但 `closeFromManager` 的 `dismiss(animated: true)` 是异步的；`clear_account_data` / `delete_account` 在移动端没有桌面端那 120/150ms 等待。`WKWebsiteDataStore.remove(forIdentifier:)` 在仍有 WKWebView 持有该 store 时的行为未在代码中处理（Android 侧会返回 false 走重试；iOS 侧未知）。
- 待实测条件：真机上登录账号 → 删除账号 → 立即重启 → 检查该 uuid 的 data store 是否仍存在、Cookie 是否可读；以及删除后立刻重建同 uuid 账号是否会读到旧 Cookie。

### 已确认好的地方（iOS）

- 隔离：每账号 `WKWebsiteDataStore(forIdentifier: uuid)`，冷启动校验 identifier 存在；`deleteProfile` 用 `remove(forIdentifier:)` 整库删除；`checkCapabilities` 用 `fetchAllDataStoreIdentifiers`。
- `fetchText` 用 `.ephemeral` 会话 + `HttpsOnlyRedirectDelegate`（仅 https 重定向），2MB 上限（注：上限在收到完整 body 后才判断，内存已占用，见 P2 备注——实际是先 `dataTask` 收完再 `payload.count > 2MB` 拒绝，超大响应仍会短暂占内存）。
- 钩子：`hookInitScript` 的 fetch 钩子有 `isPrimaryHost` 门禁，不在 Apple ID / Facebook 页读响应体（与桌面端 `NET_HOOK` 不同，见第 4 节）；输入监听在所有 https 顶层页注册，属登录捕获设计（弹窗探针只取 `typed`/`email`）。
- `openExternalUrl` 只允许 http/https。
- 探针在 `accountscenter.meta.com` 会从 body 文本取邮箱（有意设计，见第 4 节）。

---

## 3. platform.rs

文件：`src-tauri/src/platform.rs`（2453 行）。

### P1 / `build_timezone_override_init_js`（约 71-326 行）/ 时区覆盖脚本注入到所有帧，包括验证码 iframe（待实测）

- 依据：脚本重写 `Date.prototype`（get/set/toString/toLocale*、`getTimezoneOffset`）与 `Intl.DateTimeFormat`（注入 `timeZone`），并以 `document-start` 注入。注入范围：桌面端主框 + 弹窗的 initialization script（所有帧）；Android `WebViewCompat.addDocumentStartJavaScript(wv, tzScript, setOf("*"))`（所有 origin）；iOS `WKUserScript(…, forMainFrameOnly: false)`（主框 + 全部子框）。因此 hCaptcha / Turnstile / reCAPTCHA 等验证码 iframe 里 `Date` 与 `Intl` 也被改写——这类服务依赖环境指纹做机器人判定，改写可能导致验证失败或被加严；同时 `Date.prototype.toString` / `Intl.DateTimeFormat.toString` 可被页面检测到改写（指纹特征）；OAuth 页的时区与 IP 属地不一致也可能触发风控加验。
- 待实测条件：给账号设一个非常用时区（如 `America/New_York`）后，在桌面 + Android + iOS 各走一遍含 Turnstile/hCaptcha 的登录流程，对比不设时区的通过率；并检查 `Intl.DateTimeFormat().resolvedOptions().timeZone` 与 `new Date().getTimezoneOffset()` 是否与设定一致、页面有无报错。
- 建议：时区脚本只注入主框（iOS 改 `forMainFrameOnly: true`；Android 把 `setOf("*")` 收窄到 muse/hark 入口 host；桌面端同理），子框/弹窗的 OAuth 页按需注入；或改用 WebView2 的 `Emulation.setTimezoneOverride`（Windows 已有 `configure_windows_timezone` 走 CDP）/ WKWebView 的等价能力，减少 JS 改写面。
- 另：`parse_account_timezone` 只校验字符集，不校验 IANA 名是否存在；无效名在 JS 侧 `new NativeDTF(…, {timeZone: TZ})` 抛错后静默不生效（脚本直接 return），UI 上时区却显示「已设置」。建议 Rust 侧用 `chrono-tz` 或系统时区表校验，失败即报错。

### P2 / `imported_cookie_to_set_cookie_header`（约 1891-1913 行）/ Cookie 值含 `;` 可注入 Set-Cookie 属性

- 依据：`parse_imported_cookies` 只拒绝值中的 `\r`、`\n`、NUL，不拒绝 `;`；`imported_cookie_to_set_cookie_header` 用 `format!("{}={}", name, value)` 直接拼接。JSON / Netscape 导入的值含 `; Domain=…` 时，移动端（Android `CookieManager.setCookie`、iOS `HTTPCookie.cookies`）会把注入部分当作属性解析。
  - 已用临时单元测试验证：值 `abc; Domain=evil.example; Path=/x` 经 `parse_imported_cookies` → `imported_cookie_to_set_cookie_header` 后，输出头确实包含 `Domain=evil.example`（见末尾验证命令）。
  - 实际影响有限：Android 侧 `CookieManager.setCookie(entryUrl, …)` 会校验 Domain 与 URL host 的匹配关系，注入的异域 Domain 会导致整条 Cookie 被丢弃（静默失败——导入显示成功但登录态没写进去）；iOS 侧 `resolveTargetUrl` 取第一个 `Domain=` 作目标 URL，注入域的 Cookie 可能被种进该账号的 store（但只会发往该域，WebView 导航白名单去不了，见第 2 节；确切的 `HTTPCookie` 首/末属性优先级待实测）。
  - 桌面端不受影响：`imported_cookie_to_tauri` 用 `Cookie::parse("k=v; Domain=…")` 作模板再 `set_name/set_value`，值不经过头解析。
- 建议：值中出现 `;` 或 `,` 时拒绝该条（或按 RFC 6265 对值做引号包裹并转义）；同时 iOS 侧不要从头部解析 Domain（见第 2 节）。

### P2 / `write_cookie_clipboard`（约 1326-1360 行）+ 全平台 / 「一键复制 Cookie」无清理时机

- 依据：桌面端用 arboard 写剪贴板，macOS `exclude_from_history`、Windows `exclude_from_cloud().exclude_from_monitoring()`，均无定时清理；Android 仅置 `EXTRA_IS_SENSITIVE`（API 33+）；iOS `localOnly` 且无 `expirationDate`；前端 `main.ts:3917` 的 1800ms 定时器只恢复按钮文案，不碰剪贴板。复制出的 Cookie（含会话 Cookie）在剪贴板里一直留到被覆盖。
- 建议：复制后 60 秒用空内容覆盖（仅当剪贴板仍是本次 Cookie 时才清，避免清掉用户后续复制的内容）；iOS 直接给 `setItems` 加 `expirationDate`。

### P2 / `push_validated_imported_cookie`（约 1372-1450 行）/ `SameSite=None` 可不带 `Secure`

- 依据：JSON 导入时 `secure` 默认 true，但显式 `"secure": false, "sameSite": "none"` 会原样保留；生成的头含 `SameSite=None` 且无 `Secure`，Chrome/Firefox 会直接丢弃该 Cookie（静默失败，用户以为导入成功）。
- 建议：`same_site == "none"` 时强制 `secure = true`（或丢弃该条并提示）。

### P2 / `push_validated_imported_cookie` / 父域 Cookie 被拒（可用性备注）

- 依据：域名校验用 `host_matches(cookieDomain, allowed)`，要求 Cookie 的 Domain 等于或为 allowed host 的子域。`Domain=hark.com` 的 Cookie 导入到 `app.hark.com` 账号时会被拒（`host_matches("hark.com", "app.hark.com")` 为 false），而按 RFC 这是合法的父域 Cookie。方向反了不会导致越权，只会误拒。
- 建议：允许 Cookie Domain 为 entry host 的父域（同时仍要求父域本身在 allowed 列表的某个条目下或等于其一），并补测试。

### P2 / `parse_account_proxy`（约 327-375 行）/ 代理 URL 处理基本正确，两处纵深备注

- 已确认好：拒绝 userinfo（`username/password` 必须为空）、只允许 `http://host:port`、拒绝 path/query/fragment、主机字符白名单；无凭据可落盘（`accounts.json` 里只存 `http://host:port`）；代码库中无任何 `log` 宏，不存在把代理 URL 写日志的路径；下发前端的也只有 `format_account_proxy` 的 host:port 形式。
- 备注 1：Windows 上代理拼进 WebView2 的 `--proxy-server=` 启动参数（`windows_proxy_browser_args`），本地其他进程用任务管理器 / `wmic process get commandline` 可见代理地址（含端口，无凭据）。可接受，记为备注。
- 备注 2：`test_account_proxy` 的错误分支把 `reqwest::Error` 原样拼进返回消息（`format!("请求失败: {err}")`），错误文本可能含目标 URL；只发给前端展示，不写日志，风险低。

### P2 / `read_macos_cookies_for_url`（约 1240-1315 行）/ 复制 Cookie 不校验 path

- 依据：macOS 侧复制时校验 domain（点号边界后缀匹配）与 path（`path_matches`）；但 iOS 侧 `copyProfileCookie` 只校验 domain，不校验 path。复制出的是「该域全部 Cookie」，含非入口路径的 Cookie。用户主动复制场景，影响小。
- 建议：iOS 侧与 macOS 对齐，加 path 校验。

### 已确认好的地方（platform.rs）

- Cookie 域名过滤严格：`host_matches` 为精确相等或点号边界后缀匹配；`evilmuse.ai`、`muse.ai.evil.com`、`google.com` 均被现有测试与本次复核拦下（`parse_imported_cookies_supports_json_header_netscape_and_filters_domains` 通过）；`__Host-` 强制去 Domain/Path=/ /Secure，`__Secure-` 强制 Secure；过期 Cookie 跳过；`max_age` 夹在 60 秒～365 天；名字符白名单（ASCII 可见字符，禁 `;` `=`）、值禁 `\r` `\n` NUL。
- `verify_desktop_profile_binding` 在写 Cookie 前核对 macOS 的 `WKWebsiteDataStore.identifier` 与 Windows 的 WebView2 userDataFolder 是否与 uuid 一致，防串号。
- `delete_desktop_profile`：macOS 调 `removeDataStoreForIdentifier`，Windows/Linux 删 `profiles/<uuid>` 目录。

---

## 4. site_adapters.rs 与注入脚本

文件：`src-tauri/src/site_adapters.rs`（655 行）、`src-tauri/src/session_manager.rs` 的 `LOGIN_CAPTURE_INIT_JS`（约 104-1009 行）、`IDENTITY_PROBE_JS`（约 1160 行起）、`NET_HOOK_INIT_JS` 注入点（约 6490-6513、6598-6600 行）。

### P1 / `NET_HOOK_INIT_JS`（site_adapters.rs:344-596）无 host 门禁，主框登录页也会读响应体

- 依据：
  - `NET_HOOK_INIT_JS` 内没有任何 `isPrimaryHost` 判断，对 `fetch` / `EventSource` / `WebSocket` 全量挂钩：POST 且 `content-type` 为 json/text/stream 的响应会被 `consumeStream(resp.clone(), …)` 整段读出（上限 256KB，超限 `reader.cancel()`）；`status >= 400` 的响应会被 `resp.clone().text()` 读出并存 `errorSnippet`（前 240 字符）；`entry.url`（前 220 字符）、method、status、content-type 全部记入 `window.__maamNet.entries`（最多 40 条）。
  - 它只注入主框（`session_manager.rs:6513`），不注入弹窗；但主框导航白名单包含 `facebook.com`、`appleid.apple.com`、`idmsa.apple.com`、`meta.com`、`instagram.com`（`providers.rs:builtin_provider_specs` 的 muse allowed_hosts），即主框完全可能在 Apple ID / Facebook 登录页加载。此时登录接口的 POST 响应体会被读出并做 `pickTextFromJson` 提取。
  - 缓解因素：`pickTextFromJson` 只提取已知助手文本字段（`choices/delta/text/completion/answer/reply` 等），不直接存 token；`errorSnippet` 只存前 240 字符的原始错误文本。故定为 P1 而非 P0。
- 建议：与 `LOGIN_CAPTURE_INIT_JS` 对齐，在 `NET_HOOK_INIT_JS` 顶部加 `isPrimaryHost` 门禁（muse.ai / hark.com 及其子域），非主域只记录 URL/method/status，不读响应体；或把响应体读取限制在 `chat_api_patterns` 命中的 URL。

### P1 / `window.__maam*` 全局变量可被页面 JS 伪造，Rust 侧直接信任

- 依据：身份、额度、群聊回复全部经由页面可写的普通全局变量传递，Rust 侧不做二次校验：
  - `window.__maamApiEmail` / `__maamTypedIdentity`：探针（`IDENTITY_PROBE_JS`、Android/iOS 同款）读到的邮箱直接作为账号别名与身份（`apply_detected_metadata`，`session_manager.rs:8742-8873`）。页面任意脚本（含首方页面被 XSS 或恶意注入）可预置该值，导致账号被标成任意邮箱。
  - `window.__maamQuotaDetail` / `__maamQuota`：同理可伪造额度展示；`localStorage` 里的 `__maam_cached_quota_detail_v2__` 也可被页面读写，探针会回读它。
  - `window.__maamNet.entries`：群聊引擎在 `session_manager.rs:3104` 读 `window.__maamNet` 取助手流式回复；页面脚本可向 `entries` 塞入伪造条目，让群聊记录出现假回复。
  - `window.__maamAuthCheck.status`：探针据此判定 loggedOut / stale；页面脚本置 `ok` 可压住登出检测，置 `invalid` 可触发登出（含 iOS 的自动 GET 登出）。
  - `window.__maamOrigFetch`：`__maamRefreshHarkQuota` / 探针用它发 `/api/profile` 等请求，页面脚本可替换它返回伪造 JSON。
  - 另：`LOGIN_CAPTURE_INIT_JS` 用 `Object.defineProperty(globalThis, 'TURBOPACK', …)` 拦截 Turbopack 运行时（`configurable: true`），页面脚本可再定义一次抢占；`__maamRefreshBackendQuota` / `__maamRefreshIdentity` 在 muse.ai 上用硬编码兜底 action id `'40e0249eeae158685b6a8d0c3b203f53dd4f2c7d63'` 调 `callServer`（`session_manager.rs` 约 610 行处），站点构建变化后可能调到错误的 server action（见 P2）。
- 建议：身份类结论改由 Rust 侧用 profile 的 Cookie 直接请求 `/api/auth/check` 或 `/api/profile` 校验（已有 `read_macos_cookies_for_url` / `verify_desktop_profile_binding` 的基础设施），页面值只作展示 hint；额度/群聊回复至少做来源校验（如只接受同源 `location.origin` 下的 fetch 结果——现状 `inspectText` 已只看同源 `reqPath`，但 `__maamNet.entries` 本身仍可被写）；把关键全局变量用 `Object.defineProperty(…, {writable: false, configurable: false})` 封存（能抬高门槛，不能根治——同 realm 脚本总有办法，根本解是 Rust 侧校验）。

### P2 / `LOGIN_CAPTURE_INIT_JS` 的输入监听在所有页面运行（含 IdP 登录页）

- 依据：`capture` 的 `input/change/blur/submit` 监听在 `if (isPrimaryHost) {…}` 块**之外**，且 `LOGIN_CAPTURE_INIT_JS` 同时注入主框和弹窗（`session_manager.rs:6511, 6598`）。因此在 Apple ID / Facebook / Meta 登录页 typed 的邮箱与手机号会被记入 `window.__maamTypedIdentity` / `__maamTypedPhone` 与该站点的 `sessionStorage`，探针再读回 Rust。这是登录捕获的有意设计（用于识别登录的是哪个账号），但应明确记录：应用会读取第三方 IdP 登录页的输入内容。
- 对照：`LOGIN_CAPTURE` 的 fetch/WebSocket 钩子在 `isPrimaryHost` 块内，**不**在 IdP 页读响应体；`IDENTITY_PROBE_JS` 在 `accountscenter.meta.com` 会从 body 文本与 `script[type="application/json"]` 里取邮箱（有意设计），在其余 IdP host 上只返回 loggedOut/loading，不扫 body。
- 建议：文档注明该行为；技术上可把 `capture` 限制为仅在 `isPrimaryHost || isExternalLoginHost` 时注册（现状等于全站，含 `idmsa.apple.com` 等未列入 `isExternalLoginHost` 的域）。

### P2 / `NET_HOOK_INIT_JS` 读取非聊天 POST 的响应体（过度收集）

- 依据：`consumeStream` 的触发条件是「POST + json/text/stream」，不限于 `chat_api_patterns`（`/api/`、`/chat` 等）。设置、偏好等接口的 POST 响应也会被读出并做 `pickTextFromJson`。
- 建议：只对命中 `chat_api_patterns` 的 URL 读响应体；其余只记 URL/method/status。

### P2 / `site_adapters.rs:257` `host_matches_pattern` 与 `merge_adapters`

- 依据：`"*"` 通配整个 generic 兜底（有意设计）；`"*.muse.ai"` 同时匹配 `muse.ai` 本身（`h == suffix`）；`merge_adapters` 允许覆盖文件替换任意内置 adapter（含把 `muse` 的 `host_patterns` 改成 `*`）。覆盖文件 `site_adapters.json` 位于 app_data_dir，用户可写，属本地配置行为。
- 建议：覆盖文件校验时拒绝把内置 id 的 `host_patterns` 改为 `*` 或空；或至少在快照里标注「内置规则被覆盖」。

### P2 / 硬编码 server action id 兜底

- 依据：`LOGIN_CAPTURE_INIT_JS` 的 `__maamRefreshBackendQuota` 在没扫到 `fetchSubscriptionAction` 时用 `'40e0249eeae158685b6a8d0c3b203f53dd4f2c7d63'`（Android 的 `IDENTITY_PROBE_SCRIPT` 同款）。站点构建一变，该 id 可能对应别的 server action，`callServer(aid, [{includeAgreement: true}])` 可能调错接口。
- 建议：去掉硬编码兜底；扫不到时只用已缓存的额度并提示同步失败。

### 已确认好的地方（注入脚本）

- `LOGIN_CAPTURE_INIT_JS` 的 fetch/WebSocket 钩子有 `isPrimaryHost` 门禁；`NET_HOOK_INIT_JS` 不注入弹窗（桌面端弹窗只有 `LOGIN_CAPTURE_INIT_JS` + 时区脚本）；Android/iOS 的 quota hook fetch 门禁与之一致。
- `classify_top_level_url`（`providers.rs:210`）主框只放 https + allowedHosts，`accounts.google.com` 一律走外部浏览器；`blob:`/`about:blank` 仅放行空白类。
- 探针的邮箱正则、bad 前缀表（support/noreply 等）与 `@example.com` / `@sentry.io` 排除在三端一致。

### 备注：插件命令不直接暴露给前端

- `src-tauri/capabilities/manager.json` 只授权了 Rust 命令（`allow-copy-account-cookie` 等），`account-session` 插件自身无 `permissions/` 定义，移动端 `importProfileCookies` / `deleteProfile` 等只能经 Rust 侧 `platform::mobile::*` 调用，前端无法直接 invoke 绕过 Rust 校验。结论基于代码与 capability 清单，未做运行时越权测试。

---

## 5. installer.nsi

文件：`src-tauri/nsis/installer.nsi`（1030 行，Tauri 模板 + 少量定制）。`utils.nsh`（`CheckIfAppIsRunning`、`RunAsUser` 实现）不在仓库内，是 Tauri 打包器提供的模板文件，本轮无法审计其实现，以下涉及它的结论以模板行为为准并标注。

### P2 / `installer.nsi:372-409` / 卸载路径取自注册表，未校验即信任

- 依据：重装页 `PageLeaveReinstall` 从 `SHCTX\Software\<MANUFACTURER>\<PRODUCTNAME>` 的默认值读 `$4`（上次安装目录），随后 `ExecWait '$R1 _?=$4'`、`Delete "$4\uninstall.exe"`、`RMDir "$4"`；`RestorePreviousInstallLocation` 也把 `$INSTDIR` 置为该值。注册表值被篡改（同用户写 `HKCU` 即可）时，安装器会在该目录删一个 `uninstall.exe`、并把新版本装进去。`RMDir` 无 `/r`，只删空目录， blast radius 有限。
- 建议：使用 `$4` 前校验该目录下存在本应用的 `uninstall.exe` 与主程序（或要求路径位于 `%ProgramFiles%` / `%LOCALAPPDATA%` 下），否则忽略并用默认目录。

### P2 / `installer.nsi:206-216` / WiX 卸载串按 DisplayName+Publisher 匹配后直接执行

- 依据：`FindWixInstall` 遍历 `HKLM\…\Uninstall`，DisplayName+Publisher 匹配即取其 `UninstallString` 并 `ExecWait` 执行。模板注释已承认该风险（「user will have to confirm」）。同名产品的恶意注册表项可借安装器提权执行。
- 建议：执行前校验 `UninstallString` 指向的确实是 `msiexec /x {GUID}` 且 GUID 与预期一致；或去掉 WiX 迁移分支（本项目是否还有 WiX 用户待确认）。

### P2 / `installer.nsi:610-616` / WebView2 bootstrapper 下载后无哈希校验

- 依据：默认 `installWebview2Mode` 未在 `tauri.conf.json` 中配置，即 Tauri 默认的 `downloadBootstrapper`；安装器用 `NSISdl::download "https://go.microsoft.com/fwlink/p/?LinkId=2124703"` 下载后直接 `ExecWait … /install`，无哈希/签名校验（只有 TLS 保护）。
- 建议：改用 `embedBootstrapper` 或 `offlineInstaller` 并在 CI 校验哈希；或在模板里对下载文件做签名校验（属上游模板默认行为，改动需同步上游模板）。

### P2 / `installer.nsi:830-960` / 卸载递归删除路径是编译期常量，但无空值保护

- 依据：卸载时递归删除的只有 `$APPDATA\${BUNDLEID}` 与 `$LOCALAPPDATA\${BUNDLEID}`（`BUNDLEID = com.ldjx.accountmanager`，来自 `tauri.conf.json` 的 `identifier`，编译期填入），其余删除均为固定文件名 + 非递归 `RMDir $INSTDIR`。污染面小。
- 建议（纵深）：递归删除前断言 `BUNDLEID` 非空且不含 `..` / `\` / `/`（模板变量被误配为空时 `RmDir /r "$APPDATA\"` 会酿成灾难）；`$INSTDIR` 的递归删除只应在校验其包含 `uninstall.exe` 后执行。

### P2 / `.onInit`（约 519-531 行）/ `/P`、`/UPDATE` 解析无值校验

- 依据：`${GetOptions} $CMDLINE "/P"` 等只判断开关存在与否。`/P`（passive）与 `/UPDATE` 组合会跳过全部页面与卸载前确认；`/R` 只在 `.onInstSuccess`（约 797-809 行）且 passive/silent 时处理，用 `RunAsUser` 以普通用户身份重启应用（`/ARGS` 透传）。
- 建议：`/UPDATE` 模式下仍应校验安装包来源（与 P0-2 的安装前校验衔接）；`/ARGS` 的值应做长度与字符白名单校验（现状原样拼进 `RunAsUser` 的命令行）。

### 已确认好的地方（NSIS）

- 静默安装权限：`tauri.conf.json` 未配置 `installMode`，即 Tauri 默认 `currentUser`（已用 Tauri 官方 `tauri-utils` 源码 `NSISInstallerMode::default() = CurrentUser` 核对）；对应 `RequestExecutionLevel user`，安装到 `%LOCALAPPDATA%\Diehao`，**不提权、无 UAC**；更新流程 `/P /UPDATE /R` 全程以普通用户身份运行，`.onInstSuccess` 用 `RunAsUser` 重新拉起应用，不会以管理员身份跑应用。
- 降级保护：`ALLOWDOWNGRADES=false` 时 silent/passive/update 模式会先比对版本并拒绝降级。
- 卸载时 `UpdateMode=1` 会保留快捷方式、Run 键与应用数据；「删除应用数据」复选框默认不勾选，勾选后才删 `$APPDATA / $LOCALAPPDATA\<bundleId>`；删除前 `CheckIfAppIsRunning`（RestartManager）处理运行中的程序。
- `INSTALLDIRNAME` 硬编码为 `Diehao`，与产品名「叠号」不一致但无安全影响（仅安装目录名）。

---

## 6. session_manager.rs 并发

文件：`src-tauri/src/session_manager.rs`（13986 行）。逐个核查任务书点名的 5 处 `std::sync::Mutex::lock().unwrap()`，并全文件检索其余 `std::sync::Mutex` 用点。

### P2 / 5 处 `lock().unwrap()`：中毒不可达，但应改成 `into_inner()`（5 处逐个判断）

| 位置 | 守卫内容 | 临界区操作 | 结论 |
|---|---|---|---|
| 6364 `record_recent_download` | `recent_downloads: Mutex<HashMap<u64, PathBuf>>` | `insert` / `remove`（`min()` 取键） | 无 panic 路径；中毒不可达。P2 |
| 6405 `handle_desktop_download_event`（`Requested`） | `pending: Mutex<HashMap<String, VecDeque<…>>>` | `entry().or_default().push_back()` | 同上。P2 |
| 7124 `eval_paint_ready` | `tx_cell: Mutex<Option<oneshot::Sender>>` | `take()` 后 `send(raw)` | `send` 不 panic；回调在 wry 回调线程跑，`unwrap()` 一旦触发会跨 FFI unwind。P2 |
| 8402 `eval_identity_probe` | 同上 | 同上 | 同上。P2 |
| 10737（第三处同款 oneshot 回调） | 同上 | 同上 | 同上。P2 |

- 共同依据：临界区内只有 `insert/remove/push_back/pop_front/take/send`，均无 panic 路径；守卫均为语句级作用域（`if let Some(sender) = tx_inner.lock().unwrap().take()` 的临时守卫在 `send` 后即释放），**没有任何一处在持有 `std::sync::Mutex` 守卫期间 `.await`**。
- 同款模式另见 `platform.rs:975`（`configure_windows_timezone`）与 `platform.rs:1930`（`delete_desktop_profile` 的 macOS 分支），结论相同。
- 建议：全部 `lock().unwrap()` 改为 `lock().unwrap_or_else(|e| e.into_inner())`（一行改动，消除中毒后 panic 的可能，尤其回调在 wry/WebView2 COM 线程上时 panic 会跨 FFI 导致 abort）。

### P2 / `group_member_locks` 获取顺序：未发现死锁，但有一处活锁风险备注

- 依据（`session_manager.rs:11540-11670` 附近）：
  - 唯一获取点：先 `self.inner.lock().await`（tokio Mutex）取出 `Arc<tokio::sync::Mutex<()>>`，**释放 inner 后**再 `member_lock.lock_owned().await`；不存在 inner 持有期间取 member 锁的路径。
  - 持有 member 锁期间会 `.await`（执行 webview 回合、取 `group_worker_semaphore`、再取 inner），顺序恒为 member → semaphore → inner，无反向边。
  - 跨成员嵌套不存在：A2A 接力在 `run_group_a2a_chain`（约 12602-12770 行）里等当前成员回合返回（锁已释放）后才 `tauri::async_runtime::spawn` 下一跳；`run_group_member_turn_with_failover`（约 10322 行）的 failover 也是顺序重试（先释放 A 的锁再取 B 的锁）；`visited` 集合保证单链不重复。
  - 信号量 `GROUP_MAX_CONCURRENT_WORKERS = 6`：持许可的任务不再等 member 锁（先取锁后取许可），无循环等待。
- 建议：无死锁可修。备注：member 锁会持有整个 webview 回合（可达数分钟），同一成员的后续回合会在此排队；若 webview 挂起无响应，该成员的群聊会被长期阻塞——建议给 member 回合加总超时（与 P1-5/P1-6 的超时治理衔接）。

### P2 / `tokio::sync::Mutex`（`inner`）持有期间的 `.await`

- 依据：`inner` 是 tokio Mutex，持有期间 `.await` 是合法的（如 `save_store_atomic` 在持锁时同步调用，见 P1-8）；但持锁期间调用 `apply_desktop_layout_locked` 等同步函数无 await。未发现持 `inner` 时取第二个 tokio 锁后又反向取 inner 的路径。
- 建议：无改动；P1-8 的写盘阻塞问题另案处理。

---

## 7. README 与 latest.json

### 已确认：CI 生成的 `latest.json` 四个平台都有 `sha256`

- 依据：`.github/workflows/release.yml:239-306`：`mac_sha / win_sha / apk_sha / ipa_sha` 均用 `sha256sum` 对发布文件实测后由 `jq -n` 写入 `platforms.{macos,windows,android,ios}.sha256`；任一产物缺失时 `exit 1`，不会生成缺字段的清单。
- 对应消费端：`updater.rs:352,406` 的 `extract_sha256_digest` 会解析该字段（缺失/格式错误时返回 `None`，随后跳过校验——这是 P0-2 的问题，不是清单的问题）。

### P2 / 清单生成无空哈希保护

- 依据：`mac_sha=$(sha256sum … | awk '{print $1}')` 未校验结果为 64 位 hex；若 `sha256sum` 失败（配合 `set -e` 缺失时），`latest.json` 会含 `"sha256": ""`，消费端静默跳过校验。
- 建议：在 `jq` 之前断言四个 `*_sha` 均匹配 `^[0-9a-f]{64}$`，否则失败本次发布。

### P2 / README 的 SHA-256 表述与现状不符

- 依据：`README.md:30` 称「客户端内 GitHub Releases `SHA-256` 校验覆盖更新」；`README.md:126` 指导用户用 `latest.json` 的 `platforms.windows.sha256` 核对安装包。但现状是：`sha256` 缺失时桌面与 Android 都跳过校验（P0-2 / 第 1 节 P0），「校验」并非无条件成立。
- 建议：P0-2 修完（sha256 必填）后再保留该表述；或现阶段改为「有 sha256 时校验」。

---

## 附：新发现项汇总（任务书第 3 节清单之外）

1. Android APK 更新 sha256 可空（第 1 节 P0）。
2. Android FileProvider 未在仓库内声明（第 1 节 P1，待实测）。
3. 桌面端 `NET_HOOK_INIT_JS` 在主框 IdP 登录页读响应体（第 4 节 P1）。
4. `window.__maam*` 全局变量伪造（第 4 节 P1）。
5. 时区脚本注入验证码 iframe（第 3 节 P1，待实测）。
6. 剪贴板 Cookie 永不清（第 1/2/3 节 P2，全平台）。
7. Cookie 值 `;` 注入（第 3 节 P2，已用测试验证）。
8. iOS 子框导航不限 host、iOS 无下载实现（第 2 节 P2）。
9. Android HTTP 下载无大小上限、允许 https→http 降级（第 1 节 P2）。
10. NSIS 注册表安装目录信任、WiX 卸载串执行、WebView2 无哈希下载（第 5 节 P2）。
11. iOS 删除 Profile 与存活 WebView 的竞态（第 2 节，待实测）。
12. `SameSite=None` 不带 `Secure` 被浏览器静默丢弃（第 3 节 P2）。

## 验证命令及结果

- `grep` / 人工阅读：全部 7 项（文件与行号见各节）。
- Rust 单元测试（临时探针，用后即删，已用 `git checkout` 还原，工作树只剩报告文件）：
  - `export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR="$HOME/workspace/.cargo-target-maam" CARGO_BUILD_JOBS=2`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib platform::tests::`
  - 结果（2026-11-26 实测，编译 5m46s，共享 target 目录排队所致）：`5 passed; 0 failed`——`parse_and_format_account_proxy_rules`、`extract_identity_hint_from_cookies_handles_urlencoded_jwt_and_filters_noise`、`parse_imported_cookies_supports_json_header_netscape_and_filters_domains`、`summarize_cookie_expiry_prioritizes_auth_cookies_and_ignores_noise` 全部通过；临时探针 `tmp_review_semicolon_injection_probe` 通过，证实值 `abc; Domain=evil.example; Path=/x` 经 `parse_imported_cookies` → `imported_cookie_to_set_cookie_header` 后输出头确实包含 `Domain=evil.example`（第 3 节 P2 的依据）。
  - 编译另有 2 处既有 warning（与本轮无关，未动），其中一处为 `session_manager.rs:8036` 未使用的 `proxy` 变量。
- 未跑：`npm run tauri dev` / 安装运行本应用（任务禁止）；Android/iOS 真机验证（无设备）；NSIS 打包验证（需 Windows）。

## Commit 列表

- 无。本轮只出报告，未改任何代码（临时测试探针已还原，工作树干净）。
