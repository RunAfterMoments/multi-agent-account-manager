// Group-chat renderers + mention/composer helpers (P2-11 split).
//
// Moved verbatim from `main.ts`: `renderGroupMiddleContent`,
// `renderGroupRightStage`, `scanTextMentions`,
// `extractComposerAtMentions`, `renderMessageTextWithMentions`,
// `formatReviewStageChain`, `formatComposerHintText` and
// `getLatestRoundCompletedReplies`. Behavior-preserving signature
// changes: the group renderers take the pieces of main.ts state they
// read as same-name parameters (`modal`, `downloadToast`,
// `shouldAnimateModal`, `groupSending`, `newTopicConfirmGroupId`).
// Shared helpers come from `./state`, the modal overlay from
// `./modals`, and the download toast from `./render`.

import type {
  AccountKey,
  DownloadNoticeEvent,
  GroupChatMessage,
  GroupChatRecord,
  GroupWorkflowMode,
  ManagerSnapshot,
  ProviderInfo,
  ModalMode,
} from "./types";

import {
  escapeHtml,
  formatShortTime,
  groupDrafts,
  groupTargets,
  groupAutoChannels,
  syncingMsgIds,
  sameAccountKey,
  ALL_MENTION_KEYWORDS,
  resolveGroupMessageSenderName,
  resolveMemberMeta,
  renderMemberAvatarNode,
  ICON_GROUP,
  ICON_EDIT,
  ICON_TRASH,
  ICON_PIN,
  ICON_REFRESH,
  groupHasNeedsAuth,
  resolveMemberCategory,
  groupByMemberCategory,
  renderCategoryIcon,
  isAsciiWordChar,
} from "./state";

import { renderModal } from "./modals";
import { renderDownloadToast } from "./render";

export function scanTextMentions<T extends { alias: string }>(
  text: string,
  members: T[]
): Array<{ start: number; end: number; isAll: boolean; member?: T }> {
  const sorted = [...members]
    .filter((m) => m.alias.trim().length > 0)
    .sort((a, b) => b.alias.length - a.alias.length);
  const hits: Array<{ start: number; end: number; isAll: boolean; member?: T }> = [];
  let i = 0;
  while (i < text.length) {
    if (text[i] !== "@" || (i > 0 && isAsciiWordChar(text[i - 1]))) {
      i += 1;
      continue;
    }
    const rest = text.slice(i + 1);
    const restLower = rest.toLowerCase();
    let matched = false;
    for (const kw of ALL_MENTION_KEYWORDS) {
      if (restLower.startsWith(kw)) {
        const after = rest[kw.length];
        if (!isAsciiWordChar(after)) {
          hits.push({ start: i, end: i + 1 + kw.length, isAll: true });
          i += 1 + kw.length;
          matched = true;
          break;
        }
      }
    }
    if (matched) continue;
    for (const m of sorted) {
      const alias = m.alias;
      if (restLower.startsWith(alias.toLowerCase())) {
        const after = rest[alias.length];
        const lastAliasChar = alias[alias.length - 1];
        if (!(isAsciiWordChar(lastAliasChar) && isAsciiWordChar(after))) {
          hits.push({ start: i, end: i + 1 + alias.length, isAll: false, member: m });
          i += 1 + alias.length;
          matched = true;
          break;
        }
      }
    }
    if (!matched) {
      i += 1;
    }
  }
  return hits;
}
export function extractComposerAtMentions(
  text: string,
  members: Array<{ key: AccountKey; alias: string }>
): { hasMention: boolean; isAll: boolean; targets: AccountKey[]; labels: string[] } {
  const hits = scanTextMentions(text, members);
  if (hits.length === 0) {
    return { hasMention: false, isAll: false, targets: [], labels: [] };
  }
  if (hits.some((h) => h.isAll)) {
    return { hasMention: true, isAll: true, targets: [], labels: ["全体成员"] };
  }
  const targets: AccountKey[] = [];
  const labels: string[] = [];
  for (const h of hits) {
    if (h.member && !targets.some((t) => sameAccountKey(t, h.member!.key))) {
      targets.push(h.member.key);
      labels.push(h.member.alias);
    }
  }
  return { hasMention: targets.length > 0, isAll: false, targets, labels };
}
export function renderMessageTextWithMentions(content: string, memberAliases: string[]): string {
  const hits = scanTextMentions(
    content,
    memberAliases.map((alias) => ({ alias }))
  );
  if (hits.length === 0) {
    return escapeHtml(content);
  }
  let out = "";
  let cursor = 0;
  for (const hit of hits) {
    if (hit.start > cursor) {
      out += escapeHtml(content.slice(cursor, hit.start));
    }
    out += `<span class="group-mention-tag">${escapeHtml(content.slice(hit.start, hit.end))}</span>`;
    cursor = hit.end;
  }
  if (cursor < content.length) {
    out += escapeHtml(content.slice(cursor));
  }
  return out;
}
export function formatReviewStageChain(labels: string[]): string {
  if (labels.length < 2) return "";
  return labels
    .map((label, idx) => {
      const stage =
        idx === 0
          ? "初稿"
          : idx === labels.length - 1 && labels.length >= 3
            ? "综合定稿"
            : "交叉评审";
      return `${label}（${stage}）`;
    })
    .join(" → ");
}
export function formatComposerHintText(
  draft: string,
  selectedTargets: AccountKey[],
  members: Array<{ key: AccountKey; alias: string; categoryId: string }>,
  workflowMode: GroupWorkflowMode = "parallel",
  maxRelayHops = 2,
  selectedAutoChannels: Array<{ id: string; label: string }> = []
): string {
  const inline = extractComposerAtMentions(draft, members);
  const modeSuffix =
    workflowMode === "review"
      ? " · 评审流（初稿 → 评审 → 定稿）"
      : workflowMode === "a2a_relay"
        ? ` · A2A接力（最多 ${maxRelayHops} 跳）`
        : "";
  if (inline.hasMention) {
    if (inline.isAll) {
      return `检测到 @全体成员：将按排队限流向全员转发${modeSuffix}`;
    }
    const inlineCats = new Set(
      inline.targets
        .map((tk) => members.find((m) => sameAccountKey(m.key, tk))?.categoryId)
        .filter((x): x is string => Boolean(x))
    );
    const extraAuto = selectedAutoChannels.filter((c) => !inlineCats.has(c.id));
    if (workflowMode === "review") {
      const reviewLabels = [
        ...inline.labels.map((l) => `@${l}`),
        ...extraAuto.map((c) => `${c.label}(自动)`),
      ];
      if (reviewLabels.length >= 2) {
        return `检测到正文 @，评审流顺序：${formatReviewStageChain(reviewLabels)}`;
      }
    }
    if (extraAuto.length > 0) {
      return `检测到正文 @${inline.labels.join("、@")}：将同时转发给 ${extraAuto
        .map((c) => `${c.label}(自动选号)`)
        .join("、")}${modeSuffix}`;
    }
    return `检测到正文 @${inline.labels.join("、@")}：将仅转发给被提及成员${modeSuffix}`;
  }
  const hasExplicit = selectedTargets.length > 0;
  const hasAuto = selectedAutoChannels.length > 0;
  if (hasExplicit || hasAuto) {
    const parts: string[] = [];
    for (const t of selectedTargets) {
      parts.push(
        `@${members.find((m) => sameAccountKey(m.key, t))?.alias ?? t.uuid.slice(0, 6)}`
      );
    }
    for (const ac of selectedAutoChannels) {
      parts.push(`${ac.label}(自动按额度选号)`);
    }
    if (workflowMode === "review" && parts.length >= 2) {
      return `评审流顺序：${formatReviewStageChain(parts)}`;
    }
    return `仅向 ${parts.join("、")} 发送${modeSuffix}`;
  }
  if (workflowMode === "review") {
    return "评审流模式：按成员顺序串行执行（初稿 → 交叉评审 → 综合定稿）；按顺序点选上方成员或在正文按顺序 @ 可自定义先后顺序";
  }
  if (workflowMode === "a2a_relay") {
    return `A2A 接力模式：成员回复正文包含 @成员 将自动触发下一跳（上限 ${maxRelayHops} 跳）`;
  }
  return "向全体成员发送（输入 @ 可点名，Enter 发送，Shift+Enter 换行）";
}
export function renderGroupMiddleContent(
  state: ManagerSnapshot,
  provider: ProviderInfo,
  modal: ModalMode,
  downloadToast: DownloadNoticeEvent | null,
  shouldAnimateModal: boolean
): string {
  const noticeHtml = state.notice
    ? `
      <div class="notice-bar">
        <span>${escapeHtml(state.notice)}</span>
        <button id="dismiss-notice-btn" class="notice-close" type="button" title="关闭提示">✕</button>
      </div>
    `
    : "";

  const extCount = (state.externalAgents ?? []).length;

  const listBody =
    state.groupChats.length === 0
      ? `
        <div class="empty-list">
          <div class="empty-icon-circle">${ICON_GROUP}</div>
          <div>暂无群聊</div>
          <div>点击右上角「+ 创建群聊」，可混编网页账号与 API / CLI 成员协同对话</div>
        </div>
      `
      : state.groupChats
          .map((g) => {
            const active = state.activeGroupId === g.id;
            const hasNeedsAuth = groupHasNeedsAuth(g);
            const isStreaming = g.messages.some(
              (m) => m.status === "queued" || m.status === "sending" || m.status === "streaming"
            );
            const lastMsg = g.messages[g.messages.length - 1];
            const lastSenderName = lastMsg
              ? resolveGroupMessageSenderName(state, lastMsg)
              : "";
            const preview = lastMsg
              ? lastMsg.status === "needs_auth"
                ? `[待授权] ${lastSenderName}`
                : lastMsg.role === "user"
                  ? `我: ${lastMsg.content}`
                  : lastMsg.status === "queued"
                    ? `${lastSenderName}: 排队等待回复…`
                    : lastMsg.content
                      ? `${lastSenderName}: ${lastMsg.content}`
                      : `${lastSenderName} 正在回复…`
              : `${g.members.length} 位成员 · 点击开始群聊`;

            const avatarStack = g.members
              .slice(0, 3)
              .map((m) => {
                const meta = resolveMemberMeta(state, m);
                return renderMemberAvatarNode(
                  state,
                  meta.acc,
                  meta.alias || "成员",
                  "group-mini-avatar",
                  meta.ext?.kind
                );
              })
              .join("");

            return `
              <div class="group-row ${active ? "active" : ""}" data-select-group-id="${escapeHtml(g.id)}">
                <div class="group-avatar-stack">
                  ${avatarStack || `<div class="avatar custom group-mini-avatar">群</div>`}
                </div>
                <div class="group-row-meta">
                  <div class="group-row-top">
                    <span class="account-alias">${escapeHtml(g.name)}</span>
                    <span class="group-count-pill">${g.members.length}人</span>
                    ${hasNeedsAuth ? `<span class="quota-warning-badge low">待授权</span>` : isStreaming ? `<span class="loading-status-badge">回复中</span>` : ""}
                  </div>
                  <div class="group-row-preview">${escapeHtml(preview)}</div>
                </div>
                <div class="row-ops">
                  <button class="action-icon-btn" data-edit-group-id="${escapeHtml(g.id)}" title="编辑群成员与名称" type="button">${ICON_EDIT}</button>
                  <button class="action-icon-btn danger" data-delete-group-id="${escapeHtml(g.id)}" title="删除群聊" type="button">${ICON_TRASH}</button>
                </div>
              </div>
            `;
          })
          .join("");

  return `
    <div class="column-header">
      <div class="provider-title-wrap">
        <span class="provider-title">多账号群聊</span>
        ${state.groupChats.length > 0 ? `<span class="count-badge">${state.groupChats.length}</span>` : ""}
      </div>
      <div class="header-actions">
        <button id="manage-external-agents-btn" class="icon-btn ext-header-btn" type="button" title="管理直连 OpenAI API 与本地 CLI 成员">API/CLI${extCount > 0 ? ` (${extCount})` : ""}</button>
        <button id="create-group-btn" class="icon-btn primary" type="button">+ 创建群聊</button>
      </div>
    </div>
    ${noticeHtml}
    <div class="account-list">${listBody}</div>
    ${renderDownloadToast(downloadToast)}
    ${renderModal(state, provider, modal, shouldAnimateModal)}
  `;
}
export function getLatestRoundCompletedReplies(
  state: ManagerSnapshot,
  group: GroupChatRecord
): { userText: string; replies: Array<{ speaker: string; text: string }> } | null {
  let lastUserIdx = -1;
  for (let i = group.messages.length - 1; i >= 0; i -= 1) {
    if (group.messages[i].role === "user") {
      lastUserIdx = i;
      break;
    }
  }
  if (lastUserIdx < 0) return null;
  const replies = group.messages
    .slice(lastUserIdx + 1)
    .filter((m) => m.role === "assistant" && m.status === "done" && m.content.trim().length > 0)
    .map((m) => ({
      speaker: resolveGroupMessageSenderName(state, m),
      text: m.content.trim(),
    }));
  if (replies.length === 0) return null;
  return { userText: group.messages[lastUserIdx].content.trim(), replies };
}
export function renderGroupRightStage(
  state: ManagerSnapshot,
  groupSending: boolean,
  newTopicConfirmGroupId: string | null
): string {
  const activeGroup = state.activeGroupId
    ? state.groupChats.find((g) => g.id === state.activeGroupId)
    : undefined;

  if (!activeGroup) {
    return `
      <main class="right-placeholder">
        <div class="stage-card">
          <div class="stage-logo-tile group-stage-icon">${ICON_GROUP}</div>
          <div class="placeholder-title">创建你的跨服务 AI 群聊</div>
          <div class="placeholder-desc">支持 Muse / Hark 网页账号与 OpenAI API / 本地 CLI 混编入群，内置并行、评审流、A2A 接力与独立会话绑定。</div>
          <div class="stage-card-actions">
            <button id="stage-create-group-btn" class="btn primary" type="button">+ 创建群聊</button>
          </div>
        </div>
      </main>
    `;
  }

  const workflowMode: GroupWorkflowMode = activeGroup.workflowMode ?? "parallel";
  const maxRelayHops = activeGroup.maxRelayHops ?? 2;
  const draftVal = groupDrafts[activeGroup.id] ?? "";
  const selectedTargets = (groupTargets[activeGroup.id] ?? []).filter((t) =>
    activeGroup.members.some((m) => sameAccountKey(m, t))
  );
  const memberEntries = activeGroup.members.map((m) => {
    const meta = resolveMemberMeta(state, m);
    const isNeedsAuth = activeGroup.messages.some(
      (msg) =>
        msg.status === "needs_auth" && msg.sender && sameAccountKey(msg.sender, m)
    );
    const isBusy = activeGroup.messages.some(
      (msg) =>
        (msg.status === "queued" || msg.status === "sending" || msg.status === "streaming") &&
        msg.sender &&
        sameAccountKey(msg.sender, m)
    );
    const hasBinding = !meta.isExternal && Boolean(
      (activeGroup.threadBindings ?? []).some((tb) => sameAccountKey(tb.member, m))
    );
    return {
      key: m,
      isExternal: meta.isExternal,
      acc: meta.acc,
      ext: meta.ext,
      providerShort: meta.providerShort,
      alias: meta.alias,
      loggedOut: meta.loggedOut,
      warm: meta.warm,
      isNeedsAuth,
      isBusy,
      hasBinding,
    };
  });

  const targetCategories = groupByMemberCategory(
    state,
    memberEntries.filter((m) => !m.loggedOut)
  );
  const explicitTargetCats = new Set(
    selectedTargets.map((t) => resolveMemberCategory(state, t).id)
  );
  const selectedAutoIds = (groupAutoChannels[activeGroup.id] ?? []).filter(
    (catId) =>
      !explicitTargetCats.has(catId) &&
      targetCategories.some((c) => c.id === catId && c.items.length >= 2)
  );
  if ((groupAutoChannels[activeGroup.id] ?? []).length !== selectedAutoIds.length) {
    groupAutoChannels[activeGroup.id] = selectedAutoIds;
  }
  const selectedAutoChannels = selectedAutoIds
    .map((id) => {
      const cat = targetCategories.find((c) => c.id === id);
      return cat ? { id: cat.id, label: cat.label } : null;
    })
    .filter((x): x is { id: string; label: string } => Boolean(x));

  const memberAliases = memberEntries.map((m) => m.alias);
  const composerMembers = memberEntries.map((m) => ({
    key: m.key,
    alias: m.alias,
    categoryId: resolveMemberCategory(state, m.key).id,
  }));
  const targetHint = formatComposerHintText(
    draftVal,
    selectedTargets,
    composerMembers,
    workflowMode,
    maxRelayHops,
    selectedAutoChannels
  );
  const knowledgeCount =
    (activeGroup.sharedContext?.trim() ? 1 : 0) +
    (activeGroup.sharedSkills ?? []).filter((s) => s.enabled).length;
  const canPinRound = Boolean(getLatestRoundCompletedReplies(state, activeGroup));

  const msgsHtml =
    activeGroup.messages.length === 0
      ? `
        <div class="group-empty-chat">
          <div class="group-empty-title">群聊已就绪</div>
          <div class="group-empty-desc">可在下方切换「并行」「评审流」或「A2A 接力」协作模式；输入 @ 可模糊点名，网页成员将自动绑定独立专属会话。</div>
        </div>
      `
      : activeGroup.messages
          .map((msg: GroupChatMessage) => {
            if (msg.role === "user") {
              const targetSummary =
                msg.targets.length > 0 && msg.targets.length < activeGroup.members.length
                  ? `<div class="group-msg-targets">@${msg.targets
                      .map((t) => {
                        const meta = resolveMemberMeta(state, t);
                        return meta.alias;
                      })
                      .map(escapeHtml)
                      .join(" · @")}</div>`
                  : "";
              return `
                <div class="group-msg user">
                  <div class="group-bubble user">
                    ${targetSummary}
                    <div class="group-msg-text">${renderMessageTextWithMentions(msg.content, memberAliases)}</div>
                    <div class="group-msg-foot">${formatShortTime(msg.createdAtMs)} · 已转发</div>
                    <div class="group-save-ctx-wrap">
                      <button class="group-pin-icon-btn" data-pin-msg-id="${escapeHtml(msg.id)}" data-tooltip="沉淀到共享上下文" aria-label="沉淀到共享上下文" type="button">${ICON_PIN}</button>
                    </div>
                  </div>
                </div>
              `;
            }

            const senderKey = msg.sender;
            const meta = senderKey ? resolveMemberMeta(state, senderKey) : null;
            const senderDisplayName = resolveGroupMessageSenderName(state, msg);
            const jumpAttrs = senderKey
              ? meta?.isExternal
                ? `data-open-ext-agent-id="${escapeHtml(senderKey.uuid)}"`
                : `data-jump-provider="${escapeHtml(senderKey.providerId)}" data-jump-uuid="${escapeHtml(senderKey.uuid)}"`
              : "";
            const statusPill =
              msg.status === "queued"
                ? `<span class="group-status-pill queued">排队中…</span>`
                : msg.status === "sending"
                  ? `<span class="group-status-pill sending">投递中…</span>`
                  : msg.status === "streaming"
                    ? `<span class="group-status-pill streaming">回复中…</span>`
                    : msg.status === "needs_auth"
                      ? `<span class="group-status-pill warn">待授权确认</span>`
                      : msg.status === "error"
                        ? `<span class="group-status-pill err">未完成</span>`
                        : "";
            const stagePill = msg.stageLabel
              ? `<span class="group-stage-pill">${escapeHtml(msg.stageLabel)}</span>`
              : "";

            const bodyContent = msg.content.trim()
              ? `<div class="group-msg-text">${renderMessageTextWithMentions(msg.content, memberAliases)}</div>`
              : msg.status === "queued"
                ? `<div class="group-msg-pending">等待前序阶段或槽位释放后唤醒…</div>`
                : msg.status === "sending" || msg.status === "streaming"
                  ? `<div class="group-msg-pending">${meta?.isExternal ? "正在直连执行并流式接收回复…" : "正在该账号原生对话中生成回复…"}</div>`
                  : "";

            const canSyncReply = Boolean(
              senderKey &&
                !meta?.isExternal &&
                msg.status !== "queued" &&
                msg.status !== "sending" &&
                msg.status !== "streaming"
            );
            const isSyncing = syncingMsgIds.has(msg.id);
            const syncBtnHtml = canSyncReply
              ? `<button class="group-pin-icon-btn ${isSyncing ? "is-spinning" : ""}" data-sync-msg-id="${escapeHtml(msg.id)}" data-tooltip="同步最新回复（不重发）" aria-label="同步最新回复" type="button" ${isSyncing ? "disabled" : ""}>${ICON_REFRESH}</button>`
              : "";
            const pinBtnHtml = msg.content.trim()
              ? `<button class="group-pin-icon-btn" data-pin-msg-id="${escapeHtml(msg.id)}" data-tooltip="沉淀到共享上下文" aria-label="沉淀到共享上下文" type="button">${ICON_PIN}</button>`
              : "";
            const canCancelExt =
              Boolean(meta?.isExternal) &&
              (msg.status === "streaming" || msg.status === "sending");
            const cancelExtBtnHtml = canCancelExt
              ? `<button class="mini-btn" data-cancel-ext-msg-id="${escapeHtml(msg.id)}" type="button">停止</button>`
              : "";
            const pinHoverAction =
              syncBtnHtml || pinBtnHtml
                ? `<div class="group-save-ctx-wrap ${isSyncing ? "is-active" : ""}">${syncBtnHtml}${pinBtnHtml}</div>`
                : "";

            const authCard =
              msg.status === "needs_auth"
                ? `
                  <div class="group-auth-card">
                    <span>${escapeHtml(msg.error || "检测到安全授权弹窗，请切到该账号确认后重试")}</span>
                    <div class="group-error-ops">
                      <button class="mini-btn primary-text" data-retry-msg-id="${escapeHtml(msg.id)}" type="button">重试</button>
                      ${canSyncReply ? `<button class="mini-btn" data-sync-msg-id="${escapeHtml(msg.id)}" type="button" ${isSyncing ? "disabled" : ""}>${isSyncing ? "同步中…" : "同步回复"}</button>` : ""}
                      ${senderKey ? `<button class="mini-btn warn-btn" type="button" ${jumpAttrs}>前往该账号授权 →</button>` : ""}
                    </div>
                  </div>
                `
                : msg.status === "error" && msg.error
                  ? `
                    <div class="group-error-box">
                      <span>${escapeHtml(msg.error)}</span>
                      <div class="group-error-ops">
                        <button class="mini-btn primary-text" data-retry-msg-id="${escapeHtml(msg.id)}" type="button">重试</button>
                        ${canSyncReply ? `<button class="mini-btn" data-sync-msg-id="${escapeHtml(msg.id)}" type="button" ${isSyncing ? "disabled" : ""}>${isSyncing ? "同步中…" : "同步回复"}</button>` : ""}
                        ${senderKey ? `<button class="mini-btn" type="button" ${jumpAttrs}>${meta?.isExternal ? "检查配置" : "前往查看"}</button>` : ""}
                      </div>
                    </div>
                  `
                  : "";

            return `
              <div class="group-msg assistant">
                <button class="group-msg-avatar-btn" type="button" title="${meta?.isExternal ? `点击编辑「${escapeHtml(senderDisplayName)}」配置` : `点击切换到「${escapeHtml(senderDisplayName)}」原生会话页`}" ${jumpAttrs}>
                  ${renderMemberAvatarNode(state, meta?.acc, senderDisplayName, "", meta?.ext?.kind)}
                </button>
                <div class="group-msg-col">
                  <div class="group-msg-head">
                    <span class="group-msg-name">${escapeHtml(senderDisplayName)}</span>
                    ${meta ? `<span class="group-provider-tag">${escapeHtml(meta.providerShort)}</span>` : ""}
                    ${msg.autoRoutedChannel ? `<span class="group-auto-pill" title="由渠道按剩余额度自动选号">自动选号</span>` : ""}
                    ${stagePill}
                    <span class="group-msg-time">${formatShortTime(msg.createdAtMs)}</span>
                    ${statusPill}
                    ${cancelExtBtnHtml}
                  </div>
                  <div class="group-bubble assistant ${msg.status === "needs_auth" ? "needs-auth" : msg.status === "error" ? "has-error" : ""}">
                    ${bodyContent}
                    ${authCard}
                    ${pinHoverAction}
                  </div>
                </div>
              </div>
            `;
          })
          .join("");

  return `
    <main class="group-chat-stage">
      <div class="group-chat-header">
        <div class="group-header-main">
          <div class="group-header-title-row">
            <h2 class="group-chat-title">${escapeHtml(activeGroup.name)}</h2>
            <span class="count-badge">${activeGroup.members.length} 位成员</span>
          </div>
          <div class="group-member-bar">
            ${groupByMemberCategory(state, memberEntries)
              .map(
                (cat) => `
                  <div class="member-category-group">
                    <span class="member-category-badge" title="${escapeHtml(cat.label)}" aria-label="${escapeHtml(cat.label)}">${renderCategoryIcon(state, cat)}</span>
                    ${cat.items
                      .map((m) => {
                        const chipAttrs = m.isExternal
                          ? `data-open-ext-agent-id="${escapeHtml(m.key.uuid)}"`
                          : `data-jump-provider="${escapeHtml(m.key.providerId)}" data-jump-uuid="${escapeHtml(m.key.uuid)}"`;
                        const chipTitle = m.isExternal
                          ? `点击编辑「${escapeHtml(m.alias)}」直连配置`
                          : m.hasBinding
                            ? `已绑定群专属会话 · 点击进入「${escapeHtml(m.alias)}」(${escapeHtml(cat.label)}) 页面`
                            : `点击进入「${escapeHtml(m.alias)}」(${escapeHtml(cat.label)}) 原生会话页`;
                        return `
                          <button
                            class="group-member-chip ${m.isNeedsAuth ? "warn" : m.isBusy ? "busy" : m.loggedOut ? "offline" : m.warm ? "warm" : ""}"
                            ${chipAttrs}
                            title="${chipTitle}"
                            type="button"
                          >
                            <span class="group-chip-dot"></span>
                            <span>${escapeHtml(m.alias)}</span>
                            ${m.hasBinding ? `<span class="group-chip-provider">会话</span>` : ""}
                          </button>
                        `;
                      })
                      .join("")}
                  </div>
                `
              )
              .join("")}
          </div>
        </div>
        <div class="group-header-actions">
          ${newTopicConfirmGroupId === activeGroup.id
            ? `
              <div class="new-topic-confirm">
                <span class="new-topic-confirm-label">开启新话题：</span>
                <button id="group-new-topic-keep-btn" class="btn primary" type="button" title="清空记录并让网页成员各自新开对话，保留共享上下文与技能">保留共享上下文</button>
                <button id="group-new-topic-clean-btn" class="btn" type="button" title="清空记录、共享上下文，网页成员各自新开对话（技能与角色保留）">全部清空</button>
                <button id="group-new-topic-cancel-btn" class="btn" type="button">取消</button>
              </div>
            `
            : `
          <button id="group-new-topic-btn" class="btn" type="button" title="清空记录并让所有网页成员新开原生对话，开始一个干净的话题">新话题</button>
          ${canPinRound ? `<button id="group-pin-round-btn" class="btn knowledge-btn" type="button" title="将最近一轮各成员完成回复浓缩追加至共享上下文">${ICON_PIN}<span>沉淀本轮结论</span></button>` : ""}
          <button id="group-knowledge-btn" class="btn knowledge-btn" type="button">共享上下文 &amp; 技能${knowledgeCount > 0 ? `<span class="count-badge">${knowledgeCount}</span>` : ""}</button>
          <button id="group-edit-members-btn" class="btn" type="button">编辑成员</button>
          <button id="group-clear-msgs-btn" class="btn" type="button" ${activeGroup.messages.length === 0 ? "disabled" : ""}>清空记录</button>
            `}
        </div>
      </div>
      <div class="group-messages">${msgsHtml}</div>
      <div class="group-composer">
        <div class="group-workflow-bar">
          <div class="workflow-mode-group">
            <span class="workflow-label">模式</span>
            <button class="workflow-mode-btn ${workflowMode === "parallel" ? "active" : ""}" data-workflow-mode="parallel" type="button">并行</button>
            <button class="workflow-mode-btn ${workflowMode === "review" ? "active" : ""}" data-workflow-mode="review" type="button">评审流</button>
            <button class="workflow-mode-btn ${workflowMode === "a2a_relay" ? "active" : ""}" data-workflow-mode="a2a_relay" type="button">A2A接力</button>
          </div>
          ${workflowMode === "a2a_relay" ? `
            <div class="workflow-hops-group">
              <span class="workflow-label">接力上限</span>
              <button class="workflow-hop-btn ${maxRelayHops === 1 ? "active" : ""}" data-relay-hops="1" type="button">1跳</button>
              <button class="workflow-hop-btn ${maxRelayHops === 2 ? "active" : ""}" data-relay-hops="2" type="button">2跳</button>
              <button class="workflow-hop-btn ${maxRelayHops === 3 ? "active" : ""}" data-relay-hops="3" type="button">3跳</button>
            </div>
          ` : ""}
        </div>
        <div class="group-target-chips grouped">
          <button class="group-target-chip ${selectedTargets.length === 0 && selectedAutoIds.length === 0 ? "active" : ""}" data-group-target="all" type="button">全体成员</button>
          ${targetCategories
            .map((cat) => {
              const allCatSelected =
                cat.items.length > 0 &&
                cat.items.every((m) => selectedTargets.some((t) => sameAccountKey(t, m.key)));
              const isAutoActive = selectedAutoIds.includes(cat.id);
              const autoBtnHtml =
                cat.items.length >= 2
                  ? `<button
                      class="group-target-chip target-auto-chip ${isAutoActive ? "active" : ""}"
                      data-group-target-auto="${escapeHtml(cat.id)}"
                      title="${escapeHtml(cat.label)} 按剩余额度自动轮换选号（与该类具体 @ 互斥）"
                      type="button"
                    >自动</button>`
                  : "";
              return `
                <div class="target-category-group">
                  <button
                    class="target-category-badge ${allCatSelected || isAutoActive ? "active" : ""}"
                    data-group-target-cat="${escapeHtml(cat.id)}"
                    title="${escapeHtml(cat.label)} · 点击全选或取消该类全部成员"
                    aria-label="${escapeHtml(cat.label)}"
                    type="button"
                  >${renderCategoryIcon(state, cat)}</button>
                  <div class="target-category-items">
                    ${autoBtnHtml}
                    ${cat.items
                      .map((m) => {
                        const selectedIdx = selectedTargets.findIndex((t) =>
                          sameAccountKey(t, m.key)
                        );
                        const activeTarget = selectedIdx >= 0;
                        const orderPrefix =
                          workflowMode === "review" && activeTarget
                            ? `${selectedIdx + 1}·`
                            : "";
                        return `
                          <button
                            class="group-target-chip ${activeTarget ? "active" : ""}"
                            data-group-target-provider="${escapeHtml(m.key.providerId)}"
                            data-group-target-uuid="${escapeHtml(m.key.uuid)}"
                            type="button"
                          >${orderPrefix}@${escapeHtml(m.alias)}</button>
                        `;
                      })
                      .join("")}
                  </div>
                </div>
              `;
            })
            .join("")}
        </div>
        <div id="group-target-hint" class="group-composer-hint">${escapeHtml(targetHint)}</div>
        <div class="group-input-wrap">
          <div id="group-mention-menu" class="group-mention-menu" hidden></div>
          <div class="group-input-row">
            <textarea
              id="group-chat-input"
              class="group-chat-input"
              rows="2"
              maxlength="4000"
              placeholder="输入消息，输入 @ 可唤起成员点名（Enter 发送，Shift+Enter 换行）"
            >${escapeHtml(draftVal)}</textarea>
            <button id="group-send-btn" class="btn primary group-send-btn" type="button" ${groupSending ? "disabled" : ""}>${groupSending ? "发送中…" : "发送"}</button>
          </div>
        </div>
      </div>
    </main>
  `;
}
