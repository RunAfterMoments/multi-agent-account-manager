// Modal rendering + openers (P2-11 split).
//
// Moved verbatim from `main.ts`: `renderModal` (+ `modalOverlayClass`),
// `renderUpdateModal`, the four `open*Modal` openers and the pure
// `groupModalVisualSignature`. Behavior-preserving signature changes
// required by the split: `renderModal` takes the current `modal` and
// `shouldAnimateModal` as parameters (same names, bodies untouched),
// `renderUpdateModal` takes the `snapshot` it reads, and the openers take
// `setModal` as a callback (it lives in `main.ts` and rebinds the modal
// state there). `lastRenderedUpdateStage` lives here as a module-level
// binding; `main.ts` writes/reads it through the setter/getter.

import type {
  ExternalAgentKind,
  ExternalAgentView,
  GroupChatRecord,
  ManagerSnapshot,
  ProviderInfo,
  ModalMode,
} from "./types";

import {
  TIMEZONE_PRESETS,
  GROUP_SKILL_PRESETS,
  EXTERNAL_AGENT_PRESETS,
  escapeHtml,
  formatBytes,
  formatProgressSummary,
  formatGroupPickerHint,
  supportsAccountProxy,
  isAccountGroupEligible,
  isMobilePlatform,
  renderMemberAvatarNode,
  renderProviderLogo,
  resolveMemberMeta,
  sameAccountKey,
  formatCookieExpiry,
  updateState,
} from "./state";

// Last rendered update stage (moved from main.ts with renderUpdateModal).
// main.ts writes it via the setter when a download starts / progresses.
let lastRenderedUpdateStage: string | null = null;

export function getLastRenderedUpdateStage(): string | null {
  return lastRenderedUpdateStage;
}

export function setLastRenderedUpdateStage(stage: string | null): void {
  lastRenderedUpdateStage = stage;
}

export function modalOverlayClass(shouldAnimateModal: boolean): string {
  return shouldAnimateModal ? "inline-modal animate-in" : "inline-modal";
}
export function renderUpdateModal(
  snapshot: ManagerSnapshot | null,
  shouldAnimateModal: boolean
): string {
  const res = updateState.result;
  const curVer = res ? res.currentVersion : "...";
  const hasUpdate = Boolean(res?.hasUpdate);
  const title = hasUpdate
    ? `发现新版本 v${escapeHtml(res!.latestVersion)}`
    : "检查更新";

  const versionRow = res
    ? `
      <div class="update-version-row">
        <span class="update-version-pill">当前 v${escapeHtml(curVer)}</span>
        ${
          hasUpdate
            ? `<span>→</span><span class="update-version-pill latest">最新 v${escapeHtml(res.latestVersion)}</span>`
            : ""
        }
        ${res.size ? `<span class="update-size-label">${escapeHtml(formatBytes(res.size))}</span>` : ""}
      </div>
    `
    : `<div class="modal-desc">正在连接 GitHub Releases 检查版本信息...</div>`;

  let bodyHtml = "";
  if (updateState.checking && !res) {
    bodyHtml = `
      <div class="update-progress-wrap">
        <div class="update-progress-track"><div class="update-progress-fill indeterminate"></div></div>
      </div>
    `;
  } else if (res) {
    const noteText = res.releaseNotes.trim();
    if (hasUpdate) {
      bodyHtml += `<div class="update-notes">${escapeHtml(noteText || "包含稳定性改进与问题修复。")}</div>`;
      if (!res.downloadUrl) {
        bodyHtml += `<div class="modal-desc">当前发布尚未包含本平台自动安装包，请点击「发布页」手动下载。</div>`;
      }
    } else {
      bodyHtml += `<div class="modal-desc">${escapeHtml(noteText || "当前已是最新版本。")}</div>`;
    }
  }

  const prog = updateState.progress;
  const showProgress =
    prog &&
    prog.stage !== "error" &&
    (updateState.installing ||
      updateState.outcomeStatus === "restarting" ||
      prog.stage === "ready");
  if (showProgress) {
    lastRenderedUpdateStage = prog.stage;
    const isIndeterminate = prog.percent === null;
    const widthStyle = isIndeterminate ? "" : `style="width:${Math.max(0, Math.min(100, prog.percent!))}%"`;
    bodyHtml += `
      <div class="update-progress-wrap">
        <div class="update-progress-meta">
          <span id="update-progress-text">${escapeHtml(prog.message)}</span>
          <span id="update-progress-pct" class="update-progress-pct">${escapeHtml(formatProgressSummary(prog))}</span>
        </div>
        <div class="update-progress-track">
          <div id="update-progress-bar" class="update-progress-fill ${isIndeterminate ? "indeterminate" : ""}" ${widthStyle}></div>
        </div>
      </div>
    `;
  }

  if (updateState.outcomeMessage) {
    bodyHtml += `<div class="update-outcome-box">${escapeHtml(updateState.outcomeMessage)}</div>`;
  }
  if (updateState.error) {
    bodyHtml += `<div class="update-error-box">${escapeHtml(updateState.error)}</div>`;
  }

  const canInstall = hasUpdate && Boolean(res?.downloadUrl);
  const disablePrimary =
    updateState.checking ||
    updateState.installing ||
    updateState.outcomeStatus === "restarting";
  const primaryBtn = canInstall
    ? `<button id="update-install-btn" class="btn primary" type="button" ${disablePrimary ? "disabled" : ""}>${
        updateState.installing
          ? "更新中..."
          : updateState.outcomeStatus === "needs_permission"
            ? "已授权，继续安装"
            : snapshot?.platform === "ios"
              ? "前往下载 IPA"
              : "立即下载并更新"
      }</button>`
    : `<button id="update-recheck-btn" class="btn primary" type="button" ${disablePrimary ? "disabled" : ""}>${
        updateState.checking ? "检查中..." : "重新检查"
      }</button>`;

  const pageBtn = res?.releasePageUrl
    ? `<button id="update-open-page-btn" class="btn" type="button">发布页</button>`
    : "";

  return `
    <div class="${modalOverlayClass(shouldAnimateModal)}" role="dialog" aria-modal="true">
      <div class="modal-card">
        <div class="modal-title">${title}</div>
        ${versionRow}
        ${bodyHtml}
        <div class="modal-footer">
          <button id="modal-cancel-btn" class="btn" type="button" ${updateState.installing ? "disabled" : ""}>关闭</button>
          ${pageBtn}
          ${primaryBtn}
        </div>
      </div>
    </div>
  `;
}
export function renderModal(
  state: ManagerSnapshot,
  provider: ProviderInfo,
  modal: ModalMode,
  shouldAnimateModal: boolean
): string {
  const platform = state.platform;
  if (modal.kind === "none") return "";
  if (modal.kind === "update") {
    return renderUpdateModal(state, shouldAnimateModal);
  }
  if (modal.kind === "settings") {
    const saving = Boolean(modal.saving);
    const shortcutPresets = [
      { label: "默认", val: "CommandOrControl+Shift+M" },
      { label: "Alt+Space", val: "Alt+Space" },
      { label: "Cmd/Ctrl+Alt+A", val: "CommandOrControl+Alt+A" },
      { label: "禁用", val: "" },
    ];
    const statusText = state.globalShortcutActive
      ? "快捷键已生效"
      : state.globalShortcutError
        ? state.globalShortcutError
        : state.settings.globalShortcut
          ? "未激活"
          : "已禁用";
    return `
      <div class="${modalOverlayClass(shouldAnimateModal)}" role="dialog" aria-modal="true">
        <div class="modal-card">
          <div class="modal-title">桌面端设置</div>
          <label class="setting-row" for="modal-tray-checkbox">
            <span class="setting-label">关闭主窗口时最小化到系统托盘</span>
            <input id="modal-tray-checkbox" class="setting-checkbox" type="checkbox" ${modal.minimizeToTray ? "checked" : ""} ${saving ? "disabled" : ""} />
          </label>
          <label class="setting-row" for="modal-auto-approve-checkbox">
            <span class="setting-label">自动审批权限请求（前台与后台保活账号）</span>
            <input id="modal-auto-approve-checkbox" class="setting-checkbox" type="checkbox" ${modal.autoApprove ? "checked" : ""} ${saving ? "disabled" : ""} />
          </label>
          <div class="modal-hint">开启后，已预热保活的工具授权弹窗将自动点击允许，并跳过删除/退出/支付等高危操作。</div>
          <label class="setting-row" for="modal-disable-auto-update-checkbox">
            <span class="setting-label">禁用自动更新检查</span>
            <input id="modal-disable-auto-update-checkbox" class="setting-checkbox" type="checkbox" ${modal.disableAutoUpdateCheck ? "checked" : ""} ${saving ? "disabled" : ""} />
          </label>
          <div class="modal-hint">开启后，启动时不再自动检查更新，仍可手动打开「更新」检查；当前构建未配置更新源时，检查更新默认关闭。</div>
          <label class="setting-row" for="modal-auto-approve-commands-checkbox">
            <span class="setting-label">允许自动批准命令执行（执行命令 / shell / bash / 终端类审批）</span>
            <input id="modal-auto-approve-commands-checkbox" class="setting-checkbox" type="checkbox" ${modal.autoApproveCommands ? "checked" : ""} ${saving ? "disabled" : ""} />
          </label>
          <div class="modal-hint">默认关闭：命令执行类审批卡默认不自动批准，需手动处理；开启后仍会跳过危险命令（如 rm -rf、sudo）。</div>
          <div class="setting-field">
            <div class="setting-field-head">
              <label class="modal-desc" for="modal-shortcut-input">全局呼出 / 隐藏快捷键</label>
              <span class="shortcut-status-pill ${state.globalShortcutActive ? "ok" : state.globalShortcutError ? "err" : ""}" title="${escapeHtml(statusText)}">${escapeHtml(state.globalShortcutActive ? "已生效" : state.globalShortcutError ? "注册失败" : "未启用")}</span>
            </div>
            <input id="modal-shortcut-input" class="modal-input" maxlength="80" placeholder="CommandOrControl+Shift+M（留空禁用）" value="${escapeHtml(modal.globalShortcut)}" autocomplete="off" spellcheck="false" ${saving ? "disabled" : ""} />
            <div class="preset-chips">
              ${shortcutPresets
                .map(
                  (p) =>
                    `<button class="preset-chip" data-shortcut-preset="${escapeHtml(p.val)}" type="button" ${saving ? "disabled" : ""}>${escapeHtml(p.label)}</button>`
                )
                .join("")}
            </div>
            ${state.globalShortcutError && !modal.error ? `<div class="modal-hint error-hint">${escapeHtml(state.globalShortcutError)}</div>` : `<div class="modal-hint">在任意应用中按下快捷键即可显示或隐藏主窗口；托盘菜单可完全退出。</div>`}
          </div>
          ${modal.error ? `<div id="modal-error-box" class="update-error-box" role="alert">${escapeHtml(modal.error)}</div>` : ""}
          <div class="modal-footer">
            <button id="modal-cancel-btn" class="btn" type="button" ${saving ? "disabled" : ""}>取消</button>
            <button id="modal-confirm-btn" class="btn primary" type="button" ${saving ? "disabled" : ""}>${saving ? "保存中..." : "保存"}</button>
          </div>
        </div>
      </div>
    `;
  }
  if (modal.kind === "add" || modal.kind === "rename") {
    const creating = modal.kind === "add";
    const saving = Boolean(modal.saving);
    const proxyTesting = Boolean(modal.proxyTesting);
    const supportsProxy = supportsAccountProxy(platform);
    const title = creating
      ? `新增 ${escapeHtml(provider.shortName)} 账号`
      : "账号设置";
    const aliasPlaceholder = creating
      ? "备注名称（留空则自动获取登录邮箱）"
      : "备注名称（留空保留原备注）";
    const testRes = modal.proxyTestResult;
    const renameAcc = modal.kind === "rename" ? modal.account : null;
    const currentAcc = renameAcc
      ? (state.accounts.find(
          (a) => a.providerId === renameAcc.providerId && a.uuid === renameAcc.uuid
        ) ?? renameAcc)
      : null;
    const cookieInfo = currentAcc ? formatCookieExpiry(currentAcc) : null;
    return `
      <div class="${modalOverlayClass(shouldAnimateModal)}" role="dialog" aria-modal="true">
        <div class="modal-card">
          <div class="modal-title">${title}</div>
          <input id="modal-alias-input" class="modal-input" maxlength="60" placeholder="${aliasPlaceholder}" aria-label="备注名称" value="${escapeHtml(modal.draft)}" ${saving ? "disabled" : ""} />
          ${supportsProxy ? `
            <label class="modal-desc" for="modal-proxy-input">HTTP CONNECT 代理（留空使用系统网络）</label>
            <div class="proxy-input-row">
              <input id="modal-proxy-input" class="modal-input" maxlength="512" placeholder="http://127.0.0.1:7890" value="${escapeHtml(modal.proxyDraft)}" autocomplete="off" spellcheck="false" ${saving ? "disabled" : ""} />
              ${!isMobilePlatform(platform) ? `<button id="proxy-test-btn" class="btn mini-test-btn" type="button" ${saving || proxyTesting ? "disabled" : ""}>${proxyTesting ? "测速中..." : "测试连接"}</button>` : ""}
            </div>
            ${testRes ? `<div id="proxy-test-result" class="proxy-test-box ${testRes.ok ? "ok" : "err"}">${escapeHtml(testRes.message)}</div>` : ""}
            <div class="modal-hint">仅作用于内嵌页面；外部浏览器登录与更新检查不走此代理。</div>
          ` : ""}
          <label class="modal-desc" for="modal-timezone-input">自定义时区（留空跟随系统）</label>
          <input id="modal-timezone-input" class="modal-input" maxlength="64" placeholder="IANA 时区（例如 America/New_York、Asia/Tokyo）" value="${escapeHtml(modal.timezoneDraft)}" autocomplete="off" spellcheck="false" ${saving ? "disabled" : ""} />
          <div class="preset-chips">
            ${TIMEZONE_PRESETS.map(
              (p) =>
                `<button class="preset-chip" data-timezone-preset="${escapeHtml(p.value)}" type="button" ${saving ? "disabled" : ""}>${escapeHtml(p.label)}</button>`
            ).join("")}
          </div>
          <div class="modal-hint">${platform === "windows" ? "通过 WebView2 原生接口按账号独立生效，留空则使用系统时区。" : "文档加载前注入该账号独立页面，支持夏令时，留空则使用系统时区。"}</div>
          ${cookieInfo ? `<div class="cookie-detail-box ${cookieInfo.tone}" title="${escapeHtml(cookieInfo.tooltip)}"><span class="cookie-expiry-dot"></span><span>${escapeHtml(cookieInfo.detailText)}</span></div>` : ""}
          <div class="cookie-import-section">
            <label class="modal-desc" for="modal-cookie-import-input">${creating ? "导入 Cookie（可选，支持从 Chrome 等浏览器登录后粘贴）" : "导入 Cookie（支持从 Chrome 等浏览器登录后粘贴）"}</label>
            <textarea id="modal-cookie-import-input" class="modal-textarea compact-cookie-textarea" rows="3" placeholder="粘贴 Cookie 请求头（a=1; b=2）、Netscape cookies.txt 或浏览器导出的 JSON 数组..." ${saving ? "disabled" : ""}>${escapeHtml(modal.cookieDraft ?? "")}</textarea>
            ${creating ? `
              <div class="modal-hint">留空则创建后手动登录；粘贴 Cookie 后点击「创建并打开」将直接导入并登录。</div>
            ` : `
              <div class="cookie-import-actions">
                <span class="modal-hint">适用于 Google OAuth 无法在内置窗口直接登录的场景；导入后自动刷新当前账号。</span>
                <button id="modal-import-cookie-btn" class="btn mini-test-btn" type="button" ${saving || modal.cookieImporting ? "disabled" : ""}>${modal.cookieImporting ? "导入中..." : "导入 Cookie"}</button>
              </div>
            `}
          </div>
          ${modal.error ? `<div id="modal-error-box" class="update-error-box" role="alert">${escapeHtml(modal.error)}</div>` : ""}
          <div class="modal-footer">
            ${!creating ? `<button id="modal-clear-data-btn" class="btn danger-outline left-action" type="button" ${saving ? "disabled" : ""}>${modal.confirmClear ? "确认清空？" : "清空登录缓存"}</button>` : ""}
            <button id="modal-cancel-btn" class="btn" type="button" ${saving ? "disabled" : ""}>取消</button>
            <button id="modal-confirm-btn" class="btn primary" type="button" ${saving ? "disabled" : ""}>${saving ? (creating ? "创建中..." : "保存中...") : (creating ? "创建并打开" : "保存")}</button>
          </div>
        </div>
      </div>
    `;
  }
  if (modal.kind === "group") {
    const gModal = modal;
    const editing = Boolean(gModal.group);
    const saving = Boolean(gModal.saving);
    const extAgents = state.externalAgents ?? [];
    const eligibleCount =
      state.accounts.filter((a) => isAccountGroupEligible(state, a)).length + extAgents.length;
    const ineligibleCount = state.accounts.length + extAgents.length - eligibleCount;

    const providerGroupsHtml = state.providers
      .filter((p) => p.enabled)
      .map((p) => {
        const accs = state.accounts.filter((a) => {
          if (a.providerId !== p.id) return false;
          if (isAccountGroupEligible(state, a)) return true;
          return Boolean(
            editing &&
              a.authState !== "logged_out" &&
              gModal.selectedMembers.some((m) => m.providerId === a.providerId && m.uuid === a.uuid)
          );
        });
        if (accs.length === 0) return "";
        return `
          <div class="group-picker-section">
            <div class="group-picker-provider">${renderProviderLogo(p, "group-picker-plogo")}<span>${escapeHtml(p.shortName)}</span></div>
            <div class="group-member-list">
              ${accs
                .map((acc) => {
                  const checked = gModal.selectedMembers.some(
                    (m) => m.providerId === acc.providerId && m.uuid === acc.uuid
                  );
                  const cookieHint = formatCookieExpiry(acc);
                  return `
                    <button
                      class="group-member-option ${checked ? "selected" : ""}"
                      data-toggle-group-member="1"
                      data-member-provider="${escapeHtml(acc.providerId)}"
                      data-member-uuid="${escapeHtml(acc.uuid)}"
                      type="button"
                      ${saving ? "disabled" : ""}
                    >
                      <span class="group-check-box">${checked ? "✓" : ""}</span>
                      ${renderMemberAvatarNode(state, acc, acc.alias, "mini")}
                      <span class="group-option-meta">
                        <span class="group-option-name">${escapeHtml(acc.alias)}</span>
                        ${cookieHint ? `<span class="group-option-sub">${escapeHtml(cookieHint.shortText)}</span>` : ""}
                      </span>
                    </button>
                  `;
                })
                .join("")}
            </div>
          </div>
        `;
      })
      .filter(Boolean)
      .join("");

    const extKindGroups: Array<{ kind: ExternalAgentKind; title: string; pid: "api" | "cli" }> = [
      { kind: "openai_api", title: "OpenAI API", pid: "api" },
      { kind: "local_cli", title: "本地 CLI", pid: "cli" },
    ];
    const externalGroupHtml = extKindGroups
      .map(({ kind, title, pid }) => {
        const list = extAgents.filter((e) => e.kind === kind);
        if (list.length === 0) return "";
        return `
          <div class="group-picker-section">
            <div class="group-picker-provider"><span>${escapeHtml(title)}</span><span class="group-provider-tag">直连</span></div>
            <div class="group-member-list">
              ${list
                .map((ext) => {
                  const checked = gModal.selectedMembers.some(
                    (m) => m.providerId === pid && m.uuid === ext.id
                  );
                  const sub =
                    ext.kind === "openai_api"
                      ? ext.model
                      : ext.command;
                  return `
                    <button
                      class="group-member-option ${checked ? "selected" : ""}"
                      data-toggle-group-member="1"
                      data-member-provider="${pid}"
                      data-member-uuid="${escapeHtml(ext.id)}"
                      type="button"
                      ${saving ? "disabled" : ""}
                    >
                      <span class="group-check-box">${checked ? "✓" : ""}</span>
                      ${renderMemberAvatarNode(state, undefined, ext.name, "mini", ext.kind)}
                      <span class="group-option-meta">
                        <span class="group-option-name">${escapeHtml(ext.name)}</span>
                        <span class="group-option-sub">${escapeHtml(sub)}</span>
                      </span>
                    </button>
                  `;
                })
                .join("")}
            </div>
          </div>
        `;
      })
      .filter(Boolean)
      .join("");

    const pickerContent = `${providerGroupsHtml}${externalGroupHtml}`;
    const hintText = formatGroupPickerHint(
      eligibleCount,
      ineligibleCount,
      gModal.selectedMembers.length,
      Boolean(pickerContent)
    );

    return `
      <div class="${modalOverlayClass(shouldAnimateModal)}" role="dialog" aria-modal="true">
        <div class="modal-card group-modal-card">
          <div class="modal-title">${editing ? `编辑「${escapeHtml(gModal.group!.name)}」` : "创建群聊"}</div>
          <input id="modal-group-name-input" class="modal-input" maxlength="32" placeholder="群聊名称（如：产品头脑风暴）" aria-label="群聊名称" value="${escapeHtml(gModal.nameDraft)}" ${saving ? "disabled" : ""} />
          <div class="group-picker-head-row">
            <div class="modal-desc">选择参与群聊的网页账号或 API / CLI 成员</div>
            <button id="open-external-agents-from-group-btn" class="mini-btn primary-text" type="button" ${saving ? "disabled" : ""}>+ 管理 API/CLI</button>
          </div>
          <div class="group-member-picker">
            ${pickerContent || `<div class="group-picker-empty">当前没有可用成员，可登录网页或点击右上角添加 API / CLI</div>`}
          </div>
          <div id="modal-group-hint" class="modal-hint">${escapeHtml(hintText)}</div>
          ${gModal.error ? `<div id="modal-error-box" class="update-error-box" role="alert">${escapeHtml(gModal.error)}</div>` : ""}
          <div class="modal-footer">
            <button id="modal-cancel-btn" class="btn" type="button" ${saving ? "disabled" : ""}>取消</button>
            <button id="modal-confirm-btn" class="btn primary" type="button" ${saving || gModal.selectedMembers.length === 0 ? "disabled" : ""}>${saving ? (editing ? "保存中..." : "创建中...") : (editing ? "保存" : "创建群聊")}</button>
          </div>
        </div>
      </div>
    `;
  }
  if (modal.kind === "group-knowledge") {
    const kModal = modal;
    const saving = Boolean(kModal.saving);
    const enabledCount = kModal.skillsDraft.filter((s) => s.enabled).length;
    const latestGroup = state.groupChats.find((g) => g.id === kModal.group.id) ?? kModal.group;

    let tabBody = "";
    if (kModal.tab === "context") {
      tabBody = `
        <div class="modal-desc">所有被 @ 唤醒的成员在首次进入或配置变更后自动增量同步此上下文，API / CLI 成员每轮自动携带。</div>
        <textarea
          id="modal-group-context-input"
          class="modal-textarea"
          rows="8"
          maxlength="8000"
          placeholder="例如：项目采用 Tauri + Rust + TypeScript；核心约束是低内存；当前正在排查…"
          ${saving ? "disabled" : ""}
        >${escapeHtml(kModal.contextDraft)}</textarea>
        <div class="modal-hint"><span id="modal-group-context-count">${kModal.contextDraft.length}</span>/8000 字符 · 也可点消息「沉淀上下文」或顶部「沉淀本轮结论」</div>
      `;
    } else if (kModal.tab === "skills") {
      const presetsHtml = GROUP_SKILL_PRESETS.map((p, idx) => {
        const added = kModal.skillsDraft.some((s) => s.name === p.name);
        return `<button class="skill-preset-chip" data-add-skill-preset="${idx}" type="button" ${saving || added ? "disabled" : ""}>${added ? `✓ ${escapeHtml(p.name)}` : `+ ${escapeHtml(p.name)}`}</button>`;
      }).join("");

      const skillsHtml =
        kModal.skillsDraft.length === 0
          ? `<div class="group-picker-empty">暂未添加共享技能，可点击上方预设或下方自定义添加。</div>`
          : kModal.skillsDraft
              .map(
                (s) => `
              <div class="skill-card ${s.enabled ? "" : "disabled"}">
                <div class="skill-card-head">
                  <label class="skill-toggle-label">
                    <input type="checkbox" data-toggle-skill-id="${escapeHtml(s.id)}" ${s.enabled ? "checked" : ""} ${saving ? "disabled" : ""} />
                    <span class="skill-card-name">${escapeHtml(s.name)}</span>
                  </label>
                  <div class="skill-card-ops">
                    <button class="mini-btn" data-edit-skill-id="${escapeHtml(s.id)}" type="button" ${saving ? "disabled" : ""}>编辑</button>
                    <button class="mini-btn danger-text" data-delete-skill-id="${escapeHtml(s.id)}" type="button" ${saving ? "disabled" : ""}>删除</button>
                  </div>
                </div>
                <div class="skill-card-desc">${escapeHtml(s.description || s.content)}</div>
              </div>
            `
              )
              .join("");

      tabBody = `
        <div class="skill-presets-row">
          <span class="skill-presets-label">预设：</span>
          ${presetsHtml}
        </div>
        <div class="skills-list">${skillsHtml}</div>
        <div class="skill-editor-box">
          <div class="skill-editor-title">${kModal.editingSkillId ? "编辑共享技能" : "添加自定义技能"}</div>
          <input id="modal-skill-name-input" class="modal-input" maxlength="48" placeholder="技能名称（如：代码审查规范）" value="${escapeHtml(kModal.skillNameDraft)}" ${saving ? "disabled" : ""} />
          <input id="modal-skill-desc-input" class="modal-input" maxlength="120" placeholder="简短说明（选填）" value="${escapeHtml(kModal.skillDescDraft)}" ${saving ? "disabled" : ""} />
          <textarea id="modal-skill-content-input" class="modal-textarea" rows="3" maxlength="2000" placeholder="输入注入给各 AI 成员的技能指令原文…" ${saving ? "disabled" : ""}>${escapeHtml(kModal.skillContentDraft)}</textarea>
          <div class="skill-editor-actions">
            ${kModal.editingSkillId ? `<button id="modal-skill-cancel-edit-btn" class="mini-btn" type="button" ${saving ? "disabled" : ""}>取消编辑</button>` : ""}
            <button id="modal-skill-save-btn" class="mini-btn primary-text" type="button" ${saving ? "disabled" : ""}>${kModal.editingSkillId ? "更新技能" : "+ 添加技能"}</button>
          </div>
        </div>
      `;
    } else if (kModal.tab === "roles") {
      const roleRowsHtml = latestGroup.members
        .map((m) => {
          const meta = resolveMemberMeta(state, m);
          const curRole =
            kModal.rolesDraft.find((r) => sameAccountKey(r.member, m))?.role ?? "";
          const binding = !meta.isExternal
            ? (latestGroup.threadBindings ?? []).find((tb) => sameAccountKey(tb.member, m))
            : undefined;
          let boundShort = "";
          if (binding) {
            try {
              const u = new URL(binding.threadUrl);
              boundShort = `${u.hostname}${u.pathname}`;
            } catch {
              boundShort = binding.threadUrl;
            }
          }
          const bindingHtml = meta.isExternal
            ? `<div class="role-thread-row"><span class="thread-stateless-tag">无状态直连（每轮携带完整上下文）</span></div>`
            : binding
              ? `
                <div class="role-thread-row">
                  <span class="thread-bound-pill" title="${escapeHtml(binding.threadUrl)}">已绑定专属会话 · ${escapeHtml(boundShort)}</span>
                  <button class="mini-btn" data-reset-thread-provider="${escapeHtml(m.providerId)}" data-reset-thread-uuid="${escapeHtml(m.uuid)}" type="button" ${saving ? "disabled" : ""}>重置会话</button>
                </div>
              `
              : `<div class="role-thread-row"><span class="thread-unbound-tag">首次发消息时自动创建专属会话（不污染日常聊天）</span></div>`;

          return `
            <div class="role-row">
              <div class="role-member-meta">
                ${renderMemberAvatarNode(state, meta.acc, meta.alias, "mini", meta.ext?.kind)}
                <span class="role-member-name">${escapeHtml(meta.alias)}</span>
                <span class="group-provider-tag">${escapeHtml(meta.providerShort)}</span>
              </div>
              <input
                class="modal-input role-input"
                maxlength="120"
                data-role-provider="${escapeHtml(m.providerId)}"
                data-role-uuid="${escapeHtml(m.uuid)}"
                placeholder="成员分工（如：前端性能优化与交互评审）"
                value="${escapeHtml(curRole)}"
                ${saving ? "disabled" : ""}
              />
              ${bindingHtml}
            </div>
          `;
        })
        .join("");

      tabBody = `
        <div class="modal-desc">配置各成员专属分工指令，并可查看/重置网页成员绑定的群聊独立会话 URL。</div>
        <div class="role-list">${roleRowsHtml || `<div class="group-picker-empty">暂无成员</div>`}</div>
      `;
    } else {
      const snap = kModal.adaptersSnap;
      const chipsHtml = (snap?.adapters ?? [])
        .map(
          (a) =>
            `<span class="adapter-chip" title="${escapeHtml(a.hostPatterns.join(", "))}">${escapeHtml(a.name)} (${escapeHtml(a.id)})</span>`
        )
        .join("");
      tabBody = `
        <div class="modal-desc">声明式网站适配器（内置 Muse / Hark 规则，可按需热覆盖选择器）与 Fetch/SSE 只读挂钩协同工作。</div>
        <div class="adapter-chip-row">${chipsHtml || `<span class="modal-hint">正在读取规则…</span>`}</div>
        <div class="modal-hint">覆盖规则文件（保存即热生效，无需重编译）：<code>${escapeHtml(snap?.overridePath ?? "site_adapters.json")}</code></div>
        <textarea
          id="modal-adapters-json-input"
          class="modal-textarea code"
          rows="7"
          placeholder='可选：输入 JSON 数组覆盖内置规则，例如 [{"id":"muse","name":"Muse","hostPatterns":["muse.ai"],"composerSelectors":["textarea"]}]'
          ${saving ? "disabled" : ""}
        >${escapeHtml(kModal.adaptersJsonDraft)}</textarea>
        <div class="skill-editor-actions">
          <button id="modal-adapters-reset-btn" class="mini-btn" type="button" ${saving || !snap?.hasOverrides ? "disabled" : ""}>恢复内置默认</button>
        </div>
      `;
    }

    const confirmLabel =
      kModal.tab === "adapters"
        ? saving ? "保存中..." : "保存并热生效"
        : saving ? "保存中..." : "保存配置";

    return `
      <div class="${modalOverlayClass(shouldAnimateModal)}" role="dialog" aria-modal="true">
        <div class="modal-card group-knowledge-modal-card">
          <div class="modal-title">共享上下文 & 技能 · ${escapeHtml(latestGroup.name)}</div>
          <div class="knowledge-tabs">
            <button class="knowledge-tab ${kModal.tab === "context" ? "active" : ""}" data-knowledge-tab="context" type="button" ${saving ? "disabled" : ""}>共享上下文</button>
            <button class="knowledge-tab ${kModal.tab === "skills" ? "active" : ""}" data-knowledge-tab="skills" type="button" ${saving ? "disabled" : ""}>共享技能 (${enabledCount})</button>
            <button class="knowledge-tab ${kModal.tab === "roles" ? "active" : ""}" data-knowledge-tab="roles" type="button" ${saving ? "disabled" : ""}>分工 &amp; 会话</button>
            <button class="knowledge-tab ${kModal.tab === "adapters" ? "active" : ""}" data-knowledge-tab="adapters" type="button" ${saving ? "disabled" : ""}>站点规则${kModal.adaptersSnap?.hasOverrides ? " *" : ""}</button>
          </div>
          ${tabBody}
          ${kModal.error ? `<div id="modal-error-box" class="update-error-box" role="alert">${escapeHtml(kModal.error)}</div>` : ""}
          <div class="modal-footer">
            <button id="modal-cancel-btn" class="btn" type="button" ${saving ? "disabled" : ""}>取消</button>
            <button id="modal-confirm-btn" class="btn primary" type="button" ${saving ? "disabled" : ""}>${confirmLabel}</button>
          </div>
        </div>
      </div>
    `;
  }
  if (modal.kind === "external-agents") {
    const eModal = modal;
    const saving = Boolean(eModal.saving);
    const agents = state.externalAgents ?? [];
    const presetsHtml = EXTERNAL_AGENT_PRESETS.map(
      (p, idx) =>
        `<button class="skill-preset-chip" data-ext-preset-idx="${idx}" type="button" ${saving ? "disabled" : ""}>+ ${escapeHtml(p.label)}</button>`
    ).join("");
    const listHtml =
      agents.length === 0
        ? `<div class="group-picker-empty">暂未配置 API 或本地 CLI 成员，可点击下方预设或手动添加。</div>`
        : agents
            .map((ext) => {
              const desc =
                ext.kind === "openai_api"
                  ? `${ext.model} · ${ext.baseUrl}`
                  : `${ext.command} ${ext.args.join(" ")}`.trim();
              return `
                <div class="ext-agent-card">
                  <div class="ext-agent-main">
                    ${renderMemberAvatarNode(state, undefined, ext.name, "mini", ext.kind)}
                    <div class="ext-agent-meta">
                      <div class="ext-agent-name">${escapeHtml(ext.name)}</div>
                      <div class="ext-agent-sub">${escapeHtml(desc)}</div>
                    </div>
                  </div>
                  <div class="skill-card-ops">
                    <button class="mini-btn" data-edit-ext-id="${escapeHtml(ext.id)}" type="button" ${saving ? "disabled" : ""}>编辑</button>
                    <button class="mini-btn danger-text" data-delete-ext-id="${escapeHtml(ext.id)}" type="button" ${saving ? "disabled" : ""}>删除</button>
                  </div>
                </div>
              `;
            })
            .join("");

    const fieldsHtml =
      eModal.formKind === "openai_api"
        ? `
          <input id="ext-agent-url-input" class="modal-input" maxlength="260" placeholder="Base URL（例如 https://api.deepseek.com/v1 或 http://127.0.0.1:11434/v1）" value="${escapeHtml(eModal.baseUrlDraft)}" ${saving ? "disabled" : ""} />
          <input id="ext-agent-key-input" class="modal-input" maxlength="512" type="password" autocomplete="new-password" spellcheck="false" placeholder="API Key（编辑时留空表示不修改；本机 Ollama 可留空）" ${saving ? "disabled" : ""} />
          <input id="ext-agent-model-input" class="modal-input" maxlength="120" placeholder="模型名称（例如 gpt-4o / deepseek-chat / qwen2.5:14b）" value="${escapeHtml(eModal.modelDraft)}" ${saving ? "disabled" : ""} />
        `
        : `
          <input id="ext-agent-cmd-input" class="modal-input" maxlength="260" placeholder="CLI 命令（例如 claude / codex / gemini）" value="${escapeHtml(eModal.commandDraft)}" ${saving ? "disabled" : ""} />
          <input id="ext-agent-args-input" class="modal-input" maxlength="400" placeholder="命令参数，空格分隔（例如 -p 或 exec；prompt 自动走 stdin，不要写 {prompt}）" value="${escapeHtml(eModal.argsDraft)}" ${saving ? "disabled" : ""} />
          <input id="ext-agent-workdir-input" class="modal-input" maxlength="260" placeholder="工作目录（选填，例如 /Users/name/project）" value="${escapeHtml(eModal.workdirDraft)}" ${saving ? "disabled" : ""} />
        `;

    return `
      <div class="${modalOverlayClass(shouldAnimateModal)}" role="dialog" aria-modal="true">
        <div class="modal-card group-knowledge-modal-card">
          <div class="modal-title">API / 本地 CLI 混合成员</div>
          <div class="modal-desc">配置直连 OpenAI 兼容接口或本地 CLI（如 Claude Code / Codex），与网页端账号混编入同一个群聊。</div>
          <div class="ext-agent-list">${listHtml}</div>
          <div class="skill-presets-row">
            <span class="skill-presets-label">快速填充：</span>
            ${presetsHtml}
          </div>
          <div class="skill-editor-box">
            <div class="ext-form-head">
              <span class="skill-editor-title">${eModal.editingId ? "编辑成员" : "新增直连成员"}</span>
              <div class="ext-kind-switch">
                <button class="mini-btn ${eModal.formKind === "openai_api" ? "primary-text" : ""}" data-ext-kind="openai_api" type="button" ${saving ? "disabled" : ""}>OpenAI API</button>
                <button class="mini-btn ${eModal.formKind === "local_cli" ? "primary-text" : ""}" data-ext-kind="local_cli" type="button" ${saving ? "disabled" : ""}>本地 CLI</button>
              </div>
            </div>
            <input id="ext-agent-name-input" class="modal-input" maxlength="40" placeholder="群聊显示名称（如：Claude-Code / DeepSeek）" value="${escapeHtml(eModal.nameDraft)}" ${saving ? "disabled" : ""} />
            <input id="ext-agent-timeout-input" class="modal-input" inputmode="numeric" maxlength="4" placeholder="总超时（秒，默认 600，30–3600）" value="${escapeHtml(eModal.timeoutSecsDraft)}" ${saving ? "disabled" : ""} />
            <input id="ext-agent-idle-timeout-input" class="modal-input" inputmode="numeric" maxlength="3" placeholder="空闲超时（秒，默认 60，10–600；无输出才判超时）" value="${escapeHtml(eModal.idleTimeoutSecsDraft)}" ${saving ? "disabled" : ""} />
            ${fieldsHtml}
            <div class="skill-editor-actions">
              ${eModal.editingId ? `<button id="ext-agent-cancel-edit-btn" class="mini-btn" type="button" ${saving ? "disabled" : ""}>取消编辑</button>` : ""}
              <button id="ext-agent-save-btn" class="mini-btn primary-text" type="button" ${saving ? "disabled" : ""}>${saving ? "保存中..." : eModal.editingId ? "更新成员" : "+ 保存成员"}</button>
            </div>
          </div>
          ${eModal.error ? `<div id="modal-error-box" class="update-error-box" role="alert">${escapeHtml(eModal.error)}</div>` : ""}
          <div class="modal-footer">
            <button id="modal-cancel-btn" class="btn primary" type="button" ${saving ? "disabled" : ""}>完成</button>
          </div>
        </div>
      </div>
    `;
  }
  if (modal.kind === "delete-group") {
    return `
      <div class="${modalOverlayClass(shouldAnimateModal)}" role="dialog" aria-modal="true">
        <div class="modal-card">
          <div class="modal-title">删除群聊「${escapeHtml(modal.group.name)}」</div>
          <div class="modal-desc">将删除该群聊及本地群聊记录。各成员账号与原生网页历史不受影响。</div>
          <div class="modal-footer">
            <button id="modal-cancel-btn" class="btn" type="button">取消</button>
            <button id="modal-confirm-btn" class="btn danger" type="button">确认删除</button>
          </div>
        </div>
      </div>
    `;
  }
  return `
    <div class="${modalOverlayClass(shouldAnimateModal)}" role="dialog" aria-modal="true">
      <div class="modal-card">
        <div class="modal-title">移除「${escapeHtml(modal.account.alias)}」</div>
        <div class="modal-desc">将关闭该视图并彻底清除本机独立浏览资料（Cookie 与存储）。云端账号及历史对话仍由官方保留。</div>
        <div class="modal-footer">
          <button id="modal-cancel-btn" class="btn" type="button">取消</button>
          <button id="modal-confirm-btn" class="btn danger" type="button">确认移除</button>
        </div>
      </div>
    </div>
  `;
}
export async function openCreateGroupModal(
  state: ManagerSnapshot,
  setModal: (next: ModalMode) => Promise<void>
): Promise<void> {
  await setModal({
    kind: "group",
    nameDraft: `群聊 ${state.groupChats.length + 1}`,
    selectedMembers: [],
  });
}
export async function openEditGroupModal(
  group: GroupChatRecord,
  setModal: (next: ModalMode) => Promise<void>
): Promise<void> {
  await setModal({
    kind: "group",
    group,
    nameDraft: group.name,
    selectedMembers: group.members.map((m) => ({ providerId: m.providerId, uuid: m.uuid })),
  });
}
export async function openGroupKnowledgeModal(
  group: GroupChatRecord,
  setModal: (next: ModalMode) => Promise<void>,
  tab: "context" | "skills" | "roles" | "adapters" = "context"
): Promise<void> {
  await setModal({
    kind: "group-knowledge",
    group,
    tab,
    contextDraft: group.sharedContext ?? "",
    skillsDraft: (group.sharedSkills ?? []).map((s) => ({ ...s })),
    rolesDraft: (group.memberRoles ?? []).map((r) => ({
      member: { ...r.member },
      role: r.role,
    })),
    skillNameDraft: "",
    skillDescDraft: "",
    skillContentDraft: "",
    editingSkillId: null,
    adaptersSnap: null,
    adaptersJsonDraft: "",
    adaptersLoaded: false,
  });
}
export async function openExternalAgentsModal(
  editAgent: ExternalAgentView | undefined,
  setModal: (next: ModalMode) => Promise<void>
): Promise<void> {
  await setModal({
    kind: "external-agents",
    editingId: editAgent?.id ?? null,
    formKind: editAgent?.kind ?? "openai_api",
    nameDraft: editAgent?.name ?? "",
    baseUrlDraft: editAgent?.baseUrl ?? "",
    apiKeyDraft: "",
    modelDraft: editAgent?.model ?? "",
    commandDraft: editAgent?.command ?? "",
    argsDraft: editAgent ? editAgent.args.join(" ") : "",
    workdirDraft: editAgent?.workdir ?? "",
    timeoutSecsDraft: editAgent ? String(editAgent.timeoutSecs) : "",
    idleTimeoutSecsDraft: editAgent ? String(editAgent.idleTimeoutSecs) : "",
  });
}
export function groupModalVisualSignature(s: ManagerSnapshot): string {
  return JSON.stringify({
    selectedProviderId: s.selectedProviderId,
    activeGroupId: s.activeGroupId,
    notice: s.notice,
    conflicts: s.conflicts,
    providers: s.providers.map((p) => [p.id, p.enabled, p.shortName]),
    accounts: s.accounts.map((a) => [
      a.providerId,
      a.uuid,
      a.alias,
      a.authState,
      a.avatarUrl,
      a.cookieExpiresAtMs,
      a.cookieSessionOnly,
    ]),
    accountErrors: s.accountErrors,
    externalAgents: (s.externalAgents ?? []).map((e) => [
      e.id,
      e.kind,
      e.name,
      e.model,
      e.command,
    ]),
    groupChats: s.groupChats.map((g) => {
      const lastMsg = g.messages[g.messages.length - 1];
      return [
        g.id,
        g.name,
        g.members.length,
        g.messages.length,
        lastMsg ? [lastMsg.id, lastMsg.status, lastMsg.content.length] : null,
      ];
    }),
  });
}
