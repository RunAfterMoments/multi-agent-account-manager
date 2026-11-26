export type ProviderId = string;
export type AuthState = "unknown" | "authenticated" | "logged_out";
export type ThemeMode = "system" | "light" | "dark";

export interface AccountKey {
  providerId: ProviderId;
  uuid: string;
}

export interface ProviderInfo {
  id: ProviderId;
  name: string;
  shortName: string;
  enabled: boolean;
  statusText: string;
  entryUrl: string;
  homepageUrl: string;
  callbackOrigins: string[];
  verifiedMethods: string[];
}

export interface AppSettings {
  minimizeToTray: boolean;
  globalShortcut: string;
  autoApprove: boolean;
  disableAutoUpdateCheck: boolean;
  autoApproveCommands: boolean;
  sidebarCollapsed: boolean;
}

export interface ProxyTestResult {
  ok: boolean;
  latencyMs: number;
  statusCode: number | null;
  targetHost: string;
  usedProxy: boolean;
  message: string;
}

export interface QuotaDetail {
  planLabel: string;
  planValue: string;
  planSubtext: string | null;
  planUsedPercent: number | null;
  extraLabel: string | null;
  extraValue: string | null;
  extraSubtext: string | null;
  extraUsedPercent: number | null;
}

export interface AccountRecord {
  providerId: ProviderId;
  uuid: string;
  alias: string;
  createdAtMs: number;
  autoAlias: boolean;
  proxyUrl: string | null;
  timezone: string | null;
  avatarUrl: string | null;
  quotaText: string | null;
  quotaPercent: number | null;
  quotaDetail: QuotaDetail | null;
  authState: AuthState;
  cookieExpiresAtMs: number | null;
  cookieSessionOnly: boolean;
}

export interface ConflictItem {
  uuid: string;
  providers: ProviderId[];
}

export type GroupReplyStatus =
  | "queued"
  | "sending"
  | "streaming"
  | "done"
  | "needs_auth"
  | "error";

export interface GroupSharedSkill {
  id: string;
  name: string;
  description: string;
  content: string;
  enabled: boolean;
}

export interface GroupMemberRole {
  member: AccountKey;
  role: string;
}

export type GroupWorkflowMode = "parallel" | "review" | "a2a_relay";

export interface GroupThreadBinding {
  member: AccountKey;
  threadUrl: string;
  updatedAtMs: number;
}

export type ExternalAgentKind = "openai_api" | "local_cli";

/// 下发给前端的外部成员视图：不含明文 Key，只给 hasApiKey。
/// 编辑时 Key 输入框恒为空，留空表示不修改。
export interface ExternalAgentView {
  id: string;
  kind: ExternalAgentKind;
  name: string;
  baseUrl: string;
  hasApiKey: boolean;
  model: string;
  command: string;
  args: string[];
  workdir: string | null;
  timeoutSecs: number;
  idleTimeoutSecs: number;
  createdAtMs: number;
}

export interface SiteAdapterConfig {
  id: string;
  name: string;
  hostPatterns: string[];
  composerSelectors: string[];
  sendButtonSelectors: string[];
  stopButtonSelectors: string[];
  messageListSelectors: string[];
  assistantMessageSelectors: string[];
  newChatSelectors: string[];
  newChatUrl: string | null;
  threadUrlPatterns: string[];
  transientStatusWords: string[];
  chatApiPatterns: string[];
  preferEnterSubmit: boolean;
  asyncAgent?: boolean;
}

export interface SiteAdaptersSnapshot {
  adapters: SiteAdapterConfig[];
  overridePath: string;
  hasOverrides: boolean;
  overrideJson: string;
}

export interface GroupChatMessage {
  id: string;
  role: "user" | "assistant" | string;
  sender: AccountKey | null;
  senderName: string;
  content: string;
  createdAtMs: number;
  status: GroupReplyStatus;
  targets: AccountKey[];
  error: string | null;
  stageLabel?: string | null;
  hopDepth?: number;
  autoRoutedChannel?: string | null;
}

export interface GroupChatRecord {
  id: string;
  name: string;
  members: AccountKey[];
  createdAtMs: number;
  messages: GroupChatMessage[];
  sharedContext: string;
  sharedSkills: GroupSharedSkill[];
  memberRoles: GroupMemberRole[];
  workflowMode?: GroupWorkflowMode;
  maxRelayHops?: number;
  threadBindings?: GroupThreadBinding[];
}

export interface AccountPageError {
  providerId: ProviderId;
  uuid: string;
  message: string;
}

export interface ManagerSnapshot {
  platform: string;
  providers: ProviderInfo[];
  accounts: AccountRecord[];
  selectedProviderId: ProviderId;
  activeAccount: AccountKey | null;
  switchingAccount: AccountKey | null;
  warmAccounts: AccountKey[];
  lastCommitted: AccountKey | null;
  pendingCleanup: AccountKey[];
  conflicts: ConflictItem[];
  notice: string | null;
  modalActive: boolean;
  settings: AppSettings;
  globalShortcutActive: boolean;
  globalShortcutError: string | null;
  groupChats: GroupChatRecord[];
  activeGroupId: string | null;
  externalAgents: ExternalAgentView[];
  accountErrors?: AccountPageError[];
  /** 配置加载失败 / 版本过高时为 true：写盘已被阻止，需用户二选一。 */
  persistBlocked?: boolean;
  /** 当前生效的折叠态（Rust 布局与前端显隐的单一真相源）。 */
  effectiveSidebarCollapsed: boolean;
  readOnly?: boolean;
}

export interface UpdateCheckResult {
  currentVersion: string;
  latestVersion: string;
  hasUpdate: boolean;
  releaseNotes: string;
  pubDate: string | null;
  downloadUrl: string | null;
  sha256: string | null;
  filename: string | null;
  size: number | null;
  releasePageUrl: string;
  updatesDisabled: boolean;
}

export interface UpdateProgressEvent {
  stage: string;
  downloaded: number;
  total: number | null;
  percent: number | null;
  message: string;
}

export interface InstallUpdateOutcome {
  status: string;
  message: string;
}

export interface DownloadNoticeEvent {
  id: number;
  filename: string;
  status: "started" | "completed" | "failed";
  error?: string | null;
}

// Moved from main.ts (P2-11 split).
export type AccountDraft = {
  draft: string;
  proxyDraft: string;
  timezoneDraft: string;
  cookieDraft?: string;
  cookieImporting?: boolean;
  error?: string;
  saving?: boolean;
  proxyTesting?: boolean;
  proxyTestResult?: ProxyTestResult | null;
  confirmClear?: boolean;
};
export type ModalMode =
  | { kind: "none" }
  | ({ kind: "add"; providerId: ProviderId } & AccountDraft)
  | ({ kind: "rename"; account: AccountRecord } & AccountDraft)
  | { kind: "remove"; account: AccountRecord }
  | {
      kind: "group";
      group?: GroupChatRecord;
      nameDraft: string;
      selectedMembers: AccountKey[];
      error?: string;
      saving?: boolean;
    }
  | {
      kind: "group-knowledge";
      group: GroupChatRecord;
      tab: "context" | "skills" | "roles" | "adapters";
      contextDraft: string;
      skillsDraft: GroupSharedSkill[];
      rolesDraft: GroupMemberRole[];
      skillNameDraft: string;
      skillDescDraft: string;
      skillContentDraft: string;
      editingSkillId: string | null;
      adaptersSnap?: SiteAdaptersSnapshot | null;
      adaptersJsonDraft: string;
      adaptersLoaded?: boolean;
      error?: string;
      saving?: boolean;
    }
  | {
      kind: "external-agents";
      editingId: string | null;
      formKind: ExternalAgentKind;
      nameDraft: string;
      baseUrlDraft: string;
      apiKeyDraft: string;
      modelDraft: string;
      commandDraft: string;
      argsDraft: string;
      workdirDraft: string;
      timeoutSecsDraft: string;
      idleTimeoutSecsDraft: string;
      error?: string;
      saving?: boolean;
    }
  | { kind: "delete-group"; group: GroupChatRecord }
  | {
      kind: "settings";
      minimizeToTray: boolean;
      globalShortcut: string;
      autoApprove: boolean;
      disableAutoUpdateCheck: boolean;
      autoApproveCommands: boolean;
      error?: string;
      saving?: boolean;
    }
  | { kind: "update" };
export type ProviderLogoSpec = { src: string; fullBleed: boolean; monoDark?: boolean };
