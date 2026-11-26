// Shared frontend state + pure helpers (P2-11 split).
//
// Moved verbatim from `main.ts`: the static constants (presets, icons),
// the theme state + functions, the shared mutable collections
// (`groupDrafts`, `groupTargets`, `groupAutoChannels`, `syncingMsgIds`,
// `copyingCookies`, `updateState`), the scroll state object, and the pure
// helpers (formatting, quota, member/category resolution, avatar/logo
// rendering). The collections are mutated through their properties, so
// sharing the exported object across modules preserves behavior exactly.
// `syncViewModeFromSnapshot` now returns the next view mode (or null for
// "no change") instead of assigning the `viewMode` global in `main.ts`,
// and `effectiveSwitchingKey` takes `optimisticSwitching` as a parameter;
// both are behavior-preserving signature changes required by the split.

import type {
  AccountKey,
  AccountRecord,
  ExternalAgentKind,
  ExternalAgentView,
  GroupChatMessage,
  GroupChatRecord,
  GroupSharedSkill,
  ManagerSnapshot,
  ProviderId,
  ProviderInfo,
  QuotaDetail,
  ThemeMode,
  UpdateCheckResult,
  UpdateProgressEvent,
  ProviderLogoSpec,
} from "./types";

export const scrollState = {
  lastGroupListScroll: 0,
  modalOverlayScrollTop: 0,
  groupMemberPickerScrollTop: 0,
  modalCardScrollTop: 0,
  groupScrollOffsets: {} as Record<string, number>,
  groupScrolledToBottom: true,
  forceGroupScrollBottom: false,
  scrollOffsets: {} as Record<string, number>,
};

export const TIMEZONE_PRESETS: Array<{ label: string; value: string }> = [
  { label: "跟随系统", value: "" },
  { label: "纽约", value: "America/New_York" },
  { label: "洛杉矶", value: "America/Los_Angeles" },
  { label: "伦敦", value: "Europe/London" },
  { label: "东京", value: "Asia/Tokyo" },
  { label: "新加坡", value: "Asia/Singapore" },
  { label: "上海", value: "Asia/Shanghai" },
  { label: "UTC", value: "UTC" },
];
export const PROVIDER_LOGOS: Record<string, ProviderLogoSpec> = {
  muse: { src: "/logos/muse.png", fullBleed: true },
  hark: { src: "/logos/hark.svg", fullBleed: false, monoDark: true },
};
export const GROUP_SKILL_PRESETS: Array<Omit<GroupSharedSkill, "id" | "enabled">> = [
  {
    name: "极致简洁中文",
    description: "约束各成员直接给结论和关键点，禁止寒暄和铺垫。",
    content: "统一使用简体中文回复；直接给结论、关键依据和可执行方案，禁止客套和冗余铺垫。",
  },
  {
    name: "架构与风险评审",
    description: "从稳定性、边界情况、安全性和性能四个维度输出意见。",
    content: "分析技术方案时优先检查边界情况、并发竞争、权限安全与资源占用，明确指出取舍与潜在风险。",
  },
  {
    name: "交叉事实核查",
    description: "要求先指出前序发言或前提中的漏洞，再补充新结论。",
    content: "若共享上下文或前序成员发言存在矛盾、假设漏洞或遗漏，请先指出修正，再给出补充判断。",
  },
];
export const EXTERNAL_AGENT_PRESETS: Array<{
  label: string;
  kind: ExternalAgentKind;
  name: string;
  baseUrl: string;
  model: string;
  command: string;
  args: string;
}> = [
  {
    label: "OpenAI",
    kind: "openai_api",
    name: "GPT-4o",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4o",
    command: "",
    args: "",
  },
  {
    label: "DeepSeek",
    kind: "openai_api",
    name: "DeepSeek",
    baseUrl: "https://api.deepseek.com/v1",
    model: "deepseek-chat",
    command: "",
    args: "",
  },
  {
    label: "Ollama 本地",
    kind: "openai_api",
    name: "Ollama-Qwen",
    baseUrl: "http://127.0.0.1:11434/v1",
    model: "qwen2.5:14b",
    command: "",
    args: "",
  },
  {
    label: "Claude CLI",
    kind: "local_cli",
    name: "Claude-Code",
    baseUrl: "",
    model: "",
    command: "claude",
    args: "-p",
  },
  {
    label: "Codex CLI",
    kind: "local_cli",
    name: "Codex-CLI",
    baseUrl: "",
    model: "",
    command: "codex",
    args: "exec {prompt}",
  },
];
export const ICON_EXTERNAL = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>`;
export const ICON_REFRESH = `<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.5 2v6h-6M21.34 15.57a10 10 0 1 1-.57-8.38l5.67-5.67"/></svg>`;
export const ICON_EDIT = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"/></svg>`;
export const ICON_TRASH = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>`;
export const ICON_USER_PLUS = `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><line x1="19" y1="8" x2="19" y2="14"/><line x1="22" y1="11" x2="16" y2="11"/></svg>`;
export const ICON_UPDATE = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>`;
export const ICON_SETTINGS = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>`;
export const ICON_THEME_DARK = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>`;
export const ICON_THEME_LIGHT = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><line x1="12" y1="2" x2="12" y2="4"/><line x1="12" y1="20" x2="12" y2="22"/><line x1="4.93" y1="4.93" x2="6.34" y2="6.34"/><line x1="17.66" y1="17.66" x2="19.07" y2="19.07"/><line x1="2" y1="12" x2="4" y2="12"/><line x1="20" y1="12" x2="22" y2="12"/><line x1="6.34" y1="17.66" x2="4.93" y2="19.07"/><line x1="19.07" y1="4.93" x2="17.66" y2="6.34"/></svg>`;
export const ICON_THEME_SYSTEM = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>`;
export const ICON_BOLT = `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg>`;
export const ICON_SORT = `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M6 12h12"/><path d="M10 18h4"/></svg>`;
export const ICON_GROUP = `<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/><path d="M8 10h.01"/><path d="M12 10h.01"/><path d="M16 10h.01"/></svg>`;
export const ICON_PIN = `<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m19 21-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v16z"/><line x1="12" x2="12" y1="7" y2="13"/><line x1="15" x2="9" y1="10" y2="10"/></svg>`;
export const syncingMsgIds = new Set<string>();
export const THEME_STORAGE_KEY = "maam_theme_mode";
export const systemDarkQuery = window.matchMedia("(prefers-color-scheme: dark)");
export let themeMode: ThemeMode = loadSavedThemeMode();
export function loadSavedThemeMode(): ThemeMode {
  try {
    const raw = localStorage.getItem(THEME_STORAGE_KEY);
    if (raw === "light" || raw === "dark" || raw === "system") {
      return raw;
    }
  } catch {
    // ignore storage errors
  }
  return "system";
}

export function effectiveTheme(): "light" | "dark" {
  if (themeMode === "system") {
    return systemDarkQuery.matches ? "dark" : "light";
  }
  return themeMode;
}

export function applyTheme(): void {
  document.documentElement.dataset.theme = effectiveTheme();
}

export function themeLabel(mode: ThemeMode): string {
  if (mode === "dark") return "深色";
  if (mode === "light") return "浅色";
  return "跟随";
}

export function themeIcon(mode: ThemeMode): string {
  if (mode === "dark") return ICON_THEME_DARK;
  if (mode === "light") return ICON_THEME_LIGHT;
  return ICON_THEME_SYSTEM;
}

export function cycleThemeMode(afterChange?: () => void): void {
  themeMode = themeMode === "system" ? "light" : themeMode === "light" ? "dark" : "system";
  try {
    localStorage.setItem(THEME_STORAGE_KEY, themeMode);
  } catch {
    // ignore storage errors
  }
  applyTheme();
  afterChange?.();
}
export const groupDrafts: Record<string, string> = {};
export const groupTargets: Record<string, AccountKey[]> = {};
export const groupAutoChannels: Record<string, string[]> = {};
export const copyingCookies = new Set<string>();
export const updateState: {
  checking: boolean;
  installing: boolean;
  result: UpdateCheckResult | null;
  progress: UpdateProgressEvent | null;
  error: string | null;
  outcomeStatus: string | null;
  outcomeMessage: string | null;
  lastCheckedAt: number;
  lastFailedAt: number;
} = {
  checking: false,
  installing: false,
  result: null,
  progress: null,
  error: null,
  outcomeStatus: null,
  outcomeMessage: null,
  lastCheckedAt: 0,
  lastFailedAt: 0,
};
export function isAsciiWordChar(ch: string | undefined): boolean {
  return Boolean(ch && /^[A-Za-z0-9_]$/.test(ch));
}

export function isSameKey(a: AccountKey | null, b: AccountRecord): boolean {
  return Boolean(a && a.providerId === b.providerId && a.uuid === b.uuid);
}

export function sameAccountKey(a: AccountKey, b: AccountKey): boolean {
  return a.providerId === b.providerId && a.uuid === b.uuid;
}

export function isMobilePlatform(platform: ManagerSnapshot["platform"] | undefined): boolean {
  return platform === "android" || platform === "ios";
}

export function syncViewModeFromSnapshot(
  state: ManagerSnapshot | null
): "provider" | "groups" | null {
  if (!state) return null;
  if (isMobilePlatform(state.platform)) return "provider";
  if (state.activeGroupId) return "groups";
  if (state.activeAccount || state.switchingAccount) return "provider";
  return null;
}

export function findAccountPageError(
  state: ManagerSnapshot | null,
  key: AccountKey
): string | null {
  if (!state?.accountErrors) return null;
  const hit = state.accountErrors.find(
    (e) => e.providerId === key.providerId && e.uuid === key.uuid
  );
  return hit && hit.message.trim() ? hit.message.trim() : null;
}

export function isAccountGroupEligible(state: ManagerSnapshot, acc: AccountRecord): boolean {
  const provider = state.providers.find((p) => p.id === acc.providerId);
  if (!provider || !provider.enabled) return false;
  if (acc.authState !== "authenticated") return false;
  if (typeof acc.cookieExpiresAtMs === "number" && acc.cookieExpiresAtMs <= Date.now()) {
    return false;
  }
  if (findAccountPageError(state, acc)) return false;
  return true;
}

export function formatShortTime(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function groupHasNeedsAuth(group: GroupChatRecord): boolean {
  return group.messages.some((m) => m.status === "needs_auth");
}

export function isExternalProviderId(providerId: ProviderId): boolean {
  return providerId === "api" || providerId === "cli";
}

export function findAccountRecord(state: ManagerSnapshot, key: AccountKey | null): AccountRecord | undefined {
  if (!key || isExternalProviderId(key.providerId)) return undefined;
  return state.accounts.find((a) => a.providerId === key.providerId && a.uuid === key.uuid);
}

export function findExternalAgent(state: ManagerSnapshot, key: AccountKey | null): ExternalAgentView | undefined {
  if (!key || !isExternalProviderId(key.providerId)) return undefined;
  return (state.externalAgents ?? []).find((e) => e.id === key.uuid);
}

export function resolveMemberMeta(
  state: ManagerSnapshot,
  key: AccountKey
): {
  key: AccountKey;
  isExternal: boolean;
  acc?: AccountRecord;
  ext?: ExternalAgentView;
  alias: string;
  providerShort: string;
  loggedOut: boolean;
  pageError: string | null;
  warm: boolean;
} {
  if (isExternalProviderId(key.providerId)) {
    const ext = findExternalAgent(state, key);
    return {
      key,
      isExternal: true,
      ext,
      alias: ext?.name ?? key.uuid.slice(0, 6),
      providerShort: key.providerId === "api" ? "API" : "CLI",
      loggedOut: !ext,
      pageError: null,
      warm: Boolean(ext),
    };
  }
  const acc = findAccountRecord(state, key);
  const pInfo = state.providers.find((p) => p.id === key.providerId);
  const loggedOut = !acc || acc.authState === "logged_out";
  return {
    key,
    isExternal: false,
    acc,
    alias: acc?.alias ?? key.uuid.slice(0, 6),
    providerShort: pInfo?.shortName ?? key.providerId.toUpperCase(),
    loggedOut,
    pageError: loggedOut ? null : findAccountPageError(state, key),
    warm: state.warmAccounts.some((w) => sameAccountKey(w, key)),
  };
}

export function resolveMemberCategory(
  state: ManagerSnapshot,
  key: AccountKey
): { id: string; label: string } {
  if (key.providerId === "api") {
    return { id: "api", label: "OpenAI API" };
  }
  if (key.providerId === "cli") {
    return { id: "cli", label: "本地 CLI" };
  }
  const pInfo = state.providers.find((p) => p.id === key.providerId);
  return {
    id: pInfo?.id ?? key.providerId,
    label: pInfo?.shortName ?? key.providerId.toUpperCase(),
  };
}

export function groupByMemberCategory<T extends { key: AccountKey }>(
  state: ManagerSnapshot,
  items: T[]
): Array<{ id: string; label: string; items: T[] }> {
  const buckets = new Map<string, { id: string; label: string; items: T[] }>();
  for (const item of items) {
    const cat = resolveMemberCategory(state, item.key);
    let bucket = buckets.get(cat.id);
    if (!bucket) {
      bucket = { ...cat, items: [] };
      buckets.set(cat.id, bucket);
    }
    bucket.items.push(item);
  }
  const orderedIds: string[] = [
    ...state.providers.map((p) => p.id),
    "api",
    "cli",
  ];
  const result: Array<{ id: string; label: string; items: T[] }> = [];
  for (const id of orderedIds) {
    const b = buckets.get(id);
    if (b && b.items.length > 0) {
      result.push(b);
      buckets.delete(id);
    }
  }
  for (const b of buckets.values()) {
    if (b.items.length > 0) {
      result.push(b);
    }
  }
  return result;
}

export function resolveGroupMessageSenderName(state: ManagerSnapshot, msg: GroupChatMessage): string {
  if (msg.sender && isExternalProviderId(msg.sender.providerId)) {
    return findExternalAgent(state, msg.sender)?.name || msg.senderName;
  }
  const acc = findAccountRecord(state, msg.sender);
  return acc?.alias || msg.senderName;
}

export function renderMemberAvatarNode(
  _state: ManagerSnapshot,
  acc: AccountRecord | undefined,
  fallbackName: string,
  extraClass = "",
  externalKind?: ExternalAgentKind
): string {
  if (externalKind) {
    const extCls = externalKind === "openai_api" ? "ext-api" : "ext-cli";
    const badge = externalKind === "openai_api" ? "API" : "CLI";
    return `<div class="avatar ${extCls} ${escapeHtml(extraClass)}">${badge}</div>`;
  }
  if (acc?.avatarUrl) {
    return `<div class="avatar has-photo ${escapeHtml(extraClass)}"><img src="${escapeHtml(acc.avatarUrl)}" class="avatar-photo" alt="" draggable="false" /></div>`;
  }
  const cls = acc ? acc.providerId : "custom";
  const initial = Array.from((acc?.alias || fallbackName).trim())[0]?.toUpperCase() || "#";
  return `<div class="avatar ${cls} ${escapeHtml(extraClass)}">${escapeHtml(initial)}</div>`;
}

export function effectiveSwitchingKey(
  state: ManagerSnapshot,
  optimisticSwitching: AccountKey | null
): AccountKey | null {
  return state.switchingAccount ?? optimisticSwitching;
}

export function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export function renderProviderLogo(provider: ProviderInfo, extraClass = ""): string {
  const cfg = PROVIDER_LOGOS[provider.id];
  if (cfg) {
    const cls = `${extraClass} ${cfg.fullBleed ? "full-bleed" : ""} ${cfg.monoDark ? "mono-dark-logo" : ""}`.trim();
    return `<img src="${cfg.src}" alt="${escapeHtml(provider.shortName)}" class="${cls}" draggable="false" />`;
  }
  const initial = Array.from(provider.shortName.trim())[0]?.toUpperCase() || "A";
  return `<span class="group-provider-tag ${escapeHtml(extraClass)}">${escapeHtml(initial)}</span>`;
}

export function renderCategoryIcon(
  state: ManagerSnapshot,
  cat: { id: string; label: string }
): string {
  if (cat.id === "api") {
    return `<span class="cat-ext-icon ext-api" aria-hidden="true">API</span>`;
  }
  if (cat.id === "cli") {
    return `<span class="cat-ext-icon ext-cli" aria-hidden="true">&gt;_</span>`;
  }
  const provider = state.providers.find((p) => p.id === cat.id);
  if (provider) {
    return renderProviderLogo(provider, "cat-logo-img");
  }
  const initial = Array.from(cat.label.trim())[0]?.toUpperCase() || "A";
  return `<span class="group-provider-tag">${escapeHtml(initial)}</span>`;
}

export function computeAvatarText(acc: AccountRecord): string {
  if (acc.autoAlias) {
    return "@";
  }
  const clean = acc.alias.trim();
  const first = Array.from(clean)[0];
  return first ? first.toUpperCase() : "#";
}

export function splitQuotaValue(raw: string): { main: string; note: string | null } {
  const m = raw.trim().match(/^(.*?)\s*[（(]([^）)]+)[）)]\s*$/);
  if (m && m[1].trim()) {
    return { main: m[1].trim(), note: m[2].trim() };
  }
  return { main: raw.trim(), note: null };
}

export type QuotaStatus = "normal" | "extra" | "low" | "critical";

export function resolveQuotaState(acc: AccountRecord): {
  status: QuotaStatus;
  label: string;
  extraDepleted: boolean;
} {
  if (acc.quotaDetail) {
    const d = acc.quotaDetail;
    const planUsed = d.planUsedPercent;
    const planVal = d.planValue || "";
    const planDepleted =
      (planUsed !== null && planUsed >= 100) ||
      planVal.includes("用尽") ||
      planVal.includes("DEPLETED");
    const planLow =
      !planDepleted &&
      ((planUsed !== null && planUsed >= 85) ||
        planVal.includes("偏低") ||
        planVal.includes("LOW"));
    const hasExtra = Boolean(
      d.extraLabel || d.extraValue || d.extraUsedPercent !== null
    );
    const extraUsed = d.extraUsedPercent;
    const extraVal = d.extraValue || "";
    const extraDepleted =
      hasExtra &&
      ((extraUsed !== null && extraUsed >= 100) ||
        extraVal.includes("用尽") ||
        extraVal.includes("耗尽") ||
        extraVal.includes("DEPLETED") ||
        /剩余\s*0(\s|个|口|$)/.test(extraVal));
    const extraLow =
      hasExtra &&
      !extraDepleted &&
      ((extraUsed !== null && extraUsed >= 85) ||
        extraVal.includes("偏低") ||
        extraVal.includes("LOW"));
    const extraIsCap = Boolean(d.extraLabel && /月|monthly/i.test(d.extraLabel));

    if (extraIsCap) {
      if (planDepleted) {
        return { status: "critical", label: "日满", extraDepleted };
      }
      if (extraDepleted) {
        return { status: "critical", label: "月满", extraDepleted };
      }
      if (planLow || extraLow) {
        return { status: "low", label: "偏低", extraDepleted };
      }
      return { status: "normal", label: "", extraDepleted };
    }

    if (planDepleted) {
      if (!hasExtra || extraDepleted) {
        return { status: "critical", label: "用尽", extraDepleted };
      }
      if (extraLow) {
        return { status: "low", label: "偏低", extraDepleted };
      }
      return { status: "extra", label: "用额外", extraDepleted };
    }
    if (planLow) {
      if (!hasExtra || extraDepleted) {
        return { status: "low", label: "偏低", extraDepleted };
      }
      return { status: "normal", label: "", extraDepleted };
    }
    return { status: "normal", label: "", extraDepleted };
  }

  if (acc.quotaPercent === 0 || Boolean(acc.quotaText?.includes("用尽"))) {
    return { status: "critical", label: "用尽", extraDepleted: false };
  }
  if (
    (typeof acc.quotaPercent === "number" && acc.quotaPercent <= 15) ||
    Boolean(acc.quotaText?.includes("偏低"))
  ) {
    return { status: "low", label: "偏低", extraDepleted: false };
  }
  return { status: "normal", label: "", extraDepleted: false };
}

export function scoreAccountAvailability(
  acc: AccountRecord,
  pageError?: string | null
): {
  tier: number;
  score: number;
  available: boolean;
} {
  if (acc.authState === "logged_out" || pageError) {
    return { tier: 0, score: -1, available: false };
  }
  const q = resolveQuotaState(acc);
  if (q.status === "critical") {
    return { tier: 1, score: 0, available: false };
  }
  const d = acc.quotaDetail;
  if (d) {
    if (d.planUsedPercent === null && /无限|UNLIMITED/i.test(d.planValue)) {
      return { tier: 6, score: 100, available: true };
    }
    const extraIsCap = Boolean(d.extraLabel && /月|monthly/i.test(d.extraLabel));
    if (q.status === "extra") {
      const extraRem =
        typeof d.extraUsedPercent === "number" ? Math.max(0, 100 - d.extraUsedPercent) : 50;
      return { tier: 2, score: 200 + extraRem, available: true };
    }
    let rem =
      typeof d.planUsedPercent === "number"
        ? Math.max(0, 100 - d.planUsedPercent)
        : typeof acc.quotaPercent === "number"
          ? acc.quotaPercent
          : 50;
    if (extraIsCap && typeof d.extraUsedPercent === "number") {
      rem = Math.min(rem, Math.max(0, 100 - d.extraUsedPercent));
    }
    return { tier: q.status === "low" ? 3 : 5, score: rem, available: true };
  }
  if (typeof acc.quotaPercent === "number") {
    return { tier: q.status === "low" ? 3 : 5, score: acc.quotaPercent, available: true };
  }
  if (acc.authState === "authenticated") {
    return { tier: 4, score: 50, available: true };
  }
  return { tier: 2, score: 50, available: true };
}

export function compareAccountAvailability(
  a: AccountRecord,
  b: AccountRecord,
  state?: ManagerSnapshot | null
): number {
  const sa = scoreAccountAvailability(a, state ? findAccountPageError(state, a) : null);
  const sb = scoreAccountAvailability(b, state ? findAccountPageError(state, b) : null);
  if (sb.tier !== sa.tier) return sb.tier - sa.tier;
  return sb.score - sa.score;
}

export function formatShortResetText(raw: string | null): string {
  const clean = (raw ?? "").trim();
  if (!clean) return "";
  const m = clean.match(/^(?:每周限额将在|周限额重置时间[:：]?|将在)\s*(.+?)\s*$/);
  if (m && m[1]) {
    const inner = m[1].trim();
    return inner.endsWith("重置") ? inner : `${inner}重置`;
  }
  return clean;
}

export function formatShortExtraText(
  detail: QuotaDetail,
  extraDepleted: boolean
): string | null {
  if (
    !detail.extraLabel &&
    !detail.extraValue &&
    detail.extraUsedPercent === null
  ) {
    return null;
  }
  const prefix = detail.extraLabel && /月|monthly/i.test(detail.extraLabel) ? "月" : "额外";
  if (extraDepleted) {
    return `${prefix} 100% · 已耗尽`;
  }
  const parsed = splitQuotaValue(detail.extraValue ?? "");
  const pctPart =
    typeof detail.extraUsedPercent === "number"
      ? `${detail.extraUsedPercent}%`
      : parsed.main.replace(/^已使用\s*/, "").trim();
  const notePart = parsed.note
    ? parsed.note
        .replace(/^剩余\s*/, "余 ")
        .replace(/个?口令$/, "")
        .replace(/\s*tokens?$/i, "")
        .trim()
    : "";
  if (pctPart && notePart) {
    return `${prefix} ${pctPart} · ${notePart}`;
  }
  if (notePart) {
    return `${prefix} · ${notePart}`;
  }
  if (pctPart) {
    return `${prefix} ${pctPart}`;
  }
  return detail.extraLabel ?? "额外额度";
}

type CookieExpiryTone = "normal" | "warn" | "critical" | "expired" | "session";

interface CookieExpiryDisplay {
  tone: CookieExpiryTone;
  shortText: string;
  detailText: string;
  tooltip: string;
}

export function formatLocalDateTime(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function formatCookieExpiry(acc: AccountRecord): CookieExpiryDisplay | null {
  if (acc.authState === "logged_out") return null;

  if (typeof acc.cookieExpiresAtMs === "number") {
    const diffMs = acc.cookieExpiresAtMs - Date.now();
    const absStr = formatLocalDateTime(acc.cookieExpiresAtMs);
    if (diffMs <= 0) {
      return {
        tone: "expired",
        shortText: "Cookie 已过期",
        detailText: `Cookie 已于 ${absStr} 过期（需重新登录或刷新）`,
        tooltip: `Cookie 过期时间：${absStr}`,
      };
    }

    const days = Math.floor(diffMs / 86_400_000);
    const hours = Math.floor((diffMs % 86_400_000) / 3_600_000);
    const mins = Math.max(1, Math.floor((diffMs % 3_600_000) / 60_000));

    const shortText =
      days >= 365
        ? `Cookie 约 ${(days / 365).toFixed(1).replace(/\.0$/, "")} 年`
        : days >= 1
          ? `Cookie 剩 ${days} 天`
          : hours >= 1
            ? `Cookie 剩 ${hours} 小时`
            : `Cookie 剩 ${mins} 分钟`;

    const fullRem =
      days >= 1 && hours > 0
        ? `${days} 天 ${hours} 小时`
        : days >= 1
          ? `${days} 天`
          : hours >= 1
            ? `${hours} 小时 ${mins} 分钟`
            : `${mins} 分钟`;

    const tone: CookieExpiryTone =
      diffMs <= 24 * 3_600_000
        ? "critical"
        : diffMs <= 7 * 86_400_000
          ? "warn"
          : "normal";

    return {
      tone,
      shortText,
      detailText: `Cookie 有效期至 ${absStr}（剩余 ${fullRem}）`,
      tooltip: `Cookie 有效期至 ${absStr}`,
    };
  }

  if (acc.cookieSessionOnly) {
    return {
      tone: "session",
      shortText: "仅本次会话有效",
      detailText: "会话级 Cookie（关闭应用后可能需重新登录）",
      tooltip: "浏览器会话级 Cookie，未设置固定过期时间",
    };
  }

  return null;
}
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let idx = 0;
  let val = bytes;
  while (val >= 1024 && idx < units.length - 1) {
    val /= 1024;
    idx += 1;
  }
  return idx === 0 ? `${Math.round(val)} B` : `${val.toFixed(1)} ${units[idx]}`;
}

export function formatProgressSummary(evt: UpdateProgressEvent): string {
  if (evt.stage === "verifying") return "校验中";
  if (evt.stage === "installing") return "正在替换";
  if (evt.stage === "ready") return "完成";
  if (evt.stage === "needs_permission") return "待授权";
  const pct = typeof evt.percent === "number" ? `${evt.percent}%` : "";
  const transferred = evt.total
    ? `${formatBytes(evt.downloaded)} / ${formatBytes(evt.total)}`
    : evt.downloaded > 0
      ? formatBytes(evt.downloaded)
      : "";
  if (pct && transferred) return `${pct} · ${transferred}`;
  return pct || transferred;
}
export function supportsAccountProxy(platform: ManagerSnapshot["platform"]): boolean {
  return platform === "macos" || platform === "windows" || platform === "ios";
}

export function formatGroupPickerHint(
  eligibleCount: number,
  ineligibleCount: number,
  selectedCount: number,
  hasPickerContent: boolean
): string {
  if (eligibleCount === 0 && !hasPickerContent) {
    return "暂无可用成员，请先登录网页账号或添加 API / CLI 成员。";
  }
  if (ineligibleCount > 0) {
    return `已自动过滤 ${ineligibleCount} 个未登录或登录失效网页账号（已选 ${selectedCount}/30）。`;
  }
  return `支持网页账号与 API / CLI 混合协同（已选 ${selectedCount}/30）。`;
}
export const ALL_MENTION_KEYWORDS = ["全体成员", "所有人", "全体", "all"];
