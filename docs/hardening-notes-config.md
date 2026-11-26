# P0-1 加固笔记：配置加载失败保护

分支：`hardening/config-load-protection`，基线 v0.1.10（`0416f41`）。
范围：只改 `src-tauri/src/storage.rs`、`src-tauri/src/session_manager.rs`
（与配置加载/持久化直接相关），以及为两个新解除命令所必需的接线：
`src-tauri/src/lib.rs`、`src-tauri/build.rs`、
`src-tauri/capabilities/manager.json`、两份自动生成的 permission toml。
未动 `src/main.ts`、未动 `.github/workflows/release.yml`、未引入新依赖。

## 结论总览（逐项：确认 / 不成立 / 待实测）

| 任务书要求 | 结论 | 备注 |
|---|---|---|
| 1. 加载失败先备份 `accounts.json.corrupt-<unix_ms>` | 确认并修复 | 见 C2；备份失败也只写日志、不丢原文件 |
| 2. `LoadedAccountsStore.persist_blocked` + 写盘收敛到 `persist()` + notice 提示 + 用户选择后解除 | 确认并修复 | 见 C2/C5；解除命令已接线，前端按钮待接 |
| 3. 成功加载后滚动备份 `accounts.json.bak` | 确认并修复 | 见 C2；单份滚动覆盖 |
| 4. 未知 provider 账号保留原始 JSON、保存原样写回 | 确认并修复 | 见 C3；引用位置（群成员等）仍丢弃，见「已知局限」 |
| 5. version 高于 `STORE_VERSION` 视为只读 | 确认并修复 | 见 C2；同时 `persist_blocked=true` |
| 6. `let _ = save_store_atomic(...)` 失败要日志+notice | 确认并修复 | 见 C5；9 处静默忽略已全部收敛进 `persist_locked` |
| 任务书称「`save_store_atomic` 在 session_manager 里有 30 余处调用」 | 确认 | 实测非测试代码 38 处，现只剩 `persist_locked` 内 1 处直接调用 |
| 任务书称「未知服务标识会让整个迁移失败（约 402）」 | 确认 | `ProviderId::parse(s).ok_or_else(...)` 使 `migrate_from_value` 整体 Err |

## 各提交

- `dd8809d` test(storage): 损坏配置备份、未知 provider 往返、高版本只读、滚动 .bak 的失败测试
- `be3433f` fix(storage): 加载失败先备份原文件、阻止写盘、滚动备份与高版本只读
- `7234efa` fix(storage): 未知 provider 的账号保留原始 JSON，保存时原样写回
- `14c9e8a` test(session): 损坏/高版本配置加载后，后续写盘应被阻止的失败测试
- `a29cea0` fix(session): 写盘收敛到 persist()，阻止状态下拒绝写盘并可由用户解除
- `9656947` style(storage): 测试断言与 backup_file_name 按 rustfmt 断行（无行为变化）

## 行为变化（每个修复带来的变化）

1. **损坏/读不到/解析失败的配置不再被空配置覆盖。**
   加载失败时原文件先复制为 `accounts.json.corrupt-<unix_ms>`（同名已存在追加
   `-1/-2…`，永不覆盖），内存用空配置启动，`persist_blocked=true`，
   任何写盘尝试都返回 Err，磁盘文件字节不变。notice 写明备份文件名与两个选择。
   备份本身失败时（如文件被独占锁定导致复制也失败）同样阻止写盘，notice 说明备份失败。
2. **成功加载后产生 `accounts.json.bak`。** 每次成功加载（含 UUID 冲突、迁移写盘失败
   但配置本身可用的情况）都把刚加载的文件复制为单份滚动 `accounts.json.bak`。
   首次启动（无配置文件）不产生 `.bak`。
3. **高版本配置只读。** `version > STORE_VERSION` 时 `read_only=true`、
   `persist_blocked=true`，跳过迁移写盘，绝不降级写回；内存里的账号仍可正常查看。
   「以空配置继续」可丢弃它（原高版本文件已有 `.bak` 备份）。
4. **未知 provider 账号不再丢。** `providerId` 无法解析或非内置（未来版本新 provider、
   已下线的 `custom-*`、accounts 里的 `api`/`cli`）的条目保留原始 JSON
   （`AccountsStoreFile.unknown_accounts`，`#[serde(skip)]`），
   保存时由 `save_store_atomic` 原样拼回 `accounts` 数组（含未知字段）。
   无未知条目时写盘输出与原来完全一致。旧的整条丢弃逻辑已删除。
   注意：`dirty=true` 会触发一次迁移写盘，把未知条目写回——这是安全的，
   因为写回包含未知条目本身。
5. **写盘唯一入口。** 除加载器自身的迁移写盘与测试固件写盘外，全部 38 处写盘
   收敛到 `SessionManager::persist_locked`（持锁）/`persist`（不持锁）。
   失败一律 `eprintln!` 写日志 + 设置 notice（`保存配置失败: …`）。
   以往 `let _ =` 静默吞掉失败的 9 处现在同样有日志+notice。
   `resolve_uuid_conflict` 等带回滚逻辑的调用方保持「写盘失败则内存回滚」语义；
   被阻止时同样回滚，不会留下「内存已改、磁盘未改」的不一致。
6. **新增解除路径（后端已就绪，前端待接线）。**
   - Tauri 命令 `continue_with_empty_config`：以空配置继续，写盘失败则恢复阻止状态。
   - Tauri 命令 `restore_config_from_backup`：按 `accounts.json.bak`、最新
     `accounts.json.corrupt-*` 的顺序逐个尝试，首个能成功加载的胜出；
     配置正常时拒绝恢复（避免覆盖好配置）；全部备份都失败时保持阻止并返回原因。
   - `ManagerSnapshot` 新增 `persistBlocked` / `readOnly`（camelCase），供前端判断。
   - `dismiss_notice` 只关提示，**不**解除阻止（已用测试锁定）。
   - 前端 `src/main.ts` 尚未加按钮：notice 文案已告知两个选择，待前端 agent
     按 `snapshot.persistBlocked` 在提示条旁加两个按钮，分别调上述命令。
7. **UUID 冲突路径不变。** 冲突仍可加载、可 `resolve_uuid_conflict` 解决，
   不阻止写盘（解决冲突本身就需要一次写盘）。

## 已知局限（不成立 / 未覆盖）

- **未知账号的引用位置仍会被丢弃。** 群聊 `members` / `memberRoles` /
  `threadBindings`、`lastCommitted`、`pendingCleanup`、群消息 `sender`/`targets`
  里引用未知 provider 的 key 时，`parse_key_or_legacy_muse` 返回 None，
  仍按旧逻辑丢弃并标记 dirty。原因是 `AccountKey.provider_id` 无法表示未知
  provider（`ProviderId` 除 `parse` 外无公开构造），要保留引用需改动
  provider 模型，超出 P0-1 范围。账号本体不丢，群关系可能需用户重建。
- **未知 provider 账号 UUID 非法时仍整体失败。** 保留逻辑只在 provider 已识别为
  未知时先生效；若条目连 UUID 都非法，仍走旧的整体 Err 路径——但此时已有
  `.corrupt-*` 备份 + 写盘阻止，数据不再丢失。
- **恢复时每次失败尝试会产生一份新的 `.corrupt-*` 备份。**
  `restore_config_from_backup` 逐个试备份时，`load_or_migrate_store` 对每次
  失败的尝试都会再备一份。无害（不覆盖），但可能多出 1–2 个小文件。
- **`.bak` 只有一份。** 滚动覆盖，不保留历史版本。
- **日志用 `eprintln!`。** 仓库未引入 `log` 依赖（任务书允许的新依赖只有
  `keyring`/`semver`/`encoding_rs`），且代码库此前无任何日志调用，
  故失败日志走 stderr。

## 验证结果

- `cargo test --manifest-path src-tauri/Cargo.toml --lib`：**38 通过，0 失败**
  （基线 31 个；新增 12 个，含 storage 5、session 7；其中 C4/C5 的
  `corrupt_config_blocks_later_persists` 等在修复前已验证为失败）。
- `cargo test`（全量，含 bin）：与 `--lib` 一致，无 doc-test。
- `npx tsc --noEmit`：通过（前端未改）。
- `npm ci`：通过（仅为验证，未改前端依赖）。
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`：**基线即红，见下。**
  本分支实测 256 处 diff（基线约 200 处；增加量只是新增的 2 空格代码行，
  性质与基线相同，未做全库重排）。
  本分支新增/修改的代码已做等价验证：用 `rustfmt --emit stdout` 输出与文件
  逐行比对（去缩进后内容比对），我新增的行与 rustfmt 的非缩进改动交集为 **0**；
  另检查无 Tab、无行尾空格；超长行只剩 3 行用户提示字符串（rustfmt 不折行
  字符串，基线 session_manager.rs 本身就有 403 行超 100 列，长中文行是既存风格）。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`：
  **基线即红，见下。** 本分支实测：lib 19 个错误 + test 20 个错误，
  与 P2 CI 块在基线 `0416f41` 上测得的数量完全一致；
  逐条核对错误位置（`/tmp/clippy-p0-config.txt`），**0 条落在
  本分支新增的行**（含 `proxy` 未使用变量 ×2、`too many arguments`、
  `cloned_ref_to_slice_refs` 等，均为基线既有代码）。

### 基线 fmt / clippy 即红（不是本分支的回归）

协调人补充（2026-11-26）：P2 CI 块实测基线 `0416f41` 上
`cargo fmt --check`（约 200 处 diff、8 个文件，主因项目用 2 空格而
rustfmt 默认 4 空格，无 rustfmt.toml）与
`cargo clippy --all-targets -- -D warnings`（lib 19 个错误 + test 20 个）
就已经失败。本分支**没有**全库重排、**没有**修全部 clippy 错误；
**待合并后专门清理。**

- `cargo check --all-targets` 在本分支的警告：`unused variable: proxy` ×2
  （session_manager.rs:5279、8193）与 `unused dependency tauri-plugin-account-session`
  均为基线既有（`git show 0416f41` 同位置存在，本分支 diff 未碰）。
  本分支新增的唯一警告 `method persist is never used` 已处理：
  `persist()` 是对外暴露的唯一写盘入口（测试与后续不持锁调用方使用），
  库内持锁调用方走 `persist_locked`，故加 `#[allow(dead_code)]` 并注释说明。
- clippy 本分支实测：错误均位于基线既有代码（须以最终跑出的清单为准；
  本分支新增的函数/行未出现在 clippy 错误位置中，见验证命令记录）。

## 手工验证步骤（无法自动化的部分）

1. **损坏配置端到端（Linux 手动模拟，等价于真实损坏）：**
   - 备份好 `accounts.json` 后，把它改成非法 JSON，启动应用。
   - 预期：应用回到管理页，提示含 `已备份为 accounts.json.corrupt-<时间戳>`；
     同目录出现该备份，内容与损坏前一致；原文件仍在。
   - 任一改动（如改设置、新增账号）均应失败并提示写盘被阻止，文件字节不变。
   - 调 `restore_config_from_backup`（或等前端按钮）：应恢复到 `.bak`
     的内容；调 `continue_with_empty_config`：应清空并恢复写盘，
     `.corrupt-*` 备份仍在。
2. **高版本只读（模拟降级）：** 把 `accounts.json` 的 `version` 改为 99 后启动。
   预期：账号正常显示，提示已设为只读；任何改动失败；退出重进后文件仍是 v99 原样。
3. **未知 provider 往返：** 在 `accounts` 里加一条 `providerId: "futurechat"` 的账号，
   启动后该账号不在列表里（无对应服务），改一次设置退出重进，检查文件里该条目
   仍在且字段原样。
4. **Windows / macOS 专属代码：** 本次改动全部是平台无关代码（`storage.rs`、
   `session_manager.rs` 的通用路径、`lib.rs` 命令接线），无 `cfg(windows)` /
   `cfg(target_os = "macos")` 分支。Tauri capability 与两条新命令需在桌面端
   实机启动验证：打开管理页，确认无 capability 报错；损坏配置场景下确认
   notice 与两个新命令可调通（前端按钮接线后）。
5. **Android / iOS：** `load_or_migrate_store` 与 `persist_locked` 同样走移动端
   代码路径（无平台分支），但未在真机验证；移动端配置文件目录不同，
   `.corrupt-*` / `.bak` 落在应用数据目录内，需实机确认可见性。
6. **`.corrupt-*` 命名冲突：** 同一毫秒内两次损坏只会追加 `-1` 后缀，不覆盖，
   可人工制造两次快速损坏验证。

## 给前端接线 agent 的要点
- `snapshot.persistBlocked` 为 true 时，在 notice 条旁加两个按钮：
  「以空配置继续」→ `continue_with_empty_config`；
  「从备份恢复」→ `restore_config_from_backup`。
- 两个命令返回 `ManagerSnapshot`（成功）或错误字符串（失败时前端展示即可）。
- 不要用 `dismiss_notice` 解除阻止——它只关提示。
- `snapshot.readOnly` 为 true 时（高版本配置），建议只提供「以空配置继续」，
  不要提供「从备份恢复」之外的写盘入口（恢复会装回高版本配置，仍是只读）。

## 环境杂记

- 2026-11-26 10:32 左右，工作目录凭空出现未跟踪文件
  `src-tauri/permissions/autogenerated/cancel_external_member_turn.toml`
 （对应命令不存在于本分支任何源码、`build.rs`、`capabilities` 中）。
  删除后重跑 `cargo check --all-targets` 未再生成，确认不是本分支构建产物，
  已删除，未入版本。疑似并行任务的构建工具短暂写错目录所致。
