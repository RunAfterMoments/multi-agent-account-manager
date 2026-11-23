import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./styles.css";
import type {
  AccountKey,
  AccountRecord,
  DownloadNoticeEvent,
  InstallUpdateOutcome,
  ManagerSnapshot,
  ProviderId,
  ProviderInfo,
  ProxyTestResult,
  QuotaDetail,
  ThemeMode,
  UpdateCheckResult,
  UpdateProgressEvent,
} from "./types";

type AccountDraft = {
  draft: string;
  proxyDraft: string;
  error?: string;
  saving?: boolean;
  proxyTesting?: boolean;
  proxyTestResult?: ProxyTestResult | null;
  confirmClear?: boolean;
};

type ModalMode =
  | { kind: "none" }
  | ({ kind: "add"; providerId: ProviderId } & AccountDraft)
  | ({ kind: "rename"; account: AccountRecord } & AccountDraft)
  | { kind: "remove"; account: AccountRecord }
  | {
      kind: "custom-provider";
      provider?: ProviderInfo;
      nameDraft: string;
      urlDraft: string;
      error?: string;
      saving?: boolean;
      confirmDelete?: boolean;
    }
  | {
      kind: "settings";
      minimizeToTray: boolean;
      globalShortcut: string;
      error?: string;
      saving?: boolean;
    }
  | { kind: "update" };

const PROVIDER_LOGOS: Record<string, { src: string; fullBleed: boolean }> = {
  muse: { src: "/logos/muse.png", fullBleed: true },
  hark: { src: "/logos/hark.svg", fullBleed: false },
};

const CUSTOM_PROVIDER_PRESETS = [
  { name: "ChatGPT", url: "https://chatgpt.com/" },
  { name: "Claude", url: "https://claude.ai/" },
  { name: "Gemini", url: "https://gemini.google.com/" },
  { name: "DeepSeek", url: "https://chat.deepseek.com/" },
  { name: "Grok", url: "https://grok.com/" },
  { name: "Kimi", url: "https://www.kimi.com/" },
];

const CUSTOM_BADGE_COLORS = [
  "linear-gradient(135deg, #0ea5e9, #2563eb)",
  "linear-gradient(135deg, #10b981, #059669)",
  "linear-gradient(135deg, #8b5cf6, #6d28d9)",
  "linear-gradient(135deg, #f59e0b, #d97706)",
  "linear-gradient(135deg, #ec4899, #db2777)",
  "linear-gradient(135deg, #14b8a6, #0d9488)",
];

const ICON_EXTERNAL = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>`;
const ICON_REFRESH = `<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.5 2v6h-6M21.34 15.57a10 10 0 1 1-.57-8.38l5.67-5.67"/></svg>`;
const ICON_EDIT = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"/></svg>`;
const ICON_TRASH = `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>`;
const ICON_USER_PLUS = `<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><line x1="19" y1="8" x2="19" y2="14"/><line x1="22" y1="11" x2="16" y2="11"/></svg>`;
const ICON_UPDATE = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>`;
const ICON_SETTINGS = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>`;
const ICON_THEME_DARK = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>`;
const ICON_THEME_LIGHT = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><line x1="12" y1="2" x2="12" y2="4"/><line x1="12" y1="20" x2="12" y2="22"/><line x1="4.93" y1="4.93" x2="6.34" y2="6.34"/><line x1="17.66" y1="17.66" x2="19.07" y2="19.07"/><line x1="2" y1="12" x2="4" y2="12"/><line x1="20" y1="12" x2="22" y2="12"/><line x1="6.34" y1="17.66" x2="4.93" y2="19.07"/><line x1="19.07" y1="4.93" x2="17.66" y2="6.34"/></svg>`;
const ICON_THEME_SYSTEM = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>`;
const ICON_BOLT = `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg>`;
const ICON_SORT = `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M6 12h12"/><path d="M10 18h4"/></svg>`;

const THEME_STORAGE_KEY = "maam_theme_mode";
const systemDarkQuery = window.matchMedia("(prefers-color-scheme: dark)");
let themeMode: ThemeMode = loadSavedThemeMode();

function loadSavedThemeMode(): ThemeMode {
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

function effectiveTheme(): "light" | "dark" {
  if (themeMode === "system") {
    return systemDarkQuery.matches ? "dark" : "light";
  }
  return themeMode;
}

function applyTheme(): void {
  document.documentElement.dataset.theme = effectiveTheme();
}

function themeLabel(mode: ThemeMode): string {
  if (mode === "dark") return "深色";
  if (mode === "light") return "浅色";
  return "跟随";
}

function themeIcon(mode: ThemeMode): string {
  if (mode === "dark") return ICON_THEME_DARK;
  if (mode === "light") return ICON_THEME_LIGHT;
  return ICON_THEME_SYSTEM;
}

function cycleThemeMode(): void {
  themeMode = themeMode === "system" ? "light" : themeMode === "light" ? "dark" : "system";
  try {
    localStorage.setItem(THEME_STORAGE_KEY, themeMode);
  } catch {
    // ignore storage errors
  }
  applyTheme();
  rerenderPreservingScroll();
}

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
let lastRenderedUpdateStage: string | null = null;
const copyingCookies = new Set<string>();
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

const updateState: {
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

const scrollOffsets: Record<string, number> = {};

function isSameKey(a: AccountKey | null, b: AccountRecord): boolean {
  return Boolean(a && a.providerId === b.providerId && a.uuid === b.uuid);
}

function effectiveSwitchingKey(state: ManagerSnapshot): AccountKey | null {
  return state.switchingAccount ?? optimisticSwitching;
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function customBadgeColor(id: string): string {
  let hash = 0;
  for (let i = 0; i < id.length; i++) {
    hash = (hash * 31 + id.charCodeAt(i)) >>> 0;
  }
  return CUSTOM_BADGE_COLORS[hash % CUSTOM_BADGE_COLORS.length];
}

function renderProviderLogo(provider: ProviderInfo, extraClass = ""): string {
  const cfg = PROVIDER_LOGOS[provider.id];
  if (cfg) {
    const cls = `${extraClass} ${cfg.fullBleed ? "full-bleed" : ""}`.trim();
    return `<img src="${cfg.src}" alt="${escapeHtml(provider.shortName)}" class="${cls}" draggable="false" />`;
  }
  const initial = Array.from(provider.shortName.trim())[0]?.toUpperCase() || "A";
  return `<span class="custom-provider-badge ${escapeHtml(extraClass)}" style="background:${customBadgeColor(provider.id)}">${escapeHtml(initial)}</span>`;
}

function computeAvatarText(acc: AccountRecord): string {
  if (acc.autoAlias) {
    return "@";
  }
  const clean = acc.alias.trim();
  const first = Array.from(clean)[0];
  return first ? first.toUpperCase() : "#";
}

function splitQuotaValue(raw: string): { main: string; note: string | null } {
  const m = raw.trim().match(/^(.*?)\s*[（(]([^）)]+)[）)]\s*$/);
  if (m && m[1].trim()) {
    return { main: m[1].trim(), note: m[2].trim() };
  }
  return { main: raw.trim(), note: null };
}

type QuotaStatus = "normal" | "extra" | "low" | "critical";

function resolveQuotaState(acc: AccountRecord): {
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

function scoreAccountAvailability(acc: AccountRecord): {
  tier: number;
  score: number;
  available: boolean;
} {
  if (acc.authState === "logged_out") {
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

function compareAccountAvailability(a: AccountRecord, b: AccountRecord): number {
  const sa = scoreAccountAvailability(a);
  const sb = scoreAccountAvailability(b);
  if (sb.tier !== sa.tier) return sb.tier - sa.tier;
  return sb.score - sa.score;
}

function formatShortResetText(raw: string | null): string {
  const clean = (raw ?? "").trim();
  if (!clean) return "";
  const m = clean.match(/^(?:每周限额将在|周限额重置时间[:：]?|将在)\s*(.+?)\s*$/);
  if (m && m[1]) {
    const inner = m[1].trim();
    return inner.endsWith("重置") ? inner : `${inner}重置`;
  }
  return clean;
}

function formatShortExtraText(
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

function formatLocalDateTime(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function formatCookieExpiry(acc: AccountRecord): CookieExpiryDisplay | null {
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

function renderQuotaDetailPanel(
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

function saveScrollPosition(): void {
  if (!snapshot) return;
  const listEl = document.querySelector<HTMLElement>(".account-list");
  if (listEl) {
    scrollOffsets[snapshot.selectedProviderId] = listEl.scrollTop;
  }
}

function restoreScrollPosition(): void {
  if (!snapshot) return;
  const listEl = document.querySelector<HTMLElement>(".account-list");
  if (listEl) {
    listEl.scrollTop = scrollOffsets[snapshot.selectedProviderId] ?? 0;
  }
}

function rerenderPreservingScroll(): void {
  saveScrollPosition();
  render();
  restoreScrollPosition();
}

function formatBytes(bytes: number): string {
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

function formatProgressSummary(evt: UpdateProgressEvent): string {
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
  lastRenderedUpdateStage = "downloading";
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
  const wasActive = modal.kind !== "none";
  const isActive = next.kind !== "none";
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
  } else if (next.kind === "custom-provider") {
    const input = document.querySelector<HTMLInputElement>("#modal-provider-name-input");
    if (input) {
      input.focus();
      input.select();
    }
  } else if (next.kind === "settings") {
    document.querySelector<HTMLInputElement>("#modal-shortcut-input")?.focus();
  } else if (next.kind === "remove") {
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
  saveScrollPosition();
  if (optimisticTarget) {
    optimisticSwitching = optimisticTarget;
    if (snapshot) {
      snapshot = { ...snapshot, activeAccount: null };
    }
    render();
    restoreScrollPosition();
  }
  try {
    snapshot = await invoke<ManagerSnapshot>(cmd, args);
    return true;
  } catch (err) {
    if (
      (cmd === "add_account" && modal.kind === "add") ||
      (cmd === "rename_account" && modal.kind === "rename") ||
      (cmd === "clear_account_data" && modal.kind === "rename") ||
      ((cmd === "add_custom_provider" || cmd === "update_custom_provider" || cmd === "remove_custom_provider") &&
        modal.kind === "custom-provider") ||
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
    restoreScrollPosition();
  }
}

function currentProvider(state: ManagerSnapshot): ProviderInfo {
  return (
    state.providers.find((p) => p.id === state.selectedProviderId) ??
    state.providers[0]
  );
}

function renderUpdateModal(): string {
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
            : "立即下载并更新"
      }</button>`
    : `<button id="update-recheck-btn" class="btn primary" type="button" ${disablePrimary ? "disabled" : ""}>${
        updateState.checking ? "检查中..." : "重新检查"
      }</button>`;

  const pageBtn = res?.releasePageUrl
    ? `<button id="update-open-page-btn" class="btn" type="button">发布页</button>`
    : "";

  return `
    <div class="inline-modal" role="dialog" aria-modal="true">
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

function supportsAccountProxy(platform: ManagerSnapshot["platform"]): boolean {
  return platform === "macos" || platform === "windows";
}

function renderModal(state: ManagerSnapshot, provider: ProviderInfo): string {
  const platform = state.platform;
  if (modal.kind === "none") return "";
  if (modal.kind === "update") {
    return renderUpdateModal();
  }
  if (modal.kind === "custom-provider") {
    const editing = Boolean(modal.provider);
    const saving = Boolean(modal.saving);
    return `
      <div class="inline-modal" role="dialog" aria-modal="true">
        <div class="modal-card">
          <div class="modal-title">${editing ? `编辑「${escapeHtml(modal.provider!.name)}」` : "添加自定义网页服务"}</div>
          ${!editing ? `
            <div class="preset-chips">
              ${CUSTOM_PROVIDER_PRESETS.map(
                (p, idx) =>
                  `<button class="preset-chip" data-preset-idx="${idx}" type="button" ${saving ? "disabled" : ""}>${escapeHtml(p.name)}</button>`
              ).join("")}
            </div>
          ` : ""}
          <input id="modal-provider-name-input" class="modal-input" maxlength="32" placeholder="服务名称（如 ChatGPT）" aria-label="服务名称" value="${escapeHtml(modal.nameDraft)}" ${saving ? "disabled" : ""} />
          <input id="modal-provider-url-input" class="modal-input" maxlength="256" placeholder="https://chatgpt.com/" aria-label="网站入口地址" value="${escapeHtml(modal.urlDraft)}" autocomplete="off" spellcheck="false" ${saving ? "disabled" : ""} />
          <div class="modal-hint">仅支持 https:// 网页地址；支持独立会话隔离${supportsAccountProxy(platform) ? "、专属代理" : ""}与快捷键切号。</div>
          ${modal.error ? `<div id="modal-error-box" class="update-error-box" role="alert">${escapeHtml(modal.error)}</div>` : ""}
          <div class="modal-footer">
            ${editing ? `<button id="modal-provider-delete-btn" class="btn danger-outline left-action" type="button" ${saving ? "disabled" : ""}>${modal.confirmDelete ? "确认删除（含账号）" : "删除服务"}</button>` : ""}
            <button id="modal-cancel-btn" class="btn" type="button" ${saving ? "disabled" : ""}>取消</button>
            <button id="modal-confirm-btn" class="btn primary" type="button" ${saving ? "disabled" : ""}>${saving ? "保存中..." : (editing ? "保存" : "添加")}</button>
          </div>
        </div>
      </div>
    `;
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
      <div class="inline-modal" role="dialog" aria-modal="true">
        <div class="modal-card">
          <div class="modal-title">桌面端设置</div>
          <label class="setting-row" for="modal-tray-checkbox">
            <span class="setting-label">关闭主窗口时最小化到系统托盘</span>
            <input id="modal-tray-checkbox" class="setting-checkbox" type="checkbox" ${modal.minimizeToTray ? "checked" : ""} ${saving ? "disabled" : ""} />
          </label>
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
      : supportsProxy ? "账号设置" : "修改备注名称";
    const aliasPlaceholder = creating
      ? "备注名称（留空则自动获取登录邮箱）"
      : supportsProxy ? "备注名称（留空保留原备注）" : "输入新的备注名称";
    const testRes = modal.proxyTestResult;
    const renameAcc = modal.kind === "rename" ? modal.account : null;
    const currentAcc = renameAcc
      ? (state.accounts.find(
          (a) => a.providerId === renameAcc.providerId && a.uuid === renameAcc.uuid
        ) ?? renameAcc)
      : null;
    const cookieInfo = currentAcc ? formatCookieExpiry(currentAcc) : null;
    return `
      <div class="inline-modal" role="dialog" aria-modal="true">
        <div class="modal-card">
          <div class="modal-title">${title}</div>
          <input id="modal-alias-input" class="modal-input" maxlength="60" placeholder="${aliasPlaceholder}" aria-label="备注名称" value="${escapeHtml(modal.draft)}" ${saving ? "disabled" : ""} />
          ${supportsProxy ? `
            <label class="modal-desc" for="modal-proxy-input">HTTP CONNECT 代理（留空使用系统网络）</label>
            <div class="proxy-input-row">
              <input id="modal-proxy-input" class="modal-input" maxlength="512" placeholder="http://127.0.0.1:7890" value="${escapeHtml(modal.proxyDraft)}" autocomplete="off" spellcheck="false" ${saving ? "disabled" : ""} />
              <button id="proxy-test-btn" class="btn mini-test-btn" type="button" ${saving || proxyTesting ? "disabled" : ""}>${proxyTesting ? "测速中..." : "测试连接"}</button>
            </div>
            ${testRes ? `<div id="proxy-test-result" class="proxy-test-box ${testRes.ok ? "ok" : "err"}">${escapeHtml(testRes.message)}</div>` : ""}
            <div class="modal-hint">仅作用于内嵌页面；外部浏览器登录与更新检查不走此代理。</div>
          ` : ""}
          ${cookieInfo ? `<div class="cookie-detail-box ${cookieInfo.tone}" title="${escapeHtml(cookieInfo.tooltip)}"><span class="cookie-expiry-dot"></span><span>${escapeHtml(cookieInfo.detailText)}</span></div>` : ""}
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
  return `
    <div class="inline-modal" role="dialog" aria-modal="true">
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

function renderMiddleContent(state: ManagerSnapshot, provider: ProviderInfo): string {
  const providerAccounts = state.accounts.filter((a) => a.providerId === provider.id);
  const switchingKey = effectiveSwitchingKey(state);
  const isAndroid = state.platform === "android";
  const supportsProxy = supportsAccountProxy(state.platform);
  const supportsCookieCopy = supportsProxy || isAndroid;
  const canReorder = providerAccounts.length > 1 && state.conflicts.length === 0;

  const noticeHtml = state.notice
    ? `
      <div class="notice-bar">
        <span>${escapeHtml(state.notice)}</span>
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
  const androidThemeBtn = isAndroid
    ? `<button id="theme-toggle-btn" class="ghost-icon-btn" type="button" title="主题：${themeLabel(themeMode)}（点击切换）">${themeIcon(themeMode)}</button>`
    : "";
  const androidUpdateBtn = isAndroid
    ? `<button id="check-update-btn" class="ghost-icon-btn update-trigger-btn ${hasUpdate ? "has-update" : ""}" type="button" title="${hasUpdate ? `发现新版本 v${escapeHtml(updateState.result!.latestVersion)}` : "检查更新"}">${ICON_UPDATE}${hasUpdate ? `<span class="update-dot"></span>` : ""}</button>`
    : "";

  const headerBtnHtml = provider.enabled
    ? `
      ${androidThemeBtn}
      ${androidUpdateBtn}
      ${!isAndroid && !provider.isCustom ? `<button id="sync-all-btn" class="ghost-icon-btn" type="button" title="刷新已就绪账号额度">${ICON_REFRESH}</button>` : ""}
      <button id="open-site-btn" class="ghost-icon-btn" type="button" title="在浏览器中打开 ${escapeHtml(provider.shortName)} 官网">${ICON_EXTERNAL}</button>
      <button id="add-account-btn" class="icon-btn primary" type="button">+ 新增</button>
    `
    : `${androidThemeBtn}${androidUpdateBtn}<button id="open-site-btn" class="icon-btn primary" type="button">打开官网 ${ICON_EXTERNAL}</button>`;

  const listToolbarHtml =
    provider.enabled && providerAccounts.length >= 2 && state.conflicts.length === 0
      ? `
        <div class="list-toolbar">
          <button id="smart-switch-btn" class="toolbar-chip" type="button" title="${provider.isCustom ? "切换到在线可用账号" : "切换到剩余可用额度最多的账号"}">
            ${ICON_BOLT}<span>${provider.isCustom ? "切到在线账号" : "切到余量最多"}</span>
          </button>
          <button id="sort-quota-btn" class="toolbar-chip" type="button" title="${provider.isCustom ? "在线账号优先排序" : "按剩余可用额度从高到低排序"}">
            ${ICON_SORT}<span>${provider.isCustom ? "在线优先排序" : "按额度排序"}</span>
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
        const hasDetail = !isLoggedOut && Boolean(acc.quotaDetail);
        const cookieInfo = isLoggedOut ? null : formatCookieExpiry(acc);

        const quotaState = isLoggedOut
          ? { status: "normal" as QuotaStatus, label: "", extraDepleted: false }
          : resolveQuotaState(acc);
        const {
          status: alertStatus,
          label: alertLabel,
          extraDepleted,
        } = quotaState;

        const isMac = navigator.userAgent.includes("Mac");
        const shortcutHint =
          !isAndroid && idx < 9
            ? `<span class="shortcut-pill" title="快捷切号">${isMac ? "⌘" : "^"}${idx + 1}</span>`
            : "";
        const proxyBadgeHtml =
          supportsProxy && acc.proxyUrl
            ? `<span class="proxy-pill" title="代理：${escapeHtml(acc.proxyUrl)}">代理</span>`
            : "";
        const alertBadgeHtml = switching
          ? `<span class="loading-status-badge">加载中…</span>`
          : isLoggedOut
            ? `<span class="auth-status-badge">未登录</span>`
            : alertStatus !== "normal"
              ? `<span class="quota-warning-badge ${alertStatus}">${alertLabel}</span>`
              : "";

        let compactSubHtml = "";
        if (switching && !hasDetail) {
          compactSubHtml = `<div class="account-sub loading-text"><span class="quota-label-text">正在加载…</span></div>`;
        } else if (isLoggedOut) {
          const loginHint = active
            ? isAndroid
              ? "登录已失效，点击进入重新登录"
              : "登录已失效，请在右侧重新登录"
            : acc.autoAlias
              ? "尚未登录，点击前往登录"
              : "登录已失效，点击重新登录";
          compactSubHtml = `<div class="account-sub auth-logged-out"><span class="auth-dot-inline"></span><span class="quota-label-text">${loginHint}</span></div>`;
        } else if (!hasDetail) {
          const fallbackLabel = provider.isCustom
            ? acc.authState === "authenticated"
              ? "已登录"
              : "点击打开网页"
            : acc.autoAlias
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
          alertStatus !== "normal" ? `alert-${alertStatus}` : "",
        ]
          .filter(Boolean)
          .join(" ");

        const avatarClass = [
          "avatar",
          acc.autoAlias ? "pending-identity" : provider.isCustom ? "custom" : acc.providerId,
        ].join(" ");

        const avatarNodeHtml = acc.avatarUrl
          ? `<div class="avatar has-photo"><img src="${escapeHtml(acc.avatarUrl)}" class="avatar-photo" alt="" draggable="false" /></div>`
          : `<div class="${avatarClass}">${escapeHtml(avatarText)}</div>`;

        return `
          <div class="${rowClass}" data-switch-uuid="${escapeHtml(acc.uuid)}" ${!isAndroid && canReorder ? `draggable="true"` : ""}>
            <div class="account-header">
              <div class="avatar-wrap">
                ${avatarNodeHtml}
                ${switching ? `<span class="avatar-spinner"></span>` : ""}
                ${active ? `<span class="status-dot ${isLoggedOut ? "offline" : ""}"></span>` : ""}
              </div>
              <div class="account-meta">
                <div class="account-alias-row">
                  <span class="account-alias">${escapeHtml(acc.alias)}</span>
                  ${shortcutHint}
                  ${proxyBadgeHtml}
                  ${alertBadgeHtml}
                </div>
                ${compactSubHtml}
                ${!hasDetail && !isLoggedOut && !switching && cookieInfo ? `<div class="cookie-expiry-line inline ${cookieInfo.tone}" title="${escapeHtml(cookieInfo.tooltip)}"><span class="cookie-expiry-dot"></span><span>${escapeHtml(cookieInfo.shortText)}</span></div>` : ""}
              </div>
              <div class="row-ops">
                ${canReorder ? `
                  <div class="reorder-stack">
                    <button class="reorder-btn" data-move-uuid="${escapeHtml(acc.uuid)}" data-move-dir="up" title="上移" aria-label="上移" type="button" ${idx === 0 ? "disabled" : ""}>▲</button>
                    <button class="reorder-btn" data-move-uuid="${escapeHtml(acc.uuid)}" data-move-dir="down" title="下移" aria-label="下移" type="button" ${idx === providerAccounts.length - 1 ? "disabled" : ""}>▼</button>
                  </div>
                ` : ""}
                ${!isAndroid ? `
                  <button class="action-icon-btn ${switching ? "spinning" : ""}" data-reload-uuid="${escapeHtml(acc.uuid)}" title="强制刷新页面" type="button" ${switching ? "disabled" : ""}>${ICON_REFRESH}</button>
                ` : ""}
                ${supportsCookieCopy ? `
                  <button class="action-icon-btn copy-cookie-btn" data-copy-cookie-uuid="${escapeHtml(acc.uuid)}" data-copy-label="复制 Cookie" aria-label="复制 Cookie" type="button">
                    <span class="copy-cookie-icon" aria-hidden="true"></span>
                  </button>
                ` : ""}
                <button class="action-icon-btn" data-rename-uuid="${escapeHtml(acc.uuid)}" title="${supportsProxy ? "账号设置" : "修改备注"}" type="button">${ICON_EDIT}</button>
                <button class="action-icon-btn danger" data-remove-uuid="${escapeHtml(acc.uuid)}" title="从本机移除" type="button">${ICON_TRASH}</button>
              </div>
            </div>
            ${!isLoggedOut && acc.quotaDetail ? renderQuotaDetailPanel(acc.quotaDetail, alertStatus, extraDepleted) : ""}
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
        ${provider.isCustom ? `<button id="edit-provider-btn" class="inline-provider-edit-btn" title="编辑或删除自定义服务" type="button">${ICON_EDIT}</button>` : ""}
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
    ${renderDownloadToast()}
    ${renderModal(state, provider)}
  `;
}

function renderDownloadToast(): string {
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

function renderRightStage(state: ManagerSnapshot, provider: ProviderInfo): string {
  const switchingKey = effectiveSwitchingKey(state);
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

function render(): void {
  const root = document.querySelector<HTMLElement>("#app");
  if (!root || !snapshot) return;

  const focusedInput = document.activeElement;
  const preservedInputIds = new Set([
    "modal-alias-input",
    "modal-proxy-input",
    "modal-provider-name-input",
    "modal-provider-url-input",
    "modal-shortcut-input",
  ]);
  const accountInputSelection =
    focusedInput instanceof HTMLInputElement && preservedInputIds.has(focusedInput.id)
      ? {
          id: focusedInput.id,
          start: focusedInput.selectionStart,
          end: focusedInput.selectionEnd,
          direction: focusedInput.selectionDirection,
        }
      : null;

  const provider = currentProvider(snapshot);
  const isAndroid = snapshot.platform === "android";

  if (isAndroid) {
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
          <button id="add-provider-btn" class="mobile-tab-add" title="添加自定义网页服务" type="button">+</button>
        </div>
        <div class="middle-column" style="width:100%;min-width:0;flex:1;border-right:none;">
          ${renderMiddleContent(snapshot, provider)}
        </div>
      </div>
    `;
  } else {
    root.innerHTML = `
      <div class="desktop-shell">
        <aside class="service-rail">
          <div class="rail-brand" title="叠号 · AI 助手账号切换器">
            <img src="/logos/app-logo.svg" alt="叠号" class="rail-brand-img" draggable="false" />
          </div>
          <div class="rail-divider"></div>
          <div class="rail-providers">
            ${snapshot.providers
              .map(
                (p) => `
                <button class="rail-btn ${p.id === provider.id ? "active" : ""} ${!p.enabled ? "disabled" : ""}" data-provider="${escapeHtml(p.id)}" title="${escapeHtml(p.name)}" type="button">
                  <div class="rail-logo-box">${renderProviderLogo(p, "rail-logo-img")}</div>
                  <span class="rail-label">${escapeHtml(p.shortName)}</span>
                  ${!p.enabled ? `<span class="rail-pending-tag">待接入</span>` : ""}
                </button>
              `
              )
              .join("")}
            <button id="add-provider-btn" class="rail-add-btn" title="添加自定义网页服务" type="button">+</button>
          </div>
          <div class="rail-footer">
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
          ${renderMiddleContent(snapshot, provider)}
        </section>
        ${renderRightStage(snapshot, provider)}
      </div>
    `;
  }

  bindEvents(snapshot, provider);
  if (accountInputSelection) {
    const input = document.querySelector<HTMLInputElement>(`#${accountInputSelection.id}`);
    input?.focus({ preventScroll: true });
    input?.setSelectionRange(
      accountInputSelection.start,
      accountInputSelection.end,
      accountInputSelection.direction ?? undefined
    );
  }
}

function bindEvents(state: ManagerSnapshot, provider: ProviderInfo): void {
  document.querySelectorAll<HTMLElement>("[data-provider]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const nextId = btn.dataset.provider as ProviderId;
      if (!nextId || nextId === state.selectedProviderId) return;
      if (modal.kind !== "none") {
        await setModal({ kind: "none" });
      }
      await callCommand("select_provider", { providerId: nextId });
    });
  });

  document.querySelector("#add-provider-btn")?.addEventListener("click", async () => {
    await setModal({ kind: "custom-provider", nameDraft: "", urlDraft: "" });
  });

  document.querySelector("#edit-provider-btn")?.addEventListener("click", async () => {
    if (!provider.isCustom) return;
    await setModal({
      kind: "custom-provider",
      provider,
      nameDraft: provider.name,
      urlDraft: provider.entryUrl,
    });
  });

  document.querySelector("#theme-toggle-btn")?.addEventListener("click", () => {
    cycleThemeMode();
  });

  document.querySelector("#settings-btn")?.addEventListener("click", async () => {
    if (modal.kind !== "none") return;
    await setModal({
      kind: "settings",
      minimizeToTray: state.settings.minimizeToTray,
      globalShortcut: state.settings.globalShortcut,
    });
  });

  document.querySelector("#smart-switch-btn")?.addEventListener("click", async () => {
    if (modal.kind !== "none" || state.conflicts.length > 0) return;
    const candidates = state.accounts.filter((a) => a.providerId === provider.id);
    if (candidates.length === 0) return;
    const ranked = [...candidates]
      .filter((a) => scoreAccountAvailability(a).available)
      .sort(compareAccountAvailability);
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
          notice: provider.isCustom ? "当前账号已在在线列表中" : "当前账号已是可用余量最多的账号",
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
    const sorted = [...candidates].sort(compareAccountAvailability);
    const orderedUuids = sorted.map((a) => a.uuid);
    const changed = orderedUuids.some((u, idx) => u !== candidates[idx].uuid);
    if (!changed) {
      if (snapshot) {
        snapshot = {
          ...snapshot,
          notice: provider.isCustom ? "当前已是按在线状态优先排列" : "当前已是按可用余量降序排列",
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
    await setModal({ kind: "add", providerId: provider.id, draft: "", proxyDraft: "" });
  };
  document.querySelector("#add-account-btn")?.addEventListener("click", openAddModal);
  document.querySelector("#stage-add-btn")?.addEventListener("click", openAddModal);

  document.querySelector("#dismiss-notice-btn")?.addEventListener("click", async () => {
    await callCommand("dismiss_notice");
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
      if (state.platform === "android" || modal.kind !== "none" || state.conflicts.length > 0 || busy) {
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
        state.platform !== "android"
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

  document.querySelectorAll<HTMLButtonElement>("[data-preset-idx]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (modal.kind !== "custom-provider" || modal.saving) return;
      const idx = Number(btn.dataset.presetIdx);
      const preset = CUSTOM_PROVIDER_PRESETS[idx];
      if (!preset) return;
      modal.nameDraft = preset.name;
      modal.urlDraft = preset.url;
      modal.error = undefined;
      render();
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

  document.querySelector("#modal-provider-delete-btn")?.addEventListener("click", async () => {
    const currentModal = modal;
    if (currentModal.kind !== "custom-provider" || !currentModal.provider || currentModal.saving) {
      return;
    }
    if (!currentModal.confirmDelete) {
      currentModal.confirmDelete = true;
      render();
      return;
    }
    currentModal.error = undefined;
    currentModal.saving = true;
    render();
    let removed = false;
    try {
      removed = await callCommand("remove_custom_provider", {
        providerId: currentModal.provider.id,
      });
    } finally {
      currentModal.saving = false;
      if (modal === currentModal && !removed) {
        currentModal.confirmDelete = false;
        render();
      }
    }
    if (removed && modal === currentModal) {
      await setModal({ kind: "none" });
    }
  });

  document.querySelector<HTMLInputElement>("#modal-tray-checkbox")?.addEventListener("change", (ev) => {
    if (modal.kind === "settings") {
      modal.minimizeToTray = (ev.currentTarget as HTMLInputElement).checked;
    }
  });

  document
    .querySelectorAll<HTMLInputElement>(
      "#modal-alias-input, #modal-proxy-input, #modal-provider-name-input, #modal-provider-url-input, #modal-shortcut-input"
    )
    .forEach((input) => {
      input.addEventListener("input", () => {
        if (modal.kind === "add" || modal.kind === "rename") {
          if (input.id === "modal-proxy-input") {
            modal.proxyDraft = input.value;
            modal.proxyTestResult = null;
            document.querySelector("#proxy-test-result")?.remove();
          } else {
            modal.draft = input.value;
          }
          if (modal.error) {
            modal.error = undefined;
            document.querySelector("#modal-error-box")?.remove();
          }
        } else if (modal.kind === "custom-provider") {
          if (input.id === "modal-provider-name-input") {
            modal.nameDraft = input.value;
          } else if (input.id === "modal-provider-url-input") {
            modal.urlDraft = input.value;
          }
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
              modal.kind === "custom-provider" ||
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
        modal.kind === "custom-provider" ||
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
      if (currentModal.kind === "rename" && !supportsProxy && !alias) return;
      currentModal.error = undefined;
      currentModal.saving = true;
      render();
      let saved = false;
      try {
        saved = currentModal.kind === "add"
          ? await callCommand("add_account", { providerId: currentModal.providerId, alias, proxyUrl })
          : await callCommand("rename_account", {
            providerId: currentModal.account.providerId,
            uuid: currentModal.account.uuid,
            alias,
            proxyUrl,
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
    } else if (currentModal.kind === "custom-provider") {
      if (currentModal.saving) return;
      currentModal.error = undefined;
      currentModal.saving = true;
      render();
      let saved = false;
      try {
        saved = currentModal.provider
          ? await callCommand("update_custom_provider", {
            providerId: currentModal.provider.id,
            name: currentModal.nameDraft.trim(),
            entryUrl: currentModal.urlDraft.trim(),
          })
          : await callCommand("add_custom_provider", {
            name: currentModal.nameDraft.trim(),
            entryUrl: currentModal.urlDraft.trim(),
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
    saveScrollPosition();
    snapshot = event.payload;
    if (snapshot.activeAccount || !snapshot.switchingAccount) {
      optimisticSwitching = null;
    }
    render();
    restoreScrollPosition();
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
    if (barEl && textEl && pctEl && lastRenderedUpdateStage === evt.stage && evt.stage !== "error") {
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
    lastRenderedUpdateStage = evt.stage;
    rerenderPreservingScroll();
  });

  window.addEventListener("keydown", async (ev) => {
    if (modal.kind !== "none") {
      if (
        ev.key === "Escape" &&
        !(modal.kind === "add" && modal.saving) &&
        !(modal.kind === "rename" && modal.saving) &&
        !(modal.kind === "custom-provider" && modal.saving) &&
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
  render();
  setTimeout(() => {
    void runCheckForUpdates(true);
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
