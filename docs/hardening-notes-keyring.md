# 加固笔记：P1-7 API Key 进 keyring / P1-8 写盘防抖

分支：`hardening/api-key-keyring`（基线 v0.1.10，提交 `0416f41`）
工作目录：`~/workspace/maam-wt-p1-keyring`
范围：只动 API Key 存储与写盘防抖/异步化；不碰加载失败处理（P0-1 并行任务的范围）。

## 逐条确认结论

### P1-7 API Key 明文落盘并下发到前端

| 任务书说法 | 结论 | 依据 |
|---|---|---|
| `storage.rs` 约 193 `api_key: String` 明文 | **确认** | `ExternalAgentRecord.api_key` 经 `#[serde(rename_all = "camelCase")]` 以 `apiKey` 明文序列化进 `accounts.json`；`migrate_from_value`（约 612–617）直接读 `apiKey`/`api_key` 明文 |
| `save_store_atomic` 约 1057 `File::create` 默认权限 | **确认** | `storage.rs` `save_store_atomic` 用 `File::create(&tmp_path)`，临时文件权限取 umask（通常 0644），`accounts.json` 继承该权限 |
| `types.ts` 约 106、`main.ts` 约 2224 和 4596 编辑时明文回填 `<input>` | **确认** | `types.ts` `ExternalAgentRecord.apiKey: string`；`ManagerSnapshot.external_agents: Vec<ExternalAgentRecord>`（`session_manager.rs` 约 3732）经 `get_manager_state` 全量下发前端；`main.ts:2224` `apiKeyDraft: editAgent?.apiKey ?? ""`、`main.ts:4596` `modal.apiKeyDraft = ext.apiKey` 把明文填进 password 输入框（DOM 中可读，见任务书要求 3） |
| 运行时确实需要明文 Key | **确认** | `group_transport.rs:226–227` `execute_openai_api_turn` 用 `agent.api_key` 做 `bearer_auth`；内存中的记录必须保留 Key，禁的是落盘与下发 |

设计：
- 新增 `src-tauri/src/key_vault.rs`（`ApiKeyVault`）：桌面端优先用 `keyring` crate（系统凭据库），`accounts.json` 里只存引用 `apiKeyRef: "maam-api-key:<agent uuid>"`；`keyring` 不可用（Linux 无 secret service）或移动端（Android/iOS 不编译 keyring）时回退为 `<app_data_dir>/api-keys.json`，Unix 下临时文件 `OpenOptions::mode(0o600)` + 原子 rename。
- `ExternalAgentRecord.api_key` 改为 `#[serde(skip_serializing)]`（只留内存），新增 `api_key_ref: Option<String>`（序列化为 `apiKeyRef`）。
- 首次启动迁移：在 `load_or_migrate_store` 成功加载后，对含旧明文且无 `apiKeyRef` 的成员：写 keyring/回退文件 → 置 `apiKeyRef` → 标记 dirty 写盘（写盘后 JSON 里不再有明文）；含 `apiKeyRef` 的成员从 vault 取回内存 Key。
- `get_manager_state`/`ManagerSnapshot.external_agents` 改为 `ExternalAgentView`：除 `api_key` 外字段相同，另加 `hasApiKey: bool`。编辑时前端 `apiKeyDraft` 恒为空，留空表示不修改；后端 `upsert_external_agent` 收到空 Key 且为编辑时保留旧 Key。
- 删除成员时同步删 vault 条目（失败忽略，不阻塞删除）。

### P1-8 写盘阻塞异步运行时且每次整库重写

| 任务书说法 | 结论 | 依据 |
|---|---|---|
| `save_store_atomic` 在持有 `inner`（`tokio::sync::Mutex`）时被同步调用 | **确认** | `session_manager.rs` 内 37 处生产调用点全部处于 `self.inner.lock().await` 的守卫作用域内（逐点核对，见下表分类）；`save_store_atomic` 本身做 `to_vec_pretty` + `write_all` + `sync_all`，全程同步阻塞调用线程 |
| 每次整库重写（含全部群聊消息） | **确认** | `save_store_atomic` 总是对整份 `AccountsStoreFile` 做 `to_vec_pretty`；无增量写 |
| 聊天历史变大后卡锁、卡 UI | **静态推断，需实测** | 见下方基准；另注意：`migrate_from_value` 在加载时把每群消息截断到 120 条（`storage.rs` 约 1030），所以磁盘文件天然有界；但同一会话内内存消息可无界增长（`send_group_message` 追加时不截断），写盘成本随会话内增长 |

调用点分类（`session_manager.rs`，commit 8 后）：
- 错误传播 + 回滚（`if let Err … return Err` / `?`）：4780、4899、5402、5498、5628、5658、5720、5842、6175、7307、7912、9302、9357、9459、9506、9533、9621、9650、9683、9713、9763、9800、10230、10543、10712 —— commit 8 改为 `persist_and_relock`（持锁克隆 → 释放锁 → `spawn_blocking` 写盘 → 重新加锁回滚/快照），错误语义不变。注意：释放锁到重新加锁之间，其他任务可能已修改 store；失败回滚只恢复本次改动的字段，极小概率覆盖并发改动（仅写盘失败路径）。
- `let _ =` 忽略错误：4228（启动候选清理）、4276（`retry_pending_cleanups`）、4802（`hide_foreground_slot`）、5283/5335/5439（`write_imported_cookies_to_store`）、10419（`run_group_member_turn_with_failover`）、10791（`update_group_message_state`）、11054（`persist_group_thread_binding`）—— commit 9 改为防抖 `request_persist()`。
- `is_err()` 后静默返回：8591、8648、8838（页面错误/ Cookie 过期摘要）、12739（测试辅助）—— 8648 与群聊 A2A 链路（`run_group_a2a_chain`）commit 8 改为异步写盘后判错；8591、8843 在同步 `*_locked` 辅助函数内（`apply_logged_out_state_locked`、`apply_detected_metadata_locked`，无法 await），保持同步调用（低频的页面错误/元数据路径），见下。

commit 8 的例外（保持同步 `save_store_atomic`）：
- `apply_logged_out_state_locked`、`apply_detected_metadata_locked` 内的两处：同步函数、入参是 `&mut InnerState`，无法 await；调用方本就持有锁，且是低频路径（页面错误/元数据探测），保留同步写盘。

commit 9 防抖（已实现）：
- `SessionManager` 增加 `persist_epoch: AtomicU64` + `persist_notify: Arc<Notify>` + `persist_writer_started: AtomicBool`；`request_persist()` 只做 epoch+1 并 notify，不持锁、不写盘；写线程在 `run_startup_tasks` 开头以 `ensure_persist_writer(app)` 启动（只启动一次），测试用 `ensure_persist_writer_no_app()`（不带 AppHandle；`tauri::test::mock_app` 需要给 tauri 依赖加 `test` feature 才能用，本任务不改构建配置）。
- 写线程 300ms 静默窗口后：短暂持锁克隆 store → 释放锁 → `spawn_blocking` 写盘；失败写 notice「配置自动保存失败：…」并保留 dirty，等下一次请求重试（成功时发快照，见 `emit_snapshot`）。
- 写线程用 `tokio::task::spawn` 而不用 `tauri::async_runtime::spawn`，使它在 `#[tokio::test]` 下同样能启动（生产跑在 tauri 的 tokio 运行时上，两者等价）。
- 行为变化：防抖路径的写盘不再同步发生；进程在最后一次修改后 300ms 内退出可能丢最后一次自动保存（用户主动操作走同步 `persist_and_relock`，不受影响）。
- 测试：`debounced_persist_coalesces_rapid_requests_into_one_write`（10 次快速请求 → 1 次写盘；之后再请求 → 第 2 次写盘；磁盘内容与内存一致）与 `debounced_persist_failure_writes_notice_and_retries`（把 accounts.json 换成同名目录使写盘失败 → 出现失败 notice 且不计写盘；恢复可写后重试成功）。仅测试字段 `persist_write_count: AtomicU64` 计数成功写盘。
- 明确的行为变化：防抖路径的写盘不再同步发生；进程在最后一次修改后 300ms 内退出可能丢最后一次自动保存（用户主动操作走同步 `persist_and_relock`，不受影响）。

第二阶段方案（本次只写方案，不实现）：
- 群聊消息拆文件：每群一个 `<group_id>.jsonl`（每行一条消息），`accounts.json` 只保留群元数据（id/名称/成员/共享上下文/技能/角色）；加载时按需读，追加消息时 append-only 写一行；清空/删群时删文件。需处理：旧文件一次性迁移、跨文件原子性（群元数据与消息文件分两步写，崩溃后以元数据为准重建索引）、备份/恢复要打包整个目录。
- 或迁移到 SQLite：`sqlx`/`rusqlite` 单文件库，消息表按 `(group_id, created_at_ms)` 索引；写盘变事务提交，天然增量。代价：引入新依赖与迁移代码，移动端需验证。
- 建议先做 jsonl（改动面小、无新依赖），待消息量或查询需求再上 SQLite。

## 基准（P1-8）

- 首次测量（`35ed6b7`，debug 构建 + 多任务并行编译，绝对值偏高）：
  - `save_store_atomic`：1 群 × 5 万条消息约 **3.01s / 25.8MB**；500 群 × 100 条约 **3.81s / 25.9MB**。
- `save_store_atomic_async` 对比（`5f1ff32`，同条件）：sync 约 2.80s，async 约 3.82s，结果逐字节一致。单次写盘墙钟不缩短（同一份工作，只是换了线程；async 数值偏高是并行编译的 CPU 争抢噪声）。真正的收益是：写盘不再占用异步运行时工作线程（commit 7）、写盘期间释放 `inner` 锁（commit 8）、多次写盘合并为一次（commit 9 防抖）。
- 注意：`migrate_from_value` 加载时把每群消息截断到 120 条，所以磁盘文件天然有界；但同一会话内内存消息可无界增长（`send_group_message` 追加时不截断），写盘成本随会话内增长。基准用直接构造的 store 测最坏情况。
- 防抖前后对比（commit 9，`debounced_persist_coalesces_rapid_requests_into_one_write`，小 store）：10 次快速请求 → 实际写盘 1 次（未防抖时为 10 次整库重写）；第二轮再请求 1 次 → 写盘总数 2 次。单次写盘耗时本身不变（仍是整库 `to_vec_pretty` + `sync_all`，见上），防抖省的是重复写盘的次数。

## 手工验证步骤

### P1-7
1. Linux 桌面（有 secret service，如 GNOME Keyring / KWallet）：新建 API 成员填 Key → 检查 `accounts.json` 不含该 Key 明文（`grep`），`secret-tool lookup` / Seahorse 中能看到 `multi-agent-account-manager` 条目；重启应用，群聊 API 成员仍可正常调用（内存 Key 已从 keyring 恢复）；编辑成员不改 Key，保存后仍可调用。
2. Linux 无 secret service：`dbus-run-session -- env -u DBUS_SESSION_BUS_ADDRESS …` 或直接在无桌面会话的终端启动（预期走回退）：检查同目录 `api-keys.json` 权限为 `0600`，内容为引用 id → Key 的映射；`accounts.json` 仍无明文。
3. 前端：打开浏览器开发者工具看 `get_manager_state` 返回的 `externalAgents`，确认只有 `hasApiKey: true/false`，无 `apiKey` 字段；编辑成员时 Key 输入框为空，留空保存后 `hasApiKey` 仍为 true。
4. 旧配置迁移：手写一份含明文 `apiKey` 的 `accounts.json`，启动一次，检查文件被重写且明文消失，Key 已进 keyring/回退文件，重启后调用正常。
5. Windows/macOS：keyring 代码在 Linux 编译不到，需在对应平台实测 1、3、4（keyring 后端分别为 Credential Manager / Keychain）。

### P1-8
1. 造大群聊（或直接用基准测试的 store），连续发送多条群聊消息，观察 UI 不卡顿；检查 `accounts.json` 内容正确。
2. 快速连续触发多次 `let _ =` 路径的保存（如连续导入 Cookie），用 `stat` 观察写盘次数明显少于触发次数（防抖合并）。
3. 模拟写盘失败（把 `accounts.json` 所在目录改为只读）：用户主动操作应返回中文错误并回滚内存状态；防抖路径应在 UI 出现“配置自动保存失败”提示。

## 与 P0-1 的边界 / 冲突点

- P0-1（并行任务）正在 `storage.rs` 改加载失败路径（`load_or_migrate_store` 返回空配置问题）并收敛写盘到 `SessionManager::persist()`；本任务只在加载**成功**路径上加 keyring 迁移（`load_or_migrate_store` 尾部），并把写盘调用点改为 `persist_and_relock` / `request_persist`。两者都动 `load_or_migrate_store` 与全部 `save_store_atomic` 调用点，**合并时必冲突**，需以 P0-1 的 `persist()` 为准、把本任务的异步化/防抖接进 `persist()` 实现。
- 本任务不改：加载失败的备份/`persist_blocked`/未知 provider 保留逻辑；不引入 `persist()` 这个名字（避免与 P0-1 的同名方法冲突），本任务的同步写盘辅助方法命名为 `persist_and_relock`，防抖入口为 `request_persist`。
- `save_store_atomic` 本身保持同步签名不变（测试与 P0-1 仍可用），新增 `save_store_atomic_async` 做异步化；0600 临时文件权限改动在 `save_store_atomic` 内，P0-1 合并时需保留。

## fmt / clippy 基线状态（2026-11-26 协调人确认）

- `cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 在基线 `0416f41` 上就已经失败：fmt 约 200 处 diff、8 个文件，主因是项目用 2 空格缩进而 rustfmt 默认 4 空格（仓库无 `rustfmt.toml`）；clippy lib 19 个错误 + test 20 个。这是基线问题，不是本任务的回归。
- 本任务不全库重排，不试图修掉全部 clippy 错误；只保证新增/修改的代码遵循项目现有风格（2 空格缩进、中文用户提示、错误统一返回 `Result<_, String>`）。单文件 `rustfmt --check` 对本任务文件也会报 2→4 空格的缩进 diff，这是风格约定差异，不是本任务引入的格式问题；待合并后专门清理。2026-11-26 实测：对 `session_manager.rs` / `storage.rs` / `key_vault.rs` / `group_transport.rs` 做 `rustfmt --emit stdout` 再 `diff -bB`，除去 `--emit stdout` 的文件头横幅外，差异只有 4 空格重排、100 列换行与 import 排序——全是基线风格差，无新增的格式问题。

## 构建隔离（2026-11-26 协调人补充）

- P1-CLI 块报告跨 worktree 污染：P0-1 的两个命令的 autogenerated 权限文件（`continue_with_empty_config.toml`、`restore_config_from_backup.toml`）在构建中两次落入它的分支（已删除、未提交），共享 `CARGO_TARGET_DIR` 是嫌疑之一。
- 本任务从 2026-11-26 10:38 CST 起改用本分支独立 target 目录：`CARGO_TARGET_DIR="$HOME/workspace/.cargo-target-maam-hardening-api-key-keyring"`（首次全量重编约数分钟）；每次跑 cargo 前确认 cwd 在本工作目录。
- 检查本工作树：`src-tauri/permissions/autogenerated/` 只有基线自带的各命令 .toml（均为 worktree 创建时间 01:30），无 P0-1 的两个文件；`git status` 无不属于本任务的未跟踪 .toml。如后续出现，直接删除、不提交。

### 独立 target 目录的代价（2026-11-26 实测）

- 新目录首次 `cargo test --lib` 失败：`libsoup-3.0` 系统库找不到。原因是 VM 曾被替换：`/usr` 是易失的，系统里没有任何 soup/webkit 开发库（`dpkg -l` 无 soup/webkit；`find / -name "libsoup-3.0.pc"` 无结果）；旧 VM 上跑通的测试全靠共享目录 `$HOME/workspace/.cargo-target-maam` 里的缓存产物（`soup3-sys` 构建脚本输出已缓存，不再调 pkg-config）。
- 修复：`sudo apt-get update`（约 11.5 分钟，网慢）后装 Tauri Linux 构建依赖 `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev`。装时遇到版本冲突：`libzstd-dev` 要求 `libzstd1 (= 1.5.5+dfsg2-2build1.1)`，而镜像装的是本地重编版 `1.5.5+dfsg2-2build1.1+hatch2`；处理为单包降级 `libzstd1=1.5.5+dfsg2-2build1.1`（同上游版本，仅 Debian 修订号不同；dry-run 确认只降这一个包、无连带卸载），加 `--allow-downgrades` 安装。
- 注意：`cargo check --lib` 在新目录一次全量重编约 8.3 分钟；`cargo test --lib` 需另编 test harness 与链接 webkit，时间更长。

## ⚠️ 工作目录内发现并行改动（2026-11-26 02:15 UTC）

- 本任务执行中发现：`src-tauri/src/session_manager.rs`、`src/main.ts`、`src/types.ts` 在 02:15 UTC 被另一方修改（mtime 02:15:01–02:15:18），内容恰好是本任务计划中的 commit 5（`get_manager_state` 只返回 `hasApiKey`、编辑留空不修改、删除时清 vault）。修改者不明；同一时刻（02:16 UTC）有另一进程（PID 57002/57006）在本工作目录跑 `cargo test --lib` 与 `npx tsc --noEmit`（注意：它用的是工作目录内 `src-tauri/target`，不是共享 `CARGO_TARGET_DIR`）。
- 处理：未直接丢弃；逐行审查后采纳，修复了其中发现的缺陷（见下），补测试后提交。父 agent 需核查是否存在重复派发的同任务 agent，避免两边重复提交/互相覆盖。
- 审查发现的缺陷：
  1. `upsert_external_agent` 先写 vault、再写盘：写盘失败回滚内存时**未回滚 vault**，新 Key 会残留在 vault 里，与内存/磁盘的旧记录不一致，下次启动会用“编辑失败”的新 Key。02:21 UTC 的后续改动已补上回滚（按旧 Key 恢复，新建则删除刚写入的条目）；本任务另把相关测试断言改为与后端无关（`ApiKeyVault::load_key` 而非直接读 `api-keys.json`，有 secret service 的机器上回退文件不存在）。
  2. 成员 kind 由 `openai_api` 改为 `local_cli` 时，旧 vault 条目成孤儿。02:21 UTC 的后续改动已补清理，并补了专项测试。
  3. 每次 upsert/delete 都 `ApiKeyVault::new(...)` 新建实例，keyring 可用性探测（一次 D-Bus 往返）每次都重做。已保留该写法（改动面小、正确性优先），记为后续优化点：应在 `SessionManager` 里持有一个 vault 实例。

## Commit 列表

- `35ed6b7` test(persist): 5 万条消息写盘基准
- `84e4e9d` test(keyring): API Key 永不落盘明文 + 旧明文迁移（红灯）
- `071d631` feat(keyring): 新增 ApiKeyVault（keyring + 0600 文件回退）
- `e2d642f` fix(keyring): accounts.json 不再落盘明文 API Key，启动自动迁移旧明文
- `fix(keyring): get_manager_state 只返回 hasApiKey，编辑留空不修改`（已提交）：快照改发 ExternalAgentView、upsert 留空保留旧 Key 且先写 vault、delete 同步清 vault、前端去明文回填；另修两处缺陷——写盘失败回滚 vault、kind 改 local_cli 后清孤儿 vault 条目；新增 kind 切换测试，cargo test external_agent 6/6 通过，tsc 通过。
  - 其中 `9ab03d3`（2026-11-26 02:25:24 UTC，我的提交）与 `e0a9d08`（02:30:43 UTC，并行方提交，仅改笔记 1 行）同名交错，见「工作目录内发现并行改动」一节。
- `024cbea` fix(persist): 临时配置文件按 0600 创建（测试 `save_store_atomic_writes_0600_on_unix` 先红后绿）
- `5f1ff32` feat(persist): 序列化与写盘移入 spawn_blocking（`save_store_atomic_async`，与同步版逐字节一致；单次墙钟不缩短，收益是运行时线程不阻塞）
- `350f0f4` fix(persist): 写盘时不再持有 inner 锁（`persist_and_relock`，27 处：20 个 Err 点 + 5 个 `?` 点 + 2 个 is_err 点；两处同步 `*_locked` 辅助函数保持同步；`cargo test --lib` 37/37）
- `3f0931d` feat(persist): 写盘防抖合并 300ms（`request_persist` + 写线程，9 个 `let _ =` 点；`cargo test --lib` 39/39，`tsc` 通过）
