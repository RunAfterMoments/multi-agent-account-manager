use crate::providers::ProviderId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use zeroize::Zeroizing;

pub const STORE_VERSION: u32 = 2;

fn default_true() -> bool {
  true
}

fn default_relay_hops() -> u8 {
  2
}

pub fn default_global_shortcut() -> String {
  #[cfg(target_os = "macos")]
  {
    "CommandOrControl+Shift+M".to_string()
  }
  #[cfg(not(target_os = "macos"))]
  {
    "CommandOrControl+Shift+M".to_string()
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
  #[serde(default = "default_true")]
  pub minimize_to_tray: bool,
  #[serde(default = "default_global_shortcut")]
  pub global_shortcut: String,
  #[serde(default)]
  pub auto_approve: bool,
  /// 禁用自动更新检查。默认 false（配置了更新源时保持自动检查）；
  /// 未配置更新源时无论此开关如何都不检查更新。
  #[serde(default)]
  pub disable_auto_update_check: bool,
  // 允许自动批准命令执行类审批（执行命令 / shell / bash / 终端）。
  // 独立开关，默认关闭；开启后仍受 DANGER_RE 拦截危险命令。
  #[serde(default)]
  pub auto_approve_commands: bool,
  /// 折叠左侧账号栏：只留 68px 服务栏，账号 WebView 多出 312px。
  /// 旧版 accounts.json 没有该字段时默认 false，无需升 STORE_VERSION。
  #[serde(default)]
  pub sidebar_collapsed: bool,
}

impl Default for AppSettings {
  fn default() -> Self {
    Self {
      minimize_to_tray: true,
      global_shortcut: default_global_shortcut(),
      auto_approve: false,
      disable_auto_update_check: false,
      auto_approve_commands: false,
      sidebar_collapsed: false,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountKey {
  pub provider_id: ProviderId,
  pub uuid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QuotaDetail {
  #[serde(default)]
  pub plan_label: String,
  #[serde(default)]
  pub plan_value: String,
  #[serde(default)]
  pub plan_subtext: Option<String>,
  #[serde(default)]
  pub plan_used_percent: Option<u8>,
  #[serde(default)]
  pub extra_label: Option<String>,
  #[serde(default)]
  pub extra_value: Option<String>,
  #[serde(default)]
  pub extra_subtext: Option<String>,
  #[serde(default)]
  pub extra_used_percent: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
  #[default]
  Unknown,
  Authenticated,
  LoggedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRecord {
  pub provider_id: ProviderId,
  pub uuid: String,
  pub alias: String,
  pub created_at_ms: u64,
  #[serde(default)]
  pub auto_alias: bool,
  #[serde(default)]
  pub proxy_url: Option<String>,
  #[serde(default)]
  pub timezone: Option<String>,
  #[serde(default)]
  pub avatar_url: Option<String>,
  #[serde(default)]
  pub quota_text: Option<String>,
  #[serde(default)]
  pub quota_percent: Option<u8>,
  #[serde(default)]
  pub quota_detail: Option<QuotaDetail>,
  #[serde(default)]
  pub auth_state: AuthState,
  #[serde(default)]
  pub cookie_expires_at_ms: Option<u64>,
  #[serde(default)]
  pub cookie_session_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictItem {
  pub uuid: String,
  pub providers: Vec<ProviderId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupReplyStatus {
  Queued,
  Sending,
  Streaming,
  #[default]
  Done,
  NeedsAuth,
  Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupSharedSkill {
  pub id: String,
  pub name: String,
  #[serde(default)]
  pub description: String,
  #[serde(default)]
  pub content: String,
  #[serde(default = "default_true")]
  pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupMemberRole {
  pub member: AccountKey,
  #[serde(default)]
  pub role: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupWorkflowMode {
  #[default]
  Parallel,
  Review,
  A2aRelay,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupThreadBinding {
  pub member: AccountKey,
  pub thread_url: String,
  #[serde(default)]
  pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAgentKind {
  OpenaiApi,
  LocalCli,
}

/// 本地 CLI / OpenAI API 成员的默认总超时：600 秒（原先写死的 120 秒
/// 对跑工具循环的 agent 太短）。
pub fn default_agent_timeout_secs() -> u64 {
  600
}

/// 默认空闲超时：N 秒无任何输出才判超时（与总超时分开算）。
pub fn default_agent_idle_timeout_secs() -> u64 {
  60
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentRecord {
  pub id: String,
  pub kind: ExternalAgentKind,
  pub name: String,
  #[serde(default)]
  pub base_url: String,
  /// 仅内存：永不序列化落盘（见 `crate::key_vault`）。
  #[serde(default, skip_serializing)]
  pub api_key: Zeroizing<String>,
  /// 系统凭据库 / 回退文件中的引用 id，序列化为 `apiKeyRef`。
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub api_key_ref: Option<String>,
  #[serde(default)]
  pub model: String,
  #[serde(default)]
  pub command: String,
  #[serde(default)]
  pub args: Vec<String>,
  #[serde(default)]
  pub workdir: Option<String>,
  #[serde(default = "default_agent_timeout_secs")]
  pub timeout_secs: u64,
  #[serde(default = "default_agent_idle_timeout_secs")]
  pub idle_timeout_secs: u64,
  #[serde(default)]
  pub created_at_ms: u64,
}

/// 下发给前端的外部成员视图：不含明文 Key，只给 `hasApiKey`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentView {
  pub id: String,
  pub kind: ExternalAgentKind,
  pub name: String,
  #[serde(default)]
  pub base_url: String,
  pub has_api_key: bool,
  #[serde(default)]
  pub model: String,
  #[serde(default)]
  pub command: String,
  #[serde(default)]
  pub args: Vec<String>,
  #[serde(default)]
  pub workdir: Option<String>,
  #[serde(default)]
  pub created_at_ms: u64,
}

impl ExternalAgentView {
  pub fn from_record(record: &ExternalAgentRecord) -> Self {
    Self {
      id: record.id.clone(),
      kind: record.kind,
      name: record.name.clone(),
      base_url: record.base_url.clone(),
      has_api_key: !record.api_key.trim().is_empty(),
      model: record.model.clone(),
      command: record.command.clone(),
      args: record.args.clone(),
      workdir: record.workdir.clone(),
      created_at_ms: record.created_at_ms,
    }
  }

  /// 与 `ExternalAgentRecord::account_key` 同款派生键。
  /// 仅测试调用（快照里的 View 取键断言）；lib 目标的死代码检查
  /// 看不到 `#[cfg(test)]` 调用方，故此处显式 allow。
  #[allow(dead_code)]
  pub fn account_key(&self) -> AccountKey {
    AccountKey {
      provider_id: match self.kind {
        ExternalAgentKind::OpenaiApi => ProviderId::api(),
        ExternalAgentKind::LocalCli => ProviderId::cli(),
      },
      uuid: self.id.clone(),
    }
  }
}

impl ExternalAgentRecord {
  pub fn account_key(&self) -> AccountKey {
    AccountKey {
      provider_id: match self.kind {
        ExternalAgentKind::OpenaiApi => ProviderId::api(),
        ExternalAgentKind::LocalCli => ProviderId::cli(),
      },
      uuid: self.id.clone(),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupChatMessage {
  pub id: String,
  pub role: String,
  #[serde(default)]
  pub sender: Option<AccountKey>,
  #[serde(default)]
  pub sender_name: String,
  #[serde(default)]
  pub content: String,
  #[serde(default)]
  pub created_at_ms: u64,
  #[serde(default)]
  pub status: GroupReplyStatus,
  #[serde(default)]
  pub targets: Vec<AccountKey>,
  #[serde(default)]
  pub error: Option<String>,
  #[serde(default)]
  pub stage_label: Option<String>,
  #[serde(default)]
  pub hop_depth: u8,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub auto_routed_channel: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupChatRecord {
  pub id: String,
  pub name: String,
  #[serde(default)]
  pub members: Vec<AccountKey>,
  #[serde(default)]
  pub created_at_ms: u64,
  #[serde(default)]
  pub messages: Vec<GroupChatMessage>,
  #[serde(default)]
  pub shared_context: String,
  #[serde(default)]
  pub shared_skills: Vec<GroupSharedSkill>,
  #[serde(default)]
  pub member_roles: Vec<GroupMemberRole>,
  #[serde(default)]
  pub workflow_mode: GroupWorkflowMode,
  #[serde(default = "default_relay_hops")]
  pub max_relay_hops: u8,
  #[serde(default)]
  pub thread_bindings: Vec<GroupThreadBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountsStoreFile {
  pub version: u32,
  pub accounts: Vec<AccountRecord>,
  pub last_committed: Option<AccountKey>,
  pub pending_cleanup: Vec<AccountKey>,
  #[serde(default)]
  pub group_chats: Vec<GroupChatRecord>,
  #[serde(default)]
  pub external_agents: Vec<ExternalAgentRecord>,
  #[serde(default)]
  pub settings: AppSettings,
  /// 未知 / 非内置 provider 的账号原始 JSON。
  /// 加载时保留、保存时由 `save_store_atomic` 原样写回，
  /// 避免新版才认识的 provider 让整份配置加载失败后丢失。
  /// 不参与 serde：磁盘往返只走 `save_store_atomic` 的合并逻辑。
  #[serde(default, skip)]
  pub unknown_accounts: Vec<Value>,
}

impl Default for AccountsStoreFile {
  fn default() -> Self {
    Self {
      version: STORE_VERSION,
      accounts: Vec::new(),
      last_committed: None,
      pending_cleanup: Vec::new(),
      group_chats: Vec::new(),
      external_agents: Vec::new(),
      settings: AppSettings::default(),
      unknown_accounts: Vec::new(),
    }
  }
}

pub struct LoadedAccountsStore {
  pub store: AccountsStoreFile,
  pub conflicts: Vec<ConflictItem>,
  pub startup_notice: Option<String>,
  /// 加载失败或配置版本过高时为 true：`SessionManager::persist` 必须拒绝写盘，
  /// 直到用户明确选择「以空配置继续」或「从备份恢复」。
  pub persist_blocked: bool,
  /// 配置 `version` 高于当前 `STORE_VERSION` 时为 true：只读，永不写回。
  pub read_only: bool,
}

pub fn now_ms() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_millis() as u64)
    .unwrap_or(0)
}

pub fn normalize_uuid(raw: &str) -> Option<String> {
  Uuid::parse_str(raw.trim())
    .ok()
    .map(|u| u.hyphenated().to_string())
}

pub fn detect_conflicts(accounts: &[AccountRecord]) -> Vec<ConflictItem> {
  let mut by_uuid: BTreeMap<String, Vec<ProviderId>> = BTreeMap::new();
  for acc in accounts {
    let entry = by_uuid.entry(acc.uuid.clone()).or_default();
    if !entry.contains(&acc.provider_id) {
      entry.push(acc.provider_id.clone());
    }
  }
  let mut count_by_uuid: BTreeMap<String, usize> = BTreeMap::new();
  for acc in accounts {
    *count_by_uuid.entry(acc.uuid.clone()).or_insert(0) += 1;
  }
  let mut conflicts = Vec::new();
  for (uuid, providers) in by_uuid {
    let total = count_by_uuid.get(&uuid).copied().unwrap_or(0);
    if providers.len() > 1 || total > 1 {
      conflicts.push(ConflictItem { uuid, providers });
    }
  }
  conflicts
}

fn parse_key_or_legacy_muse(val: &Value) -> Option<AccountKey> {
  if let Some(raw_uuid) = val.as_str() {
    return normalize_uuid(raw_uuid).map(|uuid| AccountKey {
      provider_id: ProviderId::muse(),
      uuid,
    });
  }
  let obj = val.as_object()?;
  let raw_uuid = obj
    .get("uuid")
    .or_else(|| obj.get("accountUuid"))
    .or_else(|| obj.get("id"))
    .and_then(|v| v.as_str())?;
  let uuid = normalize_uuid(raw_uuid)?;
  let provider_id = match obj.get("providerId").or_else(|| obj.get("provider_id")) {
    Some(v) => ProviderId::parse(v.as_str()?)?,
    None => ProviderId::muse(),
  };
  Some(AccountKey { provider_id, uuid })
}

fn migrate_from_value(root: Value) -> Result<(AccountsStoreFile, bool), String> {
  let obj = root
    .as_object()
    .ok_or_else(|| "accounts.json 根节点不是对象".to_string())?;

  let file_version = obj
    .get("version")
    .and_then(|v| v.as_u64())
    .map(|v| v as u32)
    .unwrap_or(1);

  let mut dirty = file_version < STORE_VERSION;
  let is_legacy = dirty;

  if let Some(v) = obj
    .get("customProviders")
    .or_else(|| obj.get("custom_providers"))
  {
    if !v.is_null() {
      dirty = true;
    }
  }

  let mut accounts = Vec::new();
  let mut unknown_accounts: Vec<Value> = Vec::new();

  if let Some(arr) = obj.get("accounts").and_then(|v| v.as_array()) {
    for item in arr {
      let item_obj = item
        .as_object()
        .ok_or_else(|| "账号条目格式错误".to_string())?;
      // 未知 / 非内置 provider 的账号：保留原始 JSON，整份配置不失败。
      // 覆盖未来版本的新 provider、已下线的 custom-*、以及 accounts 里的 api/cli。
      if let Some(s) = item_obj
        .get("providerId")
        .or_else(|| item_obj.get("provider_id"))
        .and_then(|v| v.as_str())
      {
        match ProviderId::parse(s) {
          Some(p) if p.is_builtin() => {}
          _ => {
            dirty = true;
            unknown_accounts.push(item.clone());
            continue;
          }
        }
      }
      let raw_uuid = item_obj
        .get("uuid")
        .or_else(|| item_obj.get("id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "账号缺少 UUID".to_string())?;
      let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "账号 UUID 非法".to_string())?;
      let provider_id = match item_obj
        .get("providerId")
        .or_else(|| item_obj.get("provider_id"))
      {
        Some(v) => {
          let s = v.as_str().ok_or_else(|| "providerId 非法".to_string())?;
          ProviderId::parse(s)
            .filter(|p| p.is_builtin())
            .ok_or_else(|| format!("未知服务标识: {s}"))?
        }
        None if is_legacy => ProviderId::muse(),
        None => return Err("新版配置账号缺少 providerId".to_string()),
      };
      let alias = item_obj
        .get("alias")
        .or_else(|| item_obj.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("账号 {}", &uuid[..8]));
      let created_at_ms = item_obj
        .get("createdAtMs")
        .or_else(|| item_obj.get("created_at_ms"))
        .and_then(|v| v.as_u64())
        .unwrap_or_else(now_ms);

      let auto_alias = item_obj
        .get("autoAlias")
        .or_else(|| item_obj.get("auto_alias"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
      let proxy_url = item_obj
        .get("proxyUrl")
        .or_else(|| item_obj.get("proxy_url"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
      let timezone = item_obj
        .get("timezone")
        .or_else(|| item_obj.get("timeZone"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
      let avatar_url = item_obj
        .get("avatarUrl")
        .or_else(|| item_obj.get("avatar_url"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
      let auth_state = item_obj
        .get("authState")
        .or_else(|| item_obj.get("auth_state"))
        .and_then(|v| serde_json::from_value::<AuthState>(v.clone()).ok())
        .unwrap_or_default();
      let mut quota_detail = item_obj
        .get("quotaDetail")
        .or_else(|| item_obj.get("quota_detail"))
        .and_then(|v| serde_json::from_value::<QuotaDetail>(v.clone()).ok())
        .filter(|d| !d.plan_label.trim().is_empty() || !d.plan_value.trim().is_empty());
      let mut quota_text = item_obj
        .get("quotaText")
        .or_else(|| item_obj.get("quota_text"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
      let mut quota_percent = item_obj
        .get("quotaPercent")
        .or_else(|| item_obj.get("quota_percent"))
        .and_then(|v| v.as_u64())
        .map(|n| n.min(100) as u8);
      if quota_detail.is_none() && quota_text.as_deref() == Some("剩余额度 100%") {
        quota_text = None;
        quota_percent = None;
      }
      let mut cookie_expires_at_ms = item_obj
        .get("cookieExpiresAtMs")
        .or_else(|| item_obj.get("cookie_expires_at_ms"))
        .and_then(|v| v.as_u64());
      let mut cookie_session_only = item_obj
        .get("cookieSessionOnly")
        .or_else(|| item_obj.get("cookie_session_only"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
      if cookie_expires_at_ms.is_some() {
        cookie_session_only = false;
      }
      if auth_state == AuthState::LoggedOut {
        quota_detail = None;
        quota_text = None;
        quota_percent = None;
        cookie_expires_at_ms = None;
        cookie_session_only = false;
      }

      accounts.push(AccountRecord {
        provider_id,
        uuid,
        alias,
        created_at_ms,
        auto_alias,
        proxy_url,
        timezone,
        avatar_url,
        quota_text,
        quota_percent,
        quota_detail,
        auth_state,
        cookie_expires_at_ms,
        cookie_session_only,
      });
    }
  }

  let last_committed_raw = obj
    .get("lastCommitted")
    .or_else(|| obj.get("last_committed"))
    .or_else(|| obj.get("lastOpened"))
    .or_else(|| obj.get("last_opened"))
    .filter(|v| !v.is_null());
  let last_committed = last_committed_raw
    .and_then(parse_key_or_legacy_muse)
    .filter(|k| k.provider_id.is_builtin());
  if last_committed_raw.is_some() && last_committed.is_none() {
    dirty = true;
  }

  let mut pending_cleanup = Vec::new();
  if let Some(arr) = obj
    .get("pendingCleanup")
    .or_else(|| obj.get("pending_cleanup"))
    .and_then(|v| v.as_array())
  {
    for v in arr {
      if let Some(key) = parse_key_or_legacy_muse(v).filter(|k| k.provider_id.is_builtin()) {
        if !pending_cleanup.contains(&key) {
          pending_cleanup.push(key);
        }
      } else if !v.is_null() {
        dirty = true;
      }
    }
  }

  let mut settings = AppSettings::default();
  if let Some(settings_obj) = obj.get("settings").and_then(|v| v.as_object()) {
    if let Some(minimize) = settings_obj
      .get("minimizeToTray")
      .or_else(|| settings_obj.get("minimize_to_tray"))
      .and_then(|v| v.as_bool())
    {
      settings.minimize_to_tray = minimize;
    }
    if let Some(shortcut) = settings_obj
      .get("globalShortcut")
      .or_else(|| settings_obj.get("global_shortcut"))
      .and_then(|v| v.as_str())
    {
      settings.global_shortcut = shortcut.trim().to_string();
    }
    if let Some(auto_approve) = settings_obj
      .get("autoApprove")
      .or_else(|| settings_obj.get("auto_approve"))
      .and_then(|v| v.as_bool())
    {
      settings.auto_approve = auto_approve;
    }
    if let Some(disable_auto_update_check) = settings_obj
      .get("disableAutoUpdateCheck")
      .or_else(|| settings_obj.get("disable_auto_update_check"))
      .and_then(|v| v.as_bool())
    {
      settings.disable_auto_update_check = disable_auto_update_check;
    }
    if let Some(auto_approve_commands) = settings_obj
      .get("autoApproveCommands")
      .or_else(|| settings_obj.get("auto_approve_commands"))
      .and_then(|v| v.as_bool())
    {
      settings.auto_approve_commands = auto_approve_commands;
    }
  }

  let mut external_agents = Vec::new();
  let mut seen_external_ids = HashSet::new();
  if let Some(ext_arr) = obj
    .get("externalAgents")
    .or_else(|| obj.get("external_agents"))
    .and_then(|v| v.as_array())
  {
    for ext_val in ext_arr {
      let Some(ext_obj) = ext_val.as_object() else {
        dirty = true;
        continue;
      };
      let Some(kind) = ext_obj
        .get("kind")
        .and_then(|v| serde_json::from_value::<ExternalAgentKind>(v.clone()).ok())
      else {
        dirty = true;
        continue;
      };
      let raw_ext_id = ext_obj
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(normalize_uuid);
      let ext_id = match raw_ext_id {
        Some(eid) if !seen_external_ids.contains(&eid) => eid,
        _ => {
          dirty = true;
          continue;
        }
      };
      let ext_name: String = ext_obj
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().chars().take(40).collect())
        .unwrap_or_default();
      if ext_name.is_empty() {
        dirty = true;
        continue;
      }
      seen_external_ids.insert(ext_id.clone());
      let base_url = ext_obj
        .get("baseUrl")
        .or_else(|| ext_obj.get("base_url"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
      // 旧明文 Key：只留内存，记引用 id，标记 dirty 以便重写去明文；
      // 已迁移的记录只有 apiKeyRef，内存 Key 由 vault 在加载时补。
      let legacy_key = ext_obj
        .get("apiKey")
        .or_else(|| ext_obj.get("api_key"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
      let stored_ref = ext_obj
        .get("apiKeyRef")
        .or_else(|| ext_obj.get("api_key_ref"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
      let (api_key, api_key_ref) = if !legacy_key.is_empty() {
        dirty = true;
        (legacy_key, Some(crate::key_vault::api_key_ref_for(&ext_id)))
      } else {
        (String::new(), stored_ref)
      };
      let model = ext_obj
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().chars().take(120).collect())
        .unwrap_or_default();
      let command = ext_obj
        .get("command")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().chars().take(260).collect())
        .unwrap_or_default();
      let args = ext_obj
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
          arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .take(24)
            .collect::<Vec<_>>()
        })
        .unwrap_or_default();
      let workdir = ext_obj
        .get("workdir")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
      let ext_created = ext_obj
        .get("createdAtMs")
        .or_else(|| ext_obj.get("created_at_ms"))
        .and_then(|v| v.as_u64())
        .unwrap_or_else(now_ms);
      // 旧配置没有这两个字段时按默认值补齐；越界值视为损坏，按默认处理。
      let timeout_secs = ext_obj
        .get("timeoutSecs")
        .or_else(|| ext_obj.get("timeout_secs"))
        .and_then(|v| v.as_u64())
        .filter(|v| (30..=3600).contains(v))
        .unwrap_or_else(default_agent_timeout_secs);
      let idle_timeout_secs = ext_obj
        .get("idleTimeoutSecs")
        .or_else(|| ext_obj.get("idle_timeout_secs"))
        .and_then(|v| v.as_u64())
        .filter(|v| (10..=600).contains(v))
        .unwrap_or_else(default_agent_idle_timeout_secs);
      external_agents.push(ExternalAgentRecord {
        id: ext_id,
        kind,
        name: ext_name,
        base_url,
        api_key: Zeroizing::new(api_key),
        api_key_ref,
        model,
        command,
        args,
        workdir,
        timeout_secs,
        idle_timeout_secs,
        created_at_ms: ext_created,
      });
    }
  }

  let mut valid_account_keys: HashSet<(ProviderId, String)> = accounts
    .iter()
    .map(|a| (a.provider_id.clone(), a.uuid.clone()))
    .collect();
  for ext in &external_agents {
    let k = ext.account_key();
    valid_account_keys.insert((k.provider_id, k.uuid));
  }
  let mut group_chats = Vec::new();
  let mut seen_group_ids = HashSet::new();
  if let Some(arr) = obj
    .get("groupChats")
    .or_else(|| obj.get("group_chats"))
    .and_then(|v| v.as_array())
  {
    for item in arr {
      let Some(item_obj) = item.as_object() else {
        dirty = true;
        continue;
      };
      let raw_name = item_obj
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .unwrap_or("");
      if raw_name.is_empty() {
        dirty = true;
        continue;
      }
      let name: String = raw_name.chars().take(32).collect();
      let raw_id = item_obj
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
      let id = match raw_id {
        Some(existing) if !seen_group_ids.contains(&existing) => existing,
        _ => {
          dirty = true;
          Uuid::new_v4().hyphenated().to_string()
        }
      };
      seen_group_ids.insert(id.clone());

      let mut members = Vec::new();
      if let Some(m_arr) = item_obj.get("members").and_then(|v| v.as_array()) {
        for mv in m_arr {
          if let Some(key) = parse_key_or_legacy_muse(mv) {
            if valid_account_keys.contains(&(key.provider_id.clone(), key.uuid.clone()))
              && !members.contains(&key)
            {
              members.push(key);
            } else {
              dirty = true;
            }
          } else {
            dirty = true;
          }
        }
      }

      let created_at_ms = item_obj
        .get("createdAtMs")
        .or_else(|| item_obj.get("created_at_ms"))
        .and_then(|v| v.as_u64())
        .unwrap_or_else(now_ms);

      let mut messages = Vec::new();
      if let Some(msg_arr) = item_obj.get("messages").and_then(|v| v.as_array()) {
        for msg_val in msg_arr {
          let Some(msg_obj) = msg_val.as_object() else {
            dirty = true;
            continue;
          };
          let role = msg_obj
            .get("role")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| *s == "user" || *s == "assistant")
            .unwrap_or("assistant")
            .to_string();
          let msg_id = msg_obj
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| Uuid::new_v4().hyphenated().to_string());
          let sender = msg_obj
            .get("sender")
            .filter(|v| !v.is_null())
            .and_then(parse_key_or_legacy_muse);
          let sender_name = msg_obj
            .get("senderName")
            .or_else(|| msg_obj.get("sender_name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
          let content = msg_obj
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
          let msg_created = msg_obj
            .get("createdAtMs")
            .or_else(|| msg_obj.get("created_at_ms"))
            .and_then(|v| v.as_u64())
            .unwrap_or(created_at_ms);
          let raw_status = msg_obj
            .get("status")
            .and_then(|v| serde_json::from_value::<GroupReplyStatus>(v.clone()).ok())
            .unwrap_or(GroupReplyStatus::Done);
          let mut error = msg_obj
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
          let status = match raw_status {
            GroupReplyStatus::Queued | GroupReplyStatus::Sending | GroupReplyStatus::Streaming => {
              dirty = true;
              if content.trim().is_empty() {
                error.get_or_insert_with(|| "上次应用退出时未完成生成".to_string());
                GroupReplyStatus::Error
              } else {
                GroupReplyStatus::Done
              }
            }
            other => other,
          };
          let mut targets = Vec::new();
          if let Some(t_arr) = msg_obj.get("targets").and_then(|v| v.as_array()) {
            for tv in t_arr {
              if let Some(k) = parse_key_or_legacy_muse(tv) {
                if !targets.contains(&k) {
                  targets.push(k);
                }
              }
            }
          }
          let stage_label = msg_obj
            .get("stageLabel")
            .or_else(|| msg_obj.get("stage_label"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
          let hop_depth = msg_obj
            .get("hopDepth")
            .or_else(|| msg_obj.get("hop_depth"))
            .and_then(|v| v.as_u64())
            .map(|n| n.min(8) as u8)
            .unwrap_or(0);
          let auto_routed_channel = msg_obj
            .get("autoRoutedChannel")
            .or_else(|| msg_obj.get("auto_routed_channel"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
          messages.push(GroupChatMessage {
            id: msg_id,
            role,
            sender,
            sender_name,
            content,
            created_at_ms: msg_created,
            status,
            targets,
            error,
            stage_label,
            hop_depth,
            auto_routed_channel,
          });
        }
      }
      if messages.len() > 120 {
        let excess = messages.len() - 120;
        messages.drain(..excess);
        dirty = true;
      }

      let shared_context: String = item_obj
        .get("sharedContext")
        .or_else(|| item_obj.get("shared_context"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().chars().take(8000).collect())
        .unwrap_or_default();

      let mut shared_skills = Vec::new();
      let mut seen_skill_ids = HashSet::new();
      if let Some(sk_arr) = item_obj
        .get("sharedSkills")
        .or_else(|| item_obj.get("shared_skills"))
        .and_then(|v| v.as_array())
      {
        for sk_val in sk_arr {
          if shared_skills.len() >= 30 {
            dirty = true;
            break;
          }
          let Some(sk_obj) = sk_val.as_object() else {
            dirty = true;
            continue;
          };
          let sk_name: String = sk_obj
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().chars().take(40).collect())
            .unwrap_or_default();
          let sk_content: String = sk_obj
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().chars().take(2000).collect())
            .unwrap_or_default();
          if sk_name.is_empty() || sk_content.is_empty() {
            dirty = true;
            continue;
          }
          let sk_desc: String = sk_obj
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().chars().take(120).collect())
            .unwrap_or_default();
          let sk_enabled = sk_obj
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
          let raw_sk_id = sk_obj
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
          let sk_id = match raw_sk_id {
            Some(existing) if !seen_skill_ids.contains(&existing) => existing,
            _ => {
              dirty = true;
              Uuid::new_v4().hyphenated().to_string()
            }
          };
          seen_skill_ids.insert(sk_id.clone());
          shared_skills.push(GroupSharedSkill {
            id: sk_id,
            name: sk_name,
            description: sk_desc,
            content: sk_content,
            enabled: sk_enabled,
          });
        }
      }

      let mut member_roles = Vec::new();
      if let Some(r_arr) = item_obj
        .get("memberRoles")
        .or_else(|| item_obj.get("member_roles"))
        .and_then(|v| v.as_array())
      {
        for r_val in r_arr {
          let Some(r_obj) = r_val.as_object() else {
            dirty = true;
            continue;
          };
          let Some(m_key) = r_obj.get("member").and_then(parse_key_or_legacy_muse) else {
            dirty = true;
            continue;
          };
          if !members.contains(&m_key)
            || member_roles
              .iter()
              .any(|mr: &GroupMemberRole| mr.member == m_key)
          {
            dirty = true;
            continue;
          }
          let role_str: String = r_obj
            .get("role")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().chars().take(120).collect())
            .unwrap_or_default();
          if !role_str.is_empty() {
            member_roles.push(GroupMemberRole {
              member: m_key,
              role: role_str,
            });
          }
        }
      }

      let workflow_mode = item_obj
        .get("workflowMode")
        .or_else(|| item_obj.get("workflow_mode"))
        .and_then(|v| serde_json::from_value::<GroupWorkflowMode>(v.clone()).ok())
        .unwrap_or_default();
      let max_relay_hops = item_obj
        .get("maxRelayHops")
        .or_else(|| item_obj.get("max_relay_hops"))
        .and_then(|v| v.as_u64())
        .map(|n| n.clamp(1, 3) as u8)
        .unwrap_or_else(default_relay_hops);

      let mut thread_bindings = Vec::new();
      if let Some(tb_arr) = item_obj
        .get("threadBindings")
        .or_else(|| item_obj.get("thread_bindings"))
        .and_then(|v| v.as_array())
      {
        for tb_val in tb_arr {
          let Some(tb_obj) = tb_val.as_object() else {
            dirty = true;
            continue;
          };
          let Some(m_key) = tb_obj.get("member").and_then(parse_key_or_legacy_muse) else {
            dirty = true;
            continue;
          };
          let thread_url = tb_obj
            .get("threadUrl")
            .or_else(|| tb_obj.get("thread_url"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| s.starts_with("https://") || s.starts_with("http://"))
            .unwrap_or_default();
          if thread_url.is_empty()
            || !members.contains(&m_key)
            || thread_bindings
              .iter()
              .any(|tb: &GroupThreadBinding| tb.member == m_key)
          {
            dirty = true;
            continue;
          }
          let updated_at_ms = tb_obj
            .get("updatedAtMs")
            .or_else(|| tb_obj.get("updated_at_ms"))
            .and_then(|v| v.as_u64())
            .unwrap_or_else(now_ms);
          thread_bindings.push(GroupThreadBinding {
            member: m_key,
            thread_url,
            updated_at_ms,
          });
        }
      }

      if let Some(mm) = item_obj
        .get("memberModels")
        .or_else(|| item_obj.get("member_models"))
      {
        if mm.as_array().is_some_and(|a| !a.is_empty()) {
          dirty = true;
        }
      }

      group_chats.push(GroupChatRecord {
        id,
        name,
        members,
        created_at_ms,
        messages,
        shared_context,
        shared_skills,
        member_roles,
        workflow_mode,
        max_relay_hops,
        thread_bindings,
      });
    }
  }

  Ok((
    AccountsStoreFile {
      version: STORE_VERSION,
      accounts,
      last_committed,
      pending_cleanup,
      group_chats,
      external_agents,
      settings,
      unknown_accounts,
    },
    dirty,
  ))
}

fn corrupt_backup_path(path: &Path, timestamp_ms: u64, attempt: u32) -> PathBuf {
  let file_name = path
    .file_name()
    .map(|n| n.to_string_lossy().into_owned())
    .unwrap_or_else(|| "accounts.json".to_string());
  let suffix = if attempt == 0 {
    format!("corrupt-{timestamp_ms}")
  } else {
    format!("corrupt-{timestamp_ms}-{attempt}")
  };
  path.with_file_name(format!("{file_name}.{suffix}"))
}

/// 加载失败时把原文件复制为 `accounts.json.corrupt-<unix_ms>`。
/// 已有同名备份时追加序号，永不覆盖；复制失败返回 None 并写日志。
fn backup_corrupt_file(path: &Path) -> Option<PathBuf> {
  let timestamp_ms = now_ms();
  for attempt in 0..1000u32 {
    let candidate = corrupt_backup_path(path, timestamp_ms, attempt);
    if candidate.exists() {
      continue;
    }
    return match fs::copy(path, &candidate) {
      Ok(_) => Some(candidate),
      Err(err) => {
        eprintln!("[叠号] 备份损坏配置失败: {err}");
        None
      }
    };
  }
  eprintln!("[叠号] 备份损坏配置失败：无法找到未占用的备份文件名");
  None
}

/// 成功加载后把刚加载过的文件滚动备份为 `accounts.json.bak`（单份滚动覆盖）。
/// 失败只写日志，不影响加载结果。
fn backup_last_good_file(path: &Path) {
  let Some(file_name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
    return;
  };
  let bak = path.with_file_name(format!("{file_name}.bak"));
  if let Err(err) = fs::copy(path, &bak) {
    eprintln!("[叠号] 滚动备份配置失败: {err}");
  }
}

fn backup_file_name(path: &Path) -> String {
  path
    .file_name()
    .map(|n| n.to_string_lossy().into_owned())
    .unwrap_or_else(|| "accounts.json.corrupt-?".to_string())
}

pub fn save_store_atomic(path: &Path, store: &AccountsStoreFile) -> Result<(), String> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
  }
  // 未知 provider 的账号不参与结构体序列化，在此原样拼回 accounts，
  // 使「加载→保存」不丢条目。没有未知条目时输出与原来完全一致。
  let mut value = serde_json::to_value(store).map_err(|e| format!("序列化配置失败: {e}"))?;
  if !store.unknown_accounts.is_empty() {
    if let Some(arr) = value.get_mut("accounts").and_then(|v| v.as_array_mut()) {
      arr.extend(store.unknown_accounts.iter().cloned());
    }
  }
  let bytes = serde_json::to_vec_pretty(&value).map_err(|e| format!("序列化配置失败: {e}"))?;
  let tmp_name = format!(".accounts.json.tmp.{}", Uuid::new_v4().simple());
  let tmp_path = path.with_file_name(tmp_name);

  {
    // P1-7：临时文件按 0600 创建（Unix），最终文件经 rename 继承该权限；
    // 原子写流程（临时文件 → sync_all → rename）本身不变。
    #[cfg(unix)]
    let mut file = {
      use std::os::unix::fs::OpenOptionsExt;
      fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp_path)
        .map_err(|e| format!("创建临时配置文件失败: {e}"))?
    };
    #[cfg(not(unix))]
    let mut file = fs::File::create(&tmp_path).map_err(|e| format!("创建临时配置文件失败: {e}"))?;
    file
      .write_all(&bytes)
      .map_err(|e| format!("写入临时配置文件失败: {e}"))?;
    file
      .sync_all()
      .map_err(|e| format!("同步临时配置文件失败: {e}"))?;
  }

  fs::rename(&tmp_path, path).map_err(|e| {
    let _ = fs::remove_file(&tmp_path);
    format!("原子替换 accounts.json 失败: {e}")
  })
}

/// 异步版 `save_store_atomic`：序列化与写盘在 `spawn_blocking` 中进行，
/// 不占用异步运行时的工作线程。调用者须在调用前克隆好 store；
/// 防抖写线程在写盘期间不持有 `SessionManager::inner` 锁
/// （`persist_locked` 在写盘期间仍持锁，见其文档）。
pub async fn save_store_atomic_async(
  path: PathBuf,
  store: AccountsStoreFile,
) -> Result<(), String> {
  tokio::task::spawn_blocking(move || save_store_atomic(&path, &store))
    .await
    .map_err(|e| format!("写盘任务中断: {e}"))?
}

/// `migrate_keys=false`（移动端）：跳过 vault 迁移与写回，磁盘原文件
/// 不动，等 `run_startup_tasks` 在异步任务里补跑迁移（setup 的
/// `block_on` 里调插件会死锁）；桌面传 `true`，行为与之前完全一致。
pub async fn load_or_migrate_store(
  path: &Path,
  vault: &crate::key_vault::ApiKeyVault,
  migrate_keys: bool,
) -> LoadedAccountsStore {
  if !path.exists() {
    return LoadedAccountsStore {
      store: AccountsStoreFile::default(),
      conflicts: Vec::new(),
      startup_notice: None,
      persist_blocked: false,
      read_only: false,
    };
  }

  let raw = match fs::read_to_string(path) {
    Ok(content) => content,
    Err(err) => {
      let notice = match backup_corrupt_file(path) {
        Some(backup) => format!(
          "读取 accounts.json 失败，已备份为 {}，写盘已被阻止，请选择「以空配置继续」或「从备份恢复」: {err}",
          backup_file_name(&backup)
        ),
        None => format!(
          "读取 accounts.json 失败，且备份也失败了，写盘已被阻止，请勿手动修改 accounts.json: {err}"
        ),
      };
      return LoadedAccountsStore {
        store: AccountsStoreFile::default(),
        conflicts: Vec::new(),
        startup_notice: Some(notice),
        persist_blocked: true,
        read_only: false,
      };
    }
  };

  let parsed: Value = match serde_json::from_str(&raw) {
    Ok(v) => v,
    Err(_) => {
      let notice = match backup_corrupt_file(path) {
        Some(backup) => format!(
          "本地 accounts.json 配置损坏，已备份为 {}，写盘已被阻止，请选择「以空配置继续」或「从备份恢复」",
          backup_file_name(&backup)
        ),
        None => "本地 accounts.json 配置损坏，且备份失败，写盘已被阻止，请勿手动修改 accounts.json".to_string(),
      };
      return LoadedAccountsStore {
        store: AccountsStoreFile::default(),
        conflicts: Vec::new(),
        startup_notice: Some(notice),
        persist_blocked: true,
        read_only: false,
      };
    }
  };

  // 高版本配置只读：先记下，迁移成功也不写回，绝不降级覆盖。
  let file_version = parsed
    .get("version")
    .and_then(|v| v.as_u64())
    .map(|v| v as u32)
    .unwrap_or(1);
  let future_version = file_version > STORE_VERSION;

  let (mut store, migrated) = match migrate_from_value(parsed) {
    Ok(res) => res,
    Err(reason) => {
      let backup = backup_corrupt_file(path);
      let backup_hint = backup
        .as_ref()
        .map(|p| format!("已备份为 {}，", backup_file_name(p)))
        .unwrap_or_else(|| "备份失败，".to_string());
      let notice = if future_version {
        format!(
          "配置版本（v{file_version}）高于当前版本（v{STORE_VERSION}），且解析失败（{reason}），{backup_hint}写盘已被阻止"
        )
      } else {
        format!("解析 accounts.json 失败（{reason}），{backup_hint}写盘已被阻止，请选择「以空配置继续」或「从备份恢复」")
      };
      return LoadedAccountsStore {
        store: AccountsStoreFile::default(),
        conflicts: Vec::new(),
        startup_notice: Some(notice),
        persist_blocked: true,
        read_only: future_version,
      };
    }
  };

  let conflicts = detect_conflicts(&store.accounts);
  if !conflicts.is_empty() {
    store.last_committed = None;
    backup_last_good_file(path);
    return LoadedAccountsStore {
      store,
      conflicts,
      startup_notice: Some("检测到账号 UUID 归属冲突，已停留在管理页等待处理".to_string()),
      persist_blocked: false,
      read_only: false,
    };
  }

  if future_version {
    backup_last_good_file(path);
    return LoadedAccountsStore {
      store,
      conflicts: Vec::new(),
      startup_notice: Some(format!(
        "配置版本（v{file_version}）高于当前版本（v{STORE_VERSION}），已设为只读，写盘已被阻止；请升级应用，或选择「以空配置继续」"
      )),
      persist_blocked: true,
      read_only: true,
    };
  }

  // P1-7：API Key vault 迁移（仅成功加载路径）。见 migrate_agent_keys_to_vault。
  // migrate_keys=false 时连同写回一并跳过：明文 Key 尚未进 vault，
  // 此时写盘会把 api_key（skip_serializing）剥离，崩溃即丢 Key。
  if migrate_keys {
    let key_migration_failed = !migrate_agent_keys_to_vault(&mut store, vault).await;
    if key_migration_failed {
      return LoadedAccountsStore {
        store,
        conflicts: Vec::new(),
        startup_notice: Some(
          "API Key 未能迁移到系统凭据库，已保留原配置，下次启动重试".to_string(),
        ),
        persist_blocked: false,
        read_only: false,
      };
    }
  }

  if migrated && migrate_keys {
    if let Err(err) = save_store_atomic(path, &store) {
      backup_last_good_file(path);
      return LoadedAccountsStore {
        store,
        conflicts: Vec::new(),
        startup_notice: Some(format!("旧版 Muse 配置原子迁移写盘失败: {err}")),
        persist_blocked: false,
        read_only: false,
      };
    }
  }

  backup_last_good_file(path);
  LoadedAccountsStore {
    store,
    conflicts: Vec::new(),
    startup_notice: None,
    persist_blocked: false,
    read_only: false,
  }
}

/// 外部成员 API Key 的 vault 迁移/回填（load 与「从备份恢复」共用）：
/// 含 apiKeyRef 且内存有明文 Key（旧备份）→ 写入 vault；含 apiKeyRef
/// 但内存为空（v5/v6 备份，Key 只在 vault 里）→ 从 vault 取回内存。
/// 返回 false 表示有写入失败：调用方不要把剥离了 Key 的配置写盘，
/// 否则明文 Key 永久丢失（api_key 标了 skip_serializing，写盘即丢）。
pub async fn migrate_agent_keys_to_vault(
  store: &mut AccountsStoreFile,
  vault: &crate::key_vault::ApiKeyVault,
) -> bool {
  let mut ok = true;
  for agent in &mut store.external_agents {
    if let Some(key_ref) = agent.api_key_ref.clone() {
      if !agent.api_key.is_empty() {
        if vault.store_key(&key_ref, &agent.api_key).await.is_err() {
          ok = false;
        }
      } else {
        agent.api_key = vault.load_key(&key_ref).await.unwrap_or_default();
      }
    }
  }
  ok
}

/// 仅内存解析候选配置文本（供「从备份恢复」校验用）：不读盘、不写盘、
/// 不做损坏备份、不做 vault 迁移、不做迁移写回。成功返回可直接写回的
/// LoadedAccountsStore；失败返回原因。未来版本拒绝恢复（重写会丢
/// 未知字段）；账号 UUID 冲突的仍可恢复（last_committed 置空，UI 侧处理）。
pub fn parse_store_str(raw: &str) -> Result<LoadedAccountsStore, String> {
  let parsed: Value = serde_json::from_str(raw).map_err(|e| format!("配置 JSON 解析失败: {e}"))?;
  let file_version = parsed
    .get("version")
    .and_then(|v| v.as_u64())
    .map(|v| v as u32)
    .unwrap_or(1);
  if file_version > STORE_VERSION {
    return Err(format!(
      "配置版本（v{file_version}）高于当前版本（v{STORE_VERSION}），无法恢复"
    ));
  }
  let (mut store, _migrated) =
    migrate_from_value(parsed).map_err(|reason| format!("解析 accounts.json 失败（{reason}）"))?;
  let conflicts = detect_conflicts(&store.accounts);
  if !conflicts.is_empty() {
    store.last_committed = None;
  }
  Ok(LoadedAccountsStore {
    store,
    conflicts,
    startup_notice: None,
    persist_blocked: false,
    read_only: false,
  })
}

#[cfg(test)]
mod tests {
  use super::migrate_from_value;
  use super::{load_or_migrate_store, save_store_atomic};
  use serde_json::json;
  use std::path::PathBuf;
  use uuid::Uuid;

  fn temp_store_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("maam-storage-{tag}-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
  }

  fn corrupt_backups(dir: &std::path::Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
      .unwrap()
      .filter_map(|e| e.ok())
      .map(|e| e.path())
      .filter(|p| {
        p.file_name()
          .and_then(|n| n.to_str())
          .is_some_and(|n| n.starts_with("accounts.json.corrupt-"))
      })
      .collect()
  }

  fn make_bench_store(groups: usize, msgs_per_group: usize) -> super::AccountsStoreFile {
    use super::{GroupChatMessage, GroupChatRecord, GroupReplyStatus, GroupWorkflowMode};
    let mut group_chats = Vec::new();
    for g in 0..groups {
      let messages = (0..msgs_per_group)
        .map(|m| GroupChatMessage {
          id: format!("m-{g}-{m}"),
          role: "assistant".to_string(),
          sender: None,
          sender_name: "bench".to_string(),
          content: "这是一条用于写盘基准测试的群聊消息，内容包含中文与标点符号。".repeat(2),
          created_at_ms: 1_700_000_000_000 + m as u64,
          status: GroupReplyStatus::Done,
          targets: Vec::new(),
          error: None,
          stage_label: None,
          hop_depth: 0,
          auto_routed_channel: None,
        })
        .collect();
      group_chats.push(GroupChatRecord {
        id: format!("g-{g}"),
        name: format!("基准群 {g}"),
        members: Vec::new(),
        created_at_ms: 1,
        messages,
        shared_context: String::new(),
        shared_skills: Vec::new(),
        member_roles: Vec::new(),
        workflow_mode: GroupWorkflowMode::Parallel,
        max_relay_hops: 2,
        thread_bindings: Vec::new(),
      });
    }
    super::AccountsStoreFile {
      version: super::STORE_VERSION,
      accounts: Vec::new(),
      last_committed: None,
      pending_cleanup: Vec::new(),
      group_chats,
      external_agents: Vec::new(),
      settings: super::AppSettings::default(),
      unknown_accounts: Vec::new(),
    }
  }

  #[test]
  fn external_agent_record_serialization_never_contains_api_key() {
    let rec = super::ExternalAgentRecord {
      id: "00000000-0000-4000-8000-000000000051".to_string(),
      kind: super::ExternalAgentKind::OpenaiApi,
      name: "DeepSeek-R".to_string(),
      base_url: "https://api.deepseek.com/v1".to_string(),
      api_key: zeroize::Zeroizing::new("sk-secret-123".to_string()),
      api_key_ref: Some("maam-api-key:00000000-0000-4000-8000-000000000051".to_string()),
      model: "deepseek-chat".to_string(),
      command: String::new(),
      args: Vec::new(),
      workdir: None,
      timeout_secs: super::default_agent_timeout_secs(),
      idle_timeout_secs: super::default_agent_idle_timeout_secs(),
      created_at_ms: 1,
    };
    let text = serde_json::to_string(&rec).expect("serialize record");
    assert!(
      !text.contains("sk-secret-123"),
      "明文 API Key 不得出现在序列化结果中: {text}"
    );
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(
      value.get("apiKey").is_none(),
      "序列化结果不得含 apiKey 字段"
    );
    assert_eq!(
      value.get("apiKeyRef").and_then(|v| v.as_str()),
      Some("maam-api-key:00000000-0000-4000-8000-000000000051")
    );
    // 反序列化仍能读旧明文（兼容直接读盘的旧文件），但再序列化即丢弃。
    let legacy: super::ExternalAgentRecord = serde_json::from_value(json!({
      "id": "00000000-0000-4000-8000-000000000051",
      "kind": "openai_api",
      "name": "DeepSeek-R",
      "baseUrl": "https://api.deepseek.com/v1",
      "apiKey": "sk-legacy",
      "model": "deepseek-chat"
    }))
    .unwrap();
    assert_eq!(legacy.api_key.as_str(), "sk-legacy");
  }

  #[test]
  fn migrate_legacy_plaintext_api_key_gets_key_ref() {
    let raw = json!({
      "version": 2,
      "accounts": [],
      "externalAgents": [
        {
          "id": "00000000-0000-4000-8000-000000000051",
          "kind": "openai_api",
          "name": "DeepSeek-R",
          "baseUrl": "https://api.deepseek.com/v1",
          "apiKey": "sk-test",
          "model": "deepseek-chat"
        },
        {
          "id": "00000000-0000-4000-8000-000000000052",
          "kind": "openai_api",
          "name": "NoKey",
          "baseUrl": "https://api.example.com/v1",
          "model": "m1"
        }
      ]
    });
    let (store, dirty) = migrate_from_value(raw).expect("migrate");
    assert!(dirty, "含旧明文 Key 应标记 dirty 以便重写去明文");
    assert_eq!(store.external_agents.len(), 2);
    assert_eq!(store.external_agents[0].api_key.as_str(), "sk-test");
    assert_eq!(
      store.external_agents[0].api_key_ref.as_deref(),
      Some("maam-api-key:00000000-0000-4000-8000-000000000051")
    );
    assert!(store.external_agents[1].api_key_ref.is_none());
    // 整库序列化后不得含明文。
    let text = serde_json::to_string(&store).unwrap();
    assert!(!text.contains("sk-test"));
    // 已迁移的记录（只有 apiKeyRef）再次迁移不再 dirty，
    // 且内存 Key 为空（由 vault 在加载时补）。
    let round = serde_json::to_value(&store).unwrap();
    let (store2, dirty2) = migrate_from_value(round).unwrap();
    assert!(!dirty2);
    assert_eq!(
      store2.external_agents[0].api_key_ref,
      store.external_agents[0].api_key_ref
    );
    assert!(store2.external_agents[0].api_key.is_empty());
  }

  #[tokio::test]
  async fn load_or_migrate_store_moves_legacy_api_key_to_vault() {
    let dir = std::env::temp_dir().join(format!("maam-load-migrate-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("accounts.json");
    std::fs::write(
      &path,
      serde_json::to_string_pretty(&json!({
        "version": 2,
        "accounts": [],
        "externalAgents": [{
          "id": "00000000-0000-4000-8000-000000000051",
          "kind": "openai_api",
          "name": "DeepSeek-R",
          "baseUrl": "https://api.deepseek.com/v1",
          "apiKey": "sk-legacy-999",
          "model": "deepseek-chat"
        }]
      }))
      .unwrap(),
    )
    .unwrap();

    let vault = crate::key_vault::ApiKeyVault::new_memory(dir.clone());
    let loaded = super::load_or_migrate_store(&path, &vault, true).await;
    assert_eq!(loaded.store.external_agents.len(), 1);
    // 内存里仍有 Key（运行时调用要用）。
    assert_eq!(
      loaded.store.external_agents[0].api_key.as_str(),
      "sk-legacy-999"
    );
    assert!(loaded.store.external_agents[0].api_key_ref.is_some());
    // 磁盘上的 accounts.json 已重写，不再含明文。
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(
      !on_disk.contains("sk-legacy-999"),
      "磁盘配置不得再含明文 Key"
    );
    assert!(on_disk.contains("apiKeyRef"), "磁盘配置应存引用 id");
    // 再次加载：内存 Key 应从 vault 恢复。
    let reloaded = super::load_or_migrate_store(&path, &vault, true).await;
    assert_eq!(
      reloaded.store.external_agents[0].api_key.as_str(),
      "sk-legacy-999"
    );
    let _ = std::fs::remove_dir_all(&dir);
  }

  /// 安全后端不可用时加载含明文 Key 的旧配置：迁移失败不丢 Key，
  /// 返回原配置 + startup_notice，磁盘原文件不动（下次启动重试）。
  /// 按实际结果分情况断言，不写死环境假设（有 Keychain 的机器上
  /// 迁移成功、磁盘被剥离明文；无 secret service 的机器上迁移失败、
  /// 磁盘保留原配置）。
  #[tokio::test]
  async fn load_or_migrate_store_keeps_config_when_vault_unavailable() {
    let dir = std::env::temp_dir().join(format!("maam-load-keeps-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("accounts.json");
    std::fs::write(
      &path,
      serde_json::to_string_pretty(&json!({
        "version": 2,
        "accounts": [],
        "externalAgents": [{
          "id": "00000000-0000-4000-8000-000000000052",
          "kind": "openai_api",
          "name": "DeepSeek-R",
          "baseUrl": "https://api.deepseek.com/v1",
          "apiKey": "sk-legacy-888",
          "model": "deepseek-chat"
        }]
      }))
      .unwrap(),
    )
    .unwrap();

    let vault = crate::key_vault::ApiKeyVault::new(dir.clone());
    let loaded = super::load_or_migrate_store(&path, &vault, true).await;
    // 无论迁移成败，内存 Key 都不能丢。
    assert_eq!(
      loaded.store.external_agents[0].api_key.as_str(),
      "sk-legacy-888"
    );
    let on_disk = std::fs::read_to_string(&path).unwrap();
    if loaded
      .startup_notice
      .as_deref()
      .unwrap_or("")
      .contains("未能迁移")
    {
      assert!(
        on_disk.contains("sk-legacy-888"),
        "迁移失败时磁盘原配置应保留"
      );
    } else {
      assert!(
        !on_disk.contains("sk-legacy-888"),
        "迁移成功时磁盘应已剥离明文"
      );
      assert!(on_disk.contains("apiKeyRef"), "磁盘配置应存引用 id");
    }
    let _ = std::fs::remove_dir_all(&dir);
  }

  /// 移动端延后迁移路径：load 时 migrate_keys=false 不碰 vault 也不写盘，
  /// 磁盘原文件不动、内存 Key 保留；随后补跑迁移，Key 进 vault。
  #[tokio::test]
  async fn load_or_migrate_store_defers_key_migration_when_flag_false() {
    let dir = std::env::temp_dir().join(format!("maam-load-defer-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("accounts.json");
    std::fs::write(
      &path,
      serde_json::to_string_pretty(&json!({
        "version": 2,
        "accounts": [],
        "externalAgents": [{
          "id": "00000000-0000-4000-8000-000000000053",
          "kind": "openai_api",
          "name": "DeepSeek-R",
          "baseUrl": "https://api.deepseek.com/v1",
          "apiKey": "sk-defer-777",
          "model": "deepseek-chat"
        }]
      }))
      .unwrap(),
    )
    .unwrap();

    let vault = crate::key_vault::ApiKeyVault::new_memory(dir.clone());
    let loaded = super::load_or_migrate_store(&path, &vault, false).await;
    assert_eq!(
      loaded.store.external_agents[0].api_key.as_str(),
      "sk-defer-777"
    );
    assert!(loaded.startup_notice.is_none());
    // 磁盘原文件不动（仍含旧明文），等启动任务迁移成功后再写盘剥离。
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(
      on_disk.contains("sk-defer-777"),
      "延后迁移时磁盘原文件应保留"
    );
    // 补跑迁移（run_startup_tasks 移动端分支同款调用）：Key 进 vault。
    let mut store = loaded.store;
    assert!(super::migrate_agent_keys_to_vault(&mut store, &vault).await);
    assert_eq!(
      vault
        .load_key("maam-api-key:00000000-0000-4000-8000-000000000053")
        .await
        .as_deref()
        .map(String::as_str),
      Some("sk-defer-777")
    );
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn save_store_atomic_writes_0600_on_unix() {
    let dir = std::env::temp_dir().join(format!("maam-save-0600-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("accounts.json");
    let store = super::AccountsStoreFile::default();
    super::save_store_atomic(&path, &store).unwrap();
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
      assert_eq!(mode, 0o600, "accounts.json 应为 0600，不得用默认权限落盘");
    }
    // 覆盖写（rename 替换）后权限仍为 0600。
    super::save_store_atomic(&path, &store).unwrap();
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
      assert_eq!(mode, 0o600);
    }
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[tokio::test]
  async fn bench_save_store_async_50k_matches_sync() {
    let dir = std::env::temp_dir().join(format!("maam-bench-async-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let p_sync = dir.join("a.json");
    let p_async = dir.join("b.json");

    let store = make_bench_store(1, 50_000);
    let start = std::time::Instant::now();
    super::save_store_atomic(&p_sync, &store).unwrap();
    let sync_elapsed = start.elapsed();
    let start = std::time::Instant::now();
    super::save_store_atomic_async(p_async.clone(), store)
      .await
      .unwrap();
    let async_elapsed = start.elapsed();
    let b_sync = std::fs::read(&p_sync).unwrap();
    let b_async = std::fs::read(&p_async).unwrap();
    assert_eq!(b_sync, b_async, "异步写盘结果应与同步逐字节一致");
    eprintln!(
      "BENCH save 1x50000 sync: {sync_elapsed:?} ({} bytes); async: {async_elapsed:?}",
      b_sync.len()
    );
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn bench_save_store_50k_messages_reports_timing() {
    let dir = std::env::temp_dir().join(format!("maam-bench-save-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("accounts.json");

    // 最坏情况：1 群 5 万条消息（直接构造 store，绕过加载时每群 120 条的截断）。
    let store = make_bench_store(1, 50_000);
    let start = std::time::Instant::now();
    super::save_store_atomic(&path, &store).unwrap();
    let elapsed = start.elapsed();
    let bytes = std::fs::metadata(&path).unwrap().len();
    eprintln!("BENCH save_store_atomic 1x50000 msgs: {elapsed:?} ({bytes} bytes)");

    //  realistic 情况：500 群 x 100 条（加载后每群不超过 120 条的上界附近）。
    let store2 = make_bench_store(500, 100);
    let start = std::time::Instant::now();
    super::save_store_atomic(&path, &store2).unwrap();
    let elapsed2 = start.elapsed();
    let bytes2 = std::fs::metadata(&path).unwrap().len();
    eprintln!("BENCH save_store_atomic 500x100 msgs: {elapsed2:?} ({bytes2} bytes)");

    // 写出的文件必须是合法 JSON，且确实包含 5 万条消息量级的内容。
    assert!(bytes > 1_000_000, "50k 消息文件应大于 1MB，实际 {bytes}");
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn migrate_proxy_url_accepts_camel_snake_and_malformed_types() {
    let raw = json!({
      "version": 2,
      "accounts": [
        {
          "providerId": "muse",
          "uuid": "00000000-0000-4000-8000-000000000001",
          "alias": "camel",
          "proxyUrl": " http://127.0.0.1:7890 "
        },
        {
          "providerId": "muse",
          "uuid": "00000000-0000-4000-8000-000000000002",
          "alias": "snake",
          "proxy_url": "http://127.0.0.1:8888"
        },
        {
          "providerId": "muse",
          "uuid": "00000000-0000-4000-8000-000000000003",
          "alias": "null_and_empty",
          "proxyUrl": null,
          "proxy_url": "   "
        },
        {
          "providerId": "muse",
          "uuid": "00000000-0000-4000-8000-000000000004",
          "alias": "bad_type",
          "proxyUrl": 12345
        }
      ]
    });

    let (store, _) = migrate_from_value(raw).expect("migrate should tolerate malformed proxyUrl");
    assert_eq!(store.accounts.len(), 4);
    assert_eq!(
      store.accounts[0].proxy_url.as_deref(),
      Some("http://127.0.0.1:7890")
    );
    assert_eq!(
      store.accounts[1].proxy_url.as_deref(),
      Some("http://127.0.0.1:8888")
    );
    assert_eq!(store.accounts[2].proxy_url, None);
    assert_eq!(store.accounts[3].proxy_url, None);
    assert!(store.settings.minimize_to_tray);
    assert!(!store.settings.global_shortcut.is_empty());
  }

  #[test]
  fn migrate_custom_providers_settings_and_drops_orphans() {
    let raw = json!({
      "version": 2,
      "customProviders": [
        {
          "id": "custom-abcd1234",
          "name": "Gemini",
          "entryUrl": "https://gemini.google.com/"
        }
      ],
      "lastCommitted": {
        "providerId": "custom-abcd1234",
        "uuid": "00000000-0000-4000-8000-000000000010"
      },
      "settings": {
        "minimizeToTray": false,
        "globalShortcut": "Alt+Space"
      },
      "accounts": [
        {
          "providerId": "muse",
          "uuid": "00000000-0000-4000-8000-000000000001",
          "alias": "m1"
        },
        {
          "providerId": "custom-abcd1234",
          "uuid": "00000000-0000-4000-8000-000000000010",
          "alias": "gemini1"
        },
        {
          "providerId": "custom-deleted99",
          "uuid": "00000000-0000-4000-8000-000000000011",
          "alias": "orphan"
        }
      ]
    });

    let (store, dirty) =
      migrate_from_value(raw).expect("legacy custom provider store should migrate cleanly");
    assert!(dirty);
    assert!(store.last_committed.is_none());
    assert_eq!(store.accounts.len(), 1);
    assert_eq!(store.accounts[0].alias, "m1");
    assert!(!store.settings.minimize_to_tray);
    assert_eq!(store.settings.global_shortcut, "Alt+Space");
  }

  #[test]
  fn migrate_cookie_expiry_supports_camel_snake_and_clears_logged_out() {
    let raw = json!({
      "version": 2,
      "accounts": [
        {
          "providerId": "muse",
          "uuid": "00000000-0000-4000-8000-000000000021",
          "alias": "auth_camel",
          "authState": "authenticated",
          "cookieExpiresAtMs": 1800000000000u64,
          "cookieSessionOnly": false
        },
        {
          "providerId": "muse",
          "uuid": "00000000-0000-4000-8000-000000000022",
          "alias": "session_snake",
          "auth_state": "authenticated",
          "cookie_session_only": true
        },
        {
          "providerId": "muse",
          "uuid": "00000000-0000-4000-8000-000000000023",
          "alias": "logged_out_cleared",
          "authState": "logged_out",
          "cookieExpiresAtMs": 1800000000000u64,
          "cookieSessionOnly": true
        }
      ]
    });

    let (store, _) = migrate_from_value(raw).expect("migrate cookie expiry should succeed");
    assert_eq!(store.accounts.len(), 3);
    assert_eq!(
      store.accounts[0].cookie_expires_at_ms,
      Some(1_800_000_000_000)
    );
    assert!(!store.accounts[0].cookie_session_only);
    assert_eq!(store.accounts[1].cookie_expires_at_ms, None);
    assert!(store.accounts[1].cookie_session_only);
    assert_eq!(store.accounts[2].cookie_expires_at_ms, None);
    assert!(!store.accounts[2].cookie_session_only);
  }

  #[test]
  fn migrate_group_chats_prunes_orphans_and_recovers_inflight_messages() {
    use super::GroupReplyStatus;

    let raw = json!({
      "version": 2,
      "accounts": [
        {
          "providerId": "muse",
          "uuid": "00000000-0000-4000-8000-000000000031",
          "alias": "m1",
          "authState": "authenticated"
        }
      ],
      "groupChats": [
        {
          "id": "g1",
          "name": "研究群",
          "members": [
            { "providerId": "muse", "uuid": "00000000-0000-4000-8000-000000000031" },
            { "providerId": "muse", "uuid": "00000000-0000-4000-8000-000000000031" },
            { "providerId": "hark", "uuid": "00000000-0000-4000-8000-000000000099" }
          ],
          "sharedContext": "  使用简体中文回答  ",
          "sharedSkills": [
            {
              "id": "sk1",
              "name": "严谨校验",
              "description": "检查边界条件",
              "content": "给出结论前说明前置条件",
              "enabled": true
            }
          ],
          "memberRoles": [
            {
              "member": { "providerId": "muse", "uuid": "00000000-0000-4000-8000-000000000031" },
              "role": "主架构师"
            },
            {
              "member": { "providerId": "hark", "uuid": "00000000-0000-4000-8000-000000000099" },
              "role": "已移除成员角色"
            }
          ],
          "messages": [
            {
              "id": "m_stream_partial",
              "role": "assistant",
              "senderName": "m1",
              "content": "partial reply",
              "status": "streaming"
            },
            {
              "id": "m_sending_empty",
              "role": "assistant",
              "senderName": "m1",
              "content": "",
              "status": "sending"
            },
            {
              "id": "m_queued_empty",
              "role": "assistant",
              "senderName": "m1",
              "content": "",
              "status": "queued"
            }
          ]
        }
      ]
    });

    let (store, dirty) = migrate_from_value(raw).expect("migrate groupChats should succeed");
    assert!(dirty);
    assert_eq!(store.group_chats.len(), 1);
    let g = &store.group_chats[0];
    assert_eq!(g.members.len(), 1);
    assert_eq!(g.members[0].uuid, "00000000-0000-4000-8000-000000000031");
    assert_eq!(g.shared_context, "使用简体中文回答");
    assert_eq!(g.shared_skills.len(), 1);
    assert_eq!(g.shared_skills[0].name, "严谨校验");
    assert_eq!(g.member_roles.len(), 1);
    assert_eq!(g.member_roles[0].role, "主架构师");
    assert_eq!(g.messages.len(), 3);
    assert_eq!(g.messages[0].status, GroupReplyStatus::Done);
    assert_eq!(g.messages[1].status, GroupReplyStatus::Error);
    assert!(g.messages[1].error.is_some());
    assert_eq!(g.messages[2].status, GroupReplyStatus::Error);
  }

  #[test]
  fn migrate_external_agents_workflow_and_thread_bindings_roundtrip() {
    use super::{ExternalAgentKind, GroupWorkflowMode};

    let raw = json!({
      "version": 2,
      "accounts": [
        {
          "providerId": "muse",
          "uuid": "00000000-0000-4000-8000-000000000041",
          "alias": "m1",
          "authState": "authenticated"
        }
      ],
      "externalAgents": [
        {
          "id": "00000000-0000-4000-8000-000000000051",
          "kind": "openai_api",
          "name": "DeepSeek-R",
          "baseUrl": "https://api.deepseek.com/v1",
          "apiKey": "sk-test",
          "model": "deepseek-chat"
        },
        {
          "id": "00000000-0000-4000-8000-000000000052",
          "kind": "local_cli",
          "name": "ClaudeCLI",
          "command": "claude",
          "args": ["-p"],
          "workdir": "/tmp"
        },
        {
          "id": "invalid-id",
          "kind": "local_cli",
          "name": "BadAgent"
        }
      ],
      "groupChats": [
        {
          "id": "g_hybrid",
          "name": "混合协同群",
          "members": [
            { "providerId": "muse", "uuid": "00000000-0000-4000-8000-000000000041" },
            { "providerId": "api", "uuid": "00000000-0000-4000-8000-000000000051" },
            { "providerId": "cli", "uuid": "00000000-0000-4000-8000-000000000052" }
          ],
          "workflowMode": "review",
          "maxRelayHops": 9,
          "threadBindings": [
            {
              "member": { "providerId": "muse", "uuid": "00000000-0000-4000-8000-000000000041" },
              "threadUrl": "https://muse.ai/c/thread-123",
              "updatedAtMs": 12345
            },
            {
              "member": { "providerId": "hark", "uuid": "00000000-0000-4000-8000-000000000099" },
              "threadUrl": "https://app.hark.com/chat"
            }
          ],
          "messages": [
            {
              "id": "m1",
              "role": "assistant",
              "sender": { "providerId": "api", "uuid": "00000000-0000-4000-8000-000000000051" },
              "senderName": "DeepSeek-R",
              "content": "review ok",
              "stageLabel": "交叉评审",
              "hopDepth": 1
            }
          ]
        }
      ]
    });

    let (store, dirty) = migrate_from_value(raw).expect("migrate hybrid store should succeed");
    assert!(dirty);
    assert_eq!(store.external_agents.len(), 2);
    assert_eq!(store.external_agents[0].kind, ExternalAgentKind::OpenaiApi);
    assert_eq!(store.external_agents[1].kind, ExternalAgentKind::LocalCli);
    assert_eq!(store.group_chats.len(), 1);
    let g = &store.group_chats[0];
    assert_eq!(g.members.len(), 3);
    assert_eq!(g.workflow_mode, GroupWorkflowMode::Review);
    assert_eq!(g.max_relay_hops, 3);
    assert_eq!(g.thread_bindings.len(), 1);
    assert_eq!(
      g.thread_bindings[0].thread_url,
      "https://muse.ai/c/thread-123"
    );
    assert_eq!(g.messages[0].stage_label.as_deref(), Some("交叉评审"));
    assert_eq!(g.messages[0].hop_depth, 1);

    let reserialized = serde_json::to_value(&store).expect("serialize store");
    assert!(
      !reserialized.to_string().contains("sk-test"),
      "序列化后不得含明文 Key"
    );
    let (roundtrip, dirty2) = migrate_from_value(reserialized).expect("roundtrip migrate");
    assert!(!dirty2);
    // JSON 往返不携带内存 Key（由 vault 在加载时恢复），其余字段应一致。
    let mut expected_agents = store.external_agents.clone();
    expected_agents[0].api_key.clear();
    assert_eq!(roundtrip.external_agents, expected_agents);
    assert_eq!(
      roundtrip.group_chats[0].workflow_mode,
      GroupWorkflowMode::Review
    );
    assert_eq!(roundtrip.group_chats[0].thread_bindings, g.thread_bindings);
    assert!(!roundtrip.settings.auto_approve);

    let with_auto = serde_json::json!({
      "version": 1,
      "accounts": [],
      "settings": { "minimizeToTray": false, "autoApprove": true }
    });
    let (migrated_auto, _) =
      migrate_from_value(with_auto).expect("migrate settings with autoApprove");
    assert!(!migrated_auto.settings.minimize_to_tray);
    assert!(migrated_auto.settings.auto_approve);
    // 旧配置里没有命令执行开关时默认关闭。
    assert!(!migrated_auto.settings.auto_approve_commands);
    assert!(!roundtrip.settings.auto_approve_commands);

    let with_cmd = serde_json::json!({
      "version": 1,
      "accounts": [],
      "settings": { "autoApprove": true, "autoApproveCommands": true }
    });
    let (migrated_cmd, _) =
      migrate_from_value(with_cmd).expect("migrate settings with autoApproveCommands");
    assert!(migrated_cmd.settings.auto_approve);
    assert!(migrated_cmd.settings.auto_approve_commands);
  }
  #[tokio::test]
  async fn corrupt_json_load_keeps_original_and_writes_corrupt_backup() {
    let dir = temp_store_dir("corrupt");
    let path = dir.join("accounts.json");
    let original = "{ this is not json, missing quote";
    std::fs::write(&path, original).unwrap();

    let vault = crate::key_vault::ApiKeyVault::new(dir.clone());
    let loaded = load_or_migrate_store(&path, &vault, true).await;

    // 原文件必须原样保留，不能被空配置覆盖。
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    // 必须留下一份 .corrupt-* 备份，内容与原文件一致。
    let backups = corrupt_backups(&dir);
    assert_eq!(backups.len(), 1, "应恰好产生一份 .corrupt-* 备份");
    assert_eq!(std::fs::read_to_string(&backups[0]).unwrap(), original);
    assert!(loaded.persist_blocked, "损坏配置加载后必须阻止写盘");
    assert!(loaded
      .startup_notice
      .as_deref()
      .unwrap_or_default()
      .contains("备份"));
    assert!(loaded.store.accounts.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[tokio::test]
  async fn unreadable_config_load_is_persist_blocked() {
    let dir = temp_store_dir("unreadable");
    let path = dir.join("accounts.json");
    std::fs::create_dir(&path).unwrap();

    let vault = crate::key_vault::ApiKeyVault::new(dir.clone());
    let loaded = load_or_migrate_store(&path, &vault, true).await;

    assert!(loaded.persist_blocked, "读取失败后必须阻止写盘");
    assert!(loaded.startup_notice.is_some());
    assert!(path.is_dir(), "原路径不应被删除或覆盖");
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[tokio::test]
  async fn unknown_provider_account_survives_load_save_roundtrip() {
    let dir = temp_store_dir("unknown-provider");
    let path = dir.join("accounts.json");
    let raw = json!({
      "version": 2,
      "accounts": [
        {
          "providerId": "muse",
          "uuid": "00000000-0000-4000-8000-000000000001",
          "alias": "m1"
        },
        {
          "providerId": "futurechat",
          "uuid": "00000000-0000-4000-8000-000000000002",
          "alias": "未来服务账号",
          "customField": "keep-me"
        }
      ]
    });
    std::fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

    let vault = crate::key_vault::ApiKeyVault::new(dir.clone());
    let loaded = load_or_migrate_store(&path, &vault, true).await;
    assert_eq!(loaded.store.accounts.len(), 1);
    assert_eq!(loaded.store.accounts[0].alias, "m1");
    assert!(
      !loaded.persist_blocked,
      "未知 provider 不应导致整份配置失败"
    );

    save_store_atomic(&path, &loaded.store).unwrap();
    let saved: serde_json::Value =
      serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let accounts = saved.get("accounts").and_then(|v| v.as_array()).unwrap();
    assert_eq!(accounts.len(), 2, "未知 provider 条目在保存后不应丢失");
    let kept = accounts
      .iter()
      .find(|a| a.get("providerId").and_then(|v| v.as_str()) == Some("futurechat"))
      .expect("未知 provider 条目应原样写回");
    assert_eq!(
      kept.get("alias").and_then(|v| v.as_str()),
      Some("未来服务账号")
    );
    assert_eq!(
      kept.get("customField").and_then(|v| v.as_str()),
      Some("keep-me"),
      "未知条目未知字段也应原样保留"
    );
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[tokio::test]
  async fn future_version_store_is_read_only() {
    let dir = temp_store_dir("future-version");
    let path = dir.join("accounts.json");
    let raw = json!({
      "version": 99,
      "accounts": [
        {
          "providerId": "muse",
          "uuid": "00000000-0000-4000-8000-000000000001",
          "alias": "m1"
        }
      ]
    });
    let original = serde_json::to_string_pretty(&raw).unwrap();
    std::fs::write(&path, &original).unwrap();

    let vault = crate::key_vault::ApiKeyVault::new(dir.clone());
    let loaded = load_or_migrate_store(&path, &vault, true).await;

    assert!(loaded.read_only, "version 高于 STORE_VERSION 时应视为只读");
    assert!(loaded.persist_blocked, "只读配置必须阻止写盘");
    assert_eq!(loaded.store.accounts.len(), 1);
    assert_eq!(loaded.store.accounts[0].alias, "m1");
    // 加载本身绝不能把高版本配置降级写回磁盘。
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[tokio::test]
  async fn successful_load_writes_rolling_bak() {
    let dir = temp_store_dir("bak");
    let path = dir.join("accounts.json");
    let raw = json!({
      "version": 2,
      "accounts": [
        {
          "providerId": "muse",
          "uuid": "00000000-0000-4000-8000-000000000001",
          "alias": "m1"
        }
      ]
    });
    let original = serde_json::to_string_pretty(&raw).unwrap();
    std::fs::write(&path, &original).unwrap();

    let vault = crate::key_vault::ApiKeyVault::new(dir.clone());
    let loaded = load_or_migrate_store(&path, &vault, true).await;

    assert!(!loaded.persist_blocked);
    let bak = dir.join("accounts.json.bak");
    assert!(bak.exists(), "成功加载后应滚动备份 accounts.json.bak");
    assert_eq!(std::fs::read_to_string(&bak).unwrap(), original);
    let _ = std::fs::remove_dir_all(&dir);
  }
}
