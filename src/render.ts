// Single-provider renderers + scroll position (P2-11 split).
//
// Moved verbatim from `main.ts`: `renderMiddleContent`,
// `renderRightStage`, `renderQuotaDetailPanel`, `renderDownloadToast`,
// `currentProvider`, `saveScrollPosition` and `restoreScrollPosition`.
// Behavior-preserving signature changes: the renderers take the pieces
// of main.ts state they read as same-name parameters (`modal`,
// `optimisticSwitching`, `downloadToast`, `shouldAnimateModal`);
// `saveScrollPosition`/`restoreScrollPosition` take (`snapshot`,
// `viewMode`, `modal`). Shared helpers come from `./state` and the modal
// overlay from `./modals`.

import type {
  AccountKey,
  DownloadNoticeEvent,
  ManagerSnapshot,
  ProviderInfo,
  QuotaDetail,
  ModalMode,
} from "./types";

import type { QuotaStatus } from "./state";
import {
  escapeHtml,
  formatShortExtraText,
  formatShortResetText,
  splitQuotaValue,
  scrollState,
  effectiveSwitchingKey,
  findAccountPageError,
  formatCookieExpiry,
  isMobilePlatform,
  isSameKey,
  computeAvatarText,
  renderProviderLogo,
  resolveQuotaState,
  supportsAccountProxy,
  themeMode,
  themeIcon,
  themeLabel,
  updateState,
  ICON_UPDATE,
  ICON_REFRESH,
  ICON_EXTERNAL,
  ICON_BOLT,
  ICON_SORT,
  ICON_USER_PLUS,
  ICON_EDIT,
  ICON_TRASH,
} from "./state";

import { renderModal } from "./modals";

export function renderQuotaDetailPanel(
  detail: QuotaDetail,
  status: QuotaStatus,
  extraDepleted: boolean
): string {
  const shortVal =
    typeof detail.planUsedPercent === "number"
      ? `${detail.planUsedPercent}%`
      : splitQuotaValue(detail.planValue).main;
  const clamped =
    typeof detail.planUsedPercent === "number"
      ? Math.max(0, Math.min(100, detail.planUsedPercent))
      : null;
  const barHtml =
    clamped !== null
      ? `<div class="quota-bar-track ${status}"><div class="quota-bar-fill ${status} ${clamped > 0 ? "has-fill" : ""}" style="width:${clamped}%"></div></div>`
      : `<div class="quota-compact-spacer"></div>`;

  const rawReset = detail.planSubtext ?? "";
  const shortReset = formatShortResetText(rawReset);
  const shortExtra = formatShortExtraText(detail, extraDepleted);
  const fullExtraTitle = [
    detail.extraLabel,
    detail.extraValue,
    detail.extraSubtext,
  ]
    .filter(Boolean)
    .join(" · ");

  const metaHtml =
    shortReset || shortExtra
      ? `
        <div class="quota-compact-meta">
          <span class="quota-sub-text" title="${escapeHtml(rawReset)}">${escapeHtml(shortReset)}</span>
          ${shortExtra ? `<span class="quota-extra-pill ${status}" title="${escapeHtml(fullExtraTitle)}">${escapeHtml(shortExtra)}</span>` : ""}
        </div>
      `
      : "";

  return `
    <div class="quota-compact-main">
      <span class="quota-plan-label ${status}">${escapeHtml(detail.planLabel)}</span>
      ${barHtml}
      <span class="quota-val ${status}">${escapeHtml(shortVal)}</span>
    </div>
    ${metaHtml}
  `;
}
export function saveScrollPosition(
  snapshot: ManagerSnapshot | null,
  viewMode: "provider" | "groups"
): void {
  if (!snapshot) return;
  const listEl = document.querySelector<HTMLElement>(".account-list");
  if (listEl) {
    if (viewMode === "groups") {
      scrollState.lastGroupListScroll = listEl.scrollTop;
    } else {
      scrollState.scrollOffsets[snapshot.selectedProviderId] = listEl.scrollTop;
    }
  }
  const msgEl = document.querySelector<HTMLElement>(".group-messages");
  if (msgEl && snapshot.activeGroupId) {
    scrollState.groupScrolledToBottom = msgEl.scrollHeight - msgEl.scrollTop - msgEl.clientHeight <= 64;
    scrollState.groupScrollOffsets[snapshot.activeGroupId] = msgEl.scrollTop;
  }
  const modalOverlayEl = document.querySelector<HTMLElement>(".inline-modal");
  if (modalOverlayEl) {
    scrollState.modalOverlayScrollTop = modalOverlayEl.scrollTop;
  }
  const pickerEl = document.querySelector<HTMLElement>(".group-member-picker");
  if (pickerEl) {
    scrollState.groupMemberPickerScrollTop = pickerEl.scrollTop;
  }
  const modalCardEl = document.querySelector<HTMLElement>(".modal-card");
  if (modalCardEl) {
    scrollState.modalCardScrollTop = modalCardEl.scrollTop;
  }
}
export function restoreScrollPosition(
  snapshot: ManagerSnapshot | null,
  viewMode: "provider" | "groups",
  modal: ModalMode
): void {
  if (!snapshot) return;
  const listEl = document.querySelector<HTMLElement>(".account-list");
  if (listEl) {
    listEl.scrollTop =
      viewMode === "groups"
        ? scrollState.lastGroupListScroll
        : (scrollState.scrollOffsets[snapshot.selectedProviderId] ?? 0);
  }
  const msgEl = document.querySelector<HTMLElement>(".group-messages");
  if (msgEl && snapshot.activeGroupId) {
    if (scrollState.forceGroupScrollBottom || scrollState.groupScrolledToBottom) {
      msgEl.scrollTop = msgEl.scrollHeight;
      scrollState.forceGroupScrollBottom = false;
    } else if (snapshot.activeGroupId in scrollState.groupScrollOffsets) {
      msgEl.scrollTop = scrollState.groupScrollOffsets[snapshot.activeGroupId];
    } else {
      msgEl.scrollTop = msgEl.scrollHeight;
    }
  }
  if (modal.kind !== "none") {
    const modalOverlayEl = document.querySelector<HTMLElement>(".inline-modal");
    if (modalOverlayEl) {
      modalOverlayEl.scrollTop = scrollState.modalOverlayScrollTop;
    }
    const pickerEl = document.querySelector<HTMLElement>(".group-member-picker");
    if (pickerEl) {
      pickerEl.scrollTop = scrollState.groupMemberPickerScrollTop;
    }
    const modalCardEl = document.querySelector<HTMLElement>(".modal-card");
    if (modalCardEl) {
      modalCardEl.scrollTop = scrollState.modalCardScrollTop;
    }
  }
}
export function currentProvider(state: ManagerSnapshot): ProviderInfo {
  return (
    state.providers.find((p) => p.id === state.selectedProviderId) ??
    state.providers[0]
  );
}
export function renderMiddleContent(
  state: ManagerSnapshot,
  provider: ProviderInfo,
  modal: ModalMode,
  optimisticSwitching: AccountKey | null,
  downloadToast: DownloadNoticeEvent | null,
  shouldAnimateModal: boolean
): string {
  const providerAccounts = state.accounts.filter((a) => a.providerId === provider.id);
  const switchingKey = effectiveSwitchingKey(state, optimisticSwitching);
  const isMobile = isMobilePlatform(state.platform);
  const supportsProxy = supportsAccountProxy(state.platform);
  const supportsCookieCopy = supportsProxy || isMobile;
  const canReorder = providerAccounts.length > 1 && state.conflicts.length === 0;

  // P0-1：配置加载失败导致写盘被阻止时，给出两个可点的选择
  //（之前只有文案，没有按钮）。继续使用空配置会丢弃损坏的配置
  //（原文件已有 .corrupt-* 备份）；从备份恢复按 .bak、最新
  // .corrupt-* 的顺序逐个尝试。
  const persistActionsHtml = state.persistBlocked
    ? `
        <div class="notice-actions">
          <button id="continue-empty-config-btn" class="notice-action-btn" type="button">继续使用空配置</button>
          <button id="restore-config-backup-btn" class="notice-action-btn" type="button">从备份恢复</button>
        </div>
      `
    : "";
  const noticeHtml = state.notice
    ? `
      <div class="notice-bar">
        <span>${escapeHtml(state.notice)}</span>
        ${persistActionsHtml}
        <button id="dismiss-notice-btn" class="notice-close" type="button" title="关闭提示">✕</button>
      </div>
    `
    : "";

  const conflictsHtml = state.conflicts
    .map(
      (c) => `
      <div class="conflict-box">
        <div>检测到 UUID 归属冲突（${escapeHtml(c.uuid.slice(0, 8))}…），请选择保留归属服务：</div>
        <div class="conflict-actions">
          ${c.providers
            .map(
              (pid) =>
                `<button class="mini-btn" data-resolve-uuid="${escapeHtml(c.uuid)}" data-resolve-provider="${escapeHtml(pid)}" type="button">归属 ${escapeHtml(pid.toUpperCase())}</button>`
            )
            .join("")}
        </div>
      </div>
    `
    )
    .join("");

  const hasUpdate = Boolean(updateState.result?.hasUpdate);
  const mobileThemeBtn = isMobile
    ? `<button id="theme-toggle-btn" class="ghost-icon-btn" type="button" title="主题：${themeLabel(themeMode)}（点击切换）">${themeIcon(themeMode)}</button>`
    : "";
  const mobileUpdateBtn = isMobile
    ? `<button id="check-update-btn" class="ghost-icon-btn update-trigger-btn ${hasUpdate ? "has-update" : ""}" type="button" title="${hasUpdate ? `发现新版本 v${escapeHtml(updateState.result!.latestVersion)}` : "检查更新"}">${ICON_UPDATE}${hasUpdate ? `<span class="update-dot"></span>` : ""}</button>`
    : "";

  const headerBtnHtml = provider.enabled
    ? `
      ${mobileThemeBtn}
      ${mobileUpdateBtn}
      ${!isMobile ? `<button id="sync-all-btn" class="ghost-icon-btn" type="button" title="刷新已就绪账号额度">${ICON_REFRESH}</button>` : ""}
      <button id="open-site-btn" class="ghost-icon-btn" type="button" title="在浏览器中打开 ${escapeHtml(provider.shortName)} 官网">${ICON_EXTERNAL}</button>
      <button id="add-account-btn" class="icon-btn primary" type="button">+ 新增</button>
    `
    : `${mobileThemeBtn}${mobileUpdateBtn}<button id="open-site-btn" class="icon-btn primary" type="button">打开官网 ${ICON_EXTERNAL}</button>`;

  const listToolbarHtml =
    provider.enabled && providerAccounts.length >= 2 && state.conflicts.length === 0
      ? `
        <div class="list-toolbar">
          <button id="smart-switch-btn" class="toolbar-chip" type="button" title="切换到剩余可用额度最多的账号">
            ${ICON_BOLT}<span>切到余量最多</span>
          </button>
          <button id="sort-quota-btn" class="toolbar-chip" type="button" title="按剩余可用额度从高到低排序">
            ${ICON_SORT}<span>按额度排序</span>
          </button>
        </div>
      `
      : "";

  let listBody = "";
  if (!provider.enabled) {
    listBody = `
      <div class="empty-list">
        <div class="empty-icon-circle">${renderProviderLogo(provider)}</div>
        <div><strong>${escapeHtml(provider.name)}</strong> 暂未开放内嵌会话</div>
        <div>待核验官方网页直登路径与嵌入条款后启用，可点击右上角访问官网。</div>
      </div>
    `;
  } else if (providerAccounts.length === 0) {
    listBody = `
      <div class="empty-list">
        <div class="empty-icon-circle">${ICON_USER_PLUS}</div>
        <div>暂无 ${escapeHtml(provider.shortName)} 账号</div>
        <div>点击右上角「+ 新增」创建独立登录环境</div>
      </div>
    `;
  } else {
    listBody = providerAccounts
      .map((acc, idx) => {
        const switching = isSameKey(switchingKey, acc);
        const active = !switching && isSameKey(state.activeAccount, acc);
        const avatarText = computeAvatarText(acc);
        const isLoggedOut = acc.authState === "logged_out";
        const pageError = isLoggedOut ? null : findAccountPageError(state, acc);
        const hasPageError = Boolean(pageError);
        const hasDetail = !isLoggedOut && !hasPageError && Boolean(acc.quotaDetail);
        const cookieInfo = isLoggedOut || hasPageError ? null : formatCookieExpiry(acc);

        const quotaState =
          isLoggedOut || hasPageError
            ? { status: "normal" as QuotaStatus, label: "", extraDepleted: false }
            : resolveQuotaState(acc);
        const {
          status: alertStatus,
          label: alertLabel,
          extraDepleted,
        } = quotaState;

        const isMac = navigator.userAgent.includes("Mac");
        const shortcutHint =
          !isMobile && idx < 9
            ? `<span class="shortcut-pill" title="快捷切号">${isMac ? "⌘" : "^"}${idx + 1}</span>`
            : "";
        const proxyBadgeHtml =
          supportsProxy && acc.proxyUrl
            ? `<span class="proxy-pill" title="代理：${escapeHtml(acc.proxyUrl)}">代理</span>`
            : "";
        const timezoneBadgeHtml = acc.timezone
          ? `<span class="proxy-pill" title="时区：${escapeHtml(acc.timezone)}">${escapeHtml(acc.timezone)}</span>`
          : "";
        const alertBadgeHtml = switching
          ? `<span class="loading-status-badge">加载中…</span>`
          : isLoggedOut
            ? `<span class="auth-status-badge">未登录</span>`
            : hasPageError
              ? `<span class="auth-status-badge error">状态异常</span>`
              : alertStatus !== "normal"
                ? `<span class="quota-warning-badge ${alertStatus}">${alertLabel}</span>`
                : "";

        let compactSubHtml = "";
        if (switching && !hasDetail && !hasPageError) {
          compactSubHtml = `<div class="account-sub loading-text"><span class="quota-label-text">正在加载…</span></div>`;
        } else if (isLoggedOut) {
          const loginHint = active
            ? isMobile
              ? "登录已失效，点击进入重新登录"
              : "登录已失效，请在右侧重新登录"
            : acc.autoAlias
              ? "尚未登录，点击前往登录"
              : "登录已失效，点击重新登录";
          compactSubHtml = `<div class="account-sub auth-logged-out"><span class="auth-dot-inline"></span><span class="quota-label-text">${loginHint}</span></div>`;
        } else if (hasPageError) {
          compactSubHtml = `<div class="account-sub auth-page-error"><span class="auth-dot-inline error"></span><span class="quota-label-text">${escapeHtml(pageError!)}</span><button class="inline-relogin-btn" data-relogin-uuid="${escapeHtml(acc.uuid)}" type="button">清理重登</button></div>`;
        } else if (!hasDetail) {
          const fallbackLabel = acc.autoAlias
            ? "登录后自动同步额度"
            : "正在同步额度明细…";
          const quotaLabel = acc.quotaText ?? fallbackLabel;
          const isEmpty =
            acc.quotaPercent === 0 || Boolean(acc.quotaText?.includes("用尽"));
          const isLow =
            !isEmpty &&
            ((typeof acc.quotaPercent === "number" && acc.quotaPercent <= 20) ||
              Boolean(acc.quotaText?.includes("偏低")));
          const subClass = isEmpty
            ? "account-sub quota-empty"
            : isLow
              ? "account-sub quota-low"
              : "account-sub";
          const barHtml =
            typeof acc.quotaPercent === "number"
              ? `<span class="mini-quota-track"><span class="mini-quota-fill ${isEmpty ? "empty" : isLow ? "low" : ""}" style="width:${Math.max(0, Math.min(100, acc.quotaPercent))}%"></span></span>`
              : "";
          compactSubHtml = `<div class="${subClass}">${barHtml}<span class="quota-label-text">${escapeHtml(quotaLabel)}</span></div>`;
        }

        const rowClass = [
          "account-row",
          hasDetail ? "has-detail" : "",
          active ? "active" : "",
          switching ? "switching" : "",
          isLoggedOut ? "logged-out" : "",
          hasPageError ? "has-page-error" : "",
          alertStatus !== "normal" ? `alert-${alertStatus}` : "",
        ]
          .filter(Boolean)
          .join(" ");

        const avatarClass = [
          "avatar",
          acc.autoAlias ? "pending-identity" : acc.providerId,
        ].join(" ");

        const avatarNodeHtml = acc.avatarUrl
          ? `<div class="avatar has-photo"><img src="${escapeHtml(acc.avatarUrl)}" class="avatar-photo" alt="" draggable="false" /></div>`
          : `<div class="${avatarClass}">${escapeHtml(avatarText)}</div>`;

        const showManualAliasHint =
          !isLoggedOut &&
          !hasPageError &&
          acc.authState === "authenticated" &&
          acc.autoAlias &&
          acc.alias === "已登录账号";

        return `
          <div class="${rowClass}" data-switch-uuid="${escapeHtml(acc.uuid)}" ${!isMobile && canReorder ? `draggable="true"` : ""}>
            <div class="account-header">
              <div class="avatar-wrap">
                ${avatarNodeHtml}
                ${switching ? `<span class="avatar-spinner"></span>` : ""}
                ${active ? `<span class="status-dot ${isLoggedOut ? "offline" : hasPageError ? "error" : ""}"></span>` : ""}
              </div>
              <div class="account-meta">
                <div class="account-alias-row">
                  <span class="account-alias">${escapeHtml(acc.alias)}</span>
                  ${shortcutHint}
                  ${proxyBadgeHtml}
                  ${timezoneBadgeHtml}
                  ${alertBadgeHtml}
                </div>
                ${showManualAliasHint ? `<div class="alias-manual-hint"><span>未获取到邮箱</span><button class="inline-rename-link" data-rename-uuid="${escapeHtml(acc.uuid)}" type="button">手动改名</button></div>` : ""}
                ${compactSubHtml}
                ${!hasDetail && !isLoggedOut && !hasPageError && !switching && cookieInfo ? `<div class="cookie-expiry-line inline ${cookieInfo.tone}" title="${escapeHtml(cookieInfo.tooltip)}"><span class="cookie-expiry-dot"></span><span>${escapeHtml(cookieInfo.shortText)}</span></div>` : ""}
              </div>
              <div class="row-ops">
                ${canReorder ? `
                  <div class="reorder-stack">
                    <button class="reorder-btn" data-move-uuid="${escapeHtml(acc.uuid)}" data-move-dir="up" title="上移" aria-label="上移" type="button" ${idx === 0 ? "disabled" : ""}>▲</button>
                    <button class="reorder-btn" data-move-uuid="${escapeHtml(acc.uuid)}" data-move-dir="down" title="下移" aria-label="下移" type="button" ${idx === providerAccounts.length - 1 ? "disabled" : ""}>▼</button>
                  </div>
                ` : ""}
                ${!isMobile ? `
                  <button class="action-icon-btn ${switching ? "spinning" : ""}" data-reload-uuid="${escapeHtml(acc.uuid)}" title="强制刷新页面" type="button" ${switching ? "disabled" : ""}>${ICON_REFRESH}</button>
                ` : ""}
                ${supportsCookieCopy ? `
                  <button class="action-icon-btn copy-cookie-btn" data-copy-cookie-uuid="${escapeHtml(acc.uuid)}" data-copy-label="复制 Cookie" aria-label="复制 Cookie" type="button">
                    <span class="copy-cookie-icon" aria-hidden="true"></span>
                  </button>
                ` : ""}
                <button class="action-icon-btn" data-rename-uuid="${escapeHtml(acc.uuid)}" title="账号设置" type="button">${ICON_EDIT}</button>
                <button class="action-icon-btn danger" data-remove-uuid="${escapeHtml(acc.uuid)}" title="从本机移除" type="button">${ICON_TRASH}</button>
              </div>
            </div>
            ${!isLoggedOut && !hasPageError && acc.quotaDetail ? renderQuotaDetailPanel(acc.quotaDetail, alertStatus, extraDepleted) : ""}
            ${hasDetail && cookieInfo ? `<div class="cookie-expiry-line ${cookieInfo.tone}" title="${escapeHtml(cookieInfo.tooltip)}"><span class="cookie-expiry-dot"></span><span>${escapeHtml(cookieInfo.shortText)}</span></div>` : ""}
          </div>
        `;
      })
      .join("");
  }

  return `
    <div class="column-header">
      <div class="provider-title-wrap">
        <span class="provider-title">${escapeHtml(provider.name)}</span>
        ${
          !provider.enabled
            ? `<span class="status-pill">${escapeHtml(provider.statusText)}</span>`
            : providerAccounts.length > 0
              ? `<span class="count-badge">${providerAccounts.length}</span>`
              : ""
        }
      </div>
      <div class="header-actions">${headerBtnHtml}</div>
    </div>
    ${listToolbarHtml}
    ${noticeHtml}
    ${conflictsHtml}
    <div class="account-list">${listBody}</div>
    ${renderDownloadToast(downloadToast)}
    ${renderModal(state, provider, modal, shouldAnimateModal)}
  `;
}
export function renderDownloadToast(downloadToast: DownloadNoticeEvent | null): string {
  if (!downloadToast) return "";
  const isStarted = downloadToast.status === "started";
  const isDone = downloadToast.status === "completed";
  const isFail = downloadToast.status === "failed";
  const label = isStarted
    ? `正在下载：${downloadToast.filename}`
    : isDone
      ? `下载完成：${downloadToast.filename}`
      : `下载失败：${downloadToast.filename}${downloadToast.error ? `（${downloadToast.error}）` : ""}`;
  return `
    <div id="download-toast" class="download-toast ${isFail ? "error" : isDone ? "success" : ""}" role="status" aria-live="polite">
      <span class="download-toast-text" title="${escapeHtml(label)}">${escapeHtml(label)}</span>
      ${isDone ? `<button id="download-reveal-btn" class="download-toast-action" data-download-id="${downloadToast.id}" type="button">在文件夹中显示</button>` : ""}
      <button id="download-close-btn" class="download-toast-close" type="button" aria-label="关闭下载提示">✕</button>
    </div>
  `;
}
export function renderRightStage(
  state: ManagerSnapshot,
  provider: ProviderInfo,
  optimisticSwitching: AccountKey | null
): string {
  const switchingKey = effectiveSwitchingKey(state, optimisticSwitching);
  const switchingAcc = switchingKey
    ? state.accounts.find(
        (a) => a.providerId === switchingKey.providerId && a.uuid === switchingKey.uuid
      )
    : undefined;

  if (switchingKey) {
    const targetName = switchingAcc?.alias ?? provider.shortName;
    return `
      <main class="right-placeholder">
        <div class="stage-card">
          <div class="stage-logo-orbit">
            <span class="stage-orbit-ring"></span>
            <div class="stage-logo-tile">${renderProviderLogo(provider)}</div>
          </div>
          <div class="placeholder-title">正在加载「${escapeHtml(targetName)}」</div>
          <div class="placeholder-desc">首次打开正在渲染 ${escapeHtml(provider.shortName)} 页面，加载完成后再次切换将瞬间直达…</div>
          <div class="progress-track"><div class="progress-bar"></div></div>
        </div>
      </main>
    `;
  }

  if (!provider.enabled) {
    return `
      <main class="right-placeholder">
        <div class="stage-card">
          <div class="stage-logo-tile">${renderProviderLogo(provider)}</div>
          <div class="placeholder-title">${escapeHtml(provider.name)} · 待接入</div>
          <div class="placeholder-desc">首版暂未启用应用内嵌会话。访问官网不会覆盖上次已打开的账号记录。</div>
          <button id="stage-open-site-btn" class="btn primary" type="button">打开 ${escapeHtml(provider.shortName)} 官网</button>
        </div>
      </main>
    `;
  }

  return `
    <main class="right-placeholder">
      <div class="stage-card">
        <div class="stage-logo-tile">${renderProviderLogo(provider)}</div>
        <div class="placeholder-title">${escapeHtml(provider.name)}</div>
        <div class="placeholder-desc">从左侧选择一个账号进入对话，或新建账号开启独立隔离登录。</div>
        <button id="stage-add-btn" class="btn primary" type="button">+ 新增 ${escapeHtml(provider.shortName)} 账号</button>
      </div>
    </main>
  `;
}
