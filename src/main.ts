import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./styles.css";
import type {
  AccountKey,
  DownloadNoticeEvent,
  ExternalAgentKind,
  GroupWorkflowMode,
  InstallUpdateOutcome,
  ManagerSnapshot,
  ProviderId,
  ProviderInfo,
  ProxyTestResult,
  SiteAdaptersSnapshot,
  UpdateCheckResult,
  UpdateProgressEvent,
  ModalMode,
} from "./types";
import {
  GROUP_SKILL_PRESETS,
  EXTERNAL_AGENT_PRESETS,
  ICON_UPDATE,
  ICON_SETTINGS,
  ICON_GROUP,
  ICON_CHEVRON_LEFT,
  ICON_CHEVRON_RIGHT,
  syncingMsgIds,
  themeMode,
  themeLabel,
  themeIcon,
  cycleThemeMode,
  groupDrafts,
  groupTargets,
  groupAutoChannels,
  copyingCookies,
  updateState,
  scrollState,
  isSameKey,
  isAsciiWordChar,
  sameAccountKey,
  isMobilePlatform,
  syncViewModeFromSnapshot,
  findAccountPageError,
  isAccountGroupEligible,
  groupHasNeedsAuth,
  resolveMemberMeta,
  resolveMemberCategory,
  groupByMemberCategory,
  resolveGroupMessageSenderName,
  escapeHtml,
  renderProviderLogo,
  scoreAccountAvailability,
  compareAccountAvailability,
  formatProgressSummary,
  supportsAccountProxy,
  formatGroupPickerHint,
  applyTheme,
  systemDarkQuery,
} from "./state";
import {
  openCreateGroupModal,
  openEditGroupModal,
  openGroupKnowledgeModal,
  openExternalAgentsModal,
  groupModalVisualSignature,
  getLastRenderedUpdateStage,
  setLastRenderedUpdateStage,
} from "./modals";
import {
  renderMiddleContent,
  renderRightStage,
  currentProvider,
  saveScrollPosition,
  restoreScrollPosition,
} from "./render";
import {
  renderGroupMiddleContent,
  renderGroupRightStage,
  extractComposerAtMentions,
  formatComposerHintText,
  getLatestRoundCompletedReplies,
} from "./group-chat";

// Theme wiring (was top-level in main.ts before the P2-11 split; kept here
// because it touches the app snapshot and re-render in this module).
applyTheme();
systemDarkQuery.addEventListener("change", () => {
  if (themeMode === "system") {
    applyTheme();
    if (snapshot) {
      rerenderPreservingScroll();
    }
  }
});











let snapshot: ManagerSnapshot | null = null;
let modal: ModalMode = { kind: "none" };
let optimisticSwitching: AccountKey | null = null;
let busy = false;
let viewMode: "provider" | "groups" = "provider";
let shouldAnimateModal = false;

let newTopicConfirmGroupId: string | null = null;
let groupSending = false;
let downloadToast: DownloadNoticeEvent | null = null;
let downloadToastTimer: number | undefined;
let draggingAccountUuid: string | null = null;

function clearDownloadToastTimer(): void {
  if (downloadToastTimer !== undefined) {
    window.clearTimeout(downloadToastTimer);
    downloadToastTimer = undefined;
  }
}

function scheduleDownloadToastDismiss(): void {
  clearDownloadToastTimer();
  if (!downloadToast || downloadToast.status === "started") return;
  downloadToastTimer = window.setTimeout(() => {
    downloadToast = null;
    rerenderPreservingScroll();
  }, 12000);
}







function rerenderPreservingScroll(): void {
  saveScrollPosition(snapshot, viewMode);
  render();
  restoreScrollPosition(snapshot, viewMode, modal);
}


async function runCheckForUpdates(silent: boolean): Promise<void> {
  if (updateState.checking || updateState.installing) return;
  updateState.checking = true;
  if (!silent) {
    updateState.error = null;
    updateState.outcomeStatus = null;
    updateState.outcomeMessage = null;
    if (modal.kind === "update") {
      rerenderPreservingScroll();
    }
  }
  try {
    const res = await invoke<UpdateCheckResult>("check_for_updates");
    updateState.result = res;
    updateState.error = null;
    updateState.lastCheckedAt = Date.now();
    updateState.lastFailedAt = 0;
  } catch (err) {
    updateState.lastFailedAt = Date.now();
    if (!silent) {
      updateState.error = String(err);
    }
  } finally {
    updateState.checking = false;
    if (!silent || updateState.result?.hasUpdate || modal.kind === "update") {
      rerenderPreservingScroll();
    }
  }
}

async function maybeAutoCheckForUpdates(): Promise<void> {
  if (snapshot?.settings.disableAutoUpdateCheck) return;
  if (updateState.checking || updateState.installing) return;
  const now = Date.now();
  if (updateState.lastCheckedAt === 0) {
    if (updateState.lastFailedAt === 0 || now - updateState.lastFailedAt >= 2 * 60 * 1000) {
      await runCheckForUpdates(true);
    }
    return;
  }
  const interval = updateState.result?.hasUpdate ? 2 * 60 * 60 * 1000 : 30 * 60 * 1000;
  if (now - updateState.lastCheckedAt >= interval) {
    await runCheckForUpdates(true);
  }
}

async function runDownloadAndInstall(): Promise<void> {
  const res = updateState.result;
  if (!res?.downloadUrl || updateState.checking || updateState.installing) return;
  updateState.installing = true;
  updateState.error = null;
  updateState.outcomeStatus = null;
  updateState.outcomeMessage = null;
  setLastRenderedUpdateStage("downloading");
  updateState.progress = {
    stage: "downloading",
    downloaded: 0,
    total: res.size,
    percent: 0,
    message: "正在准备下载...",
  };
  rerenderPreservingScroll();
  try {
    const outcome = await invoke<InstallUpdateOutcome>("download_and_install_update", {
      request: {
        version: res.latestVersion,
        downloadUrl: res.downloadUrl,
        sha256: res.sha256,
        filename: res.filename,
      },
    });
    updateState.outcomeStatus = outcome.status;
    updateState.outcomeMessage = outcome.message;
  } catch (err) {
    updateState.error = String(err);
  } finally {
    updateState.installing = false;
    rerenderPreservingScroll();
  }
}

async function setModal(next: ModalMode): Promise<void> {
  const prevKind = modal.kind;
  const wasActive = prevKind !== "none";
  const isActive = next.kind !== "none";
  if (next.kind !== prevKind) {
    shouldAnimateModal = isActive;
    scrollState.modalOverlayScrollTop = 0;
    scrollState.groupMemberPickerScrollTop = 0;
    scrollState.modalCardScrollTop = 0;
  }
  modal = next;
  if (wasActive !== isActive) {
    try {
      snapshot = await invoke<ManagerSnapshot>("set_modal_active", {
        active: isActive,
      });
    } catch (err) {
      if (snapshot) {
        snapshot = { ...snapshot, notice: String(err) };
      }
    }
  }
  render();
  if (next.kind === "add" || next.kind === "rename") {
    const input = document.querySelector<HTMLInputElement>("#modal-alias-input");
    if (input) {
      input.focus();
      input.select();
    }
  } else if (next.kind === "group") {
    const input = document.querySelector<HTMLInputElement>("#modal-group-name-input");
    if (input) {
      input.focus();
      input.select();
    }
  } else if (next.kind === "group-knowledge") {
    document.querySelector<HTMLTextAreaElement>("#modal-group-context-input")?.focus();
  } else if (next.kind === "settings") {
    document.querySelector<HTMLInputElement>("#modal-shortcut-input")?.focus();
  } else if (next.kind === "remove" || next.kind === "delete-group") {
    document.querySelector<HTMLButtonElement>("#modal-confirm-btn")?.focus();
  }
}

async function callCommand(
  cmd: string,
  args: Record<string, unknown> = {},
  optimisticTarget: AccountKey | null = null
): Promise<boolean> {
  if (busy) return false;
  busy = true;
  saveScrollPosition(snapshot, viewMode);
  if (optimisticTarget) {
    optimisticSwitching = optimisticTarget;
    if (snapshot) {
      snapshot = { ...snapshot, activeAccount: null };
    }
    render();
    restoreScrollPosition(snapshot, viewMode, modal);
  }
  try {
    snapshot = await invoke<ManagerSnapshot>(cmd, args);
    const nextViewMode = syncViewModeFromSnapshot(snapshot);
    if (nextViewMode) viewMode = nextViewMode;
    return true;
  } catch (err) {
    if (
      (cmd === "add_account" && modal.kind === "add") ||
      (cmd === "rename_account" && modal.kind === "rename") ||
      (cmd === "clear_account_data" && modal.kind === "rename") ||
      (cmd === "import_account_cookies" && modal.kind === "rename") ||
      ((cmd === "create_group_chat" || cmd === "update_group_chat") && modal.kind === "group") ||
      ((cmd === "update_group_knowledge" || cmd === "reset_group_thread_binding") &&
        modal.kind === "group-knowledge") ||
      ((cmd === "upsert_external_agent" || cmd === "delete_external_agent") &&
        modal.kind === "external-agents") ||
      (cmd === "update_app_settings" && modal.kind === "settings")
    ) {
      modal.error = String(err);
    } else if (snapshot) {
      snapshot = { ...snapshot, notice: String(err) };
    }
    return false;
  } finally {
    optimisticSwitching = null;
    busy = false;
    render();
    restoreScrollPosition(snapshot, viewMode, modal);
  }
}






















function render(): void {
  const root = document.querySelector<HTMLElement>("#app");
  if (!root || !snapshot) return;

  const focusedInput = document.activeElement;
  const preservedInputIds = new Set([
    "modal-alias-input",
    "modal-proxy-input",
    "modal-timezone-input",
    "modal-shortcut-input",
    "modal-group-name-input",
    "modal-group-context-input",
    "modal-skill-name-input",
    "modal-skill-desc-input",
    "modal-skill-content-input",
    "modal-adapters-json-input",
    "ext-agent-name-input",
    "ext-agent-url-input",
    "ext-agent-key-input",
    "ext-agent-model-input",
    "ext-agent-cmd-input",
    "ext-agent-args-input",
    "ext-agent-workdir-input",
    "group-chat-input",
  ]);
  const accountInputSelection =
    (focusedInput instanceof HTMLInputElement || focusedInput instanceof HTMLTextAreaElement) &&
    preservedInputIds.has(focusedInput.id)
      ? {
          id: focusedInput.id,
          start: focusedInput.selectionStart,
          end: focusedInput.selectionEnd,
          direction: focusedInput.selectionDirection,
        }
      : null;

  const provider = currentProvider(snapshot);
  const isMobile = isMobilePlatform(snapshot.platform);
  const isGroupsView = !isMobile && viewMode === "groups";
  const anyGroupNeedsAuth = snapshot.groupChats.some(groupHasNeedsAuth);

  if (isMobile) {
    root.innerHTML = `
      <div class="mobile-shell">
        <div class="mobile-tabs">
          ${snapshot.providers
            .map(
              (p) => `
              <button class="mobile-tab ${p.id === provider.id ? "active" : ""}" data-provider="${escapeHtml(p.id)}" type="button">
                ${renderProviderLogo(p, "mobile-tab-logo")}
                <span>${escapeHtml(p.shortName)}</span>
              </button>
            `
            )
            .join("")}
        </div>
        <div class="middle-column" style="width:100%;min-width:0;flex:1;border-right:none;">
          ${renderMiddleContent(snapshot, provider, modal, optimisticSwitching, downloadToast, shouldAnimateModal)}
        </div>
      </div>
    `;
  } else {
    root.innerHTML = `
      <div class="desktop-shell${snapshot.effectiveSidebarCollapsed ? " sidebar-collapsed" : ""}">
        <aside class="service-rail">
          <div class="rail-brand" title="叠号 · AI 助手账号切换器">
            <img src="/logos/app-logo.svg" alt="叠号" class="rail-brand-img" draggable="false" />
          </div>
          <div class="rail-divider"></div>
          <div class="rail-providers">
            <button id="group-chat-rail-btn" class="rail-btn ${isGroupsView ? "active" : ""}" title="多账号群聊（桌面端）" type="button">
              <div class="rail-logo-box rail-group-icon">${ICON_GROUP}${anyGroupNeedsAuth ? `<span class="warn-dot" aria-hidden="true"></span>` : ""}</div>
              <span class="rail-label">群聊</span>
            </button>
            <div class="rail-divider"></div>
            ${snapshot.providers
              .map(
                (p) => `
                <button class="rail-btn ${!isGroupsView && p.id === provider.id ? "active" : ""} ${!p.enabled ? "disabled" : ""}" data-provider="${escapeHtml(p.id)}" title="${escapeHtml(p.name)}" type="button">
                  <div class="rail-logo-box">${renderProviderLogo(p, "rail-logo-img")}</div>
                  <span class="rail-label">${escapeHtml(p.shortName)}</span>
                  ${!p.enabled ? `<span class="rail-pending-tag">待接入</span>` : ""}
                </button>
              `
              )
              .join("")}
          </div>
          <div class="rail-footer">
            <button
              id="toggle-sidebar-btn"
              class="rail-util-btn"
              title="${snapshot.effectiveSidebarCollapsed ? "展开账号栏 (Ctrl+\\)" : "收起账号栏 (Ctrl+\\)"}"
              type="button"
            >
              ${snapshot.effectiveSidebarCollapsed ? ICON_CHEVRON_RIGHT : ICON_CHEVRON_LEFT}
              <span class="rail-util-label">${snapshot.effectiveSidebarCollapsed ? "展开" : "收起"}</span>
              ${snapshot.effectiveSidebarCollapsed && (snapshot.notice || downloadToast) ? `<span class="warn-dot" aria-hidden="true"></span>` : ""}
            </button>
            <button
              id="settings-btn"
              class="rail-util-btn ${snapshot.globalShortcutError ? "has-warn" : ""}"
              title="${snapshot.globalShortcutError ? escapeHtml(snapshot.globalShortcutError) : "托盘常驻与全局快捷键设置"}"
              type="button"
            >
              ${ICON_SETTINGS}
              <span class="rail-util-label">设置</span>
              ${snapshot.globalShortcutError ? `<span class="warn-dot" aria-hidden="true"></span>` : ""}
            </button>
            <button
              id="theme-toggle-btn"
              class="rail-util-btn"
              title="外观主题：${themeLabel(themeMode)}（点击切换）"
              type="button"
            >
              ${themeIcon(themeMode)}
              <span class="rail-util-label">${themeLabel(themeMode)}</span>
            </button>
            <button
              id="check-update-btn"
              class="rail-util-btn ${updateState.result?.hasUpdate ? "has-update" : ""}"
              title="${updateState.result?.hasUpdate ? `发现新版本 v${escapeHtml(updateState.result.latestVersion)}（点击更新）` : "检查更新"}"
              type="button"
            >
              ${ICON_UPDATE}
              <span class="rail-util-label">${updateState.result?.hasUpdate ? "新版本" : "更新"}</span>
              ${updateState.result?.hasUpdate ? `<span class="update-dot" aria-hidden="true"></span>` : ""}
            </button>
          </div>
        </aside>
        <section class="middle-column">
          ${isGroupsView ? renderGroupMiddleContent(snapshot, provider, modal, downloadToast, shouldAnimateModal) : renderMiddleContent(snapshot, provider, modal, optimisticSwitching, downloadToast, shouldAnimateModal)}
        </section>
        ${isGroupsView ? renderGroupRightStage(snapshot, groupSending, newTopicConfirmGroupId) : renderRightStage(snapshot, provider, optimisticSwitching)}
      </div>
    `;
  }

  bindEvents(snapshot, provider);
  // 密码框不在 innerHTML 里写 value（防 Key 进序列化快照）：每次重绘后
  // 把草稿值写回输入框，否则 30 秒保活刷新/群聊流式重绘会把已输的 Key
  // 视觉清空（草稿本身还在，但用户以为丢了）。放在焦点恢复之前，
  // 避免写 value 把光标顶到末尾。
  if (modal.kind === "external-agents" && modal.formKind === "openai_api") {
    const keyInput = document.querySelector<HTMLInputElement>("#ext-agent-key-input");
    if (keyInput) keyInput.value = modal.apiKeyDraft;
  }
  if (accountInputSelection) {
    const input = document.querySelector<HTMLInputElement | HTMLTextAreaElement>(
      `#${accountInputSelection.id}`
    );
    input?.focus({ preventScroll: true });
    input?.setSelectionRange(
      accountInputSelection.start,
      accountInputSelection.end,
      accountInputSelection.direction ?? undefined
    );
  }
  shouldAnimateModal = false;
}

// 切换侧边栏折叠偏好（轻量命令，不走 update_app_settings）。
async function setSidebarCollapsed(collapsed: boolean): Promise<void> {
  if (!snapshot) return;
  try {
    snapshot = await invoke<ManagerSnapshot>("set_sidebar_collapsed", {
      collapsed,
    });
  } catch (err) {
    snapshot = { ...snapshot, notice: String(err) };
  }
  render();
}

function bindEvents(state: ManagerSnapshot, provider: ProviderInfo): void {
  document.querySelector("#group-chat-rail-btn")?.addEventListener("click", async () => {
    if (state.effectiveSidebarCollapsed && !state.activeGroupId) {
      await setSidebarCollapsed(false);
    }
    if (viewMode === "groups") return;
    if (modal.kind !== "none") {
      await setModal({ kind: "none" });
    }
    viewMode = "groups";
    const targetId = state.activeGroupId ?? state.groupChats[0]?.id ?? null;
    await callCommand("select_group_chat", { groupId: targetId });
  });

  document.querySelectorAll<HTMLElement>("[data-provider]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const nextId = btn.dataset.provider as ProviderId;
      if (state.effectiveSidebarCollapsed) {
        const alreadySelected =
          viewMode === "provider" && nextId === state.selectedProviderId;
        const hasActiveAccount = state.accounts.some(
          (a) => a.providerId === nextId && isSameKey(state.activeAccount, a),
        );
        if (alreadySelected || !hasActiveAccount) {
          await setSidebarCollapsed(false);
        }
      }
      if (!nextId || (viewMode === "provider" && nextId === state.selectedProviderId)) return;
      if (modal.kind !== "none") {
        await setModal({ kind: "none" });
      }
      viewMode = "provider";
      await callCommand("select_provider", { providerId: nextId });
    });
  });

  const handleOpenCreateGroup = async () => {
    await openCreateGroupModal(state, setModal);
  };
  document.querySelector("#create-group-btn")?.addEventListener("click", handleOpenCreateGroup);
  document.querySelector("#stage-create-group-btn")?.addEventListener("click", handleOpenCreateGroup);

  const handleOpenExternalAgents = async () => {
    await openExternalAgentsModal(undefined, setModal);
  };
  document
    .querySelector("#manage-external-agents-btn")
    ?.addEventListener("click", handleOpenExternalAgents);
  document
    .querySelector("#open-external-agents-from-group-btn")
    ?.addEventListener("click", handleOpenExternalAgents);

  document.querySelectorAll<HTMLElement>("[data-open-ext-agent-id]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (modal.kind !== "none") return;
      const eid = btn.dataset.openExtAgentId;
      const ext = (state.externalAgents ?? []).find((a) => a.id === eid);
      if (ext) {
        await openExternalAgentsModal(ext, setModal);
      }
    });
  });

  document.querySelectorAll<HTMLElement>("[data-edit-group-id]").forEach((btn) => {
    btn.addEventListener("click", async (ev) => {
      ev.stopPropagation();
      const gid = btn.dataset.editGroupId;
      const g = state.groupChats.find((x) => x.id === gid);
      if (g) {
        await openEditGroupModal(g, setModal);
      }
    });
  });

  document.querySelectorAll<HTMLElement>("[data-delete-group-id]").forEach((btn) => {
    btn.addEventListener("click", async (ev) => {
      ev.stopPropagation();
      const gid = btn.dataset.deleteGroupId;
      const g = state.groupChats.find((x) => x.id === gid);
      if (g) {
        await setModal({ kind: "delete-group", group: g });
      }
    });
  });

  document.querySelectorAll<HTMLElement>("[data-select-group-id]").forEach((row) => {
    row.addEventListener("click", async () => {
      if (modal.kind !== "none") return;
      const gid = row.dataset.selectGroupId;
      if (!gid || gid === state.activeGroupId) return;
      scrollState.forceGroupScrollBottom = true;
      await callCommand("select_group_chat", { groupId: gid });
    });
  });

  document.querySelector("#group-knowledge-btn")?.addEventListener("click", async () => {
    const activeGroup = state.activeGroupId
      ? state.groupChats.find((g) => g.id === state.activeGroupId)
      : undefined;
    if (activeGroup) {
      await openGroupKnowledgeModal(activeGroup, setModal);
    }
  });

  document.querySelectorAll<HTMLButtonElement>("[data-pin-msg-id]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (modal.kind !== "none" || !state.activeGroupId) return;
      const msgId = btn.dataset.pinMsgId;
      const activeGroup = state.groupChats.find((g) => g.id === state.activeGroupId);
      const msg = activeGroup?.messages.find((m) => m.id === msgId);
      if (!msg || !msg.content.trim()) return;
      const prefix =
        msg.role === "user"
          ? "[用户]"
          : `[${resolveGroupMessageSenderName(state, msg)}]`;
      await callCommand("append_group_shared_context", {
        groupId: state.activeGroupId,
        snippet: `${prefix}: ${msg.content.trim()}`,
      });
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-retry-msg-id]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (modal.kind !== "none" || !state.activeGroupId) return;
      const msgId = btn.dataset.retryMsgId;
      if (!msgId) return;
      await callCommand("retry_group_message", {
        groupId: state.activeGroupId,
        msgId,
      });
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-cancel-ext-msg-id]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (modal.kind !== "none" || !state.activeGroupId) return;
      const msgId = btn.dataset.cancelExtMsgId;
      if (!msgId) return;
      try {
        await invoke("cancel_external_member_turn", {
          groupId: state.activeGroupId,
          msgId,
        });
      } catch {
        // 该轮已结束或已取消时忽略。
      }
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-sync-msg-id]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (modal.kind !== "none" || !state.activeGroupId || busy) return;
      const msgId = btn.dataset.syncMsgId;
      if (!msgId || syncingMsgIds.has(msgId)) return;
      syncingMsgIds.add(msgId);
      rerenderPreservingScroll();
      try {
        await callCommand("sync_group_message_reply", {
          groupId: state.activeGroupId,
          msgId,
        });
      } finally {
        syncingMsgIds.delete(msgId);
        rerenderPreservingScroll();
      }
    });
  });

  document.querySelector("#group-edit-members-btn")?.addEventListener("click", async () => {
    const activeGroup = state.activeGroupId
      ? state.groupChats.find((g) => g.id === state.activeGroupId)
      : undefined;
    if (activeGroup) {
      await openEditGroupModal(activeGroup, setModal);
    }
  });

  document.querySelector("#group-clear-msgs-btn")?.addEventListener("click", async () => {
    if (!state.activeGroupId) return;
    await callCommand("clear_group_messages", { groupId: state.activeGroupId });
  });

  document.querySelector("#group-new-topic-btn")?.addEventListener("click", () => {
    if (modal.kind !== "none" || !state.activeGroupId) return;
    newTopicConfirmGroupId = state.activeGroupId;
    rerenderPreservingScroll();
  });

  document.querySelector("#group-new-topic-cancel-btn")?.addEventListener("click", () => {
    newTopicConfirmGroupId = null;
    rerenderPreservingScroll();
  });

  const startNewTopic = async (keepSharedContext: boolean) => {
    const gid = state.activeGroupId;
    newTopicConfirmGroupId = null;
    if (!gid) return;
    groupDrafts[gid] = "";
    groupTargets[gid] = [];
    groupAutoChannels[gid] = [];
    await callCommand("start_group_new_topic", { groupId: gid, keepSharedContext });
  };

  document.querySelector("#group-new-topic-keep-btn")?.addEventListener("click", () => {
    void startNewTopic(true);
  });

  document.querySelector("#group-new-topic-clean-btn")?.addEventListener("click", () => {
    void startNewTopic(false);
  });

  document.querySelector("#group-pin-round-btn")?.addEventListener("click", async () => {
    if (modal.kind !== "none" || !state.activeGroupId) return;
    const activeGroup = state.groupChats.find((g) => g.id === state.activeGroupId);
    if (!activeGroup) return;
    const round = getLatestRoundCompletedReplies(state, activeGroup);
    if (!round) return;
    const head = round.userText
      ? `已有结论（议题：${round.userText.replace(/\s+/g, " ").slice(0, 80)}）`
      : "已有结论";
    const lines = round.replies.map(
      (r, i) => `- 观点${i + 1}: ${r.text.replace(/\s+/g, " ").slice(0, 280)}`
    );
    const snippet = `${head}\n${lines.join("\n")}`.slice(0, 1800);
    await callCommand("append_group_shared_context", {
      groupId: state.activeGroupId,
      snippet,
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-workflow-mode]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (modal.kind !== "none" || !state.activeGroupId) return;
      const mode = btn.dataset.workflowMode as GroupWorkflowMode | undefined;
      const activeGroup = state.groupChats.find((g) => g.id === state.activeGroupId);
      if (!mode || !activeGroup || (activeGroup.workflowMode ?? "parallel") === mode) return;
      await callCommand("update_group_workflow", {
        groupId: activeGroup.id,
        workflowMode: mode,
        maxRelayHops: activeGroup.maxRelayHops ?? 2,
      });
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-relay-hops]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (modal.kind !== "none" || !state.activeGroupId) return;
      const hops = Number(btn.dataset.relayHops ?? "2");
      const activeGroup = state.groupChats.find((g) => g.id === state.activeGroupId);
      if (!activeGroup || !Number.isFinite(hops) || (activeGroup.maxRelayHops ?? 2) === hops) {
        return;
      }
      await callCommand("update_group_workflow", {
        groupId: activeGroup.id,
        workflowMode: activeGroup.workflowMode ?? "a2a_relay",
        maxRelayHops: hops,
      });
    });
  });

  document.querySelectorAll<HTMLElement>("[data-jump-provider]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (modal.kind !== "none") return;
      const pid = btn.dataset.jumpProvider;
      const uuid = btn.dataset.jumpUuid;
      if (!pid || !uuid) return;
      viewMode = "provider";
      const alreadyWarm = state.warmAccounts.some(
        (k) => k.providerId === pid && k.uuid === uuid
      );
      await callCommand(
        "switch_account",
        { providerId: pid, uuid },
        alreadyWarm ? null : { providerId: pid, uuid }
      );
    });
  });

  document.querySelector('[data-group-target="all"]')?.addEventListener("click", () => {
    if (!state.activeGroupId) return;
    groupTargets[state.activeGroupId] = [];
    groupAutoChannels[state.activeGroupId] = [];
    rerenderPreservingScroll();
  });

  document.querySelectorAll<HTMLElement>("[data-group-target-auto]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const gid = state.activeGroupId;
      const catId = btn.dataset.groupTargetAuto;
      if (!gid || !catId) return;
      const currentAuto = [...(groupAutoChannels[gid] ?? [])];
      const idx = currentAuto.indexOf(catId);
      if (idx >= 0) {
        currentAuto.splice(idx, 1);
      } else {
        currentAuto.push(catId);
        // Same-channel mutual exclusion: enabling channel auto clears any @account in that channel
        groupTargets[gid] = (groupTargets[gid] ?? []).filter(
          (t) => resolveMemberCategory(state, t).id !== catId
        );
      }
      groupAutoChannels[gid] = currentAuto;
      rerenderPreservingScroll();
    });
  });

  document.querySelectorAll<HTMLElement>("[data-group-target-cat]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const gid = state.activeGroupId;
      const catId = btn.dataset.groupTargetCat;
      if (!gid || !catId) return;
      const activeGroup = state.groupChats.find((g) => g.id === gid);
      if (!activeGroup) return;
      const catMembers = activeGroup.members.filter((m) => {
        const meta = resolveMemberMeta(state, m);
        return !meta.loggedOut && resolveMemberCategory(state, m).id === catId;
      });
      if (catMembers.length === 0) return;
      const current = [...(groupTargets[gid] ?? [])];
      const allSelected = catMembers.every((cm) =>
        current.some((cur) => sameAccountKey(cur, cm))
      );
      if (allSelected) {
        groupTargets[gid] = current.filter(
          (cur) => !catMembers.some((cm) => sameAccountKey(cur, cm))
        );
      } else {
        for (const cm of catMembers) {
          if (!current.some((cur) => sameAccountKey(cur, cm))) {
            current.push({ providerId: cm.providerId, uuid: cm.uuid });
          }
        }
        groupTargets[gid] = current;
        groupAutoChannels[gid] = (groupAutoChannels[gid] ?? []).filter((c) => c !== catId);
      }
      rerenderPreservingScroll();
    });
  });

  document.querySelectorAll<HTMLElement>("[data-group-target-provider]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const gid = state.activeGroupId;
      const pid = btn.dataset.groupTargetProvider;
      const uuid = btn.dataset.groupTargetUuid;
      if (!gid || !pid || !uuid) return;
      const current = [...(groupTargets[gid] ?? [])];
      const idx = current.findIndex((k) => k.providerId === pid && k.uuid === uuid);
      if (idx >= 0) {
        current.splice(idx, 1);
      } else {
        current.push({ providerId: pid, uuid });
        const catId = resolveMemberCategory(state, { providerId: pid, uuid }).id;
        groupAutoChannels[gid] = (groupAutoChannels[gid] ?? []).filter((c) => c !== catId);
      }
      groupTargets[gid] = current;
      rerenderPreservingScroll();
    });
  });

  const submitGroupMessage = async () => {
    const gid = state.activeGroupId;
    if (!gid || groupSending) return;
    const raw = (groupDrafts[gid] ?? "").trim();
    if (!raw) return;
    const group = state.groupChats.find((g) => g.id === gid);
    const chosen = (groupTargets[gid] ?? []).filter((t) =>
      group?.members.some((m) => sameAccountKey(m, t))
    );
    const membersForMention = (group?.members ?? []).map((m) => ({
      key: m,
      alias: resolveMemberMeta(state, m).alias,
    }));
    const inline = extractComposerAtMentions(raw, membersForMention);
    let chosenAutoChannels: string[] = [];
    if (!inline.isAll) {
      const explicitForConflict = inline.hasMention ? inline.targets : chosen;
      const conflictCats = new Set(
        explicitForConflict.map((t) => resolveMemberCategory(state, t).id)
      );
      chosenAutoChannels = (groupAutoChannels[gid] ?? []).filter(
        (catId) =>
          !conflictCats.has(catId) &&
          (group?.members ?? []).some((m) => {
            const meta = resolveMemberMeta(state, m);
            return !meta.loggedOut && resolveMemberCategory(state, m).id === catId;
          })
      );
      groupAutoChannels[gid] = chosenAutoChannels;
    }
    groupSending = true;
    scrollState.forceGroupScrollBottom = true;
    groupDrafts[gid] = "";
    render();
    restoreScrollPosition(snapshot, viewMode, modal);
    let ok = false;
    try {
      ok = await callCommand("send_group_message", {
        groupId: gid,
        content: raw,
        targetMembers: chosen.length > 0 ? chosen : null,
        autoChannels: chosenAutoChannels.length > 0 ? chosenAutoChannels : null,
      });
    } finally {
      groupSending = false;
      if (!ok && !groupDrafts[gid]) {
        groupDrafts[gid] = raw;
      }
      render();
      restoreScrollPosition(snapshot, viewMode, modal);
    }
  };

  const groupInputEl = document.querySelector<HTMLTextAreaElement>("#group-chat-input");
  if (groupInputEl && state.activeGroupId) {
    const activeGid = state.activeGroupId;
    const activeGroup = state.groupChats.find((g) => g.id === activeGid);
    const mentionMenuEl = document.querySelector<HTMLElement>("#group-mention-menu");
    const targetHintEl = document.querySelector<HTMLElement>("#group-target-hint");

    type MentionCandidate = { label: string; sub: string; insert: string; groupLabel?: string };
    const composerMembers = (activeGroup?.members ?? []).map((m) => ({
      key: m,
      alias: resolveMemberMeta(state, m).alias,
      categoryId: resolveMemberCategory(state, m).id,
    }));
    const activeWorkflowMode: GroupWorkflowMode = activeGroup?.workflowMode ?? "parallel";
    const activeRelayHops = activeGroup?.maxRelayHops ?? 2;
    const composerCategories = activeGroup
      ? groupByMemberCategory(
          state,
          activeGroup.members
            .filter((m) => !resolveMemberMeta(state, m).loggedOut)
            .map((m) => ({ key: m }))
        )
      : [];
    const mentionCandidates: MentionCandidate[] = activeGroup
      ? [
          {
            label: "全体成员",
            sub: "向全部有效成员转发",
            insert: "全体成员",
          },
          ...groupByMemberCategory(
            state,
            activeGroup.members.map((m) => ({ key: m }))
          ).flatMap((cat) =>
            cat.items.map(({ key: m }) => {
              const meta = resolveMemberMeta(state, m);
              const roleText = (activeGroup.memberRoles ?? []).find((r) =>
                sameAccountKey(r.member, m)
              )?.role;
              return {
                label: meta.alias,
                sub: meta.loggedOut
                  ? `${cat.label} · 未登录`
                  : meta.pageError
                    ? `${cat.label} · 状态异常`
                    : roleText
                      ? `${cat.label} · ${roleText}`
                      : cat.label,
                insert: meta.alias,
                groupLabel: cat.label,
              };
            })
          ),
        ]
      : [];

    let filteredMentions: MentionCandidate[] = [];
    let activeMentionStart = -1;
    let activeMentionIdx = 0;

    const findActiveAtQuery = (
      val: string,
      cursor: number
    ): { start: number; query: string } | null => {
      const before = val.slice(0, cursor);
      for (let i = before.length - 1; i >= 0; i -= 1) {
        const ch = before[i];
        if (/\s/.test(ch)) return null;
        if (ch === "@") {
          if (i > 0 && isAsciiWordChar(before[i - 1])) return null;
          return { start: i, query: before.slice(i + 1) };
        }
      }
      return null;
    };

    const syncInlineMentionAutoExclusion = () => {
      if (!activeGroup) return;
      const inline = extractComposerAtMentions(groupInputEl.value, composerMembers);
      if (!inline.hasMention) return;
      const prevAuto = groupAutoChannels[activeGid] ?? [];
      if (prevAuto.length === 0) return;

      let nextAuto = prevAuto;
      if (inline.isAll) {
        nextAuto = [];
      } else {
        const conflictCats = new Set(
          inline.targets.map((t) => resolveMemberCategory(state, t).id)
        );
        nextAuto = prevAuto.filter((catId) => !conflictCats.has(catId));
      }

      if (nextAuto.length !== prevAuto.length) {
        groupAutoChannels[activeGid] = nextAuto;
        const removed = prevAuto.filter((c) => !nextAuto.includes(c));
        for (const catId of removed) {
          document
            .querySelector(`[data-group-target-auto="${CSS.escape(catId)}"]`)
            ?.classList.remove("active");
          const catInfo = composerCategories.find((c) => c.id === catId);
          const allSelected = Boolean(
            catInfo &&
              catInfo.items.length > 0 &&
              catInfo.items.every((m) =>
                (groupTargets[activeGid] ?? []).some((t) => sameAccountKey(t, m.key))
              )
          );
          if (!allSelected) {
            document
              .querySelector(`[data-group-target-cat="${CSS.escape(catId)}"]`)
              ?.classList.remove("active");
          }
        }
        const validSelected = (groupTargets[activeGid] ?? []).filter((t) =>
          activeGroup.members.some((m) => sameAccountKey(m, t))
        );
        if (validSelected.length === 0 && nextAuto.length === 0) {
          document.querySelector('[data-group-target="all"]')?.classList.add("active");
        }
      }
    };

    const updateComposerHint = () => {
      if (!targetHintEl || !activeGroup) return;
      const validSelected = (groupTargets[activeGid] ?? []).filter((t) =>
        activeGroup.members.some((m) => sameAccountKey(m, t))
      );
      const validAutoLabels = (groupAutoChannels[activeGid] ?? [])
        .map((id) => {
          const cat = composerCategories.find((c) => c.id === id);
          return cat ? { id: cat.id, label: cat.label } : null;
        })
        .filter((x): x is { id: string; label: string } => Boolean(x));
      targetHintEl.textContent = formatComposerHintText(
        groupInputEl.value,
        validSelected,
        composerMembers,
        activeWorkflowMode,
        activeRelayHops,
        validAutoLabels
      );
    };

    const hideMentionMenu = () => {
      filteredMentions = [];
      activeMentionStart = -1;
      activeMentionIdx = 0;
      if (mentionMenuEl) {
        mentionMenuEl.hidden = true;
        mentionMenuEl.innerHTML = "";
      }
    };

    const applyMentionCandidate = (idx: number) => {
      const picked = filteredMentions[idx];
      if (!picked || activeMentionStart < 0) return;
      const cursor = groupInputEl.selectionStart ?? groupInputEl.value.length;
      const val = groupInputEl.value;
      const inserted = `@${picked.insert} `;
      const nextVal = val.slice(0, activeMentionStart) + inserted + val.slice(cursor);
      groupInputEl.value = nextVal;
      groupDrafts[activeGid] = nextVal;
      const nextCursor = activeMentionStart + inserted.length;
      groupInputEl.setSelectionRange(nextCursor, nextCursor);
      hideMentionMenu();
      syncInlineMentionAutoExclusion();
      updateComposerHint();
    };

    const renderMentionMenu = () => {
      if (!mentionMenuEl || filteredMentions.length === 0) {
        hideMentionMenu();
        return;
      }
      if (activeMentionIdx >= filteredMentions.length) activeMentionIdx = 0;
      if (activeMentionIdx < 0) activeMentionIdx = filteredMentions.length - 1;
      let prevGroup = "";
      mentionMenuEl.innerHTML = filteredMentions
        .map((c, i) => {
          let headerHtml = "";
          if (c.groupLabel && c.groupLabel !== prevGroup) {
            prevGroup = c.groupLabel;
            headerHtml = `<div class="group-mention-group-title">${escapeHtml(c.groupLabel)}</div>`;
          }
          return `
            ${headerHtml}
            <button
              class="group-mention-item ${i === activeMentionIdx ? "active" : ""}"
              data-mention-idx="${i}"
              type="button"
            >
              <span class="group-mention-item-label">@${escapeHtml(c.label)}</span>
              <span class="group-mention-item-sub">${escapeHtml(c.sub)}</span>
            </button>
          `;
        })
        .join("");
      mentionMenuEl.hidden = false;
      mentionMenuEl.querySelectorAll<HTMLButtonElement>("[data-mention-idx]").forEach((btn) => {
        btn.addEventListener("mousedown", (ev) => {
          ev.preventDefault();
          const idx = Number(btn.dataset.mentionIdx ?? "-1");
          if (idx >= 0) applyMentionCandidate(idx);
        });
      });
      mentionMenuEl
        .querySelector<HTMLElement>(".group-mention-item.active")
        ?.scrollIntoView({ block: "nearest" });
    };

    const syncMentionMenu = () => {
      if (!mentionMenuEl || mentionCandidates.length === 0) {
        hideMentionMenu();
        return;
      }
      const cursor = groupInputEl.selectionStart ?? groupInputEl.value.length;
      const activeAt = findActiveAtQuery(groupInputEl.value, cursor);
      if (!activeAt) {
        hideMentionMenu();
        return;
      }
      const q = activeAt.query.trim().toLowerCase();
      const matched = mentionCandidates.filter(
        (c) =>
          !q ||
          c.label.toLowerCase().includes(q) ||
          c.sub.toLowerCase().includes(q)
      );
      if (matched.length === 0) {
        hideMentionMenu();
        return;
      }
      activeMentionStart = activeAt.start;
      filteredMentions = matched;
      activeMentionIdx = 0;
      renderMentionMenu();
    };

    groupInputEl.addEventListener("input", () => {
      groupDrafts[activeGid] = groupInputEl.value;
      syncInlineMentionAutoExclusion();
      updateComposerHint();
      syncMentionMenu();
    });
    groupInputEl.addEventListener("click", () => {
      syncMentionMenu();
    });
    groupInputEl.addEventListener("blur", () => {
      window.setTimeout(hideMentionMenu, 120);
    });
    groupInputEl.addEventListener("keydown", (ev) => {
      if (mentionMenuEl && !mentionMenuEl.hidden && filteredMentions.length > 0) {
        if (ev.key === "ArrowDown") {
          ev.preventDefault();
          activeMentionIdx = (activeMentionIdx + 1) % filteredMentions.length;
          renderMentionMenu();
          return;
        }
        if (ev.key === "ArrowUp") {
          ev.preventDefault();
          activeMentionIdx =
            (activeMentionIdx - 1 + filteredMentions.length) % filteredMentions.length;
          renderMentionMenu();
          return;
        }
        if ((ev.key === "Enter" && !ev.isComposing) || ev.key === "Tab") {
          ev.preventDefault();
          applyMentionCandidate(activeMentionIdx);
          return;
        }
        if (ev.key === "Escape") {
          ev.preventDefault();
          ev.stopPropagation();
          hideMentionMenu();
          return;
        }
      }
      if (ev.key === "Enter" && !ev.isComposing && (!ev.shiftKey || ev.metaKey || ev.ctrlKey)) {
        ev.preventDefault();
        void submitGroupMessage();
      }
    });
  }

  document.querySelector("#group-send-btn")?.addEventListener("click", () => {
    void submitGroupMessage();
  });

  document.querySelector("#toggle-sidebar-btn")?.addEventListener("click", async () => {
    await setSidebarCollapsed(!state.effectiveSidebarCollapsed);
  });

  document.querySelector("#theme-toggle-btn")?.addEventListener("click", () => {
    cycleThemeMode(rerenderPreservingScroll);
  });

  document.querySelector("#settings-btn")?.addEventListener("click", async () => {
    if (modal.kind !== "none") return;
    await setModal({
      kind: "settings",
      minimizeToTray: state.settings.minimizeToTray,
      globalShortcut: state.settings.globalShortcut,
      autoApprove: Boolean(state.settings.autoApprove),
      disableAutoUpdateCheck: Boolean(state.settings.disableAutoUpdateCheck),
      autoApproveCommands: Boolean(state.settings.autoApproveCommands),
    });
  });

  document.querySelector("#smart-switch-btn")?.addEventListener("click", async () => {
    if (modal.kind !== "none" || state.conflicts.length > 0) return;
    const candidates = state.accounts.filter((a) => a.providerId === provider.id);
    if (candidates.length === 0) return;
    const ranked = [...candidates]
      .filter((a) => scoreAccountAvailability(a, findAccountPageError(state, a)).available)
      .sort((a, b) => compareAccountAvailability(a, b, state));
    const best = ranked[0];
    if (!best) {
      if (snapshot) {
        snapshot = { ...snapshot, notice: "暂无可用在线账号，请先登录或等待同步" };
        rerenderPreservingScroll();
      }
      return;
    }
    if (isSameKey(state.activeAccount, best)) {
      if (snapshot) {
        snapshot = {
          ...snapshot,
          notice: "当前账号已是可用余量最多的账号",
        };
        rerenderPreservingScroll();
      }
      return;
    }
    const alreadyWarm = state.warmAccounts.some(
      (k) => k.providerId === best.providerId && k.uuid === best.uuid
    );
    await callCommand(
      "switch_account",
      { providerId: best.providerId, uuid: best.uuid },
      alreadyWarm ? null : { providerId: best.providerId, uuid: best.uuid }
    );
  });

  document.querySelector("#sort-quota-btn")?.addEventListener("click", async () => {
    if (modal.kind !== "none" || state.conflicts.length > 0) return;
    const candidates = state.accounts.filter((a) => a.providerId === provider.id);
    if (candidates.length <= 1) return;
    const sorted = [...candidates].sort((a, b) => compareAccountAvailability(a, b, state));
    const orderedUuids = sorted.map((a) => a.uuid);
    const changed = orderedUuids.some((u, idx) => u !== candidates[idx].uuid);
    if (!changed) {
      if (snapshot) {
        snapshot = {
          ...snapshot,
          notice: "当前已是按可用余量降序排列",
        };
        rerenderPreservingScroll();
      }
      return;
    }
    await callCommand("reorder_accounts", {
      providerId: provider.id,
      orderedUuids,
    });
  });

  const openOfficial = async () => {
    await callCommand("open_official_site", { providerId: provider.id });
  };
  document.querySelector("#open-site-btn")?.addEventListener("click", openOfficial);
  document.querySelector("#stage-open-site-btn")?.addEventListener("click", openOfficial);

  document.querySelector("#sync-all-btn")?.addEventListener("click", async () => {
    const btn = document.querySelector("#sync-all-btn");
    btn?.classList.add("spinning");
    try {
      await callCommand("trigger_quota_sync");
    } finally {
      setTimeout(() => btn?.classList.remove("spinning"), 650);
    }
  });

  const openAddModal = async () => {
    await setModal({ kind: "add", providerId: provider.id, draft: "", proxyDraft: "", timezoneDraft: "", cookieDraft: "" });
  };
  document.querySelector("#add-account-btn")?.addEventListener("click", openAddModal);
  document.querySelector("#stage-add-btn")?.addEventListener("click", openAddModal);

  document.querySelector("#dismiss-notice-btn")?.addEventListener("click", async () => {
    await callCommand("dismiss_notice");
  });

  document.querySelector("#continue-empty-config-btn")?.addEventListener("click", async () => {
    if (
      !window.confirm(
        "继续使用空配置将丢弃当前损坏的配置（已有 .corrupt-* 备份），确定吗？",
      )
    ) {
      return;
    }
    await callCommand("continue_with_empty_config");
  });
  document.querySelector("#restore-config-backup-btn")?.addEventListener("click", async () => {
    await callCommand("restore_config_from_backup");
  });

  const toastEl = document.querySelector("#download-toast");
  if (toastEl) {
    toastEl.addEventListener("mouseenter", () => {
      clearDownloadToastTimer();
    });
    toastEl.addEventListener("mouseleave", () => {
      scheduleDownloadToastDismiss();
    });
  }
  document.querySelector<HTMLButtonElement>("#download-reveal-btn")?.addEventListener("click", async (ev) => {
    const rawId = (ev.currentTarget as HTMLButtonElement).dataset.downloadId;
    const downloadId = rawId ? Number(rawId) : NaN;
    if (!Number.isFinite(downloadId)) return;
    try {
      await invoke<void>("reveal_downloaded_file", { downloadId });
    } catch (err) {
      if (snapshot) {
        snapshot = { ...snapshot, notice: String(err) };
        rerenderPreservingScroll();
      }
    }
  });
  document.querySelector("#download-close-btn")?.addEventListener("click", () => {
    clearDownloadToastTimer();
    downloadToast = null;
    rerenderPreservingScroll();
  });

  document.querySelector("#check-update-btn")?.addEventListener("click", async () => {
    if (modal.kind !== "none") return;
    await setModal({ kind: "update" });
    const stale =
      !updateState.result ||
      (!updateState.result.hasUpdate && Date.now() - updateState.lastCheckedAt > 5 * 60 * 1000);
    if (stale && !updateState.checking && !updateState.installing) {
      await runCheckForUpdates(false);
    }
  });

  document.querySelector("#update-recheck-btn")?.addEventListener("click", async () => {
    await runCheckForUpdates(false);
  });

  document.querySelector("#update-install-btn")?.addEventListener("click", async () => {
    await runDownloadAndInstall();
  });

  document.querySelector("#update-open-page-btn")?.addEventListener("click", async () => {
    const pageUrl = updateState.result?.releasePageUrl;
    if (!pageUrl) return;
    try {
      await invoke("open_external_url", { url: pageUrl });
    } catch (err) {
      updateState.error = String(err);
      rerenderPreservingScroll();
    }
  });

  document.querySelectorAll<HTMLElement>("[data-resolve-uuid]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const uuid = btn.dataset.resolveUuid;
      const keepProviderId = btn.dataset.resolveProvider;
      if (uuid && keepProviderId) {
        await callCommand("resolve_uuid_conflict", { keepProviderId, uuid });
      }
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-copy-cookie-uuid]").forEach((button) => {
    let feedbackTimer: number | undefined;
    button.addEventListener("click", async (event) => {
      event.stopPropagation();
      const uuid = button.dataset.copyCookieUuid;
      if (!uuid) return;
      const copyKey = `${provider.id}:${uuid}`;
      if (copyingCookies.has(copyKey)) return;
      copyingCookies.add(copyKey);
      window.clearTimeout(feedbackTimer);
      button.classList.remove("copied");
      button.dataset.copyLabel = "复制中…";
      try {
        await invoke<void>("copy_account_cookie", { providerId: provider.id, uuid });
        button.dataset.copyLabel = "已复制 Cookie";
        button.classList.add("copied");
        feedbackTimer = window.setTimeout(() => {
          button.classList.remove("copied");
          button.dataset.copyLabel = "复制 Cookie";
        }, 1800);
      } catch (err) {
        button.dataset.copyLabel = "复制 Cookie";
        if (snapshot?.selectedProviderId === provider.id) {
          snapshot = { ...snapshot, notice: String(err) };
          rerenderPreservingScroll();
        }
      } finally {
        copyingCookies.delete(copyKey);
      }
    });
  });

  document.querySelectorAll<HTMLElement>("[data-reload-uuid]").forEach((btn) => {
    btn.addEventListener("click", async (ev) => {
      ev.stopPropagation();
      const uuid = btn.dataset.reloadUuid;
      if (!uuid) return;
      const isActive =
        snapshot?.activeAccount?.providerId === provider.id &&
        snapshot?.activeAccount?.uuid === uuid;
      const optimistic = isActive ? { providerId: provider.id, uuid } : null;
      await callCommand("reload_account", { providerId: provider.id, uuid }, optimistic);
    });
  });

  document.querySelectorAll<HTMLElement>("[data-relogin-uuid]").forEach((btn) => {
    btn.addEventListener("click", async (ev) => {
      ev.stopPropagation();
      const uuid = btn.dataset.reloginUuid;
      if (!uuid) return;
      await callCommand(
        "clear_account_data",
        { providerId: provider.id, uuid },
        { providerId: provider.id, uuid }
      );
    });
  });

  document.querySelectorAll<HTMLElement>("[data-rename-uuid]").forEach((btn) => {
    btn.addEventListener("click", async (ev) => {
      ev.stopPropagation();
      const uuid = btn.dataset.renameUuid;
      const acc = state.accounts.find((a) => a.providerId === provider.id && a.uuid === uuid);
      if (acc) {
        await setModal({
          kind: "rename",
          account: acc,
          draft: acc.autoAlias ? "" : acc.alias,
          proxyDraft: acc.proxyUrl ?? "",
          timezoneDraft: acc.timezone ?? "",
          cookieDraft: "",
        });
      }
    });
  });

  document.querySelectorAll<HTMLElement>("[data-remove-uuid]").forEach((btn) => {
    btn.addEventListener("click", async (ev) => {
      ev.stopPropagation();
      const uuid = btn.dataset.removeUuid;
      const acc = state.accounts.find((a) => a.providerId === provider.id && a.uuid === uuid);
      if (acc) {
        await setModal({ kind: "remove", account: acc });
      }
    });
  });

  document.querySelectorAll<HTMLElement>(".row-ops").forEach((ops) => {
    ops.addEventListener("click", (ev) => {
      ev.stopPropagation();
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-move-uuid]").forEach((btn) => {
    btn.addEventListener("click", async (ev) => {
      ev.stopPropagation();
      if (modal.kind !== "none" || state.conflicts.length > 0) return;
      const uuid = btn.dataset.moveUuid;
      const dir = btn.dataset.moveDir;
      if (!uuid || (dir !== "up" && dir !== "down")) return;
      const ordered = state.accounts
        .filter((a) => a.providerId === provider.id)
        .map((a) => a.uuid);
      const idx = ordered.indexOf(uuid);
      if (idx === -1) return;
      const targetIdx = dir === "up" ? idx - 1 : idx + 1;
      if (targetIdx < 0 || targetIdx >= ordered.length) return;
      [ordered[idx], ordered[targetIdx]] = [ordered[targetIdx], ordered[idx]];
      await callCommand("reorder_accounts", {
        providerId: provider.id,
        orderedUuids: ordered,
      });
    });
  });

  document.querySelectorAll<HTMLElement>("[data-switch-uuid]").forEach((row) => {
    row.addEventListener("dragstart", (ev) => {
      if (isMobilePlatform(state.platform) || modal.kind !== "none" || state.conflicts.length > 0 || busy) {
        ev.preventDefault();
        return;
      }
      const uuid = row.dataset.switchUuid;
      if (!uuid) {
        ev.preventDefault();
        return;
      }
      draggingAccountUuid = uuid;
      if (ev.dataTransfer) {
        ev.dataTransfer.effectAllowed = "move";
        ev.dataTransfer.setData("text/plain", uuid);
      }
      requestAnimationFrame(() => row.classList.add("dragging"));
    });

    row.addEventListener("dragover", (ev) => {
      if (!draggingAccountUuid) return;
      const targetUuid = row.dataset.switchUuid;
      if (!targetUuid || targetUuid === draggingAccountUuid) return;
      ev.preventDefault();
      if (ev.dataTransfer) {
        ev.dataTransfer.dropEffect = "move";
      }
      const rect = row.getBoundingClientRect();
      const insertBefore = ev.clientY < rect.top + rect.height / 2;
      row.classList.toggle("drag-over-top", insertBefore);
      row.classList.toggle("drag-over-bottom", !insertBefore);
    });

    row.addEventListener("dragleave", (ev) => {
      if (row.contains(ev.relatedTarget as Node | null)) return;
      row.classList.remove("drag-over-top", "drag-over-bottom");
    });

    row.addEventListener("drop", async (ev) => {
      if (!draggingAccountUuid) return;
      ev.preventDefault();
      ev.stopPropagation();
      const srcUuid = draggingAccountUuid;
      const targetUuid = row.dataset.switchUuid;
      const insertBefore = row.classList.contains("drag-over-top");
      row.classList.remove("drag-over-top", "drag-over-bottom");
      if (!targetUuid || targetUuid === srcUuid) return;
      const ordered = state.accounts
        .filter((a) => a.providerId === provider.id)
        .map((a) => a.uuid);
      const fromIdx = ordered.indexOf(srcUuid);
      if (fromIdx === -1) return;
      ordered.splice(fromIdx, 1);
      const toIdx = ordered.indexOf(targetUuid);
      if (toIdx === -1) return;
      ordered.splice(insertBefore ? toIdx : toIdx + 1, 0, srcUuid);
      draggingAccountUuid = null;
      await callCommand("reorder_accounts", {
        providerId: provider.id,
        orderedUuids: ordered,
      });
    });

    row.addEventListener("dragend", () => {
      draggingAccountUuid = null;
      document
        .querySelectorAll<HTMLElement>(".account-row.dragging, .account-row.drag-over-top, .account-row.drag-over-bottom")
        .forEach((el) => {
          el.classList.remove("dragging", "drag-over-top", "drag-over-bottom");
        });
    });

    row.addEventListener("click", async () => {
      if (modal.kind !== "none" || state.conflicts.length > 0) return;
      const uuid = row.dataset.switchUuid;
      if (!uuid) return;
      if (
        state.activeAccount?.providerId === provider.id &&
        state.activeAccount?.uuid === uuid &&
        !isMobilePlatform(state.platform)
      ) {
        return;
      }
      const alreadyWarm = state.warmAccounts.some(
        (k) => k.providerId === provider.id && k.uuid === uuid
      );
      await callCommand(
        "switch_account",
        { providerId: provider.id, uuid },
        alreadyWarm ? null : { providerId: provider.id, uuid }
      );
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-shortcut-preset]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (modal.kind !== "settings" || modal.saving) return;
      modal.globalShortcut = btn.dataset.shortcutPreset ?? "";
      modal.error = undefined;
      render();
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-timezone-preset]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if ((modal.kind !== "add" && modal.kind !== "rename") || modal.saving) return;
      modal.timezoneDraft = btn.dataset.timezonePreset ?? "";
      modal.error = undefined;
      render();
    });
  });

  document.querySelector("#proxy-test-btn")?.addEventListener("click", async () => {
    const currentModal = modal;
    if (currentModal.kind !== "add" && currentModal.kind !== "rename") return;
    if (currentModal.saving || currentModal.proxyTesting) return;
    const pid =
      currentModal.kind === "add" ? currentModal.providerId : currentModal.account.providerId;
    currentModal.proxyTesting = true;
    currentModal.proxyTestResult = null;
    currentModal.error = undefined;
    render();
    try {
      const res = await invoke<ProxyTestResult>("test_account_proxy", {
        providerId: pid,
        proxyUrl: currentModal.proxyDraft.trim(),
      });
      if (modal === currentModal) {
        currentModal.proxyTestResult = res;
      }
    } catch (err) {
      if (modal === currentModal) {
        currentModal.error = String(err);
      }
    } finally {
      currentModal.proxyTesting = false;
      if (modal === currentModal) {
        render();
      }
    }
  });

  document.querySelector("#modal-clear-data-btn")?.addEventListener("click", async () => {
    const currentModal = modal;
    if (currentModal.kind !== "rename" || currentModal.saving) return;
    if (!currentModal.confirmClear) {
      currentModal.confirmClear = true;
      render();
      return;
    }
    currentModal.error = undefined;
    currentModal.saving = true;
    render();
    let cleared = false;
    try {
      cleared = await callCommand("clear_account_data", {
        providerId: currentModal.account.providerId,
        uuid: currentModal.account.uuid,
      });
    } finally {
      currentModal.saving = false;
      if (modal === currentModal && !cleared) {
        currentModal.confirmClear = false;
        render();
      }
    }
    if (cleared && modal === currentModal) {
      await setModal({ kind: "none" });
    }
  });

  document.querySelector<HTMLInputElement>("#modal-tray-checkbox")?.addEventListener("change", (ev) => {
    if (modal.kind === "settings") {
      modal.minimizeToTray = (ev.currentTarget as HTMLInputElement).checked;
    }
  });

  document.querySelector<HTMLInputElement>("#modal-auto-approve-checkbox")?.addEventListener("change", (ev) => {
    if (modal.kind === "settings") {
      modal.autoApprove = (ev.currentTarget as HTMLInputElement).checked;
    }
  });

  document.querySelector<HTMLInputElement>("#modal-disable-auto-update-checkbox")?.addEventListener("change", (ev) => {
    if (modal.kind === "settings") {
      modal.disableAutoUpdateCheck = (ev.currentTarget as HTMLInputElement).checked;
    }
  });

  document.querySelector<HTMLInputElement>("#modal-auto-approve-commands-checkbox")?.addEventListener("change", (ev) => {
    if (modal.kind === "settings") {
      modal.autoApproveCommands = (ev.currentTarget as HTMLInputElement).checked;
    }
  });

  document.querySelector<HTMLTextAreaElement>("#modal-cookie-import-input")?.addEventListener("input", (ev) => {
    if (modal.kind === "add" || modal.kind === "rename") {
      modal.cookieDraft = (ev.currentTarget as HTMLTextAreaElement).value;
      if (modal.error) {
        modal.error = undefined;
        document.querySelector("#modal-error-box")?.remove();
      }
    }
  });

  document.querySelector("#modal-import-cookie-btn")?.addEventListener("click", async () => {
    const currentModal = modal;
    if (currentModal.kind !== "rename" || currentModal.saving || currentModal.cookieImporting) return;
    const raw = (currentModal.cookieDraft ?? "").trim();
    if (!raw) {
      currentModal.error = "请先粘贴要导入的 Cookie 内容";
      render();
      return;
    }
    currentModal.error = undefined;
    currentModal.saving = true;
    currentModal.cookieImporting = true;
    render();
    let imported = false;
    try {
      imported = await callCommand("import_account_cookies", {
        providerId: currentModal.account.providerId,
        uuid: currentModal.account.uuid,
        rawCookies: raw,
      });
    } finally {
      currentModal.saving = false;
      currentModal.cookieImporting = false;
      if (modal === currentModal && !imported) {
        render();
      }
    }
    if (imported && modal === currentModal) {
      await setModal({ kind: "none" });
    }
  });

  document.querySelectorAll<HTMLButtonElement>("[data-toggle-group-member]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (modal.kind !== "group" || modal.saving) return;
      const pid = btn.dataset.memberProvider;
      const uuid = btn.dataset.memberUuid;
      if (!pid || !uuid) return;
      const idx = modal.selectedMembers.findIndex(
        (m) => m.providerId === pid && m.uuid === uuid
      );
      let nowChecked = false;
      if (idx >= 0) {
        modal.selectedMembers.splice(idx, 1);
        nowChecked = false;
      } else {
        if (modal.selectedMembers.length >= 30) {
          modal.error = "单个群聊最多支持 30 个成员";
          rerenderPreservingScroll();
          return;
        }
        modal.selectedMembers.push({ providerId: pid, uuid });
        nowChecked = true;
      }
      modal.error = undefined;
      btn.classList.toggle("selected", nowChecked);
      const checkBoxEl = btn.querySelector<HTMLElement>(".group-check-box");
      if (checkBoxEl) {
        checkBoxEl.textContent = nowChecked ? "✓" : "";
      }
      const hintEl = document.querySelector<HTMLElement>("#modal-group-hint");
      if (hintEl) {
        const extCount = (state.externalAgents ?? []).length;
        const eligibleCount =
          state.accounts.filter((a) => isAccountGroupEligible(state, a)).length + extCount;
        const ineligibleCount = state.accounts.length + extCount - eligibleCount;
        hintEl.textContent = formatGroupPickerHint(
          eligibleCount,
          ineligibleCount,
          modal.selectedMembers.length,
          true
        );
      }
      const confirmBtn = document.querySelector<HTMLButtonElement>("#modal-confirm-btn");
      if (confirmBtn) {
        confirmBtn.disabled = modal.selectedMembers.length === 0;
      }
      document.querySelector("#modal-error-box")?.remove();
    });
  });

  const commitKnowledgeSkillDraft = (
    kModal: Extract<ModalMode, { kind: "group-knowledge" }>
  ): boolean => {
    const name = kModal.skillNameDraft.trim();
    const desc = kModal.skillDescDraft.trim();
    const content = kModal.skillContentDraft.trim();
    if (!name || !content) {
      kModal.error = !name ? "请填写技能名称" : "请填写技能指令内容";
      return false;
    }
    if (kModal.editingSkillId) {
      const idx = kModal.skillsDraft.findIndex((s) => s.id === kModal.editingSkillId);
      if (idx >= 0) {
        kModal.skillsDraft[idx] = {
          ...kModal.skillsDraft[idx],
          name,
          description: desc,
          content,
        };
      }
    } else {
      if (kModal.skillsDraft.length >= 30) {
        kModal.error = "单个群聊最多支持 30 个共享技能";
        return false;
      }
      kModal.skillsDraft.push({
        id: `skill_${Date.now()}_${kModal.skillsDraft.length}`,
        name,
        description: desc,
        content,
        enabled: true,
      });
    }
    kModal.skillNameDraft = "";
    kModal.skillDescDraft = "";
    kModal.skillContentDraft = "";
    kModal.editingSkillId = null;
    kModal.error = undefined;
    return true;
  };

  document.querySelectorAll<HTMLButtonElement>("[data-knowledge-tab]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (modal.kind !== "group-knowledge" || modal.saving) return;
      const nextTab = btn.dataset.knowledgeTab as
        | "context"
        | "skills"
        | "roles"
        | "adapters"
        | undefined;
      if (nextTab && modal.tab !== nextTab) {
        modal.tab = nextTab;
        modal.error = undefined;
        render();
        if (nextTab === "adapters" && !modal.adaptersLoaded) {
          const currentModal = modal;
          try {
            const snap = await invoke<SiteAdaptersSnapshot>("get_site_adapters");
            if (modal === currentModal) {
              currentModal.adaptersSnap = snap;
              currentModal.adaptersJsonDraft = snap.overrideJson ?? "";
              currentModal.adaptersLoaded = true;
              render();
            }
          } catch (err) {
            if (modal === currentModal) {
              currentModal.error = String(err);
              currentModal.adaptersLoaded = true;
              render();
            }
          }
        }
      }
    });
  });

  const contextInputEl = document.querySelector<HTMLTextAreaElement>("#modal-group-context-input");
  contextInputEl?.addEventListener("input", () => {
    if (modal.kind !== "group-knowledge") return;
    modal.contextDraft = contextInputEl.value;
    const countEl = document.querySelector<HTMLElement>("#modal-group-context-count");
    if (countEl) {
      countEl.textContent = String(contextInputEl.value.length);
    }
    if (modal.error) {
      modal.error = undefined;
      document.querySelector("#modal-error-box")?.remove();
    }
  });

  document.querySelectorAll<HTMLButtonElement>("[data-add-skill-preset]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (modal.kind !== "group-knowledge" || modal.saving) return;
      const idx = Number(btn.dataset.addSkillPreset ?? "-1");
      const preset = GROUP_SKILL_PRESETS[idx];
      if (!preset) return;
      if (modal.skillsDraft.some((s) => s.name === preset.name)) return;
      if (modal.skillsDraft.length >= 30) {
        modal.error = "单个群聊最多支持 30 个共享技能";
        render();
        return;
      }
      modal.skillsDraft.push({
        id: `skill_${Date.now()}_${idx}`,
        name: preset.name,
        description: preset.description,
        content: preset.content,
        enabled: true,
      });
      modal.error = undefined;
      render();
    });
  });

  document.querySelectorAll<HTMLInputElement>("[data-toggle-skill-id]").forEach((cb) => {
    cb.addEventListener("change", () => {
      if (modal.kind !== "group-knowledge" || modal.saving) return;
      const sid = cb.dataset.toggleSkillId;
      const target = modal.skillsDraft.find((s) => s.id === sid);
      if (target) {
        target.enabled = cb.checked;
        modal.error = undefined;
        render();
      }
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-edit-skill-id]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (modal.kind !== "group-knowledge" || modal.saving) return;
      const sid = btn.dataset.editSkillId;
      const target = modal.skillsDraft.find((s) => s.id === sid);
      if (!target) return;
      modal.editingSkillId = target.id;
      modal.skillNameDraft = target.name;
      modal.skillDescDraft = target.description;
      modal.skillContentDraft = target.content;
      modal.error = undefined;
      render();
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-delete-skill-id]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (modal.kind !== "group-knowledge" || modal.saving) return;
      const sid = btn.dataset.deleteSkillId;
      modal.skillsDraft = modal.skillsDraft.filter((s) => s.id !== sid);
      if (modal.editingSkillId === sid) {
        modal.editingSkillId = null;
        modal.skillNameDraft = "";
        modal.skillDescDraft = "";
        modal.skillContentDraft = "";
      }
      modal.error = undefined;
      render();
    });
  });

  document
    .querySelectorAll<HTMLInputElement | HTMLTextAreaElement>(
      "#modal-skill-name-input, #modal-skill-desc-input, #modal-skill-content-input"
    )
    .forEach((el) => {
      el.addEventListener("input", () => {
        if (modal.kind !== "group-knowledge") return;
        if (el.id === "modal-skill-name-input") modal.skillNameDraft = el.value;
        else if (el.id === "modal-skill-desc-input") modal.skillDescDraft = el.value;
        else modal.skillContentDraft = el.value;
        if (modal.error) {
          modal.error = undefined;
          document.querySelector("#modal-error-box")?.remove();
        }
      });
    });

  document.querySelector("#modal-skill-save-btn")?.addEventListener("click", () => {
    if (modal.kind !== "group-knowledge" || modal.saving) return;
    commitKnowledgeSkillDraft(modal);
    render();
  });

  document.querySelector("#modal-skill-cancel-edit-btn")?.addEventListener("click", () => {
    if (modal.kind !== "group-knowledge" || modal.saving) return;
    modal.editingSkillId = null;
    modal.skillNameDraft = "";
    modal.skillDescDraft = "";
    modal.skillContentDraft = "";
    modal.error = undefined;
    render();
  });

  document.querySelectorAll<HTMLInputElement>("[data-role-provider]").forEach((input) => {
    input.addEventListener("input", () => {
      if (modal.kind !== "group-knowledge") return;
      const pid = input.dataset.roleProvider;
      const uuid = input.dataset.roleUuid;
      if (!pid || !uuid) return;
      const key: AccountKey = { providerId: pid, uuid };
      const existing = modal.rolesDraft.find((r) => sameAccountKey(r.member, key));
      if (existing) {
        existing.role = input.value;
      } else {
        modal.rolesDraft.push({ member: key, role: input.value });
      }
      if (modal.error) {
        modal.error = undefined;
        document.querySelector("#modal-error-box")?.remove();
      }
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-reset-thread-provider]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (modal.kind !== "group-knowledge" || modal.saving) return;
      const providerId = btn.dataset.resetThreadProvider;
      const uuid = btn.dataset.resetThreadUuid;
      if (!providerId || !uuid) return;
      const currentModal = modal;
      currentModal.saving = true;
      currentModal.error = undefined;
      render();
      try {
        await callCommand("reset_group_thread_binding", {
          groupId: currentModal.group.id,
          member: { providerId, uuid },
        });
      } finally {
        currentModal.saving = false;
        render();
      }
    });
  });

  const adaptersJsonEl = document.querySelector<HTMLTextAreaElement>("#modal-adapters-json-input");
  adaptersJsonEl?.addEventListener("input", () => {
    if (modal.kind !== "group-knowledge") return;
    modal.adaptersJsonDraft = adaptersJsonEl.value;
    if (modal.error) {
      modal.error = undefined;
      document.querySelector("#modal-error-box")?.remove();
    }
  });

  document.querySelector("#modal-adapters-reset-btn")?.addEventListener("click", async () => {
    if (modal.kind !== "group-knowledge" || modal.saving) return;
    const currentModal = modal;
    currentModal.saving = true;
    currentModal.error = undefined;
    render();
    try {
      const snap = await invoke<SiteAdaptersSnapshot>("reset_site_adapter_overrides");
      if (modal === currentModal) {
        currentModal.adaptersSnap = snap;
        currentModal.adaptersJsonDraft = snap.overrideJson ?? "";
      }
    } catch (err) {
      if (modal === currentModal) {
        currentModal.error = String(err);
      }
    } finally {
      currentModal.saving = false;
      render();
    }
  });

  document.querySelectorAll<HTMLButtonElement>("[data-ext-kind]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (modal.kind !== "external-agents" || modal.saving) return;
      const nextKind = btn.dataset.extKind as ExternalAgentKind | undefined;
      if (nextKind && modal.formKind !== nextKind) {
        modal.formKind = nextKind;
        // 切类型必须清空 Key 草稿：否则给 A 成员输的 Key 会跟着
        // 保存进 B 成员（另一种 kind）里。
        modal.apiKeyDraft = "";
        modal.error = undefined;
        render();
      }
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-ext-preset-idx]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (modal.kind !== "external-agents" || modal.saving) return;
      const idx = Number(btn.dataset.extPresetIdx ?? "-1");
      const preset = EXTERNAL_AGENT_PRESETS[idx];
      if (!preset) return;
      modal.formKind = preset.kind;
      modal.nameDraft = preset.name;
      modal.baseUrlDraft = preset.baseUrl ?? "";
      // 预设不带 Key：切预设必须清空 Key 草稿，否则之前给别的
      // 成员输的 Key 会跟着保存进这个预设成员里。
      modal.apiKeyDraft = "";
      // 切预设同时退出编辑态：否则正编辑的旧成员（editingId 已设，
      // Vault 里有它的 Key）会被改成这个预设的 baseUrl 后保存，
      // 旧 Key 会被发到新服务上。
      modal.editingId = null;
      modal.modelDraft = preset.model ?? "";
      modal.commandDraft = preset.command ?? "";
      modal.argsDraft = preset.args ?? "";
      modal.error = undefined;
      render();
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-edit-ext-id]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (modal.kind !== "external-agents" || modal.saving) return;
      const eid = btn.dataset.editExtId;
      const ext = (state.externalAgents ?? []).find((a) => a.id === eid);
      if (!ext) return;
      modal.editingId = ext.id;
      modal.formKind = ext.kind;
      modal.nameDraft = ext.name;
      modal.baseUrlDraft = ext.baseUrl;
      modal.apiKeyDraft = "";
      modal.modelDraft = ext.model;
      modal.commandDraft = ext.command;
      modal.argsDraft = ext.args.join(" ");
      modal.workdirDraft = ext.workdir ?? "";
      modal.timeoutSecsDraft = String(ext.timeoutSecs);
      modal.idleTimeoutSecsDraft = String(ext.idleTimeoutSecs);
      modal.error = undefined;
      render();
    });
  });

  document.querySelectorAll<HTMLButtonElement>("[data-delete-ext-id]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      if (modal.kind !== "external-agents" || modal.saving) return;
      const eid = btn.dataset.deleteExtId;
      if (!eid) return;
      const currentModal = modal;
      currentModal.saving = true;
      currentModal.error = undefined;
      render();
      try {
        const deleted = await callCommand("delete_external_agent", { id: eid });
        if (deleted && modal === currentModal && currentModal.editingId === eid) {
          currentModal.editingId = null;
          currentModal.nameDraft = "";
          currentModal.baseUrlDraft = "";
          currentModal.apiKeyDraft = "";
          currentModal.modelDraft = "";
          currentModal.commandDraft = "";
          currentModal.argsDraft = "";
          currentModal.workdirDraft = "";
          currentModal.timeoutSecsDraft = "";
          currentModal.idleTimeoutSecsDraft = "";
        }
      } finally {
        currentModal.saving = false;
        render();
      }
    });
  });

  document.querySelector("#ext-agent-cancel-edit-btn")?.addEventListener("click", () => {
    if (modal.kind !== "external-agents" || modal.saving) return;
    modal.editingId = null;
    modal.nameDraft = "";
    modal.baseUrlDraft = "";
    modal.apiKeyDraft = "";
    modal.modelDraft = "";
    modal.commandDraft = "";
    modal.argsDraft = "";
    modal.workdirDraft = "";
    modal.timeoutSecsDraft = "";
    modal.idleTimeoutSecsDraft = "";
    modal.error = undefined;
    render();
  });

  document
    .querySelectorAll<HTMLInputElement>(
      "#ext-agent-name-input, #ext-agent-url-input, #ext-agent-key-input, #ext-agent-model-input, #ext-agent-cmd-input, #ext-agent-args-input, #ext-agent-workdir-input, #ext-agent-timeout-input, #ext-agent-idle-timeout-input"
    )
    .forEach((input) => {
      input.addEventListener("input", () => {
        if (modal.kind !== "external-agents") return;
        if (input.id === "ext-agent-name-input") modal.nameDraft = input.value;
        else if (input.id === "ext-agent-url-input") modal.baseUrlDraft = input.value;
        else if (input.id === "ext-agent-key-input") modal.apiKeyDraft = input.value;
        else if (input.id === "ext-agent-model-input") modal.modelDraft = input.value;
        else if (input.id === "ext-agent-cmd-input") modal.commandDraft = input.value;
        else if (input.id === "ext-agent-args-input") modal.argsDraft = input.value;
        else if (input.id === "ext-agent-workdir-input") modal.workdirDraft = input.value;
        else if (input.id === "ext-agent-timeout-input") modal.timeoutSecsDraft = input.value;
        else if (input.id === "ext-agent-idle-timeout-input")
          modal.idleTimeoutSecsDraft = input.value;
        if (modal.error) {
          modal.error = undefined;
          document.querySelector("#modal-error-box")?.remove();
        }
      });
    });

  document.querySelector("#ext-agent-save-btn")?.addEventListener("click", async () => {
    if (modal.kind !== "external-agents" || modal.saving) return;
    const currentModal = modal;
    currentModal.saving = true;
    currentModal.error = undefined;
    render();
    try {
      const args = currentModal.argsDraft.trim()
        ? currentModal.argsDraft.trim().split(/\s+/)
        : [];
      const parseTimeoutSecs = (raw: string): number | null => {
        const n = Number.parseInt(raw.trim(), 10);
        return Number.isFinite(n) ? n : null;
      };
      const saved = await callCommand("upsert_external_agent", {
        id: currentModal.editingId,
        kind: currentModal.formKind,
        name: currentModal.nameDraft.trim(),
        baseUrl: currentModal.baseUrlDraft.trim(),
        apiKey: currentModal.apiKeyDraft.trim(),
        model: currentModal.modelDraft.trim(),
        command: currentModal.commandDraft.trim(),
        args,
        workdir: currentModal.workdirDraft.trim() || null,
        timeoutSecs: parseTimeoutSecs(currentModal.timeoutSecsDraft),
        idleTimeoutSecs: parseTimeoutSecs(currentModal.idleTimeoutSecsDraft),
      });
      if (saved && modal === currentModal) {
        currentModal.editingId = null;
        currentModal.nameDraft = "";
        currentModal.baseUrlDraft = "";
        currentModal.apiKeyDraft = "";
        currentModal.modelDraft = "";
        currentModal.commandDraft = "";
        currentModal.argsDraft = "";
        currentModal.workdirDraft = "";
        currentModal.timeoutSecsDraft = "";
        currentModal.idleTimeoutSecsDraft = "";
      }
    } finally {
      currentModal.saving = false;
      render();
    }
  });

  document
    .querySelectorAll<HTMLInputElement>(
      "#modal-alias-input, #modal-proxy-input, #modal-timezone-input, #modal-shortcut-input, #modal-group-name-input"
    )
    .forEach((input) => {
      input.addEventListener("input", () => {
        if (modal.kind === "add" || modal.kind === "rename") {
          if (input.id === "modal-proxy-input") {
            modal.proxyDraft = input.value;
            modal.proxyTestResult = null;
            document.querySelector("#proxy-test-result")?.remove();
          } else if (input.id === "modal-timezone-input") {
            modal.timezoneDraft = input.value;
          } else {
            modal.draft = input.value;
          }
          if (modal.error) {
            modal.error = undefined;
            document.querySelector("#modal-error-box")?.remove();
          }
        } else if (modal.kind === "group") {
          modal.nameDraft = input.value;
          if (modal.error) {
            modal.error = undefined;
            document.querySelector("#modal-error-box")?.remove();
          }
        } else if (modal.kind === "settings") {
          modal.globalShortcut = input.value;
          if (modal.error) {
            modal.error = undefined;
            document.querySelector("#modal-error-box")?.remove();
          }
        }
      });
      input.addEventListener("keydown", async (ev) => {
        if (ev.key === "Enter" && !ev.isComposing) {
          ev.preventDefault();
          document.querySelector<HTMLButtonElement>("#modal-confirm-btn")?.click();
        } else if (ev.key === "Escape") {
          if (
            (modal.kind === "add" ||
              modal.kind === "rename" ||
              modal.kind === "group" ||
              modal.kind === "settings") &&
            modal.saving
          ) {
            return;
          }
          ev.preventDefault();
          ev.stopPropagation();
          await setModal({ kind: "none" });
        }
      });
    });

  document.querySelector("#modal-cancel-btn")?.addEventListener("click", async () => {
    if (modal.kind === "update" && updateState.installing) return;
    if (
      (modal.kind === "add" ||
        modal.kind === "rename" ||
        modal.kind === "group" ||
        modal.kind === "group-knowledge" ||
        modal.kind === "external-agents" ||
        modal.kind === "settings") &&
      modal.saving
    ) {
      return;
    }
    await setModal({ kind: "none" });
  });

  document.querySelector("#modal-confirm-btn")?.addEventListener("click", async () => {
    const currentModal = modal;
    if (currentModal.kind === "add" || currentModal.kind === "rename") {
      if (currentModal.saving) return;
      const supportsProxy = supportsAccountProxy(state.platform);
      const alias = currentModal.draft.trim();
      const proxyUrl = supportsProxy ? currentModal.proxyDraft.trim() : undefined;
      const timezone = currentModal.timezoneDraft.trim();
      currentModal.error = undefined;
      currentModal.saving = true;
      render();
      let saved = false;
      try {
        const rawCookies = (currentModal.cookieDraft ?? "").trim();
        saved = currentModal.kind === "add"
          ? await callCommand("add_account", {
            providerId: currentModal.providerId,
            alias,
            proxyUrl,
            timezone,
            rawCookies: rawCookies || undefined,
          })
          : await callCommand("rename_account", {
            providerId: currentModal.account.providerId,
            uuid: currentModal.account.uuid,
            alias,
            proxyUrl,
            timezone,
          });
      } finally {
        currentModal.saving = false;
        if (modal === currentModal && !saved) {
          render();
        }
      }
      if (saved && modal === currentModal) {
        await setModal({ kind: "none" });
      }
    } else if (currentModal.kind === "settings") {
      if (currentModal.saving) return;
      currentModal.error = undefined;
      currentModal.saving = true;
      render();
      let saved = false;
      try {
        saved = await callCommand("update_app_settings", {
          minimizeToTray: currentModal.minimizeToTray,
          globalShortcut: currentModal.globalShortcut.trim(),
          autoApprove: currentModal.autoApprove,
          disableAutoUpdateCheck: currentModal.disableAutoUpdateCheck,
          autoApproveCommands: currentModal.autoApproveCommands,
        });
      } finally {
        currentModal.saving = false;
        if (modal === currentModal && !saved) {
          render();
        }
      }
      if (saved && modal === currentModal) {
        await setModal({ kind: "none" });
      }
    } else if (currentModal.kind === "group") {
      if (currentModal.saving) return;
      currentModal.error = undefined;
      currentModal.saving = true;
      render();
      let saved = false;
      try {
        if (currentModal.group) {
          saved = await callCommand("update_group_chat", {
            groupId: currentModal.group.id,
            name: currentModal.nameDraft.trim(),
            members: currentModal.selectedMembers,
          });
        } else {
          viewMode = "groups";
          saved = await callCommand("create_group_chat", {
            name: currentModal.nameDraft.trim(),
            members: currentModal.selectedMembers,
          });
        }
      } finally {
        currentModal.saving = false;
        if (modal === currentModal && !saved) {
          render();
        }
      }
      if (saved && modal === currentModal) {
        await setModal({ kind: "none" });
      }
    } else if (currentModal.kind === "group-knowledge") {
      if (currentModal.saving) return;
      if (currentModal.tab === "adapters") {
        currentModal.saving = true;
        currentModal.error = undefined;
        render();
        let saved = false;
        try {
          const snap = await invoke<SiteAdaptersSnapshot>("save_site_adapter_overrides", {
            rawJson: currentModal.adaptersJsonDraft,
          });
          if (modal === currentModal) {
            currentModal.adaptersSnap = snap;
            currentModal.adaptersJsonDraft = snap.overrideJson ?? "";
            saved = true;
          }
        } catch (err) {
          if (modal === currentModal) {
            currentModal.error = String(err);
          }
        } finally {
          currentModal.saving = false;
          if (modal === currentModal && !saved) {
            render();
          }
        }
        if (saved && modal === currentModal) {
          await setModal({ kind: "none" });
        }
        return;
      }
      if (currentModal.skillNameDraft.trim() || currentModal.skillContentDraft.trim()) {
        if (!commitKnowledgeSkillDraft(currentModal)) {
          render();
          return;
        }
      }
      currentModal.saving = true;
      currentModal.error = undefined;
      render();
      let saved = false;
      try {
        saved = await callCommand("update_group_knowledge", {
          groupId: currentModal.group.id,
          sharedContext: currentModal.contextDraft.trim(),
          sharedSkills: currentModal.skillsDraft,
          memberRoles: currentModal.rolesDraft
            .map((r) => ({ member: r.member, role: r.role.trim() }))
            .filter((r) => r.role.length > 0),
        });
      } finally {
        currentModal.saving = false;
        if (modal === currentModal && !saved) {
          render();
        }
      }
      if (saved && modal === currentModal) {
        await setModal({ kind: "none" });
      }
    } else if (currentModal.kind === "delete-group") {
      await setModal({ kind: "none" });
      await callCommand("delete_group_chat", { groupId: currentModal.group.id });
    } else if (currentModal.kind === "remove") {
      await setModal({ kind: "none" });
      await callCommand("remove_account", {
        providerId: currentModal.account.providerId,
        uuid: currentModal.account.uuid,
      });
    }
  });
}


async function bootstrap(): Promise<void> {
  await listen<ManagerSnapshot>("manager-state-changed", (event) => {
    const prevSnapshot = snapshot;
    const nextSnapshot = event.payload;
    if (
      modal.kind === "group" &&
      !modal.saving &&
      prevSnapshot &&
      groupModalVisualSignature(prevSnapshot) === groupModalVisualSignature(nextSnapshot)
    ) {
      snapshot = nextSnapshot;
      const nextViewMode = syncViewModeFromSnapshot(snapshot);
      if (nextViewMode) viewMode = nextViewMode;
      if (snapshot.activeAccount || !snapshot.switchingAccount) {
        optimisticSwitching = null;
      }
      return;
    }
    saveScrollPosition(snapshot, viewMode);
    snapshot = nextSnapshot;
    const nextViewMode = syncViewModeFromSnapshot(snapshot);
    if (nextViewMode) viewMode = nextViewMode;
    if (snapshot.activeAccount || !snapshot.switchingAccount) {
      optimisticSwitching = null;
    }
    render();
    restoreScrollPosition(snapshot, viewMode, modal);
  });

  await listen<DownloadNoticeEvent>("download-event", (event) => {
    downloadToast = event.payload;
    clearDownloadToastTimer();
    rerenderPreservingScroll();
    scheduleDownloadToastDismiss();
  });

  await listen<UpdateProgressEvent>("update-progress", (event) => {
    const evt = event.payload;
    updateState.progress = evt;
    if (evt.stage === "error") {
      updateState.error = evt.message;
    }
    if (modal.kind !== "update") {
      return;
    }
    const barEl = document.querySelector<HTMLElement>("#update-progress-bar");
    const textEl = document.querySelector<HTMLElement>("#update-progress-text");
    const pctEl = document.querySelector<HTMLElement>("#update-progress-pct");
    if (barEl && textEl && pctEl && getLastRenderedUpdateStage() === evt.stage && evt.stage !== "error") {
      textEl.textContent = evt.message;
      pctEl.textContent = formatProgressSummary(evt);
      if (evt.percent === null) {
        barEl.classList.add("indeterminate");
        barEl.style.width = "";
      } else {
        barEl.classList.remove("indeterminate");
        barEl.style.width = `${Math.max(0, Math.min(100, evt.percent))}%`;
      }
      return;
    }
    setLastRenderedUpdateStage(evt.stage);
    rerenderPreservingScroll();
  });

  window.addEventListener("keydown", async (ev) => {
    if (modal.kind !== "none") {
      if (
        ev.key === "Escape" &&
        !(modal.kind === "add" && modal.saving) &&
        !(modal.kind === "rename" && modal.saving) &&
        !(modal.kind === "group" && modal.saving) &&
        !(modal.kind === "group-knowledge" && modal.saving) &&
        !(modal.kind === "external-agents" && modal.saving) &&
        !(modal.kind === "settings" && modal.saving) &&
        !(modal.kind === "update" && updateState.installing)
      ) {
        ev.preventDefault();
        await setModal({ kind: "none" });
      }
      return;
    }
    const target = ev.target as HTMLElement | null;
    if (
      target &&
      (target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.isContentEditable)
    ) {
      return;
    }
    if ((ev.metaKey || ev.ctrlKey) && !ev.shiftKey && !ev.altKey) {
      const digit = parseInt(ev.key, 10);
      if (!isNaN(digit) && digit >= 1 && digit <= 9) {
        ev.preventDefault();
        if (!snapshot) return;
        const providerAccounts = snapshot.accounts.filter(
          (a) => a.providerId === snapshot!.selectedProviderId
        );
        const targetAcc = providerAccounts[digit - 1];
        if (targetAcc && !isSameKey(snapshot.activeAccount, targetAcc)) {
          const alreadyWarm = snapshot.warmAccounts.some(
            (k) =>
              k.providerId === targetAcc.providerId && k.uuid === targetAcc.uuid
          );
          await callCommand(
            "switch_account",
            { providerId: targetAcc.providerId, uuid: targetAcc.uuid },
            alreadyWarm ? null : { providerId: targetAcc.providerId, uuid: targetAcc.uuid }
          );
        }
        return;
      }
      if (ev.key === "\\") {
        ev.preventDefault();
        if (!snapshot) return;
        await setSidebarCollapsed(!snapshot.effectiveSidebarCollapsed);
        return;
      }
      if (
        ev.key === "ArrowUp" ||
        ev.key === "ArrowDown" ||
        ev.key === "[" ||
        ev.key === "]"
      ) {
        ev.preventDefault();
        if (!snapshot) return;
        const providerAccounts = snapshot.accounts.filter(
          (a) => a.providerId === snapshot!.selectedProviderId
        );
        if (providerAccounts.length <= 1) return;
        const curIdx = providerAccounts.findIndex((a) =>
          isSameKey(snapshot!.activeAccount, a)
        );
        const delta = ev.key === "ArrowUp" || ev.key === "[" ? -1 : 1;
        const nextIdx =
          (curIdx + delta + providerAccounts.length) % providerAccounts.length;
        const targetAcc = providerAccounts[nextIdx];
        if (targetAcc && !isSameKey(snapshot.activeAccount, targetAcc)) {
          const alreadyWarm = snapshot.warmAccounts.some(
            (k) =>
              k.providerId === targetAcc.providerId && k.uuid === targetAcc.uuid
          );
          await callCommand(
            "switch_account",
            { providerId: targetAcc.providerId, uuid: targetAcc.uuid },
            alreadyWarm ? null : { providerId: targetAcc.providerId, uuid: targetAcc.uuid }
          );
        }
      }
    }
  });

  snapshot = await invoke<ManagerSnapshot>("get_manager_state");
  const nextViewMode = syncViewModeFromSnapshot(snapshot);
  if (nextViewMode) viewMode = nextViewMode;
  render();
  setTimeout(() => {
    if (!snapshot?.settings.disableAutoUpdateCheck) {
      void runCheckForUpdates(true);
    }
  }, 3000);
  window.setInterval(() => {
    void maybeAutoCheckForUpdates();
  }, 30 * 60 * 1000);
  window.addEventListener("focus", () => {
    void maybeAutoCheckForUpdates();
  });
  window.addEventListener("online", () => {
    void maybeAutoCheckForUpdates();
  });
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") {
      void maybeAutoCheckForUpdates();
    }
  });
}

void bootstrap();
