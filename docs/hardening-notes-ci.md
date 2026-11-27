# 加固记录：P2-9 CSP 与 P2-10 CI / 测试

分支：`hardening/ci-csp`，基线 v0.1.10（`0416f41`）。
范围只含 P2-9 与 P2-10；`release.yml` 未改动（见下文建议）。
`updater.rs` 的 URL 测试由并行任务 P0-2 负责，本分支不重复写。

## 结论总览

| 项 | 结论 | 说明 |
|---|---|---|
| P2-9 CSP 追加四条指令 | 确认，已修复 | `base-uri 'none'; form-action 'none'; object-src 'none'; frame-ancestors 'none'` 已追加；静态核查前端无 `<form>` / `<object>` / `<embed>` / `<iframe>` / `<base>`，无影响 |
| P2-10.1 新增 ci.yml | 确认，已完成 | push + pull_request；ubuntu-24.04 与 windows-2025；npm ci、tsc、fmt、clippy、cargo test |
| P2-10.2 新工作流 action 固定 SHA | 确认，已完成 | ci.yml 全部四处 `uses:` 均为完整 commit SHA（见下） |
| P2-10.3 Dependabot | 确认，已完成 | cargo + npm + github-actions，每周 |
| P2-10.4 providers.rs 测试 | 确认，已完成 | 11 个新测试；任务书暗含的“后缀伪装可绕过”在基线**不成立**——`host_matches` 本来就是点号锚定后缀，测试用于锁定该行为 |
| P2-10.5 updater.rs 测试 | 不由本分支写 | P0-2 并行任务负责；缺口见“新发现项” |
| `cargo fmt --check` 全库门禁 | 不成立（基线即不通过） | 见“未修复项”第 1 条：基线 8 个文件、约 200 处与 rustfmt 1.11 输出不一致；本分支只加 `rustfmt.toml` 固化 2 空格风格，不做全库重排 |
| `cargo clippy -D warnings` 全库门禁 | 不成立（基线即不通过） | 见“未修复项”第 2 条：基线 19 处 clippy 错误（lib）+ 1 处仅测试目标；无一处在本分支改动的文件里 |

## P2-9 CSP 加固

改动：`src-tauri/tauri.conf.json`，`app.security.csp` 追加
`base-uri 'none'; form-action 'none'; object-src 'none'; frame-ancestors 'none'`。
原有指令（`default-src 'self'` 等）一字未动。

核查依据（静态，基线代码）：
- `src/main.ts` 与 `index.html` 全文搜索：无 `<form>`、`<object>`、`<embed>`、
  `<iframe>`、`<base>` 标签；无 `window.open` / `location.href` 跳转；
  无 `fetch` / `WebSocket` / `EventSource`（前端只走 Tauri `invoke`，即 `ipc:`）。
  因此四条新指令没有合法用途会被拦截。
- `img-src 'self' data:` 未动：头像 `avatarUrl` 来自页面抓取的
  `profile_pic_url`，理论上可能是远程 https 地址——这类地址本来就被现有
  `img-src` 拦截，与本次改动无关，不在此处放宽。
- `style-src 'self' 'unsafe-inline'` 未动：前端大量用内联 `style` 属性，
  去掉 `unsafe-inline` 会破坏 UI，不属于本次任务。
- `connect-src ipc: http://ipc.localhost` 未动：dev 模式 HMR 走
  `ws://localhost:1420`，本就不在 `connect-src` 里（Tauri 开发模式另行处理），
  本次追加的四条指令均不涉及 `connect-src`，不影响 HMR。

行为变化：无合法功能变化。变化只针对攻击面：页面内嵌的 `<base>` 劫持、
表单外发、`<object>`/`<embed>` 插件加载、被其他页面 `<iframe>` 嵌入，
现在均被浏览器直接拒绝。

### 手工验证步骤（dev 模式）

因任务要求不运行应用本身（不执行 `npm run tauri dev`），dev 模式实测未做，
按以下步骤由人在本机验证：

1. `npm run tauri dev`，等待窗口出现。
2. 主界面正常渲染：左侧账号栏、顶部服务切换、群聊入口均可点开。
3. 打开开发者工具控制台，确认无 CSP 报错（形如 `Refused to ...`）。
4. 在控制台执行 `document.createElement('form')` 并提交、插入
   `<iframe src="https://example.com">`、`<object data="x">`，均应被 CSP 拦截
   并在控制台留下 `Refused to ...` 记录；页面其余功能不受影响。

### 手工验证步骤（build 模式）

1. `npm run tauri build`（Linux 需先装 `libwebkit2gtk-4.1-dev libgtk-3-dev
   libayatana-appindicator3-dev librsvg2-dev libssl-dev build-essential pkg-config`，
   本机已齐全）。
2. 产物为 `src-tauri/target/release/bundle/` 下的安装包；安装后启动，
   重复上面 dev 步骤 2–4。
3. 另可解包检查：Tauri 会把 `tauri.conf.json` 的 CSP 写进 webview，
   用开发者工具查看响应头 / `document` 的 CSP 元信息，确认四条新指令存在。

本地已验证：`npm run build`（tsc + vite build）通过，`dist/` 正常产出；
`tauri.conf.json` 经 `python3 -m json.tool` 校验为合法 JSON；
`npx tauri build` 通过（见下文验证命令一节），即新 CSP 配置能被 Tauri 正常解析并编入应用。

## P2-10 CI 与测试

### ci.yml（新增 `.github/workflows/ci.yml`）

- 触发：`push`（所有分支）与 `pull_request`。
- 平台：`ubuntu-24.04`、`windows-2025`（`fail-fast: false`）。
- 步骤：`npm ci` → `npx tsc --noEmit` → `cargo fmt --check` →
  `cargo clippy --all-targets -- -D warnings` → `cargo test`。
- Linux 任务先 `apt-get install` Tauri 所需的系统库（与任务书第 2 节一致）。
- 顶层 `permissions: contents: read`（与 `release.yml` 一致）。
- Rust 工具链：`dtolnay/rust-toolchain@stable`，即 stable 通道（与 `release.yml` 一致，
  不是锁死某一版本号；`Cargo.toml` 的 `rust-version = "1.85"` 是下限）。

固定到 SHA 的 action（2026-11-26 查询 `git ls-remote` 确认）：

| action | 引用 | 对应版本 |
|---|---|---|
| `actions/checkout` | `3d3c42e5aac5ba805825da76410c181273ba90b1` | v7.0.1 |
| `actions/setup-node` | `249970729cb0ef3589644e2896645e5dc5ba9c38` | v6.5.0 |
| `dtolnay/rust-toolchain` | `39008827e6b4319a349e9a42dfc90579dd25f1ca` | stable 分支 2026-11-26 头提交 |
| `Swatinem/rust-cache` | `d1527c8f67ef9af357290475d4af4b0fab8f12e0` | v2.9.3 的提交 |

说明：`dtolnay/rust-toolchain` 没有 release tag，`@stable` 本是分支名，
固定 SHA 后该分支后续更新不会自动跟进，需靠 Dependabot（github-actions）
定期提 PR 更新。`Swatinem/rust-cache` 的 `v2` 是 annotated tag，
其提交 SHA 为 `d1527c8f…`（tag 对象本身是 `e9d7b9f1…`，不要用 tag 对象 SHA）。

### Dependabot（新增 `.github/dependabot.yml`）

- `cargo`（目录 `/src-tauri`）、`npm`（目录 `/`）、`github-actions`（目录 `/`），
  均为每周，每类最多 5 个未合并 PR。
- 注意：Dependabot 更新固定 SHA 的 action 时，需要仓库设置允许它修改
  workflow 文件，否则 github-actions 的更新 PR 会失败；首次合入后请在
  仓库 Settings → Code security → Dependabot 里确认。

### providers.rs 测试（新增 11 个）

位置：`src-tauri/src/providers.rs` 末尾 `#[cfg(test)] mod tests`。
先写测试、后确认行为——结果是任务书暗含的绕过在基线**不成立**：

- `host_matches`（`providers.rs`）：相等时直接命中，否则要求主机名以
  `".{domain}"` 结尾。`evilmuse.ai` 结尾是 `lmuse.ai` 而不是 `.muse.ai`，
  本来就是 false；`muse.ai.evil.com` 同理。测试把该行为锁定下来，
  防止将来有人改成裸 `ends_with(domain)`。
- `classify_top_level_url`：`http://muse.ai/`、`javascript:alert(1)`、未知主机、
  `https://muse.ai@evil.com/`（userinfo 伪装，真实主机是 evil.com）均返回 `Deny`；
  另覆盖 `AllowHttps`、`ExternalGoogleOAuth`、`about:blank`（allow 与否两态）、
  `blob:https://…`。
- `is_verified_callback_origin`：`https://muse.ai`、`https://www.muse.ai`、
  `https://muse.ai:443`（默认端口在 origin 序列化中省略）为 true；
  `http://muse.ai`、`https://muse.ai.evil.com`、`https://evilmuse.ai`、
  `https://evil.com`、`https://muse.ai:8443` 为 false；Hark 的
  `https://app.hark.com` 为 true。

行为变化：无。纯新增测试，不改任何生产代码。

### updater.rs 测试缺口（P0-2 未覆盖时记在这里）

P0-2 任务书要求覆盖的 URL 用例（其他用户仓库、`http://`、
`github.com.evil.com`、`github.com@evil.com`、`evilgithub.com`、
合法的 `objects.githubusercontent.com` 重定向目标）均由 P0-2 负责。
本分支在阅读 `updater.rs` 时注意到一个 P0-2 清单之外的缺口，
只记录、不写测试、不改代码：

- `open_external_url`（`updater.rs` 约 920 行）只校验 scheme 为 https，
  不校验主机白名单；前端传入任意 https 链接都会由 `platform::open_external_browser`
  打开。是否应与 `classify_top_level_url` 共用白名单，待 P0-2 或后续任务定夺。

## 未修复项（含原因）

1. **`cargo fmt --check` 在基线即不通过，未做全库重排。**
   - 实测：无 `rustfmt.toml` 时 rustfmt 默认 4 空格，全库 diff 约 19 万字符；
     加 `tab_spaces = 2` 后仍有约 200 处 diff、8 个文件（`session_manager.rs`
     占大头，约 150 处；`platform.rs` 约 40 处；其余为 import 排序、
     链式调用换行、闭包体单行化等 rustfmt 1.11 的风格输出）。
   - 修复手段是 `cargo fmt` 一次性重排：探针工作树实测约 +1177 / −805 行，
     集中在 `session_manager.rs`（1464 行变动）。这会与并行进行的 P0-1 / P0-3 等
     分支（同样改 `session_manager.rs`、`storage.rs`）产生大面积合并冲突，
     且与“每个子问题一个独立 commit”的粒度不符。
   - 处理：本分支只新增 `rustfmt.toml`（`tab_spaces = 2`，固化现有 2 空格风格，
     避免将来有人用默认 4 空格重排）；`ci.yml` 仍按任务要求保留
     `cargo fmt --check` 步骤；**该步骤在合入时预期为红**，直到各加固分支
     合并完成后单独提一个 `style: cargo fmt 全库格式化` commit（命令：
     `cargo fmt --manifest-path src-tauri/Cargo.toml`，之后全绿）。
   - 本分支新增代码（`providers.rs` 测试模块）已用
     `rustfmt --edition 2021 --check` 单文件验证为 clean，未引入新的 fmt diff；
     另 3 处 diff 均在基线旧代码（`providers.rs:190/197/217`），未动。
   - 附带发现：本机 Rust 1.100 初始未装 `rustfmt` / `clippy` 组件，
     已用 `rustup component add rustfmt clippy` 补装（工具链配置，非代码改动）。
2. **`cargo clippy --all-targets -- -D warnings` 在基线即不通过，未修。**
   - 实测（Rust 1.100 / clippy 0.1.100）：lib 目标 19 个错误，lib test 目标
     20 个（19 个同源 + 1 个测试代码相关）。分布：`session_manager.rs` 17 处、
     `platform.rs` 2 处、`lib.rs` 1 处；`providers.rs`（本分支唯一改动的
     Rust 文件）0 处。
   - 位置清单（行号以基线 v0.1.10 为准）：`lib.rs:626`；
     `platform.rs:1372、1641`；`session_manager.rs:5122、5495、6487、6503、
     6783、8036、8673、10367、10755、10939、10979、11315、11673、11912、
     11977、12456、13307`；另测试目标多一处 `session_manager.rs:11914`。
   - 种类：`let...else` 可改 `?`（3）、类型过复杂（2）、未使用变量 `proxy`（2，
     `session_manager.rs:8036` 及另一处）、可合并的 `if`（2）、多余 `return`、
     `clone` 后取切片、永不循环的 `loop`、函数参数过多（9/7、8/7、12/7）、
     `if` 两分支相同、可并入外层 `match` 的 `if`、借用表达式已满足 trait、
     多余 `.into_iter()`、`Iterator::last` 误用。
   - 不修的原因：这些 lint 分布在 P0-1 / P0-3 等并行任务正在改的
     `session_manager.rs` 区域，且其中“参数过多”“类型过复杂”需要改签名、
     不是纯机械修复；修错会改行为。处理与 fmt 相同：`ci.yml` 保留该步骤，
     **合入时预期为红**，待各分支合并后由一个专门的 clippy 清理 commit
     逐项修复（多为机械修复，可 `cargo clippy --fix` 辅助 + 人工复核）。
3. **Windows CI 任务未在 Linux 上实测。**`windows-2025` 的步骤与 Linux 相同，
   但 `cargo test` 里 Windows 专属代码（`cfg(windows)`）在 Linux 上编译不到，
   只能等 CI 首次运行验证；`ci.yml` 中已写明这一点（见报告）。
4. **dev 模式前端实测未做。**任务要求不运行应用本身（不执行
   `npm run tauri dev`），dev 模式验证只给了上面的手工步骤，未实际跑通。

## 新发现项

- 见上文 updater.rs `open_external_url` 主机白名单缺口（只记不改）。
- `rustfmt.toml` 缺失是基线问题：没有它，任何人本地 `cargo fmt` 都会把
  全库改成 4 空格；本次补上。
- `dtolnay/rust-toolchain@stable` 是分支引用而非 tag：固定 SHA 后
  Dependabot 必须能更新 workflow 文件，否则该 action 会一直停在
  2026-11-26 的 stable 头提交（功能上无害，只是工具链不会自动跟进新版）。

## release.yml 同样固定 SHA 的建议（未改动，仅建议）

`release.yml` 按任务要求保持不动。下面是把它全部 `uses:` 换成 SHA 的对照表，
供后续单独 commit 使用（SHA 均为 2026-11-26 查询的当前 major tag 指向）：

| 现在的写法 | 建议换成的 SHA 引用 |
|---|---|
| `actions/checkout@v7` | `actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1` # v7.0.1 |
| `actions/setup-node@v6` | `actions/setup-node@249970729cb0ef3589644e2896645e5dc5ba9c38` # v6.5.0 |
| `dtolnay/rust-toolchain@stable` | `dtolnay/rust-toolchain@39008827e6b4319a349e9a42dfc90579dd25f1ca` # stable 分支头 |
| `Swatinem/rust-cache@v2` | `Swatinem/rust-cache@d1527c8f67ef9af357290475d4af4b0fab8f12e0` # v2.9.3 |
| `actions/upload-artifact@v7` | `actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9` # v7.0.2 |
| `actions/setup-java@v5` | `actions/setup-java@b6effb05e454b25005698d916606bdc6ffcbf961` # v5.7.0 |
| `actions/download-artifact@v7` | `actions/download-artifact@37930b1c2abaa49bbe596cd826c3c89aef350131` # v7.0.0 |

注意：`release.yml` 的签名与密钥逻辑一并不要动，只换 `uses:` 的引用形式；
换完后跑一次 `workflow_dispatch`（`publish_release: false`）验证。

## 本地验证命令及结果（Linux，Rust 1.100，Node v24）

- `npm ci`：通过。
- `npx tsc --noEmit`：通过。
- `npm run build`（tsc + vite build）：通过，`dist/` 正常产出。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib providers`：通过，
  12 passed（11 个新测试 + 1 个既有 storage 测试被名字过滤命中），
  耗时 6m40s（与并行任务共享 target 目录排队）。
  11 个新测试在基线代码上即全绿，说明任务书暗含的绕过不成立（见上文）。
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`：基线即失败
  （见未修复项第 1 条）；本分支新增的测试代码单文件检查通过。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`：
  失败，exit 101，lib 19 个错误 + lib test 20 个错误，全为基线旧代码
  （`session_manager.rs` 17 处、`platform.rs` 2 处、`lib.rs` 1 处；
  位置与种类清单见本文“未修复项”第 2 条）。本分支新增代码 0 处。
- 全量 `cargo test --manifest-path src-tauri/Cargo.toml`：通过，exit 0，
  lib 37 passed / 0 failed（基线 26 个 + 本分支新增 11 个），
  另两个 0 测试的目标也通过。耗时约 3m20s（增量编译）。
- YAML 校验：`python3 -c yaml.safe_load` 对 `ci.yml` 与 `dependabot.yml` 均通过。
- `npx tauri build`（build 模式，含新 CSP 配置）：通过，exit 0，
  `Finished release profile in 28m 17s`，产物
  `$CARGO_TARGET_DIR/release/multi-agent-account-manager`（20.6 MB）。
  注：`tauri.conf.json` 的 bundle targets 是 `dmg` / `nsis`（macOS / Windows），
  Linux 上只产出可执行文件、无 `bundle/` 目录，属预期行为；
  配置（含新 CSP）在构建开始即被 Tauri 解析通过。
  产物未安装、未运行（按任务要求）。构建耗时长主因 release 编译 webkit 等依赖。
