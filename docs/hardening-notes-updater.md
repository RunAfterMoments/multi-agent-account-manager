# 加固笔记：更新器（updater / P0-0 + P0-2）

分支：`hardening/updater-integrity`，基线 v0.1.10（`0416f41`）。
范围：只改 `src-tauri/src/updater.rs` 及其直接相关的设置（`storage.rs` / `session_manager.rs` / `lib.rs` 的 `update_app_settings`）与前端开关（`src/main.ts`、`src/types.ts`）。不碰 `.github/workflows/release.yml` 的签名与密钥逻辑，不碰上游仓库。

## 逐条结论

### P0-0 更新源写死上游仓库 —— 已确认，见各 commit

- 基线 `updater.rs` 开头三个 const 写死 `ldjx7/multi-agent-account-manager`（`LATEST_MANIFEST_URL` / `GITHUB_LATEST_API_URL` / `RELEASES_PAGE_URL`），已确认。
- 改为编译期可配置：`option_env!("MAAM_UPDATE_REPO")`，格式 `owner/repo`，非法值按未配置处理。
- 未配置时：`check_for_updates` 直接返回 `updatesDisabled: true` 的结果（不发任何网络请求）；`download_and_install_update` 直接拒绝；「打开 Releases 页面」按钮因无仓库地址可指而隐藏——任务书所说的「只保留打开 Releases 页面」在无仓库时无地址可打开，故实际行为是更新入口只展示一条未配置说明；配置仓库后恢复正常（检查 + 一键安装 + 发布页）。
- 设置新增「禁用自动更新检查」开关（`disableAutoUpdateCheck`），默认 **false**（即配置了仓库时保持原有自动检查行为；未配置仓库时无论开关如何都不检查）。默认值取 false 的理由：不改对外行为；自动检查本身只读 GitHub 公开 release 信息，风险低于一键安装；真正起决定作用的是「未配置仓库即关闭」。

### P0-2-1 sha256 必填 —— 已确认

- 桌面端：基线 `download_to_updates_cache` 中 `if let Some(want) = expected_hex`，无 sha256 时直接跳过校验，且缓存复用也不校验，已确认。
- Android 分支：`AccountSessionPlugin.kt` 的 `downloadAndInstallApk` 同样把 sha256 当可选（`expectedSha == null` 时可复用旧文件、下载后跳过比对），已确认。**第二轮审查（`~/workspace/maam-wt-review/docs/hardening-notes-review.md` 第 1 节，P0）复核确认了同一条路，并补了上游链路：`updater.rs` 的 `extract_sha256_digest` 在 `latest.json` 缺少/损坏 `sha256` 时返回 `None`，空值一路透传到 Kotlin；`check_for_updates` 的 GitHub API 兜底分支同样可能产出 `None`。**
- **堵住方式（Rust 侧单点关口）**：`download_and_install_update` 是唯一调用插件 `downloadAndInstallApk` 的入口（Rust 内仅 `updater.rs` 一处调用，前端只调 Tauri 命令 `download_and_install_update`）。修复后该函数在任何平台分支执行前先强制要求 sha256：`InstallUpdateRequest.sha256` 缺失/非法 → 直接拒绝；清单重新拉取后 `sha256` 为 `None` → 拒绝安装。因此空 sha256 永远到不了 Kotlin 侧，全新下载与缓存复用两条路都被堵住。`download_to_updates_cache`（桌面端）同样改为必填参数。
- Kotlin 侧未改：`DownloadApkArgs.sha256` 仍是 `String?` 可空，插件内 `expectedSha == null` 的放行逻辑仍在。按本次任务范围（只改 `updater.rs` 及其直接相关的设置/前端开关）与本地无法编译验证 Android 的限制，Kotlin 侧改为必填（`expectedSha == null` 时拒绝安装并删除已下载文件、复用分支一律重算摘要）列为后续跟进项；只要不绕开 Rust 命令，该路径不可达。

### P0-2-2 后端不再信任前端传值 —— 已确认

- 基线 `InstallUpdateRequest` 的 `download_url` / `sha256` / `filename` 全部来自前端，`download_and_install_update` 只做 URL 主机校验就直接下载，已确认。
- 修复后：安装时先重新调 `check_for_updates` 取最新清单（当前平台），只接受 `version` 等于清单 `latestVersion`、且 `download_url` / `sha256` / `filename` 与清单完全一致的请求；任一不一致或清单缺失即拒绝。
- 行为变化：安装时多一次清单网络请求；离线或 GitHub API 限流时安装会失败并提示重新检查更新（之前是直接用前端缓存的 URL 下载）。清单以安装时刻重新拉取的为准，前端旧的检查结果不再被信任。

### P0-2-3 URL 前缀固定 —— 已确认

- 基线 `validate_download_url` / `is_allowed_github_host` 只校验 https 加 GitHub 主机（含 `*.github.com` / `*.githubusercontent.com`），不限定仓库路径，任何 GitHub 用户的 release 文件都能通过，已确认。
- 修复后：URL 必须以 `https://github.com/<配置的仓库>/releases/download/` 开头（大小写不敏感比较前缀、拒绝含 userinfo 的 URL、路径后必须有文件名段）。
- 单元测试覆盖：其他用户仓库、`http://`、`https://github.com.evil.com/`、`https://github.com@evil.com/`、`https://evilgithub.com/`、合法的 `objects.githubusercontent.com` 重定向目标（注：重定向目标不走 `validate_download_url`，由重定向策略单独校验，见下条）。

### P0-2-4 每跳重定向校验 —— 已确认

- 基线 `build_http_client` 用 `redirect(Policy::limited(10))`，只在最终响应检查 scheme，不检查每一跳主机，已确认。
- 修复后：`redirect::Policy::custom`，每一跳都要求 https 且主机在白名单内：`github.com`、`objects.githubusercontent.com`、`release-assets.githubusercontent.com`，另加 `api.github.com`（见下）。
- 与任务书的偏差：白名单多了一个 `api.github.com`。原因是 `build_http_client` 同时服务于清单 API 请求（`https://api.github.com/...`）；若该主机自身重定向（如 API 侧 301 到同主机另一路径），按任务书的三主机白名单会被误拦。初始请求 URL 本身不受重定向策略约束，只校验跳转目标，故加 `api.github.com` 不扩大下载源，只保证 API 自身跳转不断。

### P0-2-5 去隔离属性放在哈希校验之后 —— 部分确认

- macOS：基线 `install_macos_update` 在解压后（`spawn_blocking` 内）对新 `.app` 执行 `xattr -dr com.apple.quarantine`，swap 脚本成功后又执行一次。严格说，下载步骤（`download_to_updates_cache`，含哈希校验）确实先于安装步骤执行，所以「先校验、后去隔离」的时序在 sha256 存在时本就成立；真正的漏洞是 P0-2-1：sha256 缺失时校验被跳过，去隔离随后照常执行。修复 sha256 必填后该时序漏洞即被堵上；同时为彻底消除歧义，删掉了安装步骤里解压后立即去隔离的那一次，只保留 swap 脚本成功替换后的那一次（替换失败时不再去隔离）。
- Windows：基线 `install_windows_update` 只在 `download_and_install_update` 下载成功后才启动安装程序，时序同样依赖 sha256 必填；修复后哈希不通过即不启动安装程序。无额外代码改动。

### P0-2-6 长期签名方案 —— 只写方案，不改 release.yml

见文末「长期更新签名方案」一节。

### P0-2-7 swap 脚本回滚与超时 —— 已确认

- 基线回滚条件 `[ -e "$backup" ] && [ ! -e "$dst_app" ]`：`mv` 跨卷失败留下半截 `$dst_app` 目录时条件不成立，跳过回滚，应用损坏，已确认。
- 基线等待旧进程退出的循环（100 × 0.1s = 10s）超时后仍继续替换运行中的应用，已确认。
- 修复后：等待循环结束后若旧进程仍在，脚本直接 `exit 2` 中止，不做任何替换；替换失败时先 `rm -rf "$dst_app"` 再把备份移回。
- 局限（手工验证时注意）：swap 脚本是 detached 启动，父进程 350ms 后即退出，脚本的退出码无人读取；超时中止时用户看到的是旧应用未重启、更新未完成，需手动重新打开应用再试。备份在失败路径上会被保留（不删除），可手动恢复。

### P0-2-8 is_newer_version 预发布比较 —— 已确认

- 基线手写比较只在核心版本号相等且当前为预发布、新版不是时才返回 true，`1.0.0-rc.1 → 1.0.0-rc.2` 返回 false，已确认。
- 修复后：改用 `semver` crate（`semver = "1"`，已在 `Cargo.lock` 间接依赖中存在，无新增重型依赖）；`1.0` / `1.0.0` 等缺段版本号先补齐再比较；无法解析为 semver 的字符串回退到旧的数值比较。
- 注意：实测 `semver` 1.0.28 的 `Ord` 会比较构建元数据（`1.0.0+build1 < 1.0.0+build2`），与 SemVer 规范「构建元数据在比较先后时必须忽略」不符，故 `parse_semver` 在解析前丢弃 `+build` 段。

### P2 新发现：open_external_url 不校验主机 —— 已确认，已修

- P2 第二轮审查指出（`hardening-notes-review.md` 第 1 节末 / `hardening-notes-ci.md`「新发现项」）：`updater.rs` 的 `open_external_url` 只校验 scheme 为 https，不校验主机白名单，前端传任意 https 链接都会打开。已确认成立。
- 依据：唯一调用方是 `src/main.ts` 的更新弹窗「发布页」按钮（`#update-open-page-btn`），URL 取自检查结果的 `releasePageUrl`，即清单 `latest.json` 的 `release_page_url` 或 GitHub API 的 `html_url`，均未经核对。`open_official_site` 是另一条独立命令（走 provider 官网），不受影响。
- 修复后：只允许打开 `https://github.com/<配置仓库>/releases` 或 `.../releases/tag/<tag>`；拒绝其他主机、userinfo、查询参数、片段、仓库根目录、issues、下载地址；未配置更新源时直接拒绝。单元测试覆盖合法页与 9 类非法地址。
- 行为变化：清单里 `release_page_url` 若指向非配置仓库的页面（如上游仓库地址），「发布页」按钮将报错而不是打开；与 P0-0「更新源以编译期配置为准」一致。

## 新发现 / 残留项

1. **Android 重定向不校验主机**（新发现，未修；第二轮审查第 1 节列为 P1）：`AccountSessionPlugin.kt` 的 `openHttpConnection` 手动跟随重定向，每跳只校验 https，不校验主机。桌面端已修，Android 端因本次任务范围只到 Rust 侧、且本地无法编译验证 Android，建议后续在 Kotlin 侧对每一跳加与桌面端相同的白名单（`github.com` / `objects.githubusercontent.com` / `release-assets.githubusercontent.com` / `api.github.com`）。风险有限：初始 URL 已是清单校验过的 `github.com/<仓库>/releases/download/...`，攻击者需先攻破 GitHub 的重定向链才能利用。审查报告给出的完整攻击链（清单被重定向到攻击者 JSON → downloadUrl 指向攻击者 APK → 无 sha 校验直接安装）中，后两环已被本次 Rust 侧修复堵住（清单重取校验 + sha256 必填），只剩重定向主机本身未在 Kotlin 侧设防。
2. **Android 插件内 sha256 仍是可选参数**（已知，未在 Kotlin 侧改）：Rust 侧已在调用前强制要求，插件内的可选逻辑现为纵深防御缺口而非可达路径；如后续直接调用插件（绕开 Rust 命令）仍可跳过校验，建议 Kotlin 侧也改为必填。
3. **哈希与安装包同源**（任务书已指出，长期方案见下）：sha256 来自同一 release 清单，只能防传输损坏，防不了发布源被攻破。未修，需签名方案落地。

## 长期更新签名方案（P0-2-6，不改 release.yml）

目标：即使 GitHub Release（清单 + 安装包）被攻破，客户端也不安装伪造更新。

方案（Tauri updater 兼容路线，不引入新重型依赖；与现有 `latest.json` 手工清单机制并行演进）：

1. **签名对象**：CI 发布时对两个产物签名——`latest.json`（清单本身）和每个平台安装包（`.app.tar.gz` / `-setup.exe` / `-release.apk` / `.ipa`）。签名算法选 **minisign**（Ed25519，签名小、验证实现可直接用 `minisign-verify` 思路或 `ed25519-dalek`；也可直接用 Tauri updater v2 的签名格式，二者选一，勿混用）。
2. **密钥管理**：
   - 私钥只存在 CI secret 中，不进仓库、不进产物。
   - 公钥编译进二进制（`option_env!` / `include_str!`，与 `MAAM_UPDATE_REPO` 同一配置思路），随版本发布；轮换公钥即发新版。
3. **验证顺序**：下载清单 → 验清单签名（失败即停）→ 按清单下载安装包 → 验 sha256 → 验安装包签名（失败即删包、停）→ 再去隔离 / 启动安装程序。
4. **回滚与兼容**：旧版客户端无公钥，无法验证新签名，需在发布说明中注明「从 x.y.z 之前版本升级须手动下载一次」；或保留一段过渡期：清单同时带签名与未签名哈希，旧版走哈希、新版强制签名。
5. **所需 CI secret**（写进 release.yml 前需先在仓库 Settings → Secrets and variables → Actions 中创建，名称建议）：
   - `UPDATE_SIGNING_PRIVATE_KEY`：minisign 私钥全文（`minisign -G` 生成的 `.sec` 内容，或 base64 编码后存入，CI 内再解码；切勿用明文日志打印）。
   - `UPDATE_SIGNING_PUBLIC_KEY`：对应的公钥（可公开；编译进二进制用，也可存为仓库变量而非 secret，但为成对管理建议同放 secret，由 CI 在构建时注入 `MAAM_UPDATE_PUBLIC_KEY` 环境变量）。
   - （可选）`UPDATE_SIGNING_KEY_ID`：密钥轮换时标识用哪把公钥验证，便于多公钥并存过渡。
6. **CI 改动点（签名步骤本次仍未做；仓库地址已接通）**：`release.yml` 的「Prepare release assets and latest.json」步骤后加签名步骤（`minisign -S -s "$KEY" -m latest.json` 及各安装包），并把 `.minisig` 文件随产物一同上传 Release；构建 job 注入公钥环境变量。签名密钥只在发布 job 可见，沿用现有 `permissions: contents: write` 仅限发布 job 的做法。**2026-11-26 PR-E 已做**：`release.yml` 顶层 `env` 注入 `MAAM_UPDATE_REPO`，desktop/android/iOS 三个构建 job 全部生效——此前发布产物因编译期未配仓库地址，更新检查默认关闭。**2026-11-26 v6.1 改为 `vars.MAAM_UPDATE_REPO`（默认空）**：Ed25519 验签未落地前，带着已启用更新源发公开版是不安全的（已装上的无验签版本不会验签后续更新）；验签落地后，在仓库 Variables 里设 `MAAM_UPDATE_REPO=<owner/repo>` 再发版。签名本身仍待用户生成密钥对（`minisign -G`，私钥进 CI secret，公钥随构建注入）后落地。

## 手工验证步骤

### P0-0（未配置仓库默认关闭）

1. 不设 `MAAM_UPDATE_REPO` 编译（`cargo test` 即覆盖此情形，见 `parse_update_repo` 测试与 `disabled_check_result` 测试）。
2. 已配置仓库的构建上验证（需真机 / 桌面运行，本次未做——按任务要求不安装/运行应用）：
   - 设 `MAAM_UPDATE_REPO=<自己的 fork，如 user/multi-agent-account-manager>` 后构建，启动后 3 秒内应发起一次静默检查（有 Release 时更新按钮出现红点）。
   - 设置 → 勾选「禁用自动更新检查」→ 保存 → 重启应用，启动后不应再自动检查（更新按钮无红点）；手动点「更新」仍可检查。
   - 不设 `MAAM_UPDATE_REPO` 构建：启动后不发起检查；点「更新」打开弹窗，显示未配置说明，无「立即下载并更新」与「发布页」按钮。

### P0-2-2（安装时重新取清单）

桌面手工验证（需联网 + 自有仓库有 Release，本次未做）：

1. 正常检查更新得到结果后，在安装前于仓库删除/替换该 tag 的安装包（或改 `latest.json` 的 sha256），再点「立即下载并更新」，应被拒绝并提示清单不一致。
2. 用旧版检查结果（已下架版本）点安装，应被拒绝（版本与最新清单不一致）。

### P0-2-5 / P0-2-7（macOS，去隔离与 swap 脚本）

Linux 上无法编译 macOS 分支，以下为 macOS 手工验证：

1. 在 macOS 上以自有仓库构建并安装旧版，发布含正确 sha256 的新版，点更新：更新后新版 `.app` 应能正常启动（Gatekeeper 不拦截，说明去隔离发生在替换成功后）。
2. 故意把 `latest.json` 的 sha256 改错再点更新：应提示校验失败，旧版 `.app` 完好、仍可启动。
3. swap 超时：更新时让旧进程 10 秒内不退出（如挂起），10 秒后 swap 脚本应中止，旧应用仍在原位、可手动恢复使用。

### Windows（哈希通过才启动安装程序）

Windows 上手工验证：同上改错 sha256 再点更新，应只提示校验失败，不应弹出安装程序窗口。

## 验证命令与结果

- `npm ci`：通过（19 个包）。
- `npx tsc --noEmit`：通过。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`：通过，43 passed（基线 26 + 本分支新增 17）。
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`：**基线即不通过，未做全库重排。** 无 `rustfmt.toml` 时 rustfmt 默认 4 空格，全库约 19 万字符 diff；P2 CI 块实测加 `tab_spaces = 2` 后仍有约 200 处 diff、8 个文件。按协调人要求：本分支不重排、不修全部，只保证新增/修改的代码 fmt-clean——已用 `rustfmt --edition 2021 --config-path /tmp/rustfmt-2sp.toml --check`（内容仅 `tab_spaces = 2`）对 `updater.rs` / `storage.rs` / `lib.rs` / `session_manager.rs` 单文件验证，未引入新的 fmt diff（与基线 fmt 输出对比，差异仅为本分支改动引起的行号平移）。待各分支合并后由一个专门的 `style: cargo fmt 全库格式化` commit 清理（P2 CI 块的 `rustfmt.toml` 合入后生效）。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`：**基线即不通过，未修。** P2 CI 块实测基线 lib 19 个错误 + lib test 20 个（`session_manager.rs` 17 处、`platform.rs` 2 处、`lib.rs` 1 处），无一处在本分支改动的 updater.rs / storage.rs。本次实测初版曾报 22/22，多出的 3 处均在本分支新增代码（`MACOS_SWAP_SCRIPT` 在 Linux 非测试目标未使用、两处手写大小写不敏感比较），已全部修复并单独 commit；复测回到基线数 lib 19 / lib test 20，位置与基线一致（`lib.rs` 一处由 :626 平移到 :633，仅因本分支插入行数导致），本分支新增/修改代码 0 处。待合并后专门清理。
- Windows/macOS 专属代码（`cfg(windows)` / `cfg(target_os = "macos")`）在 Linux 上编译不到，只做代码审查 + 手工验证步骤，见各节与下文。
