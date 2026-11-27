# P0-3 自动审批加固笔记

分支：`hardening/auto-approve`，基线 v0.1.10（`0416f41`）。
范围：自动审批（`session_manager.rs` 注入脚本、`storage.rs` 设置、
`lib.rs` 设置命令、`src/main.ts` / `src/types.ts` 设置弹窗）。
验证环境：Linux（本机），Rust 1.100，Node v24。

## 结论总表

| # | 任务书要求 | 结论 | 备注 |
|---|---|---|---|
| 1 | 只点单次按钮，删 `|| approveBtns[0]` 回退；始终允许不点并提示手动处理 | 确认，已修复 | 见下 |
| 2 | 扩充 `DANGER_RE`（命令类、凭据类，中英文） | 确认，已修复 | 见下 |
| 3 | 命令执行类审批默认不自动批准；设置新增独立开关，默认关 | 确认，已修复 | 行为变化，见下 |
| 4 | 每次自动点击写审计日志；同一账号 60 秒内 >10 次熔断关闭 | 确认，已修复 | 见下 |
| 5 | 注入脚本抽成独立文件，可用 node + jsdom 表驱动测试 | 确认，已修复 | `src-tauri/src/js/auto_approve.js` |
| — | 自动审批默认值关闭（`storage.rs` 约 48 行） | 确认，不改 | `auto_approve: false` 保持不变 |

## 1. 只点击单次授权按钮（commit `df636e8`）

- 确认：原 `findApprovalTarget`（`session_manager.rs` 约 930 行）
  `approveBtns.find(SINGLE...) || approveBtns[0]` 确实会在没有单次按钮时
  回退点第一个，可能是「始终允许」。`APPROVE_BTN_RE` 含
  `始终允许|总是允许|始终批准|always allow|always approve`。
- 修复（`src-tauri/src/js/auto_approve.js`）：
  - 删除 `|| approveBtns[0]` 回退；没有 `SINGLE_APPROVE_BTN_RE` 命中的按钮
    时不点击，置 `window.__maamAutoApproveNeedsManual`
   （`{ at, text: 卡片文本前 140 字, reason: 'always_only' }`，同一张卡
    60 秒内只记一次）。
  - `APPROVE_BTN_RE` 仍保留「始终/总是/always」作为审批卡的识别条件
    （否则只有始终允许的卡连识别都做不到、无法提示）；候选过滤只认
    `SINGLE_APPROVE_BTN_RE`。
- 手动处理提示的链路：页面本身不能直接调 Tauri 命令，Rust 侧在静默同步
  （`trigger_silent_sync`，每 5 分钟遍历保活 webview）末尾调
  `drain_auto_approve_report` 读回标记（drain 语义，读走即清），再经
  `notice` + snapshot 推到 UI（见第 4 节）。
  群聊发送路径已有兜底：`__maamScanAutoApprove()` 返回 false 后
  `findAuthPrompt()` 返回卡片文本，走 `needs_auth`
  （「检测到页面授权确认，请前往该账号页面点击允许」），行为不变。
- 测试：`tests/auto_approve.test.mjs`（`npm run test:js`）中英文
  「只有 Always allow / 始终允许」两项。

## 2. 危险词扩充（commit `9c16771`）

- 确认：原 `DANGER_RE` 只有删除/支付/密码等词，无命令执行与凭据类。
- 扩充后新增（中英文，见 `auto_approve.js` 内 `DANGER_RE`）：
  - 命令：`rm -rf` / `--recursive`、`sudo`、`chmod`、`chown`、
    `kill` / `killall` / `pkill`、`git push --force`、
    `git reset --hard`、`curl|sh`、`wget|sh`、`drop table` /
    `drop database`、`mkfs`、`dd`、`fdisk`、`shutdown`、`reboot`。
  - 凭据/破坏：删除文件、删除数据、清空数据、格式化、恢复出厂、清除数据、
    密钥、口令、凭证、令牌、私钥、环境变量、`.env`、`id_rsa` /
    `id_ed25519`、`.pem`、`api key`、`access token`、`secret`、
    `credential`、`.ssh`、`/etc/passwd`、`/etc/shadow`，
    以及 `delete` / `remove` / `erase` / `wipe`（加了词边界，
    避免 `skill` 这类词误伤）。
- 有意为之的保守点：`git push --force-with-lease` 因含 `--force` 前缀
  一并拦截（fail-closed）；`secret` / `token` 在卡片文本里出现即拦截，
  宁可误拦也不放行。
- 测试：`rm -rf`、`sudo`、`git push --force`、`curl|sh`、`.env` 五项。

## 3. 命令执行类审批默认不自动批准（commit `922991b`）

- 确认：此前只要总开关开启，命令执行类审批卡也会被自动批准，无单独控制。
- 修复：
  - JS：新增 `COMMAND_RE`（执行命令、运行命令、执行/运行 shell、
    shell/bash/终端命令、命令行、`execute a command`、`run a command`、
    `shell` / `bash` / `terminal`）。命中审批卡且
    `window.__maamAutoApproveAllowCommands !== true` 时不点击，
    置 needsManual（`reason: 'command'`）。
    门控只在 `PROMPT_RE` 命中的审批卡内生效，避免聊天记录里提到
    shell/bash 就误拦无关卡片。
  - Rust：`AppSettings` 新增 `auto_approve_commands: bool`，默认 `false`；
    旧配置缺失该字段时迁移为 `false`（`storage.rs`，camelCase
    `autoApproveCommands` 与 snake_case 均兼容）。
    `SessionManager` 新增同名 `AtomicBool`，`update_app_settings` 与
    Tauri 命令新增同名可选参数，保存后同步到全部保活 webview；
    新建 webview 的初始化脚本同时下发两个 flag。
  - 前端：设置弹窗新增独立复选框「允许自动批准命令执行
    （执行命令 / shell / bash / 终端类审批）」，默认关闭；
    文案注明开启后仍会跳过危险命令（如 rm -rf、sudo）。
- **行为变化**：此前总开关开启时命令执行类审批卡会被自动批准；
  此后默认不再自动批准，需用户在设置里单独开启。自动审批总开关
  默认仍为关闭，未改。
- 测试：jsdom「bash 默认不批」「开关开启后可批」两项；
  `storage.rs` 新增迁移/默认值断言；`npx tsc --noEmit` 通过。

## 4. 审计日志与熔断（commit `88758e6`）

- JS（`auto_approve.js`）：
  - 每次自动点击追加 `{ at, text: 卡片文本前 140 字, button: 按钮文案 }`
    到 `window.__maamAutoApproveLog`（页内最多保留最近 50 条）。
  - 熔断：同一页面（每个账号对应独立 webview，计数按页隔离，
    即按账号隔离）60 秒内自动点击达到 10 次后，第 11 次起不再点击，
    立即置 `window.__maamAutoApprove = false`、
    `window.__maamAutoApproveTripped = true`。
  - `window.__maamAutoApproveReport()` 以 JSON 字符串返回
    `{ entries, tripped, trippedAt, needsManual }`，读走即清
    （与仓库内其他 `eval_json_callback` 调用保持一致）。
- Rust（`session_manager.rs`）：
  - `drain_auto_approve_report`：在 `trigger_silent_sync` 的每个保活
    账号任务末尾调用（探针跑完后），用 `eval_json_callback` 读回上报。
    审计条目追加写到应用数据目录下的 `auto-approve-audit.log`
    （JSONL，每行含 `at_ms`、`provider_id`、`uuid`、`text`、
    `button`；写盘失败只丢弃本次审计行，不影响主流程）。
    熔断时把 `settings.auto_approve` 持久化置 false 并发 notice
    （「自动审批在 60 秒内自动点击超过 10 次，已自动关闭，请检查该
    账号页面的审批弹窗」）；有 needsManual 时发 notice
    （「检测到仅有「始终允许」的审批 / 命令执行类审批，自动审批已跳过，
    请前往该账号页面手动处理：<卡片文本前 140 字>」）。
  - 延迟说明：熔断的本页立即停用是即时的；持久化关闭与 UI 提示依赖
    下一次静默同步（默认每 5 分钟，或手动触发同步），其间其他账号
    的 webview 仍可能继续自动审批；页面重载后以 Rust 侧持久化值
    为准（已关闭则不再开启）。
- 测试：jsdom「审计日志字段」与「11 次点击熔断」两项；
  Rust 单元测试 `append_auto_approve_audit_writes_jsonl_with_account_key_and_button`
  覆盖 JSONL 落盘与追加不覆盖。

## 5. 注入脚本抽取与测试（commit `1a1bfe2`）

- 原 `LOGIN_CAPTURE_INIT_JS` 内的 watcher 区块
  （`session_manager.rs` 约 873–982 行）原样移出到
  `src-tauri/src/js/auto_approve.js`，以 `include_str!` 引入为
  `AUTO_APPROVE_INIT_JS`，作为独立 initialization script 注册到
  账号 webview（`LOGIN_CAPTURE_INIT_JS` 之后）。
- 未动弹窗 webview：弹窗此前只注册 `LOGIN_CAPTURE_INIT_JS`、没有
  `__maamAutoApprove` flag，自动审批本来就不启用，抽取后保持不变。
- `window.__maamAutoApproveDestroy()` 仅供 jsdom 测试清理定时器/观察器；
  生产环境页面卸载时由 webview 回收。
- 测试：`tests/auto_approve.test.mjs`（`npm run test:js`，node --test +
  jsdom 31 devDependency）。jsdom 的 `getBoundingClientRect` 默认
  返回全 0，会被 `isElVisible` 判为不可见，测试里按真实浏览器行为
  打桩为可见尺寸。抽取时先写测试再修：初始 5 通过 / 10 失败，
  全部修复后 15 通过 / 0 失败。
- `package.json` 新增 `test:js` 脚本与 `jsdom` devDependency；
  未引入其他依赖。

## 手工验证步骤（无法自动测试项）

1. 熔断提示与持久化关闭（桌面端，macOS / Windows 实测）：
   - 开启自动审批，打开一个账号页面，连续触发 11 次以上审批卡
     （或在页面控制台连点单次授权按钮 11 次）。
   - 预期：第 11 次起不再自动点击；触发一次静默同步（或等 5 分钟）
     后，主界面出现 notice「自动审批在 60 秒内自动点击超过 10 次，
     已自动关闭」，设置里的自动审批变为关闭，重启应用后仍为关闭。
   - 检查应用数据目录下的 `auto-approve-audit.log`，应有 10 行 JSONL，
     每行含 `at_ms`、`provider_id`、`uuid`、`text`、`button`。
2. 仅有「始终允许」的卡（桌面端）：
   - 开启自动审批，打开出现只有「始终允许」按钮的审批卡的页面。
   - 预期：不自动点击；触发同步后主界面出现 notice
     「检测到仅有「始终允许」的审批，自动审批已跳过，请前往该账号
     页面手动处理：<卡片文本>」；手动点「始终允许」仍可正常授权。
3. 命令执行开关（桌面端）：
   - 总开关开、命令开关关：出现 bash / shell 命令审批卡，不自动批准，
     同步后有 notice；再开启命令开关（不重载页面，保存后即同步到
     保活 webview），同类卡恢复自动批准（危险命令仍不批）。
   - 旧配置升级：把 `accounts.json` 里 `settings.autoApproveCommands`
     字段删掉再启动，开关应为关闭，且不影响原有 `autoApprove` 值。
4. Windows / macOS 专属：以上 Rust 代码均为跨平台（`#[cfg(desktop)]`），
   但实际只在 Linux 上编译与测试过；Windows 的 WebView2 与 macOS 的
   WKWebView 初始化脚本行为需在对应平台按上述步骤实测。
   移动端（Android / iOS）：自动审批注入脚本为 `#[cfg(desktop)]`，
   本次改动不涉及移动端，开关字段随配置持久化但无实际作用。

## 未修复 / 待实测

- `cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings`：
  基线 `0416f41` 上即已失败（fmt 约 200 处 diff、8 个文件，主因是项目
  手写 2 空格缩进、无 `rustfmt.toml`，而 rustfmt 默认 4 空格；
  clippy lib 19 个错误 + test 20 个）。这是基线问题，不是本次改动的
  回归：不要全库重排，不要试图修掉全部 clippy 错误，待合并后专门清理。
  本次改动已按仓库 2 空格风格逐文件验证为 fmt-clean：
  `rustfmt --edition 2021 --config tab_spaces=2 --check` 单文件
  （隔离复制，避免 `lib.rs` 的 `mod` 牵连整棵模块树）比对基线——
  `storage.rs` 9 处（与基线相同）、`lib.rs` 0 处、
  `session_manager.rs` 134 处（与基线相同）；与本次改动行区间重叠的
  仅剩 2 处，且都是未动的既有代码、在基线里以同样内容被标出
  （`SessionManager::new` 里的 `.filter(|pid| {...})`、
  `update_app_settings` 里的 `self.minimize_to_tray.store(...)`），
  故未动。clippy 方面：本次改动一度引入 1 个新错误（新测试里的
  `assert_eq!(..., true)`，`bool_assert_comparison`），已改为
  `assert!(...)` 修复；最终复核确认本次改动新增 clippy 错误为 0，
  其余 19 + 20 个错误位置均为既有代码。
- 熔断计数的账号隔离依赖「每个账号独立 webview」这一现有结构；
  同一账号多 webview（弹窗登录页不启用自动审批）不计入。
- 审计日志文件 `auto-approve-audit.log` 无轮转/清理，长期高频点击
  会持续增长；如需控制体积，后续可加按大小轮转（未做，保持最小改动）。

## 新发现

- `findAuthPrompt` 的两份拷贝（群聊发送 JS，约 2069、2601 行）仍用
  旧正则识别 `always allow` 为授权提示——仅用于检测、不点击，
  与本次修复不冲突，留待后续统一抽取时处理，未动。
- `__maamScanAutoApprove` 被群聊发送路径直接调用，返回 true 即视为
  已处理；本次改为「始终允许」时返回 false，会正确落入后续的
  `needs_auth` 提示，未改变该路径的兜底行为。
