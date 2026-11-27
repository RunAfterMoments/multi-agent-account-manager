# 叠号加固汇总（hardening/consolidated）

- 基线：v0.1.10，提交 `0416f41`。
- 合并分支：`hardening/consolidated`（从 `main` 新建，7 个加固分支依次合并）。
- 合并顺序：P2-9/P2-10（ci-csp）→ 第二轮审查报告 → P0-0/P0-2（updater-integrity）
  → P1-4/5/6（cli-lifecycle）→ P0-3（auto-approve）→ P0-1（config-load-protection）
  → P1-7/P1-8（api-key-keyring）。
- Commit 总数（`main..hardening/consolidated`）：**62**（各分支 53 + 合并提交 7
  + 合并后修复提交 2）。各分支 commit 数：updater-integrity 12、
  config-load-protection 7、auto-approve 8、cli-lifecycle 9、api-key-keyring 11、
  ci-csp 5、second-round-review 1。
- 改动规模：31 个文件，+7067 / −331 行。
- 分块笔记（7 份原始记录）：各工作树 `docs/hardening-notes-*.md`
  （updater / config / approve / cli / keyring / ci / review）；本文是它们的合成，
  合并中发现的新问题与合并本身的改动只记在这里。
- 规则遵守：本地工作，未 push、未开 PR、未碰上游仓库；未改 `.github/workflows/release.yml`
  的签名与密钥逻辑；未改 Muse / Hark 业务适配逻辑与额度解析；未重写 UI、未引入前端框架；
  未引入重型依赖（新增依赖只有 `keyring`、`semver`；`encoding_rs` 未实际用上）。

## 0. 验证结果总览（Linux 实测，Rust 1.100，Node v24）

| 命令 | 结果 |
|---|---|
| `npm ci` | 通过 |
| `npx tsc --noEmit` | 通过（exit 0） |
| `npm run test:js`（jsdom，自动审批） | 15/15 通过 |
| `cargo test`（全量） | **lib 93 passed / 0 failed**；bin 0 测试；doc 0 测试；exit 0 |
| `cargo fmt --check` | **红（基线即红）**：拆分后 260 处 diff；同配置下基线 200 处；合并完成时 237 处，拆分新增约 23 处（子模块 import 块与签名折叠），见第 4.5 节 |
| `cargo clippy --all-targets -- -D warnings` | **红（基线即红）**：lib 19 / test 20，与基线逐条一致，**0 新增**（合并引入的 3 处与拆分引入的 4 处已修，见第 4.5 节） |

> Windows / macOS 专属代码（`cfg(windows)` / `cfg(target_os = "macos")`、Android Kotlin、
> iOS Swift、NSIS）在 Linux 上编译不到，只做了代码审查 + 手工验证步骤（第 5 节）。
> 凡标注「待实测」的项均未在对应平台跑过，不要当作已验证。

## 1. 逐问题结论

### P0-0 更新源写死上游仓库 —— 确认，已修复

- 基线 `updater.rs` 开头三个 const 写死 `ldjx7/multi-agent-account-manager`，已确认。
- 改为编译期可配置：`option_env!("MAAM_UPDATE_REPO")`，格式 `owner/repo`，非法值按未配置处理。
- **行为变化（默认值）**：**未配置更新源时更新默认关闭**——`check_for_updates` 直接返回
  `updatesDisabled: true`（不发任何网络请求）；`download_and_install_update` 直接拒绝；
  「打开 Releases 页面」按钮因无仓库地址可指而隐藏。配置仓库后恢复正常。
- 设置新增「禁用自动更新检查」开关（`disableAutoUpdateCheck`），默认 **false**
  （配置了仓库时保持原有自动检查行为；未配置仓库时无论开关如何都不检查）。

### P0-1 配置加载失败保护 —— 确认，已修复

- 损坏/读不到/解析失败的配置不再被空配置覆盖：加载失败时原文件先复制为
  `accounts.json.corrupt-<unix_ms>`（同名追加 `-1/-2…`，永不覆盖），内存用空配置启动，
  `persist_blocked=true`，任何写盘尝试都返回 Err，磁盘文件字节不变；
  notice 写明备份文件名与两个选择。备份本身失败时同样阻止写盘。
- 成功加载后滚动备份 `accounts.json.bak`（单份滚动覆盖；首次启动无配置文件时不产生）。
- 高版本配置只读：`version > STORE_VERSION` 时 `read_only=true`、`persist_blocked=true`，
  跳过迁移写盘，绝不降级写回；内存里的账号仍可正常查看。
- 未知 provider 账号保留原始 JSON（`AccountsStoreFile.unknown_accounts`，`#[serde(skip)]`），
  保存时由 `save_store_atomic` 原样拼回 `accounts` 数组；无未知条目时写盘输出与原来完全一致。
- 写盘唯一入口：除加载器自身的迁移写盘与测试固件写盘外，全部 38 处写盘收敛到
  `SessionManager::persist_locked`（持锁）/`persist`（不持锁）；失败一律日志 + notice。
  以往 `let _ =` 静默吞掉失败的 9 处现在同样有日志 + notice。
- 新增解除路径（后端已就绪，**前端按钮待接线**）：Tauri 命令 `continue_with_empty_config`
  （以空配置继续，写盘失败则恢复阻止状态）、`restore_config_from_backup`
  （按 `.bak`、最新 `.corrupt-*` 顺序逐个尝试；配置正常时拒绝恢复）；
  `ManagerSnapshot` 新增 `persistBlocked` / `readOnly`；`dismiss_notice` 只关提示、不解除阻止。
- 任务书说法核实：`save_store_atomic` 在 session_manager 里确有 38 处非测试调用；
  未知服务标识确实会让整个迁移失败（`ProviderId::parse(s).ok_or_else(...)` 使
  `migrate_from_value` 整体 Err）。
- 已知局限：未知账号在群成员 / `lastCommitted` / `pendingCleanup` / 群消息 sender 等
  引用位置仍会被丢弃（`AccountKey.provider_id` 无法表示未知 provider，需改 provider 模型，
  超出 P0-1 范围；账号本体不丢）；未知 provider 账号 UUID 非法时仍整体失败
  （但已有 `.corrupt-*` 备份 + 写盘阻止，数据不再丢失）；恢复时每次失败尝试会多产生
  一份 `.corrupt-*` 备份；`.bak` 只有一份；日志用 `eprintln!`（仓库无 `log` 依赖）。

### P0-2 更新器完整性 —— 确认，已修复（P0-2-1～P0-2-8）

- **P0-2-1 sha256 必填**：基线桌面端 `download_to_updates_cache` 无 sha256 时跳过校验、
  缓存复用也不校验；Android `downloadAndInstallApk` 同样把 sha256 当可选。
  **行为变化（默认值）**：**sha256 必填**——`download_and_install_update` 在任何平台分支
  执行前先强制要求 `InstallUpdateRequest.sha256` 缺失/非法即拒绝；清单重新拉取后
  `sha256` 为 `None` 即拒绝安装；`download_to_updates_cache` 改为必填参数。
  Kotlin 侧未改（`DownloadApkArgs.sha256` 仍可空，插件内放行逻辑仍在）：只要不绕开
  Rust 命令，该路径不可达；Kotlin 侧改为必填列为后续跟进项。
- **P0-2-2 后端不再信任前端传值**：安装时先重新调 `check_for_updates` 取最新清单，
  只接受 `version` 等于清单 `latestVersion` 且 `download_url` / `sha256` / `filename`
  与清单完全一致的请求。行为变化：安装时多一次清单网络请求；离线或 GitHub API
  限流时安装会失败并提示重新检查更新。
- **P0-2-3 URL 前缀固定**：URL 必须以 `https://github.com/<配置的仓库>/releases/download/`
  开头（大小写不敏感、拒绝 userinfo、路径后必须有文件名段）。基线只校验 GitHub 主机，
  任何 GitHub 用户的 release 文件都能通过。
- **P0-2-4 每跳重定向校验**：`redirect::Policy::custom`，每一跳都要求 https 且主机在白名单内：
  `github.com`、`objects.githubusercontent.com`、`release-assets.githubusercontent.com`，
  另加 `api.github.com`（与任务书的偏差：`build_http_client` 同时服务清单 API 请求，
  API 自身 301 跳转会被三主机白名单误拦；初始请求 URL 本身不受重定向策略约束，
  加 `api.github.com` 不扩大下载源）。
- **P0-2-5 去隔离属性放在哈希校验之后**：部分确认。时序上下载（含哈希校验）本就先于安装；
  真正的漏洞是 P0-2-1（sha256 缺失时校验被跳过，去隔离照常执行），修 sha256 必填后即被堵上；
  同时删掉了安装步骤里解压后立即去隔离的那一次，只保留 swap 脚本成功替换后的那一次。
- **P0-2-6 长期签名方案**：只写方案，不改 release.yml。见第 2 节。
- **P0-2-7 swap 脚本回滚与超时**：等待循环结束后若旧进程仍在，脚本直接 `exit 2` 中止，
  不做任何替换；替换失败时先 `rm -rf "$dst_app"` 再把备份移回。局限：swap 脚本是 detached
  启动，父进程 350ms 后即退出，脚本退出码无人读取；超时中止时用户看到的是旧应用未重启，
  需手动重新打开应用再试；备份在失败路径上会被保留。
- **P0-2-8 `is_newer_version` 预发布比较**：改用 `semver` crate（`semver = "1"`，已在
  `Cargo.lock` 间接依赖中存在）；`1.0` / `1.0.0` 等缺段版本号先补齐再比较；
  无法解析为 semver 的字符串回退到旧的数值比较。注意：实测 `semver` 1.0.28 的 `Ord`
  会比较构建元数据（`1.0.0+build1 < 1.0.0+build2`），与 SemVer 规范不符，
  故 `parse_semver` 在解析前丢弃 `+build` 段。
- **P2 新发现：`open_external_url` 不校验主机**（已确认，已修）：只校验 scheme 为 https，
  前端传任意 https 链接都会打开。修复后只允许打开
  `https://github.com/<配置仓库>/releases` 或 `.../releases/tag/<tag>`；未配置更新源时直接拒绝。
  行为变化：清单里 `release_page_url` 若指向非配置仓库的页面，「发布页」按钮将报错而不是打开。

### P0-3 自动审批收紧 —— 确认，已修复

- 只点单次按钮：删除 `|| approveBtns[0]` 回退；没有单次按钮时不点击，置
  `window.__maamAutoApproveNeedsManual`（同一张卡 60 秒内只记一次），
  由静默同步末尾的 `drain_auto_approve_report` 读回并经 notice 推到 UI。
  `APPROVE_BTN_RE` 仍保留「始终/总是/always」作为审批卡的识别条件。
- `DANGER_RE` 扩充：命令类（`rm -rf` / `sudo` / `chmod` / `kill` / `git push --force` /
  `curl|sh` / `drop table` / `mkfs` / `dd` / `shutdown` 等）与凭据类
  （密钥/口令/令牌/私钥/`.env` / `id_rsa` / `.pem` / `api key` / `secret` / `.ssh` 等），
  中英文；`delete` / `remove` 等加词边界避免误伤 `skill`。保守点：
  `git push --force-with-lease` 因含 `--force` 前缀一并拦截；`secret` / `token`
  出现即拦截，宁可误拦也不放行。
- **行为变化（默认值）**：**命令执行类审批默认不自动批准**——新增 `COMMAND_RE`
  （执行/运行命令、shell/bash/终端等）与独立开关 `auto_approve_commands`，
  默认 **false**（旧配置缺失该字段时迁移为 `false`）；总开关开启时命令类卡也不批，
  需用户在设置里单独开启（开启后仍跳过危险命令）。自动审批总开关默认仍为关闭，未改。
- 审计日志与熔断：每次自动点击追加 `{ at, text, button }` 到页内
  `window.__maamAutoApproveLog`（最近 50 条），Rust 侧 `drain_auto_approve_report`
  读回后追加写 `auto-approve-audit.log`（JSONL，含 `at_ms` / `provider_id` / `uuid` /
  `text` / `button`；写盘失败只丢弃本次审计行）；同一页面 60 秒内自动点击达到 10 次后
  第 11 次起不再点击，立即停用本页并置 tripped；熔断时把 `settings.auto_approve`
  持久化置 false 并发 notice。延迟说明：熔断的本页立即停用是即时的；持久化关闭与 UI 提示
  依赖下一次静默同步（默认每 5 分钟），其间其他账号仍可能继续自动审批。
- 注入脚本抽取：`LOGIN_CAPTURE_INIT_JS` 内的 watcher 区块原样移出到
  `src-tauri/src/js/auto_approve.js`，以 `include_str!` 引入；弹窗 webview 本来就不启用
  自动审批，抽取后保持不变。测试：`tests/auto_approve.test.mjs`（jsdom），15/15 通过。
- 未修：审计日志文件无轮转/清理，长期高频点击会持续增长（后续可加按大小轮转）。

### P1-4 本地 CLI 输出 UTF-8 截断 —— 确认，已修复

- 读线程对每个 read 块直接 `String::from_utf8_lossy`，中文（3 字节）跨块即变 U+FFFD。
- 修复：读线程发送原始 `Vec<u8>`，接收端用新增 `Utf8ChunkDecoder` 增量解码，
  尾部不完整字节留给下一块；真正无效的字节才按单个 U+FFFD 输出；结束时 `finish()` 收尾。
- 行为变化：CLI 输出中的中文不再出现半字乱码；其余行为不变。

### P1-5 本地 CLI 生命周期 —— 确认，已修复（a/b/c/d/e）

- **(a) 子进程退出后仍等到 120 秒超时**：接收循环只在所有 sender 被 drop 时退出。
  修复：收到 `Finished` 后只给读线程约 500ms 排空，到时即用已有输出返回。
  行为变化：CLI 退出后最长再等 500ms 即返回；此前会等到总超时。
- **(b) 超时只杀直接子进程**：watcher 只 `child.kill()`，孙进程残留。
  修复：Unix 用 `Command::process_group(0)` 自成进程组，超时/取消时 `killpg(pgid, SIGKILL)`，
  失败回退 `child.kill()`；Windows 挂入 Job Object，超时/取消时 `TerminateJobObject`，
  挂入失败回退 `taskkill /T /F`。行为变化：超时或手动停止后孙进程不再残留。
  Windows Job Object 分支只在 Linux 上编译不到，待 Windows 实测（Unix `killpg` 分支已单测）。
- **(c) 120 秒总超时太短**：`deadline` 写死 120 秒；超时返回时丢弃已收集输出；且前端没有
  面向外部成员的「停止」按钮（群聊界面的「停止」选择器针对 Muse/Hark 网页自身的停止按钮）。
  修复：`ExternalAgentRecord` 新增 `timeoutSecs`（默认 **600 秒**）与 `idleTimeoutSecs`
  （默认 **60 秒**，无输出才判超时），旧配置缺失时按默认值补齐；范围校验
  （总超时 30–3600 秒，空闲超时 10–600 秒，越界拒绝；迁移解析越界值按默认处理）；
  超时错误附带已收集的部分输出（前 500 字）；新增 Tauri 命令 `cancel_external_member_turn`
  与群聊界面「停止」按钮；取消后 CLI 走进程组杀灭，API 丢弃响应流，已有部分输出保留。
  **行为变化（默认值）**：默认总超时由 120 秒改为 **600 秒**；新增 60 秒空闲超时；
  超时不再丢弃已收集输出；新增手动停止能力。
- **(d) Windows 找不到 `.cmd` 垫片**：待在 Windows 实测。代码审查确认：`Command::new("codex")`
  在 Windows 上只自动补 `.exe`；Rust ≥ 1.77 对含 `"` / `%` 的批处理参数会拒绝启动。
  修复（仅 Windows 代码，Linux 上编译不到）：新增 `resolve_cli_executable`
  （遍历 `PATH × PATHEXT`）；对 `.cmd` / `.bat`：prompt 一律走 stdin，`{prompt}` 与
  批处理垫片同时出现时返回明确中文错误。
- **(e) macOS 的 PATH 补全不完整**：`build_augmented_path` 只补 homebrew 等固定目录，
  没有 nvm / volta / bun / pnpm / `~/.claude/local`。修复（仅 macOS 代码）：
  启动 CLI 前用 `$SHELL -ilc 'printf %s "$PATH"'`（3 秒超时）读取登录 shell 的 PATH
  并用 `OnceLock` 缓存；失败回退到现有硬编码列表（另补 `~/.volta/bin`、`~/.bun/bin`、
  `~/Library/pnpm`、`~/.claude/local/bin`）。行为变化：Finder 启动的 GUI 应用也能找到
  nvm/volta/bun 等安装的 CLI；读取失败时行为与原来一致；首次读取最多阻塞 3 秒（之后走缓存）。

### P1-6 OpenAI 兼容 API 的 120 秒总超时 —— 确认，已修复

- `execute_openai_api_turn` 用 `reqwest::Client::builder().timeout(120s)`，覆盖整个响应体。
- 修复：去掉整体 `.timeout`，改用 `.read_timeout(60s)` 做空闲超时；外层用
  `tokio::time::timeout` 包裹整轮调用，时长取成员配置的 `timeout_secs`（默认 600 秒）。
  流式累积文本同步写入共享的 `Arc<Mutex<String>>`；超时或断流时若已有累积文本则返回
  累积文本并追加「（回复被截断）」，无任何文本时才返回错误；取消检查改为
  `tokio::select!` + 200ms 节拍，流停滞时取消最多 200ms 内生效。
- 行为变化：长流式回复不再在 120 秒被切断；被截断时消息中明确标注，不再静默丢失后半部分。

### P1-7 API Key 明文落盘并下发到前端 —— 确认，已修复

- `ExternalAgentRecord.api_key` 经 `#[serde(rename_all = "camelCase")]` 以 `apiKey`
  明文序列化进 `accounts.json`；`save_store_atomic` 用 `File::create`，临时文件权限
  取 umask（通常 0644）；`get_manager_state` 全量下发前端，编辑时明文回填 `<input>`。
- 修复：新增 `src-tauri/src/key_vault.rs`（`ApiKeyVault`）：桌面端优先用 `keyring` crate
  （系统凭据库），`accounts.json` 里只存引用 `apiKeyRef: "maam-api-key:<agent uuid>"`；
  `keyring` 不可用（Linux 无 secret service）或移动端（Android/iOS 不编译 keyring）时
  回退为 `<app_data_dir>/api-keys.json`，Unix 下临时文件 `OpenOptions::mode(0o600)` +
  原子 rename。`ExternalAgentRecord.api_key` 改为 `#[serde(skip_serializing)]`（只留内存），
  新增 `api_key_ref: Option<String>`（序列化为 `apiKeyRef`）。
- 首次启动迁移：在 `load_or_migrate_store` 成功加载后，对含旧明文且无 `apiKeyRef` 的成员：
  写 keyring/回退文件 → 置 `apiKeyRef` → 标记 dirty 写盘（写盘后 JSON 里不再有明文）；
  含 `apiKeyRef` 的成员从 vault 取回内存 Key。迁移失败时返回带启动提示的
  `LoadedAccountsStore`（保留原配置，下次启动重试；不阻止写盘）。
- `get_manager_state` / `ManagerSnapshot.external_agents` 改为 `ExternalAgentView`：
  除 `api_key` 外字段相同，另加 `hasApiKey: bool`。编辑时前端 `apiKeyDraft` 恒为空，
  留空表示不修改；后端 `upsert_external_agent` 收到空 Key 且为编辑时保留旧 Key；
  删除成员时同步删 vault 条目（失败忽略，不阻塞删除）。
- 写盘失败回滚 vault（先写 vault、再写盘：写盘失败回滚内存时按旧 Key 恢复 vault，
  新建则删除刚写入的条目）；成员 kind 由 `openai_api` 改为 `local_cli` 时同步清除
  旧 vault 条目，不留孤儿。
- **行为变化**：`accounts.json` 里不再有明文 API Key；前端再也收不到明文 Key
  （只有 `hasApiKey`）；编辑成员时 Key 输入框恒为空，留空保存即保留旧 Key。

### P1-8 写盘阻塞异步运行时且每次整库重写 —— 确认，已修复（异步化 + 防抖；增量写只写方案）

- `save_store_atomic` 在持有 `inner`（`tokio::sync::Mutex`）时被同步调用，37 处生产
  调用点全部处于守卫作用域内；每次对整份 `AccountsStoreFile` 做 `to_vec_pretty`，
  无增量写。基准：1 群 × 5 万条消息约 3.01s / 25.8MB；500 群 × 100 条约 3.81s / 25.9MB
  （debug 构建，最坏情况直接构造 store；加载时每群消息截断到 120 条，故磁盘文件天然有界，
  但同一会话内内存消息可无界增长）。
- 修复（合并后最终形态，见第 4.2 节）：`save_store_atomic_async`（`spawn_blocking`，
  与同步版逐字节一致；单次写盘墙钟不缩短，收益是运行时线程不阻塞）；
  `persist_locked` 为全库唯一写盘收敛点（async，先查 `persist_blocked`，再克隆 store、
  经 `save_store_atomic_async` 写盘）；9 处火后即忘点走 `request_persist()` +
  300ms 防抖写线程（短暂持锁克隆后释放锁再写盘；失败写 notice「配置自动保存失败：…」
  并保留 dirty，等下一次请求重试）。
- **行为变化**：防抖路径的写盘不再同步发生；进程在最后一次修改后 300ms 内退出可能丢
  最后一次自动保存（用户主动操作走同步 `persist_locked`，不受影响）；10 次快速请求
  合并为 1 次写盘。`save_store_atomic` 临时文件按 **0600** 创建（Unix），最终文件经
  rename 继承该权限（原为 umask 默认，通常 0644）。
- 未做（只写方案，不实现）：群聊消息拆文件（每群一个 `<group_id>.jsonl`，append-only）
  或迁移到 SQLite（`sqlx`/`rusqlite`，消息表按 `(group_id, created_at_ms)` 索引）。
  建议先做 jsonl（改动面小、无新依赖），待消息量或查询需求再上 SQLite。
- 已知 trade-off（合并时决定，见第 4.2 节）：错误传播路径写盘期间仍持 `inner` 锁，
  P1-8 原方案的「写盘时释放锁」只在防抖路径实现。

### P2-9 CSP 追加四条指令 —— 确认，已修复

- `src-tauri/tauri.conf.json` 的 `app.security.csp` 追加
  `base-uri 'none'; form-action 'none'; object-src 'none'; frame-ancestors 'none'`；
  原有指令一字未动。静态核查前端无 `<form>` / `<object>` / `<embed>` / `<iframe>` /
  `<base>`，无影响。`img-src 'self' data:`、`style-src 'self' 'unsafe-inline'`、
  `connect-src ipc: http://ipc.localhost` 均未动（头像远程地址本来就被现有 `img-src` 拦截；
  前端大量内联 `style`；dev 模式 HMR 不走 `connect-src`）。
- 行为变化：无合法功能变化。只针对攻击面：页面内嵌的 `<base>` 劫持、表单外发、
  `<object>`/`<embed>` 插件加载、被其他页面 `<iframe>` 嵌入，现在均被浏览器直接拒绝。

### P2-10 CI / 测试 —— 确认，已完成

- 新增 `.github/workflows/ci.yml`：触发 push + pull_request；平台 ubuntu-24.04 与
  windows-2025（`fail-fast: false`）；步骤 `npm ci` → `npx tsc --noEmit` →
  `cargo fmt --check` → `cargo clippy --all-targets -- -D warnings` → `cargo test`；
  Linux 任务先装 Tauri 系统库；顶层 `permissions: contents: read`；
  Rust 工具链 `dtolnay/rust-toolchain@stable`。
- 全部四处 `uses:` 固定为完整 commit SHA（`actions/checkout` v7.0.1、
  `actions/setup-node` v6.5.0、`dtolnay/rust-toolchain` stable 分支 2026-11-26 头提交、
  `Swatinem/rust-cache` v2.9.3）。注意：`dtolnay/rust-toolchain@stable` 本是分支名，
  固定 SHA 后需靠 Dependabot 定期更新，否则工具链停在 2026-11-26 的头提交。
- 新增 `.github/dependabot.yml`：cargo（`/src-tauri`）、npm（`/`）、github-actions（`/`），
  每周，每类最多 5 个未合并 PR。注意：Dependabot 更新固定 SHA 的 action 时，
  需要仓库设置允许它修改 workflow 文件。
- `providers.rs` 新增 11 个测试：任务书暗含的「后缀伪装可绕过」在基线**不成立**——
  `host_matches` 本来就是点号锚定后缀（`evilmuse.ai` 结尾是 `lmuse.ai` 而不是 `.muse.ai`），
  测试用于锁定该行为。另覆盖 `classify_top_level_url` 与 `is_verified_callback_origin`
  的正反例。行为变化：无，纯新增测试。
- `cargo fmt --check` 全库门禁：**不成立（基线即不通过）**——见第 4.5 节。
  本分支只加 `rustfmt.toml`（`tab_spaces = 2`，固化现有 2 空格风格）；
  `ci.yml` 仍按任务要求保留该步骤，**合入时预期为红**，直到各加固分支合并完成后
  单独提一个 `style: cargo fmt 全库格式化` commit。
- `cargo clippy -D warnings` 全库门禁：**不成立（基线即不通过）**——见第 4.5 节；
  `ci.yml` 保留该步骤，合入时预期为红，待专门的 clippy 清理 commit。
- Windows CI 任务未在 Linux 上实测（`cfg(windows)` 代码在 Linux 上编译不到，
  只能等 CI 首次运行验证）。

## 2. 长期更新签名方案（P0-2-6，不改 release.yml）

目标：即使 GitHub Release（清单 + 安装包）被攻破，客户端也不安装伪造更新。

方案（Tauri updater 兼容路线，不引入新重型依赖；与现有 `latest.json` 手工清单机制并行演进）：

1. **签名对象**：CI 发布时对两个产物签名——`latest.json`（清单本身）和每个平台安装包
   （`.app.tar.gz` / `-setup.exe` / `-release.apk` / `.ipa`）。签名算法选 **minisign**
   （Ed25519，签名小、验证实现可直接用 `minisign-verify` 思路或 `ed25519-dalek`；
   也可直接用 Tauri updater v2 的签名格式，二者选一，勿混用）。
2. **密钥管理**：
   - 私钥只存在 CI secret 中，不进仓库、不进产物。
   - 公钥编译进二进制（`option_env!` / `include_str!`，与 `MAAM_UPDATE_REPO` 同一配置思路），
     随版本发布；轮换公钥即发新版。
3. **验证顺序**：下载清单 → 验清单签名（失败即停）→ 按清单下载安装包 → 验 sha256 →
   验安装包签名（失败即删包、停）→ 再去隔离 / 启动安装程序。
4. **回滚与兼容**：旧版客户端无公钥，无法验证新签名，需在发布说明中注明
   「从 x.y.z 之前版本升级须手动下载一次」；或保留一段过渡期：清单同时带签名与未签名哈希，
   旧版走哈希、新版强制签名。
5. **所需 CI secret**（写进 release.yml 前需先在仓库 Settings → Secrets and variables →
   Actions 中创建，名称建议）：
   - `UPDATE_SIGNING_PRIVATE_KEY`：minisign 私钥全文（`minisign -G` 生成的 `.sec` 内容，
     或 base64 编码后存入，CI 内再解码；切勿用明文日志打印）。
   - `UPDATE_SIGNING_PUBLIC_KEY`：对应的公钥（可公开；编译进二进制用，也可存为仓库变量
     而非 secret，但为成对管理建议同放 secret，由 CI 在构建时注入
     `MAAM_UPDATE_PUBLIC_KEY` 环境变量）。
   - （可选）`UPDATE_SIGNING_KEY_ID`：密钥轮换时标识用哪把公钥验证，便于多公钥并存过渡。
6. **CI 改动点（本次未做）**：`release.yml` 的「Prepare release assets and latest.json」
   步骤后加签名步骤（`minisign -S -s "$KEY" -m latest.json` 及各安装包），并把 `.minisig`
   文件随产物一同上传 Release；构建 job 注入公钥环境变量。签名密钥只在发布 job 可见，
   沿用现有 `permissions: contents: write` 仅限发布 job 的做法。
7. **现状提醒**：哈希与安装包同源（sha256 来自同一 release 清单），只能防传输损坏，
   防不了发布源被攻破；签名方案落地前，`latest.json` 的 sha256 缺失/格式错误时
   消费端曾静默跳过校验（P0-2 已修为必填），但清单本身仍无签名保护。
   另：`release.yml` 生成清单时未校验四个 `*_sha` 是否为 64 位 hex（见第 3 节第 7 项 P2），
   建议签名落地时一并断言。

## 3. 第二轮审查报告（49 项：P0 1、P1 6、P2 33、待实测 5）

只审查，不改代码（临时测试探针已还原，工作树干净）。行号以基线 `0416f41` 为准。
已修复的项在备注中标注（P0-2 修掉了其中两环；其余未修）。

### 3.1 Android（P0 1、P1 2、P2 7、待实测 2）

| 级别 | 位置 | 依据（摘要） | 建议 / 状态 |
|---|---|---|---|
| P0 | `AccountSessionPlugin.kt:downloadAndInstallApk` | sha256 为 `String?` 可空；为空时全新下载只算摘要不比对、直接安装；缓存复用分支只凭 `lastDownloadedUrl == args.url` 就信任旧文件。上游 `updater.rs:841` 的 `extract_sha256_digest` 在清单缺 sha256 时返回 `None`，空值一路透传；`check_for_updates` 的 GitHub API 兜底分支同样可能产出 `None`。与 P0-2 同根，但 Android 分支此前未被覆盖。 | Rust 侧已修（P0-2-1：sha256 缺失直接拒绝，空值到不了 Kotlin）；Kotlin 侧改为必填列为后续项（`expectedSha == null` 时拒绝安装并删除已下载文件；复用分支一律重算摘要）。 |
| P1 | `AccountSessionPlugin.kt:openHttpConnection`、`fetchText` | 手动重定向循环只校验 https scheme，不校验主机。结合上一条，攻击链：清单被重定向到攻击者 JSON → `downloadUrl` 指向攻击者 APK → 无 sha 校验直接安装。利用条件高（需攻破 GitHub 侧或 TLS），故 P1。 | Kotlin 侧实现白名单重定向（`github.com` / `objects.githubusercontent.com` / `release-assets.githubusercontent.com`），每跳校验。后两环已被 P0-2 修复堵住，只剩重定向主机本身未在 Kotlin 侧设防。 |
| P1 | `AndroidManifest.xml` + 两处 `FileProvider.getUriForFile` | 仓库内无任何 `<provider android:name="androidx.core.content.FileProvider">` 声明，也无 `res/xml/file_paths.xml`；插件 manifest 只声明了一个 `exported="false"` 的 Activity；两处调用点（APK 安装、旧版下载打开）都用 authority `"${packageName}.fileprovider"`。若 Tauri 生成的工程也没有声明该 provider，两处调用都会抛 `IllegalArgumentException`，应用内更新安装在 Android 上直接失败。 | 待实测：跑一次 `tauri android init` / 实际打包，检查 `src-tauri/gen/android/app/src/main/AndroidManifest.xml` 与 `res/xml/file_paths.xml`；或在真机上走一遍应用内更新。若缺失，在插件 manifest 里补 `<provider … android:exported="false" android:grantUriPermissions="true">` + 限定 `cache-path` 的 `file_paths.xml`（只暴露 `cacheDir/updates`，不要 `external-path`）。 |
| P2 | `AccountWebActivity.kt:1839-1842` | `mixedContentMode = MIXED_CONTENT_COMPATIBILITY_MODE`（http 子资源会把同域 Cookie 明文发出）；`setAcceptThirdPartyCookies(wv, true)`。 | `mixedContentMode = MIXED_CONTENT_NEVER_ALLOW`；第三方 Cookie 默认关（登录流程如需再放开到指定域名）。 |
| P2 | `AccountWebActivity.kt:1999-2080` | `handleDownloadRequest` 接受 `http://` 下载；重定向允许 https→http 降级；HTTP 下载无大小上限（与 blob 下载的 256MB 上限不对称），恶意页面可触发超大下载占满存储；下载无需用户确认。 | 下载只允许 https（或至少禁止 https→http 降级）；HTTP 下载加大小上限与下载前确认。文件名已清洗（`.apk` 仍允许，属有意为之，用户仍需手动点安装）。 |
| P2 | `AccountWebActivity.kt:1874`、AndroidBlobDownloadBridge | JS Bridge 只靠随机 token 隔离（`__maamAndroidDownload`，主框和弹窗都注册；token 由原生生成、经 IIFE 传入，页面 JS 无法直接读到；文件名已清洗，256MB 上限）。设计基本成立。 | 纵深：token 一次有效且 `finish`/`fail` 后立即失效（现状基本满足）；`hintName` 取自页面可写的 `window.__maamLastDownload`，已清洗，仅作文件名。无直接漏洞，记为备注。 |
| P2 | `AccountWebActivity.kt:1879-1891` | `handleWebPermissionRequest` 对 `RESOURCE_PROTECTED_MEDIA_ID` 直接 `grant`（可用于追踪的设备标识，DRM 场景下无需用户确认即放行）。 | 首次遇到时弹确认，或仅对 muse/hark 主域放行并记录。 |
| P2 | `AccountWebActivity.kt`（弹窗与主框的输入监听） | `IDENTITY_PROBE_SCRIPT` 的 `input/change` 监听在弹窗也注册，探针每 4.5 秒对弹窗执行 `evaluateJavascript`，只取 `typed`（Apple ID / Facebook 登录页 typed 的邮箱会被记为 `capturedIdentityHint`，用作身份回退）。属登录捕获的有意设计，但应在文档中明确。 | 文档注明；或将弹窗探针限制为仅在 `isExternalLoginHost` 时取 `typed`（现状已是只取 `typed`，保持即可）。 |
| P2 | `AccountSessionPlugin.kt:copyProfileCookie` | 复制 Cookie 后无任何定时清理；仅在 API 33+ 置 `ClipDescription.EXTRA_IS_SENSITIVE`。 | 复制后起 60 秒定时器，用空内容覆盖剪贴板（只在剪贴板内容仍是本次 Cookie 时才清）；或在下次打开账号时清理。 |
| P2 | `AccountSessionPlugin.kt:46` | `OpenSessionArgs.proxyUrl` 传到 Kotlin 后从未被使用；Rust 侧 `SUPPORTS_ACCOUNT_PROXY` 在 Android 上为 `false`，字段恒为空。无安全影响，仅是死代码。 | 删除 Kotlin 侧字段，或在注释中说明 Android 不支持账号代理。 |
| 待实测 | 账号删除与 Profile 删除的竞态 | `deleteProfile` 在视图仍活动时拒绝，Rust 侧先记 `pending_cleanup`、下次启动重试，并先调 `RecentAccountPages.discardProfile` 销毁已停放的 WebView；`delete_account` / `clear_account_data` 在移动端不像桌面端那样先 sleep 120/150ms 等视图销毁。Android 上视图未销毁时返回 false → 走重试，数据最终能清；但「删除后立即重启是否残留 Cookie」未实测。 | 真机：登录账号 → 删除账号（视图仍在前台时）→ 立即重启 → 检查该 uuid 的 Profile 是否仍存在、Cookie 是否可读。 |
| — | 已确认好的地方 | 插件 manifest 只有一个 Activity（`exported="false"`，无 intent-filter）；权限只有 `INTERNET` / `ACCESS_NETWORK_STATE` / `REQUEST_INSTALL_PACKAGES` / `WRITE_EXTERNAL_STORAGE maxSdkVersion=28`；未声明 `usesCleartextTraffic`（targetSdk≥28 默认禁明文）；主框导航 `hostMatchesAllowed` 强制 https；`allowFileAccess=false`、`allowContentAccess=false`；无 `onReceivedSslError` 改写；`hostMatchesAllowed` 用点号边界后缀匹配；下载重定向发 Cookie 前有 `isSameSiteOrAllowedHost` 检查。 | — |

### 3.2 iOS（P1 1、P2 6、待实测 1）

| 级别 | 位置 | 依据（摘要） | 建议 / 状态 |
|---|---|---|---|
| P1 | `AccountWebViewController.swift:decidePolicyFor` | 对非主框只校验 scheme（http/https/about/blob/data），**不校验 host**。主框受 `hostMatchesAllowed` 限制，但任意 iframe/subframe 可加载任意 https（乃至 http）页面，与主框共享同一个 `WKWebsiteDataStore`（同一账号的 Cookie jar）。同时 `blob:https://` 与 `blob:null/` 主框导航也被无条件放行。 | 子框导航也校验 host 白名单（至少禁 http，第三方 iframe 仅放行验证码/widget 域名）；`blob:` 导航校验内层 origin。另：`javaScriptCanOpenWindowsAutomatically = true`（Android 侧为 false），建议与 Android 对齐为 false。 |
| P2 | `AccountSessionPlugin.swift:copyProfileCookie` | `UIPasteboard.general.setItems(…, options: [.localOnly: true])`（`localOnly` 阻止跨设备同步，好）；但未设 `expirationDate`，Cookie 一直留在剪贴板。域名过滤只校验 host，不校验 path，且未排除公共后缀（浏览器一般不会签发公共后缀 Cookie，且 Rust 侧导入已拦，故为备注级）。 | `setItems` 时加 `.expirationDate: Date(timeIntervalSinceNow: 60)`；或复制后 60 秒主动清空（仅当剪贴板仍是本次内容时）。 |
| P2 | `AccountSessionPlugin.swift:importProfileCookies` | `resolveTargetUrl` 从 Set-Cookie 头里取第一个 `Domain=` 自行拼出目标 URL，等于是用头部自己的 Domain 为自己背书。若头部值含 `;` 注入（见 3.3 P2），第一个 `Domain=` 可能是注入值。正常流程中 Rust 侧已过滤域名，属纵深防御问题；且该插件命令不直接暴露给前端，只经 Rust 调用。 | iOS 侧不再从头部解析 Domain，改用传入的 `url`（entry URL）作为目标 URL；或解析后二次校验 Domain 属于 allowedHosts。 |
| P2 | `AccountSessionPlugin.swift:buildHttpConnectProxyConfig` | 只解析 scheme/host/port；`http://user:pass@host:port` 的 userinfo 被静默丢弃（代理无认证会连不上，用户看到的只是「代理不可用」）；无端口时默认 80。Rust 侧已拒绝 userinfo 与无端口地址，正常流程到不了这里。 | 解析失败/含 userinfo 时返回 nil 并让调用方报错（现状 `bindSessionSpec` 里返回 nil 时静默保留旧的 `proxyConfigurations`，见下条）。 |
| P2 | `AccountWebViewController.swift:397-402` | `proxyRaw` 非空但解析失败时，既不清空也不报错，data store 沿用上一次的代理配置；只有 `proxyRaw` 为空时才清空。 | 解析失败时显式 `dataStore.proxyConfigurations = []` 并向 Rust 报错。 |
| P2 | `AccountWebViewController.swift` | 全文件无 `WKDownloadDelegate` / `decidePolicyFor navigationResponse`。WKWebView 默认对 `Content-Disposition: attachment` 的导航无可见处理，iOS 端点下载链接基本无反应（与 Android 的完整下载实现不对称）。 | 功能缺口，非安全问题；如补实现，应复用 Android 的同款校验（https-only、host 校验、大小上限、落盘前确认）。 |
| P2 | `AccountWebViewController.swift:697-700` | 连续两次探针判 loggedOut 且 `staleSession` 为真时，自动 `load(https://muse.ai/api/logout)`（hark 则跳登录页）。这是用 GET 触发的状态变更，且依赖探针判断；若探针误判（如页面结构变化导致 composer 检测失败），会把已登录用户登出。 | 改为仅提示用户手动处理，或在触发前用 `__maamVerifySession` 再确认一次。 |
| 待实测 | 删除账号时 `remove(forIdentifier:)` 与存活 WebView 的竞态 | `deleteProfile` 只查 `isUuidActive`，但 `closeFromManager` 的 `dismiss(animated: true)` 是异步的；移动端没有桌面端那 120/150ms 等待。`WKWebsiteDataStore.remove(forIdentifier:)` 在仍有 WKWebView 持有该 store 时的行为未在代码中处理（Android 侧会返回 false 走重试；iOS 侧未知）。 | 真机：登录账号 → 删除账号 → 立即重启 → 检查该 uuid 的 data store 是否仍存在、Cookie 是否可读；以及删除后立刻重建同 uuid 账号是否会读到旧 Cookie。 |
| — | 已确认好的地方 | 每账号 `WKWebsiteDataStore(forIdentifier: uuid)`，冷启动校验 identifier 存在；`fetchText` 用 `.ephemeral` 会话 + `HttpsOnlyRedirectDelegate`（仅 https 重定向），2MB 上限（注：上限在收到完整 body 后才判断，超大响应仍会短暂占内存）；钩子的 fetch 有 `isPrimaryHost` 门禁；`openExternalUrl` 只允许 http/https。 | — |

### 3.3 platform.rs（P1 1、P2 6、待实测 1）

| 级别 | 位置 | 依据（摘要） | 建议 / 状态 |
|---|---|---|---|
| P1 | `build_timezone_override_init_js`（约 71-326 行） | 时区脚本重写 `Date.prototype` 与 `Intl.DateTimeFormat`，以 `document-start` 注入到所有帧：桌面端主框 + 弹窗、Android 全部 origin、iOS 主框 + 全部子框。hCaptcha / Turnstile / reCAPTCHA 等验证码 iframe 里 `Date` 与 `Intl` 也被改写——这类服务依赖环境指纹做机器人判定，改写可能导致验证失败或被加严；同时 `Date.prototype.toString` / `Intl.DateTimeFormat.toString` 可被页面检测到改写（指纹特征）；OAuth 页的时区与 IP 属地不一致也可能触发风控加验。 | 时区脚本只注入主框（iOS 改 `forMainFrameOnly: true`；Android 把 `setOf("*")` 收窄到 muse/hark 入口 host；桌面端同理），子框/弹窗的 OAuth 页按需注入；或改用 WebView2 的 `Emulation.setTimezoneOverride`（Windows 已有 `configure_windows_timezone` 走 CDP）/ WKWebView 的等价能力。另：`parse_account_timezone` 只校验字符集，不校验 IANA 名是否存在；无效名在 JS 侧抛错后静默不生效，UI 上时区却显示「已设置」——建议 Rust 侧用 `chrono-tz` 或系统时区表校验，失败即报错。待实测：给账号设非常用时区后，在桌面 + Android + iOS 各走一遍含 Turnstile/hCaptcha 的登录流程，对比不设时区的通过率。 |
| P2 | `imported_cookie_to_set_cookie_header`（约 1891-1913 行） | `parse_imported_cookies` 只拒绝值中的 `\r`、`\n`、NUL，不拒绝 `;`；拼接用 `format!("{}={}", name, value)`。JSON / Netscape 导入的值含 `; Domain=…` 时，移动端会把注入部分当作属性解析。**已用临时单元测试验证**：值 `abc; Domain=evil.example; Path=/x` 经解析后输出头确实包含 `Domain=evil.example`。实际影响有限：Android 侧 `CookieManager.setCookie` 会校验 Domain 与 URL host 的匹配关系，注入的异域 Domain 会导致整条 Cookie 被丢弃（静默失败）；iOS 侧 `resolveTargetUrl` 取第一个 `Domain=` 作目标 URL，注入域的 Cookie 可能被种进该账号的 store。桌面端不受影响（`Cookie::parse` 作模板再 `set_name/set_value`，值不经过头解析）。 | 值中出现 `;` 或 `,` 时拒绝该条（或按 RFC 6265 对值做引号包裹并转义）；同时 iOS 侧不要从头部解析 Domain（见 3.2）。 |
| P2 | `write_cookie_clipboard`（约 1326-1360 行）+ 全平台 | 桌面端用 arboard 写剪贴板（macOS `exclude_from_history`、Windows `exclude_from_cloud().exclude_from_monitoring()`），均无定时清理；Android 仅置 `EXTRA_IS_SENSITIVE`（API 33+）；iOS `localOnly` 且无 `expirationDate`；前端 `main.ts:3917` 的 1800ms 定时器只恢复按钮文案，不碰剪贴板。复制出的 Cookie（含会话 Cookie）在剪贴板里一直留到被覆盖。 | 复制后 60 秒用空内容覆盖（仅当剪贴板仍是本次 Cookie 时才清）；iOS 直接给 `setItems` 加 `expirationDate`。 |
| P2 | `push_validated_imported_cookie`（约 1372-1450 行） | JSON 导入时 `secure` 默认 true，但显式 `"secure": false, "sameSite": "none"` 会原样保留；生成的头含 `SameSite=None` 且无 `Secure`，Chrome/Firefox 会直接丢弃该 Cookie（静默失败，用户以为导入成功）。 | `same_site == "none"` 时强制 `secure = true`（或丢弃该条并提示）。 |
| P2 | `push_validated_imported_cookie`（父域 Cookie 被拒） | 域名校验用 `host_matches(cookieDomain, allowed)`，要求 Cookie 的 Domain 等于或为 allowed host 的子域。`Domain=hark.com` 的 Cookie 导入到 `app.hark.com` 账号时会被拒（方向反了不会导致越权，只会误拒）。 | 允许 Cookie Domain 为 entry host 的父域（同时仍要求父域本身在 allowed 列表的某个条目下或等于其一），并补测试。 |
| P2 | `parse_account_proxy`（约 327-375 行） | 基本正确：拒绝 userinfo、只允许 `http://host:port`、拒绝 path/query/fragment、主机字符白名单；无凭据可落盘；代码库中无任何 `log` 宏，不存在把代理 URL 写日志的路径；下发前端的也只有 host:port 形式。备注 1：Windows 上代理拼进 WebView2 的 `--proxy-server=` 启动参数，本地其他进程用任务管理器可见代理地址（含端口，无凭据）。备注 2：`test_account_proxy` 的错误分支把 `reqwest::Error` 原样拼进返回消息，错误文本可能含目标 URL；只发给前端展示，不写日志，风险低。 | 备注，无改动。 |
| P2 | `read_macos_cookies_for_url`（约 1240-1315 行） | macOS 侧复制时校验 domain 与 path；但 iOS 侧 `copyProfileCookie` 只校验 domain，不校验 path。复制出的是「该域全部 Cookie」，含非入口路径的 Cookie。用户主动复制场景，影响小。 | iOS 侧与 macOS 对齐，加 path 校验。 |
| — | 已确认好的地方 | Cookie 域名过滤严格：`host_matches` 为精确相等或点号边界后缀匹配；`__Host-` 强制去 Domain/Path=/ /Secure，`__Secure-` 强制 Secure；过期 Cookie 跳过；`max_age` 夹在 60 秒～365 天；名字符白名单（禁 `;` `=`）、值禁 `\r` `\n` NUL。`verify_desktop_profile_binding` 在写 Cookie 前核对 macOS 的 `WKWebsiteDataStore.identifier` 与 Windows 的 WebView2 userDataFolder 是否与 uuid 一致，防串号。 | — |

### 3.4 site_adapters.rs 与注入脚本（P1 2、P2 4）

| 级别 | 位置 | 依据（摘要） | 建议 / 状态 |
|---|---|---|---|
| P1 | `NET_HOOK_INIT_JS`（site_adapters.rs:344-596） | 无任何 `isPrimaryHost` 判断，对 `fetch` / `EventSource` / `WebSocket` 全量挂钩：POST 且 `content-type` 为 json/text/stream 的响应会被整段读出（上限 256KB）；`status >= 400` 的响应会被读出并存 `errorSnippet`（前 240 字符）。它只注入主框，不注入弹窗；但主框导航白名单包含 `facebook.com`、`appleid.apple.com`、`idmsa.apple.com`、`meta.com`、`instagram.com`，即主框完全可能在 Apple ID / Facebook 登录页加载，此时登录接口的 POST 响应体会被读出并做 `pickTextFromJson` 提取。缓解：`pickTextFromJson` 只提取已知助手文本字段，不直接存 token；`errorSnippet` 只存前 240 字符。故 P1 而非 P0。 | 与 `LOGIN_CAPTURE_INIT_JS` 对齐，在 `NET_HOOK_INIT_JS` 顶部加 `isPrimaryHost` 门禁（muse.ai / hark.com 及其子域），非主域只记录 URL/method/status，不读响应体；或把响应体读取限制在 `chat_api_patterns` 命中的 URL。 |
| P1 | `window.__maam*` 全局变量 | 身份、额度、群聊回复全部经由页面可写的普通全局变量传递，Rust 侧不做二次校验：`__maamApiEmail` / `__maamTypedIdentity`（探针读到的邮箱直接作为账号别名与身份）；`__maamQuotaDetail` / `__maamQuota`（可伪造额度展示；`localStorage` 里的缓存也可被页面读写）；`__maamNet.entries`（群聊引擎读它取助手流式回复，页面脚本可塞入伪造条目）；`__maamAuthCheck.status`（页面脚本置 `ok` 可压住登出检测，置 `invalid` 可触发登出，含 iOS 的自动 GET 登出）；`__maamOrigFetch`（可被替换返回伪造 JSON）；`TURBOPACK` 拦截用 `configurable: true`，页面脚本可再定义一次抢占；`__maamRefreshBackendQuota` 用硬编码兜底 action id `'40e0249eeae158685b6a8d0c3b203f53dd4f2c7d63'` 调 `callServer`，站点构建变化后可能调到错误的 server action。 | 身份类结论改由 Rust 侧用 profile 的 Cookie 直接请求 `/api/auth/check` 或 `/api/profile` 校验，页面值只作展示 hint；额度/群聊回复至少做来源校验；把关键全局变量用 `Object.defineProperty(…, {writable: false, configurable: false})` 封存（能抬高门槛，不能根治——同 realm 脚本总有办法，根本解是 Rust 侧校验）。 |
| P2 | `LOGIN_CAPTURE_INIT_JS` 的输入监听 | `capture` 的 `input/change/blur/submit` 监听在 `if (isPrimaryHost) {…}` 块**之外**，且同时注入主框和弹窗。因此在 Apple ID / Facebook / Meta 登录页 typed 的邮箱与手机号会被记入 `window.__maamTypedIdentity` / `__maamTypedPhone` 与该站点的 `sessionStorage`，探针再读回 Rust。这是登录捕获的有意设计，但应明确记录：应用会读取第三方 IdP 登录页的输入内容。对照：fetch/WebSocket 钩子在 `isPrimaryHost` 块内，不在 IdP 页读响应体；`IDENTITY_PROBE_JS` 在 `accountscenter.meta.com` 会从 body 文本取邮箱（有意设计），在其余 IdP host 上只返回 loggedOut/loading，不扫 body。 | 文档注明该行为；技术上可把 `capture` 限制为仅在 `isPrimaryHost \|\| isExternalLoginHost` 时注册（现状等于全站）。 |
| P2 | `NET_HOOK_INIT_JS` 读取非聊天 POST 的响应体 | `consumeStream` 的触发条件是「POST + json/text/stream」，不限于 `chat_api_patterns`。设置、偏好等接口的 POST 响应也会被读出并做 `pickTextFromJson`。 | 只对命中 `chat_api_patterns` 的 URL 读响应体；其余只记 URL/method/status。 |
| P2 | `site_adapters.rs:257` `host_matches_pattern` 与 `merge_adapters` | `"*"` 通配整个 generic 兜底（有意设计）；`"*.muse.ai"` 同时匹配 `muse.ai` 本身；`merge_adapters` 允许覆盖文件替换任意内置 adapter（含把 `muse` 的 `host_patterns` 改成 `*`）。覆盖文件位于 app_data_dir，用户可写，属本地配置行为。 | 覆盖文件校验时拒绝把内置 id 的 `host_patterns` 改为 `*` 或空；或至少在快照里标注「内置规则被覆盖」。 |
| P2 | 硬编码 server action id 兜底 | `LOGIN_CAPTURE_INIT_JS` 的 `__maamRefreshBackendQuota` 在没扫到 `fetchSubscriptionAction` 时用硬编码 id（Android 的 `IDENTITY_PROBE_SCRIPT` 同款）。站点构建一变，该 id 可能对应别的 server action。 | 去掉硬编码兜底；扫不到时只用已缓存的额度并提示同步失败。 |
| — | 已确认好的地方 | `LOGIN_CAPTURE_INIT_JS` 的 fetch/WebSocket 钩子有 `isPrimaryHost` 门禁；`NET_HOOK_INIT_JS` 不注入弹窗；Android/iOS 的 quota hook fetch 门禁与之一致；`classify_top_level_url` 主框只放 https + allowedHosts，`accounts.google.com` 一律走外部浏览器；探针的邮箱正则、bad 前缀表与排除域名在三端一致。另：插件命令不直接暴露给前端（`capabilities/manager.json` 只授权 Rust 命令，`account-session` 插件自身无 `permissions/` 定义，移动端命令只能经 Rust 侧调用；结论基于代码与 capability 清单，未做运行时越权测试）。 | — |

### 3.5 installer.nsi（P2 5、待实测 1）

| 级别 | 位置 | 依据（摘要） | 建议 / 状态 |
|---|---|---|---|
| P2 | `installer.nsi:372-409` | 卸载路径取自注册表，未校验即信任：重装页从 `SHCTX\Software\<MANUFACTURER>\<PRODUCTNAME>` 读上次安装目录，随后 `ExecWait '$R1 _?=$4'`、`Delete "$4\uninstall.exe"`、`RMDir "$4"`。注册表值被篡改（同用户写 `HKCU` 即可）时，安装器会在该目录删一个 `uninstall.exe`、并把新版本装进去。`RMDir` 无 `/r`，只删空目录，blast radius 有限。 | 使用前校验该目录下存在本应用的 `uninstall.exe` 与主程序（或要求路径位于 `%ProgramFiles%` / `%LOCALAPPDATA%` 下），否则忽略并用默认目录。 |
| P2 | `installer.nsi:206-216` | WiX 卸载串按 DisplayName+Publisher 匹配后直接执行：遍历 `HKLM\…\Uninstall`，匹配即取其 `UninstallString` 并 `ExecWait` 执行。模板注释已承认该风险。同名产品的恶意注册表项可借安装器提权执行。 | 执行前校验 `UninstallString` 指向的确实是 `msiexec /x {GUID}` 且 GUID 与预期一致；或去掉 WiX 迁移分支（本项目是否还有 WiX 用户待确认）。 |
| P2 | `installer.nsi:610-616` | WebView2 bootstrapper 下载后无哈希校验：默认 `installWebview2Mode` 未配置，即 Tauri 默认的 `downloadBootstrapper`；安装器下载后直接 `ExecWait … /install`，无哈希/签名校验（只有 TLS 保护）。 | 改用 `embedBootstrapper` 或 `offlineInstaller` 并在 CI 校验哈希；或在模板里对下载文件做签名校验（属上游模板默认行为，改动需同步上游模板）。 |
| P2 | `installer.nsi:830-960` | 卸载递归删除路径是编译期常量，但无空值保护：递归删除的只有 `$APPDATA\${BUNDLEID}` 与 `$LOCALAPPDATA\${BUNDLEID}`（`BUNDLEID = com.ldjx.accountmanager`，编译期填入），其余删除均为固定文件名 + 非递归 `RMDir $INSTDIR`。污染面小。 | 纵深：递归删除前断言 `BUNDLEID` 非空且不含 `..` / `\` / `/`（模板变量被误配为空时 `RmDir /r "$APPDATA\"` 会酿成灾难）；`$INSTDIR` 的递归删除只应在校验其包含 `uninstall.exe` 后执行。 |
| P2 | `.onInit`（约 519-531 行） | `/P`、`/UPDATE` 解析无值校验：`/P`（passive）与 `/UPDATE` 组合会跳过全部页面与卸载前确认；`/R` 只在 `.onInstSuccess` 且 passive/silent 时处理，用 `RunAsUser` 以普通用户身份重启应用（`/ARGS` 透传）。 | `/UPDATE` 模式下仍应校验安装包来源（与 P0-2 的安装前校验衔接）；`/ARGS` 的值应做长度与字符白名单校验（现状原样拼进 `RunAsUser` 的命令行）。 |
| — | 已确认好的地方 | 静默安装权限：未配置 `installMode`，即 Tauri 默认 `currentUser`；对应 `RequestExecutionLevel user`，安装到 `%LOCALAPPDATA%\Diehao`，**不提权、无 UAC**；更新流程 `/P /UPDATE /R` 全程以普通用户身份运行，`.onInstSuccess` 用 `RunAsUser` 重新拉起应用，不会以管理员身份跑应用。降级保护：`ALLOWDOWNGRADES=false` 时 silent/passive/update 模式会先比对版本并拒绝降级。卸载时 `UpdateMode=1` 会保留快捷方式、Run 键与应用数据；「删除应用数据」复选框默认不勾选。`INSTALLDIRNAME` 硬编码为 `Diehao`，与产品名「叠号」不一致但无安全影响。`utils.nsh`（`CheckIfAppIsRunning`、`RunAsUser` 实现）不在仓库内，是 Tauri 打包器提供的模板文件，本轮无法审计其实现，以上涉及它的结论以模板行为为准。 | — |

### 3.6 session_manager.rs 并发（P2 3）

| 级别 | 位置 | 依据（摘要） | 建议 / 状态 |
|---|---|---|---|
| P2 | 5 处 `lock().unwrap()`（6364、6405、7124、8402、10737） | 逐个判断：守卫内容分别为 `recent_downloads` / `pending` / oneshot `Sender`（3 处）；临界区内只有 `insert/remove/push_back/pop_front/take/send`，均无 panic 路径；守卫均为语句级作用域，**没有任何一处在持有 `std::sync::Mutex` 守卫期间 `.await`**。中毒不可达。同款模式另见 `platform.rs:975` 与 `platform.rs:1930`，结论相同。 | 全部改为 `lock().unwrap_or_else(\|e\| e.into_inner())`（一行改动，消除中毒后 panic 的可能，尤其回调在 wry/WebView2 COM 线程上时 panic 会跨 FFI 导致 abort）。 |
| P2 | `group_member_locks` 获取顺序 | 唯一获取点：先 `self.inner.lock().await`（tokio Mutex）取出 `Arc<tokio::sync::Mutex<()>>`，**释放 inner 后**再 `member_lock.lock_owned().await`；不存在 inner 持有期间取 member 锁的路径。持有 member 锁期间会 `.await`（执行 webview 回合、取 `group_worker_semaphore`、再取 inner），顺序恒为 member → semaphore → inner，无反向边。跨成员嵌套不存在（A2A 接力等当前成员回合返回、锁已释放后才 spawn 下一跳；failover 顺序重试；`visited` 集合保证单链不重复）。信号量 `GROUP_MAX_CONCURRENT_WORKERS = 6`：持许可的任务不再等 member 锁，无循环等待。 | 无死锁可修。备注：member 锁会持有整个 webview 回合（可达数分钟），同一成员的后续回合会在此排队；若 webview 挂起无响应，该成员的群聊会被长期阻塞——建议给 member 回合加总超时（与 P1-5/P1-6 的超时治理衔接）。 |
| P2 | `tokio::sync::Mutex`（`inner`）持有期间的 `.await` | `inner` 是 tokio Mutex，持有期间 `.await` 是合法的；持锁期间调用 `apply_desktop_layout_locked` 等同步函数无 await。未发现持 `inner` 时取第二个 tokio 锁后又反向取 inner 的路径。 | 无改动；P1-8 的写盘阻塞问题另案处理。 |

### 3.7 README 与 latest.json（P2 2）

- 已确认：CI 生成的 `latest.json` 四个平台都有 `sha256`（`release.yml:239-306`：`mac_sha / win_sha /
  apk_sha / ipa_sha` 均用 `sha256sum` 对发布文件实测后由 `jq -n` 写入；任一产物缺失时 `exit 1`，
  不会生成缺字段的清单）。消费端 `updater.rs:352,406` 的 `extract_sha256_digest` 会解析该字段
  （缺失/格式错误时返回 `None`，随后跳过校验——这是 P0-2 的问题，不是清单的问题）。
- P2 / 清单生成无空哈希保护：`mac_sha=$(sha256sum … | awk '{print $1}')` 未校验结果为 64 位 hex；
  若 `sha256sum` 失败（配合 `set -e` 缺失时），`latest.json` 会含 `"sha256": ""`，消费端静默跳过校验。
  建议：在 `jq` 之前断言四个 `*_sha` 均匹配 `^[0-9a-f]{64}$`，否则失败本次发布。
- P2 / README 的 SHA-256 表述与现状不符：`README.md:30` 称「客户端内 GitHub Releases `SHA-256`
  校验覆盖更新」；`README.md:126` 指导用户用 `latest.json` 的 `platforms.windows.sha256` 核对安装包。
  但现状是 sha256 缺失时桌面与 Android 都跳过校验（P0-2 / 3.1 P0）。建议：P0-2 修完（sha256 必填）
  后再保留该表述；或现阶段改为「有 sha256 时校验」。**注：P0-2 已修（Rust 侧 sha256 必填），
  该表述现已基本成立，但 Kotlin 侧仍是可空参数（见 1 节 P0-2-1），严格说 Android 分支仍未闭合。**

### 3.8 新发现项汇总（任务书第 3 节清单之外，共 12 项）

1. Android APK 更新 sha256 可空（3.1 P0；Rust 侧已修，Kotlin 侧待跟进）。
2. Android FileProvider 未在仓库内声明（3.1 P1，待实测）。
3. 桌面端 `NET_HOOK_INIT_JS` 在主框 IdP 登录页读响应体（3.4 P1）。
4. `window.__maam*` 全局变量伪造（3.4 P1）。
5. 时区脚本注入验证码 iframe（3.3 P1，待实测）。
6. 剪贴板 Cookie 永不清（3.1/3.2/3.3 P2，全平台）。
7. Cookie 值 `;` 注入（3.3 P2，已用临时测试验证）。
8. iOS 子框导航不限 host、iOS 无下载实现（3.2 P2）。
9. Android HTTP 下载无大小上限、允许 https→http 降级（3.1 P2）。
10. NSIS 注册表安装目录信任、WiX 卸载串执行、WebView2 无哈希下载（3.5 P2）。
11. iOS 删除 Profile 与存活 WebView 的竞态（3.2，待实测）。
12. `SameSite=None` 不带 `Secure` 被浏览器静默丢弃（3.3 P2）。

最值得优先修的 3 项：Android APK 更新不校验 sha256（P0，已修 Rust 侧）；桌面端 NET_HOOK 在主框
登录页读取响应体（P1）；页面 JS 可伪造 `window.__maam*` 全局变量（P1）。

## 4. 合并过程记录

### 4.1 合并顺序与冲突概况

7 个分支依次合并进 `hardening/consolidated`，每合一个分支跑 `cargo test --lib` 保持全绿。
冲突主要集中在 `session_manager.rs`（4 个分支改过）与 `storage.rs`（加载/迁移/Key 迁移三方交汇）：
- ci-csp、second-round-review：干净合并（前者是新增 CI/CSP/测试与 `rustfmt.toml`，后者只有报告文档）。
- updater-integrity：干净合并。
- cli-lifecycle：干净合并。
- auto-approve：13 处冲突，均为 additive 设置字段（`disableAutoUpdateCheck` vs
  `autoApproveCommands`，两边都保留）与一处 `storage.rs` 迁移解析多余 `}`。
- config-load-protection：2 处冲突，均在 `session_manager.rs`（`update_app_settings`
  的两路设置赋值 + 测试模块末尾的 approve 审计测试与 config 7 个新测试，全保留）。
- api-key-keyring：3 个文件冲突（`Cargo.lock` 1 处、`storage.rs` 2 处、`session_manager.rs` 37 处），
  见 4.2–4.4 节。

### 4.2 写盘收敛点统一（P0-1 × P1-7/P1-8 的核心冲突）

两分支都动了 `load_or_migrate_store` 与全部 `save_store_atomic` 调用点，合并时按任务书要求
以 P0-1 的 `persist()` 为准，把异步化/防抖接进 `persist()` 实现：

- `persist_locked(&mut InnerState)` 保持签名（守卫持有者与 `&mut InnerState` 辅助函数都能调，
  单收敛点才成立），改为 `async`：先查 `persist_blocked`（拒绝写盘 + notice），再克隆
  store 与路径，经 `save_store_atomic_async`（`spawn_blocking`，临时文件 0600）写盘。
  `persist()`（不持锁入口）内部加锁后走它；原 keyring 分支的 `persist_and_relock` 已删除，
  其逻辑并入 `persist_locked`。
- 27 处错误传播点走 `persist_locked`（`.await`，回滚语义不变）；9 处火后即忘点走
  `request_persist()`（300ms 防抖写线程）；测试双方全保留。
- **Trade-off（合并时决定，如实记录）**：`persist_locked` 在写盘期间仍持 `inner` 锁——
  P1-8 原方案的「写盘时释放锁」（`persist_and_relock`：持锁克隆 → 释放锁 → 写盘 → 重新加锁）
  在错误传播路径上没有保留，原因是把它推广到全部 38 处调用点需要 owned-guard 重构，
  风险大于收益；P0-1 原设计同样持锁同步写盘，因此这不是相对 P0-1 的回归。
  防抖路径（`request_persist` + 写线程）仍在写盘期间释放锁。后续改进项：把错误传播点
  也改成持锁克隆 → 释放锁 → 异步写盘 → 重新加锁（需逐点处理回滚语义）。
- `apply_logged_out_state_locked` 与 `apply_detected_metadata_locked` 两个同步辅助函数
  改为 async（调用点 9 处补 `.await`），否则无法调用 async 的 `persist_locked`。
  keyring 分支原本让这两处保持同步 `save_store_atomic`，config 分支则已把它们收敛进
  `persist_locked`（当时还是同步函数）；合并后统一为 async。

### 4.3 合并时补上的两个 P0-1 漏洞

合并中发现、合并时直接修复（均不在任一分支的原始改动里）：

1. **防抖写线程会绕过 `persist_blocked`**：keyring 分支的 `persist_writer_loop` 不检查
   `persist_blocked`，配置损坏/高版本只读时，`request_persist()` 会在 300ms 后把内存里的
   空配置写盘，覆盖损坏/高版本文件——正是 P0-1 要防止的数据丢失。修复：写线程克隆 store
   时同时取 `persist_blocked`，为真时跳过本次写盘。
2. **自动审批熔断处的直写绕过 `persist_blocked`**：`session_manager.rs` 里熔断后持久化关闭
   的唯一一处直接 `save_store_atomic` 调用（`if tripped` 分支），同样会覆盖损坏/高版本配置。
   修复：加 `!locked.persist_blocked` 守卫。加载器自身的迁移写盘与测试固件写盘仍是
   直接调用（前者是 P0-1 设计的例外，后者只在测试里）。

### 4.4 跨分支 API 漂移（合并时修的 11 处编译错误）

各分支基于同一基线独立开发、互相不知道对方新增的字段/参数，合并后出现 11 处编译错误，
全部是机械补齐，无行为变化：

- `group_transport.rs` 测试 helper（`test_cli_agent` / `test_api_agent`）补 `api_key_ref: None`（2 处）。
- `storage.rs`：key 迁移失败早退路径的 `LoadedAccountsStore` 补 `persist_blocked: false` /
  `read_only: false`；bench store 的 `AccountsStoreFile` 补 `unknown_accounts: Vec::new()`；
  序列化测试的 `ExternalAgentRecord` 补 `timeout_secs` / `idle_timeout_secs`（默认值 600/60）。
- `session_manager.rs`：4 处 `upsert_external_agent` 测试调用补 `timeout_secs` /
  `idle_timeout_secs` 参数（`None, None`）。
- `storage.rs` 冲突解决时丢了一个闭合 `}`（`corrupt_backups` 函数体在冲突边界处截断，
  闭合括号原本在共享上下文里）：编译报错后补回。另：`save_store_atomic` 入口处多了一个
  空行（冲突解决脚本的换行叠加所致），`rustfmt --check` 发现后删除（单独 commit）。

### 4.5 fmt / clippy 基线状态（合并后实测，非猜测）

**`cargo fmt --check`（基线即红；合并后仍红；不重排全仓库）：**

- 项目用 2 空格缩进，rustfmt 默认 4 空格，基线无 `rustfmt.toml`。
  P2-10 已加 `rustfmt.toml`（`tab_spaces = 2`），合并完成时 `cargo fmt --check` 有
  **237 处 diff**；同一配置下基线为 **200 处**（无 `rustfmt.toml` 时默认配置下基线 247 处）。
  合并新增约 37 处，集中在 `session_manager.rs`（134→148）、`storage.rs`（9→23）、
  新文件 `key_vault.rs`（7 处）、`updater.rs`（9→14），其余文件不增反减
  （`lib.rs` 6→5、`group_transport.rs` 2→1、`platform.rs` 32→32、`providers.rs` 3→3、
  `site_adapters.rs` 5→5）。
- **P2-11 拆分后复测：260 处 diff**（+23）。分布：`session_manager.rs` 83、
  `session_manager/group_chat.rs` 39、`platform.rs` 32、`session_manager/webview.rs` 27、
  `storage.rs` 22、`updater.rs` 14、`session_manager/quota_sync.rs` 12、`key_vault.rs` 7、
  `site_adapters.rs` 5、`lib.rs` 5、`session_manager/a2a.rs` 4、`session_manager/scripts.rs` 3、
  `providers.rs` 3、`session_manager/quota.rs` 2、`session_manager/store.rs` 1、
  `group_transport.rs` 1。拆分新增的 diff 集中在新子模块的 import 块与函数签名折叠
  （`pub(super)` 方法的签名换行等），原 237 处的构成基本不变，只是随代码平移到了子模块。
  新增 diff 均为 rustfmt 的链式换行 / import 排序 / 函数签名折叠等风格偏好，不是缩进问题；
  各分支按 `tab_spaces = 2` 单文件自查时未发现（自查方法漏掉了全文件检查的这部分规则）。
- 合并时我本人的改动行经逐行核对：与我改动行重叠的 fmt diff 均为分支既有代码的风格债
  （如 `apply_detected_metadata_locked` 体内的链式换行、`backup_file_name` 的链式开头），
  唯一一处由我引入的是 `save_store_atomic` 入口的多余空行，已删除（单独 commit）。
- 处理：按任务书与 P2-10 的计划，**不**全库重排；待 P2-11 拆分完成后单独提一个
  `style: cargo fmt 全库格式化` commit 清理（命令：`cargo fmt --manifest-path src-tauri/Cargo.toml`，
  之后全绿）。`ci.yml` 保留 `cargo fmt --check` 步骤，合入时预期为红。

**`cargo clippy --all-targets -- -D warnings`（基线即红；合并后与基线逐条一致，0 新增）：**

- 基线实测（Rust 1.100 / clippy 0.1.100，本次在 `0416f41` 上重跑确认）：lib **19** 个错误 +
  test **20** 个错误（19 个同源 + 1 个测试代码相关）。分布：`session_manager.rs` 17 处、
  `platform.rs` 2 处、`lib.rs` 1 处；种类：`let...else` 可改 `?`（3）、类型过复杂（2）、
  未使用变量 `proxy`（2）、可合并的 `if`（2）、多余 `return`、`clone` 后取切片、
  永不循环的 `loop`、函数参数过多（9/7、8/7、12/7）、`if` 两分支相同、可并入外层 `match`
  的 `if`、借用表达式已满足 trait、多余 `.into_iter()`、`Iterator::last` 误用。
- 合并后初测为 lib 22 / test 22，多出的 3 处均为合并引入，已修（单独 commit）：
  1. `storage.rs` 的 `File` import 只在 `#[cfg(not(unix))]` 分支用上（P1-7 的 0600 改动
     把 `File::create` 换成了 Unix 下的 `OpenOptions`），Linux 上成死导入：
     改用 `fs::File::create`（行为不变，各平台同款）。
  2. `ExternalAgentView::account_key` 仅测试调用，lib 目标的死代码检查看不到
     `#[cfg(test)]` 调用方：加 `#[allow(dead_code)]` 并注释。
     （教训：初版直接删除该方法，导致 2 处测试编译失败 E0599——调用在
     `session_manager.rs:13703/13722` 的测试里，此前 `grep | head` 截断了输出，
     没有数全调用点；已恢复并改用 allow。）
  3. `lib.rs` 的 `update_app_settings` Tauri 命令 8 个参数（P0-0 与 P0-3 各加一个可选开关），
     触发 `too-many-arguments`（8/7）：Tauri 命令参数由前端扁平传入，无法收成结构体
     而不改前端调用，加 `#[allow(clippy::too_many_arguments)]` 并注释。
- 合并后复测：**lib 19 / test 20，与基线逐条一致**（位置仅因插入行数平移）。
  基线既有的 19+20 个错误未修，待专门的 clippy 清理 commit 逐项修复
  （多为机械修复，可 `cargo clippy --fix` 辅助 + 人工复核）。
- P2-11 拆分后初测为 lib 23 / test 20，多出的 4 处均为拆分引入（父文件顶部 import 在
  代码移走后失去非测试调用方），已修（单独 commit `3d7e11d`）：`session_manager.rs`
  顶部的 `classify_top_level_url` / `NavigationDecision`（仅 `#[cfg(mobile)]` 的
  `commit_mobile_session` 与测试使用）改为 `#[cfg(any(mobile, test))]`；
  `GroupChatMessage` / `GroupThreadBinding` / `ExternalAgentKind` / `GroupMemberRole` /
  `GroupReplyStatus` / `GroupSharedSkill` / `GroupWorkflowMode`（仅桌面门控的测试经
  `super::*` 使用）改为 `#[cfg(all(desktop, test))]`。子模块均直接从 `crate::` 导入
  这些名字，不依赖父文件 import，故门控安全。拆分后复测：**lib 19 / test 20**，
  唯一错误位置集合（20 处）与基线逐条一致，**0 新增**。

### 4.6 共享 target 目录污染（构建环境备注）

- 多个并行任务曾共用 `$HOME/workspace/.cargo-target-maam`，且各 worktree 是同一 crate
  （同名同版本），构建脚本指纹可能碰撞：P0-1 的两个命令的 autogenerated 权限文件
  （`continue_with_empty_config.toml`、`restore_config_from_backup.toml`）在 P1-CLI 块与
  P0-1 块的构建中两次落入无关 worktree 的 `src-tauri/permissions/autogenerated/`。
  均已删除、未提交。合并阶段改用独立目录
  `$HOME/workspace/.cargo-target-maam-consolidated`，每次跑 cargo 前确认 cwd；
  提交前检查 `git status` 有无陌生 autogenerated 文件。合并后工作树干净，无陌生文件。
- 独立 target 目录首次全量编译需重装 Tauri Linux 构建依赖（VM 替换后 `/usr` 易失，
  旧缓存产物不在）：`libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libssl-dev
  libappindicator3-dev`；其中 `libzstd1` 需单包降级（`1.5.5+dfsg2-2build1.1+hatch2` →
  `1.5.5+dfsg2-2build1.1`，同上游版本，仅 Debian 修订号不同）。

## 5. 手工验证步骤（无法自动测试项汇总）

### 5.1 更新器（P0-0 / P0-2）

- 未配置仓库默认关闭：不设 `MAAM_UPDATE_REPO` 编译；已配置仓库的构建上验证：
  设 `MAAM_UPDATE_REPO=<自己的 fork>` 后构建，启动后 3 秒内应发起一次静默检查
  （有 Release 时更新按钮出现红点）；设置 → 勾选「禁用自动更新检查」→ 保存 → 重启，
  启动后不应再自动检查；手动点「更新」仍可检查。不设 `MAAM_UPDATE_REPO` 构建：
  启动后不发起检查；点「更新」打开弹窗，显示未配置说明，无「立即下载并更新」与「发布页」按钮。
- 安装时重新取清单（需联网 + 自有仓库有 Release）：正常检查更新得到结果后，在安装前于仓库
  删除/替换该 tag 的安装包（或改 `latest.json` 的 sha256），再点「立即下载并更新」，
  应被拒绝并提示清单不一致；用旧版检查结果（已下架版本）点安装，应被拒绝。
- macOS（去隔离与 swap 脚本）：在 macOS 上以自有仓库构建并安装旧版，发布含正确 sha256 的新版，
  点更新：更新后新版 `.app` 应能正常启动（Gatekeeper 不拦截，说明去隔离发生在替换成功后）；
  故意把 `latest.json` 的 sha256 改错再点更新：应提示校验失败，旧版 `.app` 完好、仍可启动；
  swap 超时：更新时让旧进程 10 秒内不退出（如挂起），10 秒后 swap 脚本应中止，
  旧应用仍在原位、可手动恢复使用。
- Windows（哈希通过才启动安装程序）：同上改错 sha256 再点更新，应只提示校验失败，
  不应弹出安装程序窗口。
- Android（P0-2-1 的 Kotlin 侧缺口）：真机上走一遍应用内更新，确认 `expectedSha == null`
  时的行为与 FileProvider 声明（见 3.1 P1 待实测）。

### 5.2 配置加载保护（P0-1）

- 损坏配置端到端（Linux 手动模拟，等价于真实损坏）：备份好 `accounts.json` 后改成非法 JSON，
  启动应用。预期：应用回到管理页，提示含 `已备份为 accounts.json.corrupt-<时间戳>`；
  同目录出现该备份，内容与损坏前一致；原文件仍在。任一改动均应失败并提示写盘被阻止，
  文件字节不变。调 `restore_config_from_backup`（或等前端按钮）：应恢复到 `.bak` 的内容；
  调 `continue_with_empty_config`：应清空并恢复写盘，`.corrupt-*` 备份仍在。
- 高版本只读（模拟降级）：把 `accounts.json` 的 `version` 改为 99 后启动。预期：账号正常显示，
  提示已设为只读；任何改动失败；退出重进后文件仍是 v99 原样。
- 未知 provider 往返：在 `accounts` 里加一条 `providerId: "futurechat"` 的账号，启动后该账号
  不在列表里，改一次设置退出重进，检查文件里该条目仍在且字段原样。
- 桌面端实机：打开管理页，确认无 capability 报错；损坏配置场景下确认 notice 与两个新命令可调通
  （前端按钮接线后）。
- 移动端：`load_or_migrate_store` 与 `persist_locked` 同样走移动端代码路径，但未在真机验证；
  `.corrupt-*` / `.bak` 落在应用数据目录内，需实机确认可见性。
- 前端接线要点（给前端 agent）：`snapshot.persistBlocked` 为 true 时，在 notice 条旁加两个按钮：
  「以空配置继续」→ `continue_with_empty_config`；「从备份恢复」→ `restore_config_from_backup`。
  两个命令返回 `ManagerSnapshot`（成功）或错误字符串（失败时前端展示即可）。
  不要用 `dismiss_notice` 解除阻止——它只关提示。`snapshot.readOnly` 为 true 时（高版本配置），
  建议只提供「以空配置继续」，不要提供「从备份恢复」之外的写盘入口。

### 5.3 自动审批（P0-3，桌面端 macOS / Windows 实测）

- 熔断提示与持久化关闭：开启自动审批，打开一个账号页面，连续触发 11 次以上审批卡
  （或在页面控制台连点单次授权按钮 11 次）。预期：第 11 次起不再自动点击；触发一次静默同步
  （或等 5 分钟）后，主界面出现 notice「自动审批在 60 秒内自动点击超过 10 次，已自动关闭」，
  设置里的自动审批变为关闭，重启应用后仍为关闭。检查应用数据目录下的 `auto-approve-audit.log`，
  应有 10 行 JSONL，每行含 `at_ms`、`provider_id`、`uuid`、`text`、`button`。
- 仅有「始终允许」的卡：开启自动审批，打开出现只有「始终允许」按钮的审批卡的页面。
  预期：不自动点击；触发同步后主界面出现 notice「检测到仅有「始终允许」的审批，自动审批已跳过，
  请前往该账号页面手动处理：<卡片文本>」；手动点「始终允许」仍可正常授权。
- 命令执行开关：总开关开、命令开关关：出现 bash / shell 命令审批卡，不自动批准，同步后有 notice；
  再开启命令开关（不重载页面，保存后即同步到保活 webview），同类卡恢复自动批准（危险命令仍不批）。
  旧配置升级：把 `accounts.json` 里 `settings.autoApproveCommands` 字段删掉再启动，
  开关应为关闭，且不影响原有 `autoApprove` 值。
- Windows 的 WebView2 与 macOS 的 WKWebView 初始化脚本行为需在对应平台按上述步骤实测。
  移动端不涉及（自动审批注入脚本为 `#[cfg(desktop)]`）。

### 5.4 CLI 与 API 超时（P1-4/5/6）

- P1-5(b) 的 Windows Job Object 分支（Unix `killpg` 分支已单测）：Windows 上超时/取消后
  检查孙进程是否残留。
- P1-5(d) 全部（Windows）：`npm i -g @openai/codex`，确认 `%APPDATA%\npm\codex.cmd` 存在、
  `codex` 在 PATH 里。群聊里新建本地 CLI 成员，命令填 `codex`，参数留空（走 stdin），
  发一句话：应正常出回复，不再报「找不到程序」。同一成员参数改成 `exec {prompt}`：
  保存/发送时应看到中文错误，提示去掉 `{prompt}` 改用 stdin。命令填不存在的名字：
  应看到中文错「已按 PATH × PATHEXT 查找，请确认已安装并加入 PATH」。prompt 含 `"` 或 `%`
  （如 `说 "你好" 100%`），走 stdin 的成员应正常执行，不报批处理参数错误。
- P1-5(e) 的 `login_shell_path`（macOS）：在 `~/.zshrc` 里确认有 nvm/volta/bun 的 PATH 配置；
  从 Finder 启动本应用（不要从终端启动），群聊里新建本地 CLI 成员，命令填 `node` 或 `bun`，
  发一句话：应正常执行。对照：把 `~/.zshrc` 里的 PATH 配置临时注释掉，从 Finder 重启应用，
  同一成员应报找不到（证明走的是登录 shell 的 PATH）。把 `$SHELL` 改成不存在的路径再启动：
  应用应正常启动、CLI 回退到硬编码列表，不卡住（3 秒超时内返回）。
- P1-7：Linux 桌面（有 secret service）：新建 API 成员填 Key → 检查 `accounts.json` 不含该 Key
  明文，`secret-tool lookup` / Seahorse 中能看到 `multi-agent-account-manager` 条目；重启应用，
  群聊 API 成员仍可正常调用；编辑成员不改 Key，保存后仍可调用。Linux 无 secret service：
  检查同目录 `api-keys.json` 权限为 `0600`，内容为引用 id → Key 的映射；`accounts.json` 仍无明文。
  前端：开发者工具看 `get_manager_state` 返回的 `externalAgents`，确认只有 `hasApiKey`，
  无 `apiKey` 字段；编辑成员时 Key 输入框为空，留空保存后 `hasApiKey` 仍为 true。
  旧配置迁移：手写一份含明文 `apiKey` 的 `accounts.json`，启动一次，检查文件被重写且明文消失，
  Key 已进 keyring/回退文件，重启后调用正常。Windows/macOS：keyring 后端分别为
  Credential Manager / Keychain，需在对应平台实测。
- P1-8：造大群聊（或直接用基准测试的 store），连续发送多条群聊消息，观察 UI 不卡顿；
  检查 `accounts.json` 内容正确。快速连续触发多次 `let _ =` 路径的保存（如连续导入 Cookie），
  用 `stat` 观察写盘次数明显少于触发次数（防抖合并）。模拟写盘失败（把 `accounts.json`
  所在目录改为只读）：用户主动操作应返回中文错误并回滚内存状态；防抖路径应在 UI 出现
  「配置自动保存失败」提示。

### 5.5 CSP 与 CI（P2-9/P2-10，dev/build 模式）

- dev 模式：`npm run tauri dev`，主界面正常渲染；开发者工具控制台无 CSP 报错；
  控制台执行 `document.createElement('form')` 并提交、插入 `<iframe src="https://example.com">`、
  `<object data="x">`，均应被 CSP 拦截并在控制台留下 `Refused to ...` 记录；页面其余功能不受影响。
- build 模式：`npm run tauri build`，安装后启动，重复 dev 步骤；另可解包检查 CSP 元信息，
  确认四条新指令存在。Linux 上只产出可执行文件、无 `bundle/` 目录，属预期行为
  （`tauri.conf.json` 的 bundle targets 是 `dmg` / `nsis`）。
- CI：Windows 任务（`windows-2025`）只能等 CI 首次运行验证；`ci.yml` 中已写明这一点。

## 6. 验证命令与结果（平台说明）

### Linux（本机 VM，Rust 1.100，Node v24）——实际跑过

| 命令 | 结果 |
|---|---|
| `npm ci` | 通过 |
| `npx tsc --noEmit` | 通过，exit 0 |
| `npm run test:js`（jsdom，自动审批注入脚本） | 15/15 通过 |
| `cargo test`（全量） | exit 0：lib **93 passed / 0 failed**；bin 0 测试；doc 0 测试 |
| `cargo fmt --check` | exit 1，260 处 diff（合并完成时 237 处；拆分后 260 处，见 4.5 节） |
| `cargo clippy --all-targets -- -D warnings` | exit 101，lib 19 / test 20（见 4.5 节） |
| `npm run build`（tsc + vite build，拆分后补跑） | 通过，exit 0；11 个模块打包，`dist/assets/index-*.js` 133.13 kB（gzip 36.60 kB） |
| `npx tauri build`（P2-10 分支上跑过，含新 CSP 配置） | 通过，exit 0，产物 20.6 MB；未安装、未运行 |
| `npx tauri build --debug`（最终分支，拆分完成后） | 通过，exit 0；产物 debug 二进制；仅 2 处基线既有的 `unused variable: proxy` 警告 |

- **环境事故（2026-11-26 晚，验证期间）**：VM 被替换，`/usr` 下的 Tauri -dev 包丢失，
  最终二进制链接失败（`rust-lld: unable to find library -lgdk-3 / -lgdk_pixbuf-2.0`），
  与代码无关——当时 `cargo test --lib` 复用已缓存的测试二进制仍能跑，强制重链接
  （`touch src/lib.rs` 后 `cargo test --lib --no-run`）同样失败，证实是环境问题。
  按 4.6 节重装 `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libssl-dev
  libappindicator3-dev libgdk-pixbuf2.0-dev` 后恢复（`libzstd1` 降级本次需加
  `--allow-downgrades`，否则 apt 拒绝执行）；重装后 `cargo test --lib` 重新全绿 93/93，
  `tauri build --debug` 通过。**release 构建在最终分支上重跑时于约 14 分钟被外部
  SIGTERM 中断（正值 VM 替换，非构建错误），未完成**；最后一次成功的 release 构建
  仍为 P2-10 分支的 20.6 MB 产物。拆分未改任何公开 API、Tauri 命令、Cargo.toml 与
  前端入口，debug 打包已验证链接与打包流程。

- 基线对照：在独立工作树 `~/workspace/maam-wt-baseline`（`0416f41`）上用独立
  `CARGO_TARGET_DIR` 重跑确认——`cargo test --lib`：**26 passed / 0 failed**（合并后 93，
  +67）；`cargo fmt --check`：默认配置 247 处 diff，`tab_spaces = 2` 配置下 200 处；
  `cargo clippy --all-targets`：lib 19 / test 20。
  （分块笔记里基线测试数有 26 / 31 / 23 三种说法，以本次实测的 26 为准。）
- 合并后每次合并提交都跑过 `cargo test --lib`（全绿才继续）：合并 5 后 68 passed、
  合并 6 后 80 passed、合并 7 后 93 passed。
- P2-11 拆分每步提交前都验证通过才继续：Rust 侧 7 步每步 `cargo test --lib` 93/93；
  前端侧 4 步每步 `npx tsc --noEmit` 全绿；拆分完成后全量复测（`npm ci`、`npx tsc`、
  `npm run test:js` 15/15、`npm run build`、`cargo test` 全量 lib 93/93、`cargo fmt`、
  `cargo clippy`）结果见上表与第 10 节。

### Windows / macOS / Android / iOS —— 只做代码审查，未实际跑过

- `cfg(windows)` / `cfg(target_os = "macos")` 代码、Android Kotlin、iOS Swift、NSIS
  在 Linux 上编译不到，只做了代码审查与手工验证步骤（第 5 节）。
- 凡标注「待实测」的项（P1-5(b) Windows Job Object、P1-5(d) 全部、P1-5(e) macOS 登录 shell、
  P1-7 Windows/macOS keyring 后端、P0-2-5/2-7 macOS/Windows、Android FileProvider、
  iOS 删除竞态、时区脚本验证码影响、Windows CI 任务）均未在对应平台跑过，
  不要当作已验证。
- `npx tauri build` 的产物未安装、未运行（按任务要求）。

## 7. 未修复项（含原因）

1. **`cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 基线即红，未修。**
   见 4.5 节。fmt 待 P2-11 拆分完成后的单独 `style: cargo fmt` commit；clippy 待专门的
   清理 commit（多为机械修复，可 `cargo clippy --fix` 辅助 + 人工复核）。
   `ci.yml` 的两个步骤合入时预期为红。
2. **P0-1 前端按钮未接线。** `continue_with_empty_config` / `restore_config_from_backup`
   两个 Tauri 命令与 `snapshot.persistBlocked` / `readOnly` 字段已就绪，`src/main.ts`
   尚未加按钮（notice 文案已告知两个选择）。待前端 agent 按 5.2 节接线。
3. **Kotlin 侧 sha256 仍是可选参数**（P0-2-1 的 Android 分支）：Rust 侧已在调用前强制要求，
   插件内的可选逻辑现为纵深防御缺口而非可达路径；如后续直接调用插件（绕开 Rust 命令）
   仍可跳过校验。建议 Kotlin 侧也改为必填（`expectedSha == null` 时拒绝安装并删除已下载文件、
   复用分支一律重算摘要）。
4. **Android 更新重定向不校验主机**（3.1 P1）：桌面端已修，Android 端因本次任务范围只到 Rust 侧、
   且本地无法编译验证 Android，未修。建议后续在 Kotlin 侧对每一跳加与桌面端相同的白名单。
5. **第二轮审查的 49 项中除 P0-2 已堵住的两环外，其余均未修**：P1 5 项（NET_HOOK 主框 IdP 读响应体、
   `__maam*` 伪造、时区脚本注入验证码 iframe、iOS 子框不限 host、Android FileProvider 待实测）
   与 P2 33 项（剪贴板 Cookie 永不清、Cookie `;` 注入、NSIS 三项、iOS 下载缺口等）均为
   报告-only，按严重程度排期修。其中最值得优先修的 3 项见 3.8 节末。
6. **P1-8 增量写未做**：群聊消息拆文件（jsonl）或 SQLite 迁移只写了方案，未实现。
   当前仍是每次整库重写；防抖只减少写盘次数，不减少单次写盘成本。
7. **P1-5(d)(e) 与 P1-5(b) 的 Windows 分支、P1-7 的 Windows/macOS keyring 后端、
   P0-2-5/2-7 的 macOS/Windows 分支**：代码已写、只做了审查，未在对应平台实测（见第 5 节）。
8. **审计日志无轮转**：`auto-approve-audit.log` 无轮转/清理，长期高频点击会持续增长；
   如需控制体积，后续可加按大小轮转。
9. **P1-7 的 vault 实例每次新建**：每次 upsert/delete 都 `ApiKeyVault::new(...)` 新建实例，
   keyring 可用性探测（一次 D-Bus 往返）每次都重做。已保留该写法（改动面小、正确性优先），
   后续优化点：应在 `SessionManager` 里持有一个 vault 实例。
10. **`release.yml` 的 action 未固定 SHA**：P2-10 只改了新增的 `ci.yml`，`release.yml`
    按任务要求保持不动；对照表见 P2-10 分块笔记（7 处 `uses:` 的 SHA），供后续单独 commit 使用。
    换完后跑一次 `workflow_dispatch`（`publish_release: false`）验证。

## 8. 新发现项汇总（本次工作新确认的、任务书清单之外的）

- `open_external_url` 不校验主机（P2，已修，见 1 节 P0-2 末）。
- Android APK 更新 sha256 可空（3.1 P0；Rust 侧已修，Kotlin 侧待跟进）。
- Android FileProvider 未在仓库内声明（3.1 P1，待实测）。
- 桌面端 NET_HOOK 在主框 IdP 登录页读响应体（3.4 P1）。
- 页面 JS 可伪造 `window.__maam*` 全局变量（3.4 P1）。
- 时区脚本注入验证码 iframe（3.3 P1，待实测）。
- 剪贴板 Cookie 永不清（全平台 P2）。
- Cookie 值 `;` 注入（3.3 P2，已用临时测试验证）。
- iOS 子框导航不限 host、iOS 无下载实现（3.2 P2）。
- Android HTTP 下载无大小上限、允许 https→http 降级（3.1 P2）。
- NSIS 注册表安装目录信任、WiX 卸载串执行、WebView2 无哈希下载（3.5 P2）。
- iOS 删除 Profile 与存活 WebView 的竞态（3.2，待实测）。
- `SameSite=None` 不带 `Secure` 被浏览器静默丢弃（3.3 P2）。
- `findAuthPrompt` 的两份拷贝（群聊发送 JS）仍用旧正则识别 `always allow` 为授权提示——
  仅用于检测、不点击，与 P0-3 修复不冲突，留待后续统一抽取时处理。
- P0-1 的已知局限：未知账号的引用位置（群成员 / `lastCommitted` 等）仍会被丢弃；
  未知 provider 账号 UUID 非法时仍整体失败（但已有备份 + 写盘阻止）。
- 合并中新发现：防抖写线程与熔断直写会绕过 `persist_blocked`（已修，见 4.3 节）；
  `update_app_settings` 命令因两路开关新增参数触发 clippy `too-many-arguments`（已 allow，见 4.5 节）。

## 9. 过程记录：重复派发与并行改动（按事实）

- 本次加固任务只派发过**一个**协调人，没有重复派发过任务。
- 执行期间疑似运行时重启导致任务被重复派发：在 `maam-wt-p1-keyring` 工作目录，
  2026-11-26 02:15 UTC 左右出现另一方并行改动（`session_manager.rs`、`src/main.ts`、
  `src/types.ts` 的 mtime 为 02:15:01–02:15:18），内容恰好是该任务计划中的 commit 5
  （`get_manager_state` 只返回 `hasApiKey`、编辑留空不修改、删除时清 vault）；
  同一时刻（02:16 UTC）有另一进程（PID 57002/57006）在该工作目录跑 `cargo test --lib`
  与 `npx tsc --noEmit`（用的是工作目录内 `src-tauri/target`，不是共享 `CARGO_TARGET_DIR`）。
- 处理：未直接丢弃；逐行审查后采纳，并修复了其中发现的两个缺陷——（1）`upsert_external_agent`
  先写 vault、再写盘：写盘失败回滚内存时未回滚 vault，新 Key 会残留在 vault 里，
  下次启动会用「编辑失败」的新 Key（02:21 UTC 的后续改动已补上回滚；本任务另把相关测试断言
  改为与后端无关）；（2）成员 kind 由 `openai_api` 改为 `local_cli` 时，旧 vault 条目成孤儿
  （02:21 UTC 的后续改动已补清理，并补了专项测试）。另有同名交错提交：
  `9ab03d3`（02:25:24 UTC，本任务提交）与 `e0a9d08`（02:30:43 UTC，并行方提交，仅改笔记 1 行）。
- 另有两起疑似共享 target 目录污染（见 4.6 节）：P0-1 的两个命令的 autogenerated 权限文件
  两次落入无关 worktree，均已删除、未提交；合并阶段改用独立 target 目录后未再出现。
- 建议：以后给并行 agent 配不同 git 身份（`user.name` / `user.email`）以便追溯；
  各 worktree 用独立 `CARGO_TARGET_DIR`，或至少提交前检查 `git status` 有无陌生
  autogenerated 文件。以上是事实记录，不作超出证据的解释。

## 10. P2-11 结构拆分（状态：已完成，2026-11-26）

任务书要求仅在 P0 和 P1 全部完成并有测试后进行：`session_manager.rs` 按职责拆成
store/persist、webview 生命周期、群聊引擎、A2A、额度同步、注入脚本；
`main.ts` 拆成 state、render、modals、group-chat；每一步 `cargo test` 必须全绿，
不要顺手改逻辑。**已完成，11 个拆分 commit + 1 个 clippy 修复 commit，均在
`hardening/consolidated` 上；行为不变，未改任何逻辑。**

### Rust 侧（7 个 commit）

`src-tauri/src/session_manager.rs` 14,910 → **4,397 行**（含约 1,700 行测试模块，
测试模块未动）。拆出 7 个子模块（`session_manager/` 下，均为 `session_manager` 的
子模块，可访问父级私有项；移出的方法改为 `pub(super)`，`Self::` / `self.` 调用
跨模块照常解析）：

| commit | 模块 | 行数 | 内容 |
|---|---|---|---|
| `504a829` | `scripts.rs` | 3,544 | 5 个 `*_INIT_JS` 常量 + 9 个 `build_*_js` 函数，原样移出 |
| `8a95e9a` | `quota.rs` | 153 | `AccountQuotaEval` + 额度耗尽/偏低判定 + `evaluate_account_quota` |
| `7ff9b38` | `store.rs` | 367 | `snapshot_from_locked`、`persist_locked`/`persist`、P0-1 解锁路径、快照下发、防抖持久化写线程、待清理 profile |
| `2e4e5de` | `a2a.rs` | 359 | `select_a2a_relay_targets`、`run_group_review_pipeline`、`run_group_a2a_chain` |
| `00d0d9e` | `group_chat.rs` | 3,773 | 55 个群聊引擎方法 |
| `3b3fb17` | `webview.rs` | 1,521 | 暖池、账号 webview 构建、预热、桌面弹窗注册、`is_view_instance_valid` |
| `f80341e` | `quota_sync.rs` | 1,003 | 身份探针、cookie 过期刷新、检测到的元数据应用、登出/页面错误状态应用 |

父文件剩余：核心/启动、账号管理、移动端生命周期、结构体定义、`mod tests`。
每步提交前均跑 `cargo test --lib`，全程 93/93 全绿。

### 前端侧（4 个 commit）

`src/main.ts` 5,217 → **2,575 行**。拆出 4 个模块，依赖方向无环：
`state.ts` ← `modals.ts` ← `render.ts` ← `group-chat.ts` ← `main.ts`。
未做 `appState` 对象重写（风险太大）；移出的函数按同名参数透传所需状态，
函数体一字未动。每步提交前均跑 `npx tsc --noEmit`（全绿）+ 拆分完成后补跑
`npm run build`（vite build 通过）与 `npm run test:js`（15/15）：

| commit | 模块 | 行数 | 内容 |
|---|---|---|---|
| `a6fb506` | `state.ts` | 785 | 静态常量（预设/图标）、主题状态与函数、共享可变集合（`groupDrafts` 等 Record、`syncingMsgIds` 等 Set、`updateState`）、8 个滚动量组成的 `scrollState` 对象、纯辅助函数；`types.ts` 补 `AccountDraft` / `ModalMode` / `ProviderLogoSpec` |
| `83a0122` | `modals.ts` | 823 | `renderModal`（+ `modalOverlayClass`）、`renderUpdateModal`、4 个 `open*Modal`、纯函数 `groupModalVisualSignature`；`lastRenderedUpdateStage` 留在此模块，main.ts 经 setter/getter 读写 |
| `65ca00d` | `render.ts` | 517 | `renderMiddleContent`、`renderRightStage`、`renderQuotaDetailPanel`、`renderDownloadToast`、`currentProvider`、`saveScrollPosition` / `restoreScrollPosition` |
| `079a3fd` | `group-chat.ts` | 744 | `renderGroupMiddleContent`、`renderGroupRightStage`、`scanTextMentions`、`extractComposerAtMentions`、`renderMessageTextWithMentions`、`formatReviewStageChain`、`formatComposerHintText`、`getLatestRoundCompletedReplies`；共享的 `isAsciiWordChar` 移入 `state.ts`（两边共用） |

行为保持的签名调整（仅参数透传，函数体不动）：`syncViewModeFromSnapshot` 改为
返回下一视图模式（或 null）而非直接赋值全局；`effectiveSwitchingKey` 增加
`optimisticSwitching` 参数；`renderModal` 增加 `modal` / `shouldAnimateModal`
参数；`renderUpdateModal` 增加 `snapshot` / `shouldAnimateModal` 参数；
`open*Modal` 增加 `setModal` 回调参数；`renderMiddleContent` /
`renderGroupMiddleContent` 增加 `modal` / `downloadToast` / `shouldAnimateModal`
等参数；`renderRightStage` / `renderGroupRightStage` 增加 `optimisticSwitching` /
`groupSending` / `newTopicConfirmGroupId` 参数；`saveScrollPosition` 取
`(snapshot, viewMode)`、`restoreScrollPosition` 取 `(snapshot, viewMode, modal)`；
`renderDownloadToast` 取 `downloadToast` 参数。

### 拆分引入的问题与修复

- 拆分后 clippy 初测 lib 23（基线 19）：父文件顶部 4 组 import 失去非测试调用方。
  已修（commit `3d7e11d`，门控而非删除，见 4.5 节）：复测 lib 19 / test 20，
  与基线逐条一致，0 新增。
- 拆分后 fmt：237 → 260 处 diff（新增约 23 处，集中在新子模块 import 块与签名折叠），
  仍按计划留待单独的 `style: cargo fmt` commit 清理，不在本任务内重排。
- 另有 2 处预存的 rustc `unused variable: proxy` 警告（`test_account_proxy` 等处，
  `let proxy = platform::parse_account_proxy(...)`），基线即有，随代码平移，未动
  （纯重构范围外）。
