use crate::platform::{self, ProxyTestResult};
#[cfg(any(mobile, test))]
use crate::providers::{classify_top_level_url, NavigationDecision};
use crate::providers::{resolve_provider, ProviderId, ProviderInfo};
#[cfg(all(desktop, test))]
use crate::storage::GroupChatMessage;
#[cfg(all(desktop, test))]
use crate::storage::GroupThreadBinding;
use crate::storage::{
  detect_conflicts, load_or_migrate_store, normalize_uuid, now_ms, AccountKey, AccountRecord,
  AccountsStoreFile, AppSettings, AuthState, ConflictItem, ExternalAgentView, GroupChatRecord,
  QuotaDetail,
};
#[cfg(all(desktop, test))]
use crate::storage::{
  ExternalAgentKind, GroupMemberRole, GroupReplyStatus, GroupSharedSkill, GroupWorkflowMode,
};
use serde::Serialize;
#[cfg(desktop)]
use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
#[cfg(desktop)]
use tauri::Manager;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;

pub const APP_TITLE: &str = "叠号";
const MAX_GROUP_MEMBERS: usize = 30;
#[cfg(desktop)]
const REVIEW_INLINE_CHAR_LIMIT: usize = 1200;
#[cfg(desktop)]
const REVIEW_FALLBACK_CHAR_LIMIT: usize = 4500;
#[cfg_attr(not(desktop), allow(dead_code))]
const AUTO_ROUTE_STICKY_MAX_USED_PCT: u8 = 85;
#[cfg_attr(not(desktop), allow(dead_code))]
const AUTO_ROUTE_COOLDOWN_MS: u64 = 120_000;
#[cfg(desktop)]
const LEFT_PANE_WIDTH: f64 = 380.0;
#[cfg(desktop)]
const MAX_WARM_DESKTOP_VIEWS: usize = 10;
#[cfg(desktop)]
const GROUP_MAX_CONCURRENT_WORKERS: usize = 6;
#[cfg(desktop)]
const GROUP_IDLE_RECLAIM_MS: u64 = 10 * 60 * 1000;
#[cfg(desktop)]
const MAX_CONCURRENT_PREWARMS: usize = 2;

mod a2a;
mod group_chat;
mod quota_sync;
mod scripts;
mod store;
mod webview;

#[cfg(desktop)]
pub(crate) use scripts::{
  build_group_attach_file_js, build_group_check_attached_js, build_group_poll_js,
  build_group_send_js, build_group_sync_latest_js, build_hark_click_preview_card_js,
  build_hark_close_expanded_js, build_hark_read_expanded_js, build_new_chat_click_js,
  AUTO_APPROVE_INIT_JS, BACKGROUND_RAF_INIT_JS, DOM_PAINT_READY_JS, IDENTITY_PROBE_JS,
  LOGIN_CAPTURE_INIT_JS,
};

#[cfg(desktop)]
#[derive(Debug, Clone)]
pub(crate) struct GroupAttachment {
  filename: String,
  mime_type: String,
  content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountPageError {
  pub provider_id: ProviderId,
  pub uuid: String,
  pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerSnapshot {
  pub platform: &'static str,
  pub providers: Vec<ProviderInfo>,
  pub accounts: Vec<AccountRecord>,
  pub selected_provider_id: ProviderId,
  pub active_account: Option<AccountKey>,
  pub switching_account: Option<AccountKey>,
  pub warm_accounts: Vec<AccountKey>,
  pub last_committed: Option<AccountKey>,
  pub pending_cleanup: Vec<AccountKey>,
  pub conflicts: Vec<ConflictItem>,
  pub notice: Option<String>,
  pub modal_active: bool,
  pub settings: AppSettings,
  pub global_shortcut_active: bool,
  pub global_shortcut_error: Option<String>,
  pub group_chats: Vec<GroupChatRecord>,
  pub active_group_id: Option<String>,
  pub external_agents: Vec<ExternalAgentView>,
  pub account_errors: Vec<AccountPageError>,
  /// 配置加载失败或版本过高时为 true：写盘已被阻止（见 `persist_locked`）。
  pub persist_blocked: bool,
  /// 配置 version 高于当前 STORE_VERSION 时为 true：只读。
  pub read_only: bool,
}

#[cfg(desktop)]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadNoticeEvent {
  pub id: u64,
  pub filename: String,
  pub status: &'static str,
  pub error: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct PendingExternalAuth {
  account_key: AccountKey,
  switch_seq: u64,
  nonce: String,
}

#[cfg(desktop)]
struct WarmDesktopEntry {
  account_key: AccountKey,
  view_instance_id: u64,
  webview_label: String,
  popup_label: Option<String>,
  paint_ready: bool,
  staged_painted: bool,
  navigation_reached: bool,
  load_gen: Arc<AtomicU64>,
  last_url: Option<Url>,
  captured_identity_hint: Option<String>,
  watcher_running: bool,
  probe_notify: Arc<tokio::sync::Notify>,
  last_used_ms: u64,
  alive_flag: Arc<AtomicBool>,
}

struct ActiveSessionSlot {
  account_key: AccountKey,
  view_instance_id: u64,
  #[cfg_attr(desktop, allow(dead_code))]
  switch_seq: u64,
  committed: bool,
}

mod quota;

#[cfg_attr(not(desktop), allow(dead_code))]
pub(crate) use quota::{evaluate_account_quota, AccountQuotaEval};

struct InnerState {
  store_path: PathBuf,
  app_data_dir: PathBuf,
  store: AccountsStoreFile,
  conflicts: Vec<ConflictItem>,
  /// 配置加载失败或版本过高时为 true：
  /// 所有写盘都必须经 `persist_locked` 拒绝，
  /// 直到用户明确选择「以空配置继续」或「从备份恢复」。
  persist_blocked: bool,
  read_only: bool,
  selected_provider_id: ProviderId,
  active_slot: Option<ActiveSessionSlot>,
  last_active_per_provider: HashMap<ProviderId, AccountKey>,
  #[cfg(desktop)]
  warm_pool: HashMap<AccountKey, WarmDesktopEntry>,
  #[cfg(desktop)]
  prewarm_failed: HashSet<AccountKey>,
  #[cfg(desktop)]
  cookie_checked_at: HashMap<AccountKey, u64>,
  #[cfg(desktop)]
  in_flight_group_members: HashMap<AccountKey, usize>,
  /// 外部成员（API/CLI）每轮的取消令牌：(group_id, msg_id) -> 是否已停止。
  /// 前端「停止」按钮置位后，执行循环杀掉子进程/丢弃响应流并保留已有输出。
  #[cfg(desktop)]
  external_turn_cancel: HashMap<(String, String), Arc<AtomicBool>>,
  #[cfg(desktop)]
  group_context_synced: HashMap<(String, AccountKey), (u64, u64)>,
  #[cfg(desktop)]
  group_member_locks: HashMap<AccountKey, Arc<tokio::sync::Mutex<()>>>,
  #[cfg_attr(not(desktop), allow(dead_code))]
  group_channel_last_account: HashMap<(String, ProviderId), AccountKey>,
  #[cfg_attr(not(desktop), allow(dead_code))]
  account_last_assigned_ms: HashMap<AccountKey, u64>,
  #[cfg_attr(not(desktop), allow(dead_code))]
  account_cooldown_until_ms: HashMap<AccountKey, u64>,
  account_page_errors: HashMap<AccountKey, String>,
  cookie_identity_hints: HashMap<AccountKey, String>,
  pending_external_auth: Option<PendingExternalAuth>,
  notice: Option<String>,
  modal_active: bool,
  global_shortcut_active: bool,
  global_shortcut_error: Option<String>,
  active_group_id: Option<String>,
}

pub struct SessionManager {
  current_seq: Arc<AtomicU64>,
  active_view_id: Arc<AtomicU64>,
  view_counter: AtomicU64,
  last_sync_timestamp: AtomicU64,
  minimize_to_tray: AtomicBool,
  auto_approve: AtomicBool,
  auto_approve_commands: AtomicBool,
  #[cfg(desktop)]
  download_counter: AtomicU64,
  #[cfg(desktop)]
  recent_downloads: std::sync::Mutex<HashMap<u64, PathBuf>>,
  #[cfg(desktop)]
  group_worker_semaphore: Arc<tokio::sync::Semaphore>,
  inner: Mutex<InnerState>,
  /// P1-7 优化：SessionManager 持有唯一的 ApiKeyVault 实例。
  /// 原先每次 upsert/delete 都新建实例，keyring 可用性探测
  ///（一次 D-Bus 往返）每次都重做；复用后探测只做一次。
  api_key_vault: crate::key_vault::ApiKeyVault,
  /// 防抖写盘：请求计数（每次 `request_persist` +1）与唤醒信号。
  persist_epoch: AtomicU64,
  persist_notify: Arc<tokio::sync::Notify>,
  /// 写线程只启动一次（`run_startup_tasks` / 测试入口共用）。
  persist_writer_started: AtomicBool,
  /// 仅测试：防抖写线程成功写盘的次数，用于断言合并效果。
  #[cfg(test)]
  persist_write_count: AtomicU64,
}

impl SessionManager {
  pub fn new(app_data_dir: PathBuf) -> Self {
    let store_path = app_data_dir.join("accounts.json");
    let loaded = load_or_migrate_store(&store_path);
    let initial_minimize = loaded.store.settings.minimize_to_tray;
    let initial_auto_approve = loaded.store.settings.auto_approve;
    let initial_auto_approve_commands = loaded.store.settings.auto_approve_commands;
    let initial_provider = loaded
      .store
      .last_committed
      .as_ref()
      .map(|k| k.provider_id.clone())
      .filter(|pid| resolve_provider(pid).is_some_and(|spec| spec.info.enabled))
      .unwrap_or_else(ProviderId::muse);

    Self {
      current_seq: Arc::new(AtomicU64::new(1)),
      active_view_id: Arc::new(AtomicU64::new(0)),
      view_counter: AtomicU64::new(1),
      last_sync_timestamp: AtomicU64::new(0),
      minimize_to_tray: AtomicBool::new(initial_minimize),
      auto_approve: AtomicBool::new(initial_auto_approve),
      auto_approve_commands: AtomicBool::new(initial_auto_approve_commands),
      #[cfg(desktop)]
      download_counter: AtomicU64::new(1),
      #[cfg(desktop)]
      recent_downloads: std::sync::Mutex::new(HashMap::new()),
      #[cfg(desktop)]
      group_worker_semaphore: Arc::new(tokio::sync::Semaphore::new(GROUP_MAX_CONCURRENT_WORKERS)),
      api_key_vault: crate::key_vault::ApiKeyVault::new(app_data_dir.clone()),
      inner: Mutex::new(InnerState {
        store_path,
        app_data_dir,
        store: loaded.store,
        conflicts: loaded.conflicts,
        persist_blocked: loaded.persist_blocked,
        read_only: loaded.read_only,
        selected_provider_id: initial_provider,
        active_slot: None,
        last_active_per_provider: HashMap::new(),
        #[cfg(desktop)]
        warm_pool: HashMap::new(),
        #[cfg(desktop)]
        prewarm_failed: HashSet::new(),
        #[cfg(desktop)]
        cookie_checked_at: HashMap::new(),
        #[cfg(desktop)]
        in_flight_group_members: HashMap::new(),
        #[cfg(desktop)]
        external_turn_cancel: HashMap::new(),
        #[cfg(desktop)]
        group_context_synced: HashMap::new(),
        #[cfg(desktop)]
        group_member_locks: HashMap::new(),
        group_channel_last_account: HashMap::new(),
        account_last_assigned_ms: HashMap::new(),
        account_cooldown_until_ms: HashMap::new(),
        account_page_errors: HashMap::new(),
        cookie_identity_hints: HashMap::new(),
        pending_external_auth: None,
        notice: loaded.startup_notice,
        modal_active: false,
        global_shortcut_active: false,
        global_shortcut_error: None,
        active_group_id: None,
      }),
      persist_epoch: AtomicU64::new(0),
      persist_notify: Arc::new(tokio::sync::Notify::new()),
      persist_writer_started: AtomicBool::new(false),
      #[cfg(test)]
      persist_write_count: AtomicU64::new(0),
    }
  }

  #[cfg_attr(not(desktop), allow(dead_code))]
  pub fn minimize_to_tray_enabled(&self) -> bool {
    self.minimize_to_tray.load(Ordering::SeqCst)
  }

  #[cfg(desktop)]
  pub fn toggle_main_window(app: &AppHandle) {
    let Some(win) = app.get_window("main") else {
      return;
    };
    let visible = win.is_visible().unwrap_or(false);
    let focused = win.is_focused().unwrap_or(false);
    if visible && focused {
      let _ = win.hide();
    } else {
      let _ = win.unminimize();
      let _ = win.show();
      let _ = win.set_focus();
    }
  }

  #[cfg(desktop)]
  async fn apply_global_shortcut_on(app: &AppHandle, shortcut: &str) -> Result<bool, String> {
    let trimmed = shortcut.trim().to_string();
    let app_handle = app.clone();
    tokio::task::spawn_blocking(move || {
      use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

      let gs = app_handle.global_shortcut();
      let _ = gs.unregister_all();
      if trimmed.is_empty() {
        return Ok(false);
      }
      gs.on_shortcut(trimmed.as_str(), move |app_cb, _hotkey, event| {
        if event.state == ShortcutState::Pressed {
          Self::toggle_main_window(app_cb);
        }
      })
      .map_err(|e| format!("快捷键格式无效或已被其他应用占用: {e}"))?;
      Ok(true)
    })
    .await
    .map_err(|e| format!("应用全局快捷键中断: {e}"))?
  }

  pub async fn run_startup_tasks(self: &Arc<Self>, app: &AppHandle) {
    // 先启动防抖写线程，后续启动流程里的 `request_persist` 都会被接住。
    self.ensure_persist_writer(app);
    #[cfg(desktop)]
    {
      let initial_shortcut = {
        let locked = self.inner.lock().await;
        locked.store.settings.global_shortcut.clone()
      };
      let shortcut_res = Self::apply_global_shortcut_on(app, &initial_shortcut).await;
      let mut locked = self.inner.lock().await;
      match shortcut_res {
        Ok(active) => {
          locked.global_shortcut_active = active;
          locked.global_shortcut_error = None;
        }
        Err(err) => {
          locked.global_shortcut_active = false;
          locked.global_shortcut_error = Some(err);
        }
      }
    }

    #[cfg(mobile)]
    {
      let mgr = Arc::clone(self);
      let app_clone = app.clone();
      let channel = tauri::ipc::Channel::new(move |body| {
        if let tauri::ipc::InvokeResponseBody::Json(raw_json) = body {
          if let Ok(evt) =
            serde_json::from_str::<crate::platform::mobile::MobileSessionEvent>(&raw_json)
          {
            let mgr_inner = Arc::clone(&mgr);
            let app_inner = app_clone.clone();
            tauri::async_runtime::spawn(async move {
              mgr_inner.handle_mobile_event(&app_inner, evt).await;
            });
          }
        }
        Ok(())
      });
      let _ = crate::platform::mobile::subscribe_events(app, channel).await;
    }

    self.retry_pending_cleanups(app).await;

    let mgr_periodic = Arc::clone(self);
    let app_periodic = app.clone();
    tauri::async_runtime::spawn(async move {
      let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5 * 60));
      interval.tick().await;
      loop {
        interval.tick().await;
        mgr_periodic.trigger_silent_sync(&app_periodic, false).await;
      }
    });

    #[cfg(desktop)]
    {
      let mgr_reclaim = Arc::clone(self);
      let app_reclaim = app.clone();
      tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
        interval.tick().await;
        loop {
          interval.tick().await;
          mgr_reclaim
            .reclaim_idle_desktop_webviews(&app_reclaim)
            .await;
        }
      });
    }

    let candidate = {
      let mut locked = self.inner.lock().await;
      if !locked.conflicts.is_empty() || locked.notice.is_some() {
        None
      } else if let Some(key) = locked.store.last_committed.clone() {
        let in_cleanup = locked.store.pending_cleanup.contains(&key);
        let exists = locked
          .store
          .accounts
          .iter()
          .any(|a| a.provider_id == key.provider_id && a.uuid == key.uuid);
        let enabled = resolve_provider(&key.provider_id).is_some_and(|spec| spec.info.enabled);
        if in_cleanup || !exists {
          locked.store.last_committed = None;
          self.request_persist();
          locked.notice = Some("上次记录的账号键不存在或已清理，已回到管理页".to_string());
          None
        } else if !enabled {
          None
        } else {
          locked.selected_provider_id = key.provider_id.clone();
          Some(key)
        }
      } else {
        None
      }
    };

    let had_startup_candidate = candidate.is_some();
    if let Some(key) = candidate {
      let _ = self.switch_account_internal(app, key, true).await;
    } else {
      let snap = self.get_snapshot().await;
      self.emit_snapshot(app, &snap);
    }

    let mgr_prewarm = Arc::clone(self);
    let app_prewarm = app.clone();
    tauri::async_runtime::spawn(async move {
      tokio::time::sleep(std::time::Duration::from_millis(600)).await;
      mgr_prewarm.prewarm_next_cold_account(&app_prewarm).await;
      if !had_startup_candidate {
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        mgr_prewarm.prewarm_next_cold_account(&app_prewarm).await;
      }
    });
  }

  pub async fn select_provider(
    self: &Arc<Self>,
    app: &AppHandle,
    raw_provider: &str,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id =
      ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;

    #[cfg(desktop)]
    let warm_restore = {
      let mut locked = self.inner.lock().await;
      let spec =
        resolve_provider(&provider_id).ok_or_else(|| "未找到对应的服务配置".to_string())?;
      locked.active_group_id = None;
      locked.selected_provider_id = provider_id.clone();
      self.hide_foreground_slot(app, next_seq, &mut locked).await;
      if spec.info.enabled && locked.conflicts.is_empty() {
        locked
          .last_active_per_provider
          .get(&provider_id)
          .cloned()
          .filter(|k| {
            locked
              .warm_pool
              .get(k)
              .map(|e| e.paint_ready)
              .unwrap_or(false)
              && locked
                .store
                .accounts
                .iter()
                .any(|a| a.provider_id == k.provider_id && a.uuid == k.uuid)
          })
      } else {
        None
      }
    };

    #[cfg(desktop)]
    if let Some(restore_key) = warm_restore {
      return self.switch_account_internal(app, restore_key, false).await;
    }

    #[cfg(not(desktop))]
    {
      let mut locked = self.inner.lock().await;
      let _ = resolve_provider(&provider_id).ok_or_else(|| "未找到对应的服务配置".to_string())?;
      locked.active_group_id = None;
      locked.selected_provider_id = provider_id;
      self.hide_foreground_slot(app, next_seq, &mut locked).await;
      return Ok(Self::snapshot_from_locked(&locked));
    }

    #[cfg(desktop)]
    Ok(self.get_snapshot().await)
  }

  pub async fn add_account(
    self: &Arc<Self>,
    app: &AppHandle,
    raw_provider: &str,
    raw_alias: &str,
    raw_proxy_url: Option<&str>,
    raw_timezone: Option<&str>,
    raw_cookies: Option<&str>,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id =
      ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let trimmed = raw_alias.trim();
    let (alias, auto_alias) = if trimmed.is_empty() {
      ("待登录账号".to_string(), true)
    } else {
      (trimmed.to_string(), false)
    };
    let proxy_url = platform::parse_account_proxy(raw_proxy_url.unwrap_or_default())?
      .map(|url| platform::format_account_proxy(&url));
    let timezone = platform::parse_account_timezone(raw_timezone.unwrap_or_default())?;
    let proxy_raw = proxy_url.clone().unwrap_or_default();

    let cookie_raw_opt = raw_cookies.map(|s| s.trim()).filter(|s| !s.is_empty());

    let (parsed_cookie_specs, cookie_hint, url_opt, spec_enabled_check) = {
      let spec =
        resolve_provider(&provider_id).ok_or_else(|| "未找到对应的服务配置".to_string())?;
      if !spec.info.enabled {
        return Err(format!(
          "{} 当前待接入，暂不支持添加内嵌账号",
          spec.info.name
        ));
      }
      if let Some(raw_ck) = cookie_raw_opt {
        let url = Url::parse(&spec.info.entry_url).map_err(|_| "服务入口地址无效".to_string())?;
        let default_host = url
          .host_str()
          .ok_or_else(|| "服务入口域名无效".to_string())?
          .to_string();
        let specs = platform::parse_imported_cookies(
          raw_ck,
          &default_host,
          &spec.allowed_hosts,
          (now_ms() / 1000) as i64,
        )?;
        let hint = platform::extract_identity_hint_from_cookies(&specs);
        (Some(specs), hint, Some(url), spec.info.name)
      } else {
        (None, None, None, spec.info.name)
      }
    };
    let _ = spec_enabled_check;

    let (created_key, profile_dir) = {
      let mut locked = self.inner.lock().await;
      if !locked.conflicts.is_empty() {
        return Err("请先处理 UUID 归属冲突后再新增账号".to_string());
      }

      let uuid = loop {
        let candidate = Uuid::new_v4().hyphenated().to_string();
        let used_in_accounts = locked.store.accounts.iter().any(|a| a.uuid == candidate);
        let used_in_cleanup = locked
          .store
          .pending_cleanup
          .iter()
          .any(|k| k.uuid == candidate);
        if !used_in_accounts && !used_in_cleanup {
          break candidate;
        }
      };

      let initial_auth = if parsed_cookie_specs.is_some() {
        AuthState::Unknown
      } else {
        AuthState::LoggedOut
      };

      locked.store.accounts.push(AccountRecord {
        provider_id: provider_id.clone(),
        uuid: uuid.clone(),
        alias,
        created_at_ms: now_ms(),
        auto_alias,
        proxy_url,
        timezone,
        avatar_url: None,
        quota_text: None,
        quota_percent: None,
        quota_detail: None,
        auth_state: initial_auth,
        cookie_expires_at_ms: None,
        cookie_session_only: false,
      });
      if let Err(err) = Self::persist_locked(&mut locked).await {
        locked.store.accounts.pop();
        return Err(err);
      }
      locked.selected_provider_id = provider_id.clone();
      let dir = locked.app_data_dir.join("profiles").join(&uuid);
      (AccountKey { provider_id, uuid }, dir)
    };

    if let (Some(specs), Some(url)) = (parsed_cookie_specs.as_ref(), url_opt.as_ref()) {
      let write_res = self
        .write_imported_cookies_to_store(app, &created_key, url, &proxy_raw, &profile_dir, specs)
        .await;
      let win_cold_summary = match write_res {
        Ok(summary) => summary,
        Err(err) => {
          let mut locked = self.inner.lock().await;
          locked.cookie_identity_hints.remove(&created_key);
          locked
            .store
            .accounts
            .retain(|a| !(a.provider_id == created_key.provider_id && a.uuid == created_key.uuid));
          self.request_persist();
          return Err(err);
        }
      };

      if let Some(hint) = cookie_hint {
        let mut locked = self.inner.lock().await;
        locked
          .cookie_identity_hints
          .insert(created_key.clone(), hint);
      }

      #[cfg(desktop)]
      if let Some(summary) = win_cold_summary {
        self
          .apply_cookie_expiry_summary(app, &created_key, summary)
          .await;
      }
      #[cfg(not(desktop))]
      let _ = win_cold_summary;

      let _ = self
        .switch_account_internal(app, created_key, false)
        .await?;
      let snap = {
        let mut locked = self.inner.lock().await;
        locked.notice = Some("已通过 Cookie 添加一个账号".to_string());
        Self::snapshot_from_locked(&locked)
      };
      self.emit_snapshot(app, &snap);
      return Ok(snap);
    }

    self.switch_account_internal(app, created_key, false).await
  }

  pub async fn test_account_proxy(
    &self,
    raw_provider: &str,
    raw_proxy_url: &str,
  ) -> Result<ProxyTestResult, String> {
    let provider_id =
      ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let entry_url = {
      let locked = self.inner.lock().await;
      let _ = locked;
      resolve_provider(&provider_id)
        .ok_or_else(|| "未找到对应的服务配置".to_string())?
        .info
        .entry_url
    };
    platform::test_account_proxy(&entry_url, raw_proxy_url).await
  }

  pub async fn update_account_settings(
    self: &Arc<Self>,
    app: &AppHandle,
    raw_provider: &str,
    raw_uuid: &str,
    raw_alias: &str,
    raw_proxy_url: Option<&str>,
    raw_timezone: Option<&str>,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id =
      ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;
    let alias = raw_alias.trim();
    if alias.is_empty()
      && raw_timezone.is_none()
      && (raw_proxy_url.is_none() || !platform::SUPPORTS_ACCOUNT_PROXY)
    {
      return Err("账号别名不能为空".to_string());
    }
    let proxy_update = raw_proxy_url
      .map(platform::parse_account_proxy)
      .transpose()?;
    let timezone_update = raw_timezone
      .map(platform::parse_account_timezone)
      .transpose()?;

    let mut locked = self.inner.lock().await;
    let spec = resolve_provider(&provider_id).ok_or_else(|| "未找到对应的服务配置".to_string())?;
    let index = locked
      .store
      .accounts
      .iter()
      .position(|a| a.provider_id == provider_id && a.uuid == uuid)
      .ok_or_else(|| "未找到匹配的账号记录".to_string())?;
    let mut updated = locked.store.accounts[index].clone();
    if !alias.is_empty() {
      updated.alias = alias.to_string();
      updated.auto_alias = false;
    }
    if let Some(proxy) = proxy_update {
      updated.proxy_url = proxy.map(|url| platform::format_account_proxy(&url));
    }
    if let Some(tz) = timezone_update {
      updated.timezone = tz;
    }
    let proxy_changed = updated.proxy_url != locked.store.accounts[index].proxy_url;
    let timezone_changed = updated.timezone != locked.store.accounts[index].timezone;
    if (proxy_changed || timezone_changed) && !locked.conflicts.is_empty() {
      return Err("请先处理 UUID 归属冲突后再修改代理或时区".to_string());
    }
    if updated == locked.store.accounts[index] {
      return Ok(Self::snapshot_from_locked(&locked));
    }
    let previous = std::mem::replace(&mut locked.store.accounts[index], updated);
    if let Err(err) = Self::persist_locked(&mut locked).await {
      locked.store.accounts[index] = previous;
      return Err(err);
    }

    #[cfg(desktop)]
    if proxy_changed || timezone_changed {
      let key = AccountKey {
        provider_id: provider_id.clone(),
        uuid: uuid.clone(),
      };
      let was_active = locked
        .active_slot
        .as_ref()
        .is_some_and(|slot| slot.account_key == key);
      locked.prewarm_failed.remove(&key);
      if let Some(entry) = locked.warm_pool.remove(&key) {
        Self::close_warm_entry(app, entry);
      }
      if was_active {
        let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.hide_foreground_slot(app, next_seq, &mut locked).await;
        locked.notice = None;
        let account = locked.store.accounts[index].clone();
        let app_data_dir = locked.app_data_dir.clone();
        let view_instance_id = self.view_counter.fetch_add(1, Ordering::SeqCst);
        drop(locked);
        return self
          .open_desktop_session(
            app,
            account,
            app_data_dir,
            next_seq,
            view_instance_id,
            false,
          )
          .await;
      }
    }

    if let Some(slot) = &locked.active_slot {
      if slot.committed
        && slot.account_key.provider_id == provider_id
        && slot.account_key.uuid == uuid
      {
        let title = format!(
          "{} · {}",
          spec.info.short_name, locked.store.accounts[index].alias
        );
        #[cfg(desktop)]
        if let Some(main_win) = app.get_window("main") {
          let _ = main_win.set_title(&title);
        }
        #[cfg(mobile)]
        {
          let _ = crate::platform::mobile::update_account_title(app, slot.switch_seq, &title).await;
        }
      }
    }

    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn copy_account_cookie(
    &self,
    app: &AppHandle,
    raw_provider: &str,
    raw_uuid: &str,
  ) -> Result<(), String> {
    let provider_id =
      ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;
    let key = AccountKey {
      provider_id: provider_id.clone(),
      uuid,
    };
    let locked = self.inner.lock().await;
    if !locked.conflicts.is_empty() {
      return Err("存在 UUID 归属冲突，请先完成归属裁决".to_string());
    }
    if locked.store.pending_cleanup.contains(&key)
      || !locked
        .store
        .accounts
        .iter()
        .any(|account| account.provider_id == key.provider_id && account.uuid == key.uuid)
    {
      return Err("账号归属核对失败，未找到可用的账号".to_string());
    }
    let spec = resolve_provider(&provider_id).ok_or_else(|| "未找到对应的服务配置".to_string())?;
    if !spec.info.enabled {
      return Err("该服务尚未开放应用内嵌".to_string());
    }
    let url = Url::parse(&spec.info.entry_url).map_err(|_| "服务入口地址无效".to_string())?;

    #[cfg(target_os = "macos")]
    {
      drop(locked);
      let uuid = Uuid::parse_str(&key.uuid).map_err(|_| "账号资料标识无效".to_string())?;
      let (header, summary) = platform::read_macos_cookies_for_url(app, uuid, url).await?;
      self.apply_cookie_expiry_summary(app, &key, summary).await;
      platform::write_cookie_clipboard(app, header).await
    }
    #[cfg(windows)]
    {
      let res = self
        .read_windows_cookies_for_url(app, &locked, &key, url)
        .await;
      drop(locked);
      let (header, summary) = res?;
      self.apply_cookie_expiry_summary(app, &key, summary).await;
      platform::write_cookie_clipboard(app, header).await
    }
    #[cfg(mobile)]
    {
      drop(locked);
      platform::mobile::copy_profile_cookie(app, &key.uuid, url.as_str()).await
    }
    #[cfg(not(any(target_os = "macos", windows, mobile)))]
    {
      drop(locked);
      let _ = (app, url);
      Err("当前平台不支持复制 Cookie".to_string())
    }
  }

  #[cfg(windows)]
  async fn read_windows_webview_cookies(
    webview: tauri::Webview,
    url: Url,
  ) -> Result<(String, platform::CookieExpirySummary), String> {
    tauri::async_runtime::spawn_blocking(move || {
      let current_ms = now_ms();
      let cookies = webview
        .cookies_for_url(url)
        .map_err(|_| "读取账号 Cookie 失败".to_string())?;
      let mut pairs = Vec::with_capacity(cookies.len());
      let mut metas = Vec::with_capacity(cookies.len());
      for cookie in &cookies {
        let (session_only, expires_at_ms) = match cookie.expires_datetime() {
          None => (true, None),
          Some(dt) => match platform::unix_secs_i64_to_ms(dt.unix_timestamp()) {
            Some(exp) if exp > current_ms => (false, Some(exp)),
            _ => continue,
          },
        };
        let name = cookie.name().to_string();
        metas.push(platform::CookieMeta {
          name: name.clone(),
          http_only: cookie.http_only().unwrap_or(false),
          session_only,
          expires_at_ms,
        });
        pairs.push(format!("{}={}", cookie.name(), cookie.value()));
      }
      let header = pairs.join("; ");
      let summary = platform::summarize_cookie_expiry(&metas, current_ms);
      Ok((header, summary))
    })
    .await
    .map_err(|_| "读取账号 Cookie 中断".to_string())?
  }

  #[cfg(windows)]
  async fn read_windows_cookies_for_url(
    &self,
    app: &AppHandle,
    locked: &InnerState,
    key: &AccountKey,
    url: Url,
  ) -> Result<(String, platform::CookieExpirySummary), String> {
    let profile_dir = locked.app_data_dir.join("profiles").join(&key.uuid);
    let uuid = Uuid::parse_str(&key.uuid).map_err(|_| "账号资料标识无效".to_string())?;
    let existing = locked
      .warm_pool
      .get(key)
      .and_then(|entry| app.get_webview(&entry.webview_label));
    let temporary = existing.is_none();
    let webview = match existing {
      Some(webview) => webview,
      None => {
        if !profile_dir.is_dir() {
          return Err("该账号暂无可复制的 Cookie，请先登录".to_string());
        }
        let account = locked
          .store
          .accounts
          .iter()
          .find(|account| account.provider_id == key.provider_id && account.uuid == key.uuid)
          .ok_or_else(|| "未找到可用的账号".to_string())?;
        let proxy =
          platform::parse_account_proxy(account.proxy_url.as_deref().unwrap_or_default())?;
        let window = app
          .get_window("main")
          .ok_or_else(|| "主窗口不存在".to_string())?;
        let instance_id = self.view_counter.fetch_add(1, Ordering::SeqCst);
        let builder = tauri::webview::WebviewBuilder::new(
          format!("cookie-{instance_id}"),
          tauri::WebviewUrl::External("about:blank".parse().expect("valid blank url")),
        )
        .data_directory(profile_dir.clone())
        .on_navigation(|url| url.as_str() == "about:blank")
        .focused(false);
        let builder =
          platform::with_windows_account_proxy(builder, proxy.as_ref(), &app.config().identifier)?;
        let (position, size) = Self::initial_hidden_child_rect(&window);
        let webview = window
          .add_child(builder, position, size)
          .map_err(|_| "无法打开该账号的会话资料，请重试".to_string())?;
        let _ = webview.hide();
        webview
      }
    };

    let result = async {
      platform::verify_desktop_profile_binding(&webview, uuid, &profile_dir).await?;
      Self::read_windows_webview_cookies(webview.clone(), url).await
    }
    .await;
    if temporary {
      let _ = webview.close();
    }
    result
  }

  async fn write_imported_cookies_to_store(
    &self,
    app: &AppHandle,
    key: &AccountKey,
    url: &Url,
    proxy_raw: &str,
    profile_dir: &Path,
    specs: &[platform::ImportedCookieSpec],
  ) -> Result<Option<platform::CookieExpirySummary>, String> {
    #[cfg(desktop)]
    {
      let _ = url;
      let parsed_uuid = Uuid::parse_str(&key.uuid).map_err(|_| "账号资料标识无效".to_string())?;
      let tauri_cookies = specs
        .iter()
        .map(platform::imported_cookie_to_tauri)
        .collect::<Result<Vec<_>, _>>()?;

      let existing_wv = {
        let locked = self.inner.lock().await;
        locked
          .warm_pool
          .get(key)
          .and_then(|entry| app.get_webview(&entry.webview_label))
      };

      let (wv, created_temp) = match existing_wv {
        Some(wv) => (wv, false),
        None => {
          let proxy = platform::parse_account_proxy(proxy_raw)?;
          let window = app
            .get_window("main")
            .ok_or_else(|| "主窗口不存在".to_string())?;
          let instance_id = self.view_counter.fetch_add(1, Ordering::SeqCst);
          #[allow(unused_mut)]
          let mut builder = tauri::webview::WebviewBuilder::new(
            format!("cookie-import-{instance_id}"),
            tauri::WebviewUrl::External("about:blank".parse().expect("valid blank url")),
          )
          .on_navigation(|nav_url| nav_url.as_str() == "about:blank")
          .focused(false);
          #[cfg(target_os = "macos")]
          {
            let _ = &proxy;
            builder = builder
              .data_store_identifier(*parsed_uuid.as_bytes())
              .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15");
          }
          #[cfg(not(target_os = "macos"))]
          {
            builder = builder.data_directory(profile_dir.to_path_buf());
          }
          #[cfg(windows)]
          {
            builder = platform::with_windows_account_proxy(
              builder,
              proxy.as_ref(),
              &app.config().identifier,
            )?;
          }
          let (position, size) = Self::initial_hidden_child_rect(&window);
          let temp_wv = window
            .add_child(builder, position, size)
            .map_err(|_| "无法创建 Cookie 导入视图，请重试".to_string())?;
          let _ = temp_wv.hide();
          (temp_wv, true)
        }
      };

      let apply_res: Result<Option<platform::CookieExpirySummary>, String> = async {
        platform::verify_desktop_profile_binding(&wv, parsed_uuid, profile_dir).await?;
        for cookie in tauri_cookies {
          wv.set_cookie(cookie)
            .map_err(|err| format!("写入 Cookie 失败: {err}"))?;
        }
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        #[cfg(windows)]
        {
          if created_temp {
            if let Ok((_, summary)) =
              Self::read_windows_webview_cookies(wv.clone(), url.clone()).await
            {
              return Ok(Some(summary));
            }
          }
        }
        Ok(None)
      }
      .await;

      if created_temp {
        let _ = wv.close();
      }
      apply_res
    }

    #[cfg(mobile)]
    {
      let _ = (self, proxy_raw, profile_dir);
      let headers = specs
        .iter()
        .map(platform::imported_cookie_to_set_cookie_header)
        .collect::<Vec<_>>();
      platform::mobile::import_profile_cookies(app, &key.uuid, url.as_str(), headers).await?;
      Ok(None)
    }

    #[cfg(not(any(desktop, mobile)))]
    {
      let _ = (self, app, key, url, proxy_raw, profile_dir, specs);
      Err("当前平台不支持导入 Cookie".to_string())
    }
  }

  pub async fn import_account_cookies(
    self: &Arc<Self>,
    app: &AppHandle,
    raw_provider: &str,
    raw_uuid: &str,
    raw_cookies: &str,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id =
      ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;
    let key = AccountKey {
      provider_id: provider_id.clone(),
      uuid: uuid.clone(),
    };

    let (spec, url, default_host, proxy_raw, profile_dir) = {
      let locked = self.inner.lock().await;
      if !locked.conflicts.is_empty() {
        return Err("存在 UUID 归属冲突，请先完成归属裁决".to_string());
      }
      if locked.store.pending_cleanup.contains(&key) {
        return Err("该账号正在清理缓存，请稍后再试".to_string());
      }
      let account = locked
        .store
        .accounts
        .iter()
        .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
        .ok_or_else(|| "未找到对应的账号记录".to_string())?;
      let spec =
        resolve_provider(&provider_id).ok_or_else(|| "未找到对应的服务配置".to_string())?;
      if !spec.info.enabled {
        return Err("该服务尚未开放应用内嵌".to_string());
      }
      let url = Url::parse(&spec.info.entry_url).map_err(|_| "服务入口地址无效".to_string())?;
      let default_host = url
        .host_str()
        .ok_or_else(|| "服务入口域名无效".to_string())?
        .to_string();
      (
        spec,
        url,
        default_host,
        account.proxy_url.clone().unwrap_or_default(),
        locked.app_data_dir.join("profiles").join(&key.uuid),
      )
    };

    let specs = platform::parse_imported_cookies(
      raw_cookies,
      &default_host,
      &spec.allowed_hosts,
      (now_ms() / 1000) as i64,
    )?;
    let cookie_hint = platform::extract_identity_hint_from_cookies(&specs);

    let win_cold_summary = self
      .write_imported_cookies_to_store(app, &key, &url, &proxy_raw, &profile_dir, &specs)
      .await?;

    #[cfg(desktop)]
    {
      let (was_active, was_warm) = {
        let mut locked = self.inner.lock().await;
        locked.account_page_errors.remove(&key);
        locked.prewarm_failed.remove(&key);
        locked.cookie_checked_at.remove(&key);
        if let Some(hint) = cookie_hint {
          locked.cookie_identity_hints.insert(key.clone(), hint);
        }
        if let Some(target) = locked
          .store
          .accounts
          .iter_mut()
          .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
        {
          target.auth_state = AuthState::Unknown;
          target.cookie_expires_at_ms = None;
          target.cookie_session_only = false;
        }
        self.request_persist();
        let active = locked
          .active_slot
          .as_ref()
          .is_some_and(|s| s.account_key == key);
        let warm = locked.warm_pool.contains_key(&key);
        (active, warm)
      };

      if was_active || was_warm {
        let _ = self.reload_account(app, raw_provider, raw_uuid).await?;
        let snap = {
          let mut locked = self.inner.lock().await;
          locked.notice = Some("已通过 Cookie 更新账号登录态并刷新页面".to_string());
          Self::snapshot_from_locked(&locked)
        };
        self.emit_snapshot(app, &snap);
        return Ok(snap);
      }

      if let Some(summary) = win_cold_summary {
        self.apply_cookie_expiry_summary(app, &key, summary).await;
      } else {
        self
          .refresh_account_cookie_expiry(app, key.clone(), true)
          .await;
      }

      let snap = {
        let mut locked = self.inner.lock().await;
        locked.notice = Some("已通过 Cookie 更新账号登录态，点击账号即可加载".to_string());
        Self::snapshot_from_locked(&locked)
      };
      self.emit_snapshot(app, &snap);
      Ok(snap)
    }

    #[cfg(mobile)]
    {
      let _ = win_cold_summary;
      let snap = {
        let mut locked = self.inner.lock().await;
        locked.account_page_errors.remove(&key);
        if let Some(hint) = cookie_hint {
          locked.cookie_identity_hints.insert(key.clone(), hint);
        }
        if let Some(target) = locked
          .store
          .accounts
          .iter_mut()
          .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
        {
          target.auth_state = AuthState::Unknown;
        }
        self.request_persist();
        locked.notice = Some("已通过 Cookie 更新账号登录态，点击账号即可进入".to_string());
        Self::snapshot_from_locked(&locked)
      };
      self.emit_snapshot(app, &snap);
      Ok(snap)
    }

    #[cfg(not(any(desktop, mobile)))]
    {
      let _ = (win_cold_summary, cookie_hint);
      Err("当前平台不支持导入 Cookie".to_string())
    }
  }

  pub async fn remove_account(
    &self,
    app: &AppHandle,
    raw_provider: &str,
    raw_uuid: &str,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id =
      ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;
    let key = AccountKey {
      provider_id: provider_id.clone(),
      uuid: uuid.clone(),
    };

    let closed_desktop_view = {
      let mut locked = self.inner.lock().await;
      if !locked.conflicts.is_empty() {
        return Err("存在 UUID 归属冲突，请先完成归属裁决".to_string());
      }
      let idx = locked
        .store
        .accounts
        .iter()
        .position(|a| a.provider_id == provider_id && a.uuid == uuid)
        .ok_or_else(|| "账号归属核对失败，未找到该账号".to_string())?;

      locked.store.accounts.remove(idx);
      for group in &mut locked.store.group_chats {
        group
          .members
          .retain(|m| !(m.provider_id == provider_id && m.uuid == uuid));
        group
          .member_roles
          .retain(|mr| !(mr.member.provider_id == provider_id && mr.member.uuid == uuid));
        group
          .thread_bindings
          .retain(|tb| !(tb.member.provider_id == provider_id && tb.member.uuid == uuid));
      }
      #[cfg(desktop)]
      locked
        .group_context_synced
        .retain(|(_, k), _| !(k.provider_id == provider_id && k.uuid == uuid));
      if locked.store.last_committed.as_ref() == Some(&key) {
        locked.store.last_committed = None;
      }
      if locked.last_active_per_provider.get(&provider_id) == Some(&key) {
        locked.last_active_per_provider.remove(&provider_id);
      }
      locked.account_page_errors.remove(&key);
      locked.cookie_identity_hints.remove(&key);
      if !locked.store.pending_cleanup.contains(&key) {
        locked.store.pending_cleanup.push(key.clone());
      }
      Self::persist_locked(&mut locked).await?;

      let active_matches = locked
        .active_slot
        .as_ref()
        .map(|s| s.account_key == key)
        .unwrap_or(false);
      if active_matches {
        let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.hide_foreground_slot(app, next_seq, &mut locked).await;
      }

      #[cfg(desktop)]
      {
        locked.prewarm_failed.remove(&key);
        locked.cookie_checked_at.remove(&key);
        if let Some(warm_entry) = locked.warm_pool.remove(&key) {
          Self::close_warm_entry(app, warm_entry);
          Self::apply_desktop_layout_locked(app, &locked);
          true
        } else {
          false
        }
      }
      #[cfg(not(desktop))]
      {
        false
      }
    };

    if closed_desktop_view {
      tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    }

    if self.delete_profile_for_key(app, &uuid).await.is_ok() {
      let mut locked = self.inner.lock().await;
      locked.store.pending_cleanup.retain(|k| k != &key);
      self.request_persist();
    }

    Ok(self.get_snapshot().await)
  }

  pub async fn reorder_accounts(
    &self,
    raw_provider: &str,
    ordered_uuids: Vec<String>,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id =
      ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let mut locked = self.inner.lock().await;
    if !locked.conflicts.is_empty() {
      return Err("存在 UUID 归属冲突，请先处理冲突后再调整顺序".to_string());
    }

    let slot_indices: Vec<usize> = locked
      .store
      .accounts
      .iter()
      .enumerate()
      .filter_map(|(idx, acc)| (acc.provider_id == provider_id).then_some(idx))
      .collect();

    if ordered_uuids.len() != slot_indices.len() {
      return Err("账号数量变化，请刷新后重试".to_string());
    }

    let existing_by_uuid: HashMap<String, usize> = slot_indices
      .iter()
      .map(|&idx| (locked.store.accounts[idx].uuid.clone(), idx))
      .collect();

    let mut seen = HashSet::with_capacity(ordered_uuids.len());
    let mut reordered = Vec::with_capacity(ordered_uuids.len());
    for raw_uuid in ordered_uuids {
      let uuid = normalize_uuid(&raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;
      if !seen.insert(uuid.clone()) {
        return Err("存在重复的账号排序项".to_string());
      }
      let &src_idx = existing_by_uuid
        .get(&uuid)
        .ok_or_else(|| "账号列表已变更，请刷新后重试".to_string())?;
      reordered.push(locked.store.accounts[src_idx].clone());
    }

    let unchanged = slot_indices
      .iter()
      .zip(reordered.iter())
      .all(|(&idx, record)| locked.store.accounts[idx].uuid == record.uuid);
    if unchanged {
      return Ok(Self::snapshot_from_locked(&locked));
    }

    let previous = locked.store.accounts.clone();
    for (slot_idx, record) in slot_indices.into_iter().zip(reordered.into_iter()) {
      locked.store.accounts[slot_idx] = record;
    }
    if let Err(err) = Self::persist_locked(&mut locked).await {
      locked.store.accounts = previous;
      return Err(err);
    }

    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn reload_account(
    self: &Arc<Self>,
    app: &AppHandle,
    raw_provider: &str,
    raw_uuid: &str,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id =
      ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;
    let key = AccountKey {
      provider_id: provider_id.clone(),
      uuid: uuid.clone(),
    };

    #[cfg(desktop)]
    {
      let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;
      let action = {
        let mut locked = self.inner.lock().await;
        let Some(acc) = locked
          .store
          .accounts
          .iter()
          .find(|a| a.provider_id == provider_id && a.uuid == uuid)
          .cloned()
        else {
          return Err("未找到对应的账号记录".to_string());
        };
        let spec =
          resolve_provider(&provider_id).ok_or_else(|| "未找到对应的服务配置".to_string())?;
        if !spec.info.enabled {
          return Err("该服务尚未开放应用内嵌".to_string());
        }
        let was_active = locked
          .active_slot
          .as_ref()
          .is_some_and(|s| s.account_key == key);
        locked.account_page_errors.remove(&key);
        locked.prewarm_failed.remove(&key);
        if let Some(entry) = locked.warm_pool.remove(&key) {
          Self::close_warm_entry(app, entry);
        }
        if was_active {
          self.hide_foreground_slot(app, next_seq, &mut locked).await;
          let app_data_dir = locked.app_data_dir.clone();
          let vid = self.view_counter.fetch_add(1, Ordering::SeqCst);
          Some((acc, app_data_dir, vid))
        } else {
          Self::apply_desktop_layout_locked(app, &locked);
          None
        }
      };

      if let Some((account, app_data_dir, vid)) = action {
        return self
          .open_desktop_session(app, account, app_data_dir, next_seq, vid, false)
          .await;
      }
      return Ok(self.get_snapshot().await);
    }

    #[cfg(not(desktop))]
    {
      let _ = app;
      let locked = self.inner.lock().await;
      if !locked
        .store
        .accounts
        .iter()
        .any(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
      {
        return Err("未找到对应的账号记录".to_string());
      }
      Ok(Self::snapshot_from_locked(&locked))
    }
  }

  pub async fn clear_account_data(
    self: &Arc<Self>,
    app: &AppHandle,
    raw_provider: &str,
    raw_uuid: &str,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id =
      ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;
    let key = AccountKey {
      provider_id: provider_id.clone(),
      uuid: uuid.clone(),
    };

    let (was_active, closed_desktop_view) = {
      let mut locked = self.inner.lock().await;
      if !locked.conflicts.is_empty() {
        return Err("存在 UUID 归属冲突，请先完成归属裁决".to_string());
      }
      let idx = locked
        .store
        .accounts
        .iter()
        .position(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
        .ok_or_else(|| "未找到对应的账号记录".to_string())?;

      let active_matches = locked
        .active_slot
        .as_ref()
        .is_some_and(|s| s.account_key == key);
      if active_matches {
        let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.hide_foreground_slot(app, next_seq, &mut locked).await;
      }

      locked.store.accounts[idx].auth_state = AuthState::LoggedOut;
      locked.store.accounts[idx].quota_detail = None;
      locked.store.accounts[idx].quota_text = None;
      locked.store.accounts[idx].quota_percent = None;
      locked.store.accounts[idx].avatar_url = None;
      locked.store.accounts[idx].cookie_expires_at_ms = None;
      locked.store.accounts[idx].cookie_session_only = false;
      locked.account_page_errors.remove(&key);
      locked.cookie_identity_hints.remove(&key);
      if !locked.store.pending_cleanup.contains(&key) {
        locked.store.pending_cleanup.push(key.clone());
      }
      Self::persist_locked(&mut locked).await?;

      #[cfg(desktop)]
      let closed = {
        locked.prewarm_failed.remove(&key);
        locked.cookie_checked_at.remove(&key);
        locked.group_context_synced.retain(|(_, k), _| k != &key);
        if let Some(entry) = locked.warm_pool.remove(&key) {
          Self::close_warm_entry(app, entry);
          Self::apply_desktop_layout_locked(app, &locked);
          true
        } else {
          false
        }
      };
      #[cfg(not(desktop))]
      let closed = false;

      (active_matches, closed)
    };

    if closed_desktop_view {
      tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }

    match self.delete_profile_for_key(app, &uuid).await {
      Ok(()) => {
        let snap = {
          let mut locked = self.inner.lock().await;
          locked.store.pending_cleanup.retain(|k| k != &key);
          Self::persist_locked(&mut locked).await?;
          Self::snapshot_from_locked(&locked)
        };
        if was_active {
          return self.switch_account_internal(app, key, false).await;
        }
        Ok(snap)
      }
      Err(err) => {
        let snap = self.get_snapshot().await;
        self.emit_snapshot(app, &snap);
        Err(err)
      }
    }
  }

  pub async fn update_app_settings(
    &self,
    app: &AppHandle,
    minimize_to_tray: bool,
    raw_shortcut: &str,
    auto_approve: Option<bool>,
    disable_auto_update_check: Option<bool>,
    auto_approve_commands: Option<bool>,
  ) -> Result<ManagerSnapshot, String> {
    let shortcut = raw_shortcut.trim().to_string();
    if shortcut.chars().count() > 80 {
      return Err("全局快捷键长度过长".to_string());
    }

    #[cfg(desktop)]
    let (global_shortcut_active, global_shortcut_error) = {
      let (old_shortcut, old_active) = {
        let locked = self.inner.lock().await;
        (
          locked.store.settings.global_shortcut.clone(),
          locked.global_shortcut_active,
        )
      };
      match Self::apply_global_shortcut_on(app, &shortcut).await {
        Ok(active) => (active, None),
        Err(err) => {
          if old_active && !old_shortcut.trim().is_empty() {
            let _ = Self::apply_global_shortcut_on(app, &old_shortcut).await;
          }
          return Err(err);
        }
      }
    };

    #[cfg(not(desktop))]
    let (global_shortcut_active, global_shortcut_error) = {
      let _ = app;
      (false, None)
    };

    let next_auto_approve =
      auto_approve.unwrap_or_else(|| self.auto_approve.load(Ordering::SeqCst));
    let next_auto_approve_commands =
      auto_approve_commands.unwrap_or_else(|| self.auto_approve_commands.load(Ordering::SeqCst));

    let (snap, warm_labels) = {
      let mut locked = self.inner.lock().await;
      let previous = locked.store.settings.clone();
      locked.store.settings.minimize_to_tray = minimize_to_tray;
      locked.store.settings.global_shortcut = shortcut;
      locked.store.settings.auto_approve = next_auto_approve;
      if let Some(v) = disable_auto_update_check {
        locked.store.settings.disable_auto_update_check = v;
      }
      locked.store.settings.auto_approve_commands = next_auto_approve_commands;
      if let Err(err) = Self::persist_locked(&mut locked).await {
        locked.store.settings = previous;
        return Err(err);
      }
      self
        .minimize_to_tray
        .store(minimize_to_tray, Ordering::SeqCst);
      self.auto_approve.store(next_auto_approve, Ordering::SeqCst);
      self
        .auto_approve_commands
        .store(next_auto_approve_commands, Ordering::SeqCst);
      locked.global_shortcut_active = global_shortcut_active;
      locked.global_shortcut_error = global_shortcut_error;
      #[cfg(desktop)]
      let labels = locked
        .warm_pool
        .values()
        .map(|e| e.webview_label.clone())
        .collect::<Vec<_>>();
      #[cfg(not(desktop))]
      let labels: Vec<String> = Vec::new();
      (Self::snapshot_from_locked(&locked), labels)
    };

    #[cfg(desktop)]
    {
      let sync_js = format!(
        "(() => {{ try {{ window.__maamAutoApprove = {}; window.__maamAutoApproveAllowCommands = {}; if (window.__maamAutoApprove && typeof window.__maamScanAutoApprove === 'function') window.__maamScanAutoApprove(); }} catch (_) {{}} }})()",
        if next_auto_approve { "true" } else { "false" },
        if next_auto_approve_commands {
          "true"
        } else {
          "false"
        }
      );
      for label in warm_labels {
        if let Some(wv) = app.get_webview(&label) {
          let _ = wv.eval(&sync_js);
        }
      }
    }
    #[cfg(not(desktop))]
    let _ = warm_labels;

    Ok(snap)
  }

  pub async fn reveal_downloaded_file(&self, download_id: u64) -> Result<(), String> {
    #[cfg(desktop)]
    {
      let path = self
        .recent_downloads
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&download_id)
        .cloned()
        .ok_or_else(|| "未找到对应的下载文件记录".to_string())?;
      platform::reveal_path_in_file_manager(&path)
    }
    #[cfg(not(desktop))]
    {
      let _ = download_id;
      Err("当前平台不支持打开本地文件夹".to_string())
    }
  }

  pub async fn set_modal_active(
    &self,
    app: &AppHandle,
    active: bool,
  ) -> Result<ManagerSnapshot, String> {
    let mut locked = self.inner.lock().await;
    locked.modal_active = active;
    #[cfg(desktop)]
    Self::apply_desktop_layout_locked(app, &locked);
    #[cfg(not(desktop))]
    let _ = app;
    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn open_official_site(
    &self,
    app: &AppHandle,
    raw_provider: &str,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id =
      ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let homepage = {
      let locked = self.inner.lock().await;
      let _ = locked;
      resolve_provider(&provider_id)
        .ok_or_else(|| "未找到对应的服务配置".to_string())?
        .info
        .homepage_url
    };

    #[cfg(desktop)]
    {
      let _ = app;
      platform::open_external_browser(&homepage)?;
    }
    #[cfg(mobile)]
    {
      crate::platform::mobile::open_external_url(app, &homepage).await?;
    }

    Ok(self.get_snapshot().await)
  }

  pub async fn resolve_uuid_conflict(
    &self,
    keep_provider_raw: &str,
    raw_uuid: &str,
  ) -> Result<ManagerSnapshot, String> {
    let keep_provider =
      ProviderId::parse(keep_provider_raw).ok_or_else(|| "无效的服务标识".to_string())?;
    let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;

    let mut locked = self.inner.lock().await;
    let mut kept_first = false;
    for acc in &mut locked.store.accounts {
      if acc.uuid == uuid {
        if acc.provider_id == keep_provider && !kept_first {
          kept_first = true;
        } else {
          acc.uuid = Uuid::new_v4().hyphenated().to_string();
        }
      }
    }
    locked.conflicts = detect_conflicts(&locked.store.accounts);
    if locked.conflicts.is_empty() {
      locked.notice = None;
    }
    Self::persist_locked(&mut locked).await?;
    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn dismiss_notice(&self) -> ManagerSnapshot {
    let mut locked = self.inner.lock().await;
    locked.notice = None;
    Self::snapshot_from_locked(&locked)
  }

  pub async fn switch_account(
    self: &Arc<Self>,
    app: &AppHandle,
    raw_provider: &str,
    raw_uuid: &str,
  ) -> Result<ManagerSnapshot, String> {
    let provider_id =
      ProviderId::parse(raw_provider).ok_or_else(|| "无效的服务标识".to_string())?;
    let uuid = normalize_uuid(raw_uuid).ok_or_else(|| "无效的账号 UUID".to_string())?;
    let key = AccountKey { provider_id, uuid };
    self.switch_account_internal(app, key, false).await
  }

  pub async fn switch_account_by_index(
    self: &Arc<Self>,
    app: &AppHandle,
    index_1_based: u8,
  ) -> Result<ManagerSnapshot, String> {
    if index_1_based == 0 {
      return Ok(self.get_snapshot().await);
    }
    let (provider_id, target_uuid) = {
      let locked = self.inner.lock().await;
      let pid = locked.selected_provider_id.clone();
      let accounts: Vec<_> = locked
        .store
        .accounts
        .iter()
        .filter(|a| a.provider_id == pid)
        .collect();
      let idx = (index_1_based - 1) as usize;
      if let Some(acc) = accounts.get(idx) {
        (pid, acc.uuid.clone())
      } else {
        return Ok(Self::snapshot_from_locked(&locked));
      }
    };
    self
      .switch_account(app, provider_id.as_str(), &target_uuid)
      .await
  }

  pub async fn trigger_silent_sync(self: &Arc<Self>, app: &AppHandle, force: bool) {
    let now = now_ms();
    if !force {
      let last = self.last_sync_timestamp.load(Ordering::SeqCst);
      if now > last && now - last < 30_000 {
        return;
      }
    }
    self.last_sync_timestamp.store(now, Ordering::SeqCst);

    #[cfg(not(desktop))]
    {
      let _ = app;
    }

    #[cfg(desktop)]
    {
      let warm_targets = {
        let mut locked = self.inner.lock().await;
        if force {
          locked.prewarm_failed.clear();
        }
        locked
          .warm_pool
          .iter()
          .map(|(k, v)| (k.clone(), v.webview_label.clone(), v.view_instance_id))
          .collect::<Vec<_>>()
      };

      #[cfg(target_os = "macos")]
      let warm_set: HashSet<AccountKey> = warm_targets.iter().map(|(k, _, _)| k.clone()).collect();

      for (key, wv_label, vid) in warm_targets {
        if let Some(wv) = app.get_webview(&wv_label) {
          let _ = wv.eval(
            r#"(() => {
            try {
              if (typeof window.__maamVerifySession === 'function') {
                window.__maamVerifySession(true);
              }
              if (typeof window.__maamRefreshBackendQuota === 'function') {
                window.__maamRefreshBackendQuota(true);
              }
            } catch (_) {}
          })()"#,
          );

          let mgr = Arc::clone(self);
          let app_handle = app.clone();
          // 探针分支里 key 会被 move，此处先克隆一份给末尾的自动审批上报用。
          let key_for_report = key.clone();
          tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(850)).await;
            if let Some(probe) = Self::eval_identity_probe(&wv).await {
              if probe.is_auth {
                let resolved_identity = if !probe.email.is_empty() {
                  Some(probe.email)
                } else if !probe.typed.is_empty() {
                  Some(probe.typed)
                } else {
                  let locked = mgr.inner.lock().await;
                  locked.cookie_identity_hints.get(&key).cloned()
                };
                let soft_identity = if resolved_identity.is_none() && !probe.display_name.is_empty()
                {
                  Some(probe.display_name)
                } else {
                  None
                };
                let (just_authenticated, _) = mgr
                  .apply_detected_metadata(
                    &app_handle,
                    key.clone(),
                    vid,
                    resolved_identity,
                    soft_identity,
                    if probe.avatar.is_empty() {
                      None
                    } else {
                      Some(probe.avatar)
                    },
                    if probe.quota_text.is_empty() {
                      None
                    } else {
                      Some(probe.quota_text)
                    },
                    probe.quota_pct,
                    probe.quota_detail,
                  )
                  .await;
                mgr
                  .refresh_account_cookie_expiry(&app_handle, key, force || just_authenticated)
                  .await;
              } else if probe.logged_out {
                mgr
                  .apply_logged_out_state(&app_handle, key.clone(), vid)
                  .await;
                if probe.stale_session {
                  if key.provider_id.is_muse() {
                    if let Ok(u) = Url::parse("https://muse.ai/api/logout") {
                      let _ = wv.navigate(u);
                    }
                  } else if key.provider_id.is_hark() {
                    if let Ok(u) = Url::parse("https://app.hark.com/login") {
                      let _ = wv.navigate(u);
                    }
                  }
                }
              } else if !probe.page_error.is_empty() {
                mgr
                  .apply_page_error_state(&app_handle, key, vid, probe.page_error)
                  .await;
              }
            }
            // 读回自动审批的审计日志 / 熔断 / 手动处理标记（drain 语义）。
            mgr
              .drain_auto_approve_report(&app_handle, &key_for_report, &wv)
              .await;
          });
        }
      }

      #[cfg(target_os = "macos")]
      {
        let cold_auth_keys: Vec<AccountKey> = {
          let locked = self.inner.lock().await;
          locked
            .store
            .accounts
            .iter()
            .filter(|a| a.auth_state == AuthState::Authenticated)
            .map(|a| AccountKey {
              provider_id: a.provider_id.clone(),
              uuid: a.uuid.clone(),
            })
            .filter(|k| !warm_set.contains(k))
            .collect()
        };
        for key in cold_auth_keys {
          let mgr_cold = Arc::clone(self);
          let app_cold = app.clone();
          tauri::async_runtime::spawn(async move {
            mgr_cold
              .refresh_account_cookie_expiry(&app_cold, key, force)
              .await;
          });
        }
      }

      let mgr_prewarm = Arc::clone(self);
      let app_prewarm = app.clone();
      tauri::async_runtime::spawn(async move {
        mgr_prewarm.prewarm_next_cold_account(&app_prewarm).await;
      });
    }
  }

  async fn switch_account_internal(
    self: &Arc<Self>,
    app: &AppHandle,
    key: AccountKey,
    is_cold_start: bool,
  ) -> Result<ManagerSnapshot, String> {
    {
      let locked = self.inner.lock().await;
      if locked.active_group_id.is_none() {
        if let Some(slot) = &locked.active_slot {
          if slot.committed && slot.account_key == key && !is_cold_start {
            #[cfg(desktop)]
            return Ok(Self::snapshot_from_locked(&locked));
          }
        }
      }
    }

    let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;

    #[cfg(desktop)]
    {
      enum DesktopSwitchPlan {
        InstantWarm {
          view_instance_id: u64,
          need_identity_watcher: bool,
          probe_notify: Arc<tokio::sync::Notify>,
        },
        #[cfg(target_os = "macos")]
        StageWarmPaint {
          view_instance_id: u64,
          expected_load_gen: u64,
          loaded_url: Url,
          loading_snap: ManagerSnapshot,
        },
        AwaitWarmLoading {
          view_instance_id: u64,
        },
        CreateCold {
          account: AccountRecord,
          app_data_dir: PathBuf,
          view_instance_id: u64,
        },
        Abort(ManagerSnapshot),
      }

      let plan = {
        let mut locked = self.inner.lock().await;
        if !locked.conflicts.is_empty() {
          return Err("存在 UUID 归属冲突，已停留在管理页".to_string());
        }
        let spec =
          resolve_provider(&key.provider_id).ok_or_else(|| "未找到对应的服务配置".to_string())?;
        if !spec.info.enabled {
          return Err(format!("{} 尚未开放应用内嵌", spec.info.name));
        }
        if locked.store.pending_cleanup.contains(&key) {
          self.hide_foreground_slot(app, next_seq, &mut locked).await;
          locked.notice = Some("该账号正在清理本地资料，请稍后重试或重新创建".to_string());
          DesktopSwitchPlan::Abort(Self::snapshot_from_locked(&locked))
        } else {
          let Some(acc) = locked
            .store
            .accounts
            .iter()
            .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
            .cloned()
          else {
            self.hide_foreground_slot(app, next_seq, &mut locked).await;
            locked.notice = Some("账号归属核对失败，已回到管理页".to_string());
            return Ok(Self::snapshot_from_locked(&locked));
          };

          locked.active_group_id = None;
          locked.selected_provider_id = key.provider_id.clone();
          locked.notice = None;
          locked.prewarm_failed.remove(&key);
          self.hide_foreground_slot(app, next_seq, &mut locked).await;

          let warm_state = locked.warm_pool.get(&key).and_then(|entry| {
            if app.get_webview(&entry.webview_label).is_some() {
              Some((
                entry.view_instance_id,
                entry.paint_ready,
                entry.staged_painted,
                entry.watcher_running,
                Arc::clone(&entry.probe_notify),
                Arc::clone(&entry.load_gen),
                entry.last_url.clone(),
              ))
            } else {
              None
            }
          });

          if let Some((
            vid,
            paint_ready,
            staged_painted,
            watcher_running,
            probe_notify,
            load_gen,
            last_url,
          )) = warm_state
          {
            #[cfg(not(target_os = "macos"))]
            let _ = (staged_painted, &load_gen, &last_url);
            #[cfg(target_os = "macos")]
            let needs_stage_paint = paint_ready && !staged_painted;
            #[cfg(not(target_os = "macos"))]
            let needs_stage_paint = false;

            if needs_stage_paint {
              #[cfg(target_os = "macos")]
              {
                if let Some(entry) = locked.warm_pool.get_mut(&key) {
                  entry.last_used_ms = now_ms();
                }
                locked.active_slot = Some(ActiveSessionSlot {
                  account_key: key.clone(),
                  view_instance_id: vid,
                  switch_seq: next_seq,
                  committed: false,
                });
                self.active_view_id.store(vid, Ordering::SeqCst);
                Self::apply_desktop_layout_locked(app, &locked);
                let fallback_url = Url::parse(&spec.info.entry_url)
                  .unwrap_or_else(|_| Url::parse("about:blank").expect("valid blank url"));
                DesktopSwitchPlan::StageWarmPaint {
                  view_instance_id: vid,
                  expected_load_gen: load_gen.load(Ordering::SeqCst),
                  loaded_url: last_url.unwrap_or(fallback_url),
                  loading_snap: Self::snapshot_from_locked(&locked),
                }
              }
              #[cfg(not(target_os = "macos"))]
              unreachable!()
            } else if paint_ready {
              let prev_last = locked.store.last_committed.clone();
              locked.store.last_committed = Some(key.clone());
              if let Err(err) = Self::persist_locked(&mut locked).await {
                locked.store.last_committed = prev_last;
                locked.notice = Some(format!("提交账号记录写盘失败: {err}"));
                DesktopSwitchPlan::Abort(Self::snapshot_from_locked(&locked))
              } else {
                locked
                  .last_active_per_provider
                  .insert(key.provider_id.clone(), key.clone());
                locked.active_slot = Some(ActiveSessionSlot {
                  account_key: key.clone(),
                  view_instance_id: vid,
                  switch_seq: next_seq,
                  committed: true,
                });
                self.active_view_id.store(vid, Ordering::SeqCst);
                let title = format!("{} · {}", spec.info.short_name, acc.alias);
                if let Some(main_win) = app.get_window("main") {
                  let _ = main_win.set_title(&title);
                }
                let need_watcher = !watcher_running;
                if let Some(entry) = locked.warm_pool.get_mut(&key) {
                  entry.last_used_ms = now_ms();
                  if need_watcher {
                    entry.watcher_running = true;
                  }
                }
                Self::apply_desktop_layout_locked(app, &locked);
                DesktopSwitchPlan::InstantWarm {
                  view_instance_id: vid,
                  need_identity_watcher: need_watcher,
                  probe_notify,
                }
              }
            } else {
              if let Some(entry) = locked.warm_pool.get_mut(&key) {
                entry.last_used_ms = now_ms();
              }
              locked.active_slot = Some(ActiveSessionSlot {
                account_key: key.clone(),
                view_instance_id: vid,
                switch_seq: next_seq,
                committed: false,
              });
              self.active_view_id.store(vid, Ordering::SeqCst);
              Self::apply_desktop_layout_locked(app, &locked);
              DesktopSwitchPlan::AwaitWarmLoading {
                view_instance_id: vid,
              }
            }
          } else {
            if let Some(entry) = locked.warm_pool.remove(&key) {
              Self::close_warm_entry(app, entry);
            }
            let vid = self.view_counter.fetch_add(1, Ordering::SeqCst);
            let dir = locked.app_data_dir.clone();
            DesktopSwitchPlan::CreateCold {
              account: acc,
              app_data_dir: dir,
              view_instance_id: vid,
            }
          }
        }
      };

      match plan {
        DesktopSwitchPlan::Abort(snap) => Ok(snap),
        DesktopSwitchPlan::InstantWarm {
          view_instance_id,
          need_identity_watcher,
          probe_notify,
        } => {
          if need_identity_watcher {
            self.spawn_desktop_identity_watcher(app.clone(), key, view_instance_id, probe_notify);
          } else {
            probe_notify.notify_one();
          }
          Ok(self.get_snapshot().await)
        }
        #[cfg(target_os = "macos")]
        DesktopSwitchPlan::StageWarmPaint {
          view_instance_id,
          expected_load_gen,
          loaded_url,
          loading_snap,
        } => {
          self.emit_snapshot(app, &loading_snap);
          let mgr_stage = Arc::clone(self);
          let app_stage = app.clone();
          let key_stage = key;
          tauri::async_runtime::spawn(async move {
            mgr_stage
              .wait_paint_and_finalize_desktop(
                &app_stage,
                key_stage,
                view_instance_id,
                loaded_url,
                expected_load_gen,
                true,
              )
              .await;
          });
          Ok(loading_snap)
        }
        DesktopSwitchPlan::AwaitWarmLoading { view_instance_id } => {
          let _ = view_instance_id;
          Ok(self.get_snapshot().await)
        }
        DesktopSwitchPlan::CreateCold {
          account,
          app_data_dir,
          view_instance_id,
        } => {
          self
            .open_desktop_session(
              app,
              account,
              app_data_dir,
              next_seq,
              view_instance_id,
              is_cold_start,
            )
            .await
        }
      }
    }

    #[cfg(mobile)]
    {
      let view_instance_id = self.view_counter.fetch_add(1, Ordering::SeqCst);
      let account_record = {
        let mut locked = self.inner.lock().await;
        if !locked.conflicts.is_empty() {
          return Err("存在 UUID 归属冲突，已停留在管理页".to_string());
        }
        let spec =
          resolve_provider(&key.provider_id).ok_or_else(|| "未找到对应的服务配置".to_string())?;
        if !spec.info.enabled {
          return Err(format!("{} 尚未开放应用内嵌", spec.info.name));
        }
        if locked.store.pending_cleanup.contains(&key) {
          self.hide_foreground_slot(app, next_seq, &mut locked).await;
          locked.notice = Some("该账号正在清理本地资料，请稍后重试或重新创建".to_string());
          return Ok(Self::snapshot_from_locked(&locked));
        }
        let Some(acc) = locked
          .store
          .accounts
          .iter()
          .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
          .cloned()
        else {
          self.hide_foreground_slot(app, next_seq, &mut locked).await;
          locked.notice = Some("账号归属核对失败，已回到管理页".to_string());
          return Ok(Self::snapshot_from_locked(&locked));
        };
        platform::parse_account_proxy(acc.proxy_url.as_deref().unwrap_or_default())?;
        locked.active_group_id = None;
        locked.selected_provider_id = key.provider_id;
        locked.notice = None;
        self.hide_foreground_slot(app, next_seq, &mut locked).await;
        acc
      };

      self
        .open_mobile_session(
          app,
          account_record,
          next_seq,
          view_instance_id,
          is_cold_start,
        )
        .await
    }
  }

  #[cfg(desktop)]
  fn extract_download_filename(path: Option<&std::path::Path>, url: &Url) -> String {
    if let Some(name) = path
      .and_then(|p| p.file_name())
      .and_then(|s| s.to_str())
      .map(str::trim)
      .filter(|s| !s.is_empty())
    {
      return name.to_string();
    }
    url
      .path_segments()
      .and_then(|mut segments| segments.rfind(|s| !s.is_empty()))
      .filter(|s| !s.is_empty())
      .unwrap_or("下载文件")
      .to_string()
  }

  #[cfg(desktop)]
  fn record_recent_download(&self, download_id: u64, path: PathBuf) {
    let mut recent = self
      .recent_downloads
      .lock()
      .unwrap_or_else(|e| e.into_inner());
    recent.insert(download_id, path);
    while recent.len() > 32 {
      if let Some(&oldest_id) = recent.keys().min() {
        recent.remove(&oldest_id);
      } else {
        break;
      }
    }
  }

  #[cfg(desktop)]
  fn handle_desktop_download_event(
    &self,
    app: &AppHandle,
    pending: &std::sync::Mutex<HashMap<String, VecDeque<(u64, PathBuf)>>>,
    event: tauri::webview::DownloadEvent<'_>,
  ) -> bool {
    match event {
      tauri::webview::DownloadEvent::Requested { url, destination } => {
        let download_id = self.download_counter.fetch_add(1, Ordering::SeqCst);
        let filename = Self::extract_download_filename(Some(destination.as_path()), &url);
        pending
          .lock()
          .unwrap_or_else(|e| e.into_inner())
          .entry(url.as_str().to_string())
          .or_default()
          .push_back((download_id, destination.clone()));
        let _ = app.emit(
          "download-event",
          DownloadNoticeEvent {
            id: download_id,
            filename,
            status: "started",
            error: None,
          },
        );
        true
      }
      tauri::webview::DownloadEvent::Finished { url, path, success } => {
        let entry = {
          let mut map = pending.lock().unwrap_or_else(|e| e.into_inner());
          let popped = map.get_mut(url.as_str()).and_then(VecDeque::pop_front);
          if map.get(url.as_str()).is_some_and(VecDeque::is_empty) {
            map.remove(url.as_str());
          }
          popped
        };
        let (download_id, fallback_path) = match entry {
          Some((id, dest)) => (id, Some(dest)),
          None => (self.download_counter.fetch_add(1, Ordering::SeqCst), None),
        };
        let final_path = path.or(fallback_path);
        let filename = Self::extract_download_filename(final_path.as_deref(), &url);
        if success {
          if let Some(saved_path) = final_path {
            self.record_recent_download(download_id, saved_path);
          }
          let _ = app.emit(
            "download-event",
            DownloadNoticeEvent {
              id: download_id,
              filename,
              status: "completed",
              error: None,
            },
          );
        } else {
          let _ = app.emit(
            "download-event",
            DownloadNoticeEvent {
              id: download_id,
              filename,
              status: "failed",
              error: Some("下载未完成或已取消".to_string()),
            },
          );
        }
        true
      }
      _ => true,
    }
  }

  #[cfg(mobile)]
  async fn open_mobile_session(
    self: &Arc<Self>,
    app: &AppHandle,
    account: AccountRecord,
    expected_seq: u64,
    view_instance_id: u64,
    is_cold_start: bool,
  ) -> Result<ManagerSnapshot, String> {
    let caps = match crate::platform::mobile::check_capabilities(app).await {
      Ok(c) => c,
      Err(err) => {
        let mut locked = self.inner.lock().await;
        locked.notice = Some(format!("检测移动端 WebView 独立容器能力失败: {err}"));
        return Ok(Self::snapshot_from_locked(&locked));
      }
    };

    if !caps.multi_profile_supported {
      let mut locked = self.inner.lock().await;
      #[cfg(target_os = "ios")]
      let msg = "当前系统 WKWebView 不支持独立会话隔离（需 iOS 17+）";
      #[cfg(not(target_os = "ios"))]
      let msg = "当前设备系统 WebView 不支持 MULTI_PROFILE 多资料隔离，请先更新 Android System WebView 后重试";
      locked.notice = Some(msg.to_string());
      return Ok(Self::snapshot_from_locked(&locked));
    }

    if is_cold_start && !caps.profile_names.iter().any(|n| n == &account.uuid) {
      let mut locked = self.inner.lock().await;
      locked.notice = Some("上次已提交账号的系统 Profile 已丢失，已停留在管理页".to_string());
      return Ok(Self::snapshot_from_locked(&locked));
    }

    let key = AccountKey {
      provider_id: account.provider_id.clone(),
      uuid: account.uuid.clone(),
    };

    let spec = {
      let mut locked = self.inner.lock().await;
      if self.current_seq.load(Ordering::SeqCst) != expected_seq {
        return Ok(Self::snapshot_from_locked(&locked));
      }
      let spec =
        resolve_provider(&account.provider_id).ok_or_else(|| "未找到对应的服务配置".to_string())?;
      locked.active_slot = Some(ActiveSessionSlot {
        account_key: key,
        view_instance_id,
        switch_seq: expected_seq,
        committed: false,
      });
      spec
    };

    let timezone_init_script = account
      .timezone
      .as_deref()
      .map(platform::build_timezone_override_init_js);
    let payload = crate::platform::mobile::OpenMobileAccountPayload {
      provider_id: account.provider_id.as_str().to_string(),
      provider_name: spec.info.short_name,
      uuid: account.uuid.clone(),
      alias: account.alias.clone(),
      auto_alias: account.auto_alias,
      entry_url: spec.info.entry_url,
      allowed_hosts: spec.allowed_hosts,
      proxy_url: account.proxy_url.clone(),
      timezone: account.timezone.clone(),
      timezone_init_script,
      switch_seq: expected_seq,
      view_instance_id,
      is_cold_start,
    };

    if let Err(err) = crate::platform::mobile::open_account_session(app, payload).await {
      let mut locked = self.inner.lock().await;
      if self.current_seq.load(Ordering::SeqCst) == expected_seq {
        locked.active_slot = None;
        locked.notice = Some(format!("打开独立会话页面失败: {err}"));
      }
      return Ok(Self::snapshot_from_locked(&locked));
    }

    Ok(self.get_snapshot().await)
  }

  #[cfg(mobile)]
  async fn handle_mobile_event(
    self: &Arc<Self>,
    app: &AppHandle,
    event: crate::platform::mobile::MobileSessionEvent,
  ) {
    use crate::platform::mobile::MobileSessionEvent;
    match event {
      MobileSessionEvent::PageCommitVisible {
        provider_id,
        uuid,
        switch_seq,
        view_instance_id,
        url,
      } => {
        let Some(pid) = ProviderId::parse(&provider_id) else {
          return;
        };
        let Some(norm_uuid) = normalize_uuid(&uuid) else {
          return;
        };
        let Ok(parsed_url) = Url::parse(&url) else {
          return;
        };
        self
          .commit_mobile_session(
            app,
            AccountKey {
              provider_id: pid,
              uuid: norm_uuid,
            },
            view_instance_id,
            switch_seq,
            parsed_url,
          )
          .await;
      }
      MobileSessionEvent::IdentityDetected {
        provider_id,
        uuid,
        switch_seq,
        view_instance_id,
        identity,
        avatar_url,
        quota_text,
        quota_percent,
        quota_detail_json,
      } => {
        let Some(pid) = ProviderId::parse(&provider_id) else {
          return;
        };
        let Some(norm_uuid) = normalize_uuid(&uuid) else {
          return;
        };
        let _ = switch_seq;
        let parsed_detail = quota_detail_json
          .as_deref()
          .and_then(|s| serde_json::from_str::<QuotaDetail>(s).ok());
        self
          .apply_detected_metadata(
            app,
            AccountKey {
              provider_id: pid,
              uuid: norm_uuid,
            },
            view_instance_id,
            if identity.is_empty() {
              None
            } else {
              Some(identity)
            },
            None,
            avatar_url,
            quota_text,
            quota_percent,
            parsed_detail,
          )
          .await;
      }
      MobileSessionEvent::SessionFailed {
        switch_seq,
        view_instance_id,
        reason,
      } => {
        let snap = {
          let mut locked = self.inner.lock().await;
          if self.current_seq.load(Ordering::SeqCst) != switch_seq {
            return;
          }
          let matches = locked
            .active_slot
            .as_ref()
            .map(|s| s.switch_seq == switch_seq && s.view_instance_id == view_instance_id)
            .unwrap_or(false);
          if !matches {
            return;
          }
          locked.active_slot = None;
          locked.notice = Some(reason);
          Self::snapshot_from_locked(&locked)
        };
        self.emit_snapshot(app, &snap);
      }
      MobileSessionEvent::ActivityClosed {
        switch_seq,
        view_instance_id,
      } => {
        let snap = {
          let mut locked = self.inner.lock().await;
          let matches = locked
            .active_slot
            .as_ref()
            .map(|s| s.switch_seq == switch_seq && s.view_instance_id == view_instance_id)
            .unwrap_or(false);
          if !matches {
            return;
          }
          locked.active_slot = None;
          locked.pending_external_auth = None;
          Self::snapshot_from_locked(&locked)
        };
        self.emit_snapshot(app, &snap);
      }
      MobileSessionEvent::ExternalAuthUnavailable {
        switch_seq,
        view_instance_id,
        reason,
      } => {
        let snap = {
          let mut locked = self.inner.lock().await;
          let matches = locked
            .active_slot
            .as_ref()
            .map(|s| s.switch_seq == switch_seq && s.view_instance_id == view_instance_id)
            .unwrap_or(false);
          if !matches {
            return;
          }
          locked.pending_external_auth = None;
          locked.notice = Some(reason);
          Self::snapshot_from_locked(&locked)
        };
        self.emit_snapshot(app, &snap);
      }
      MobileSessionEvent::RecreateRequested {
        previous_switch_seq,
      } => {
        let fallback = {
          let mut locked = self.inner.lock().await;
          if self.current_seq.load(Ordering::SeqCst) == previous_switch_seq {
            locked.active_slot = None;
          }
          locked.store.last_committed.clone()
        };
        if let Some(committed_key) = fallback {
          let _ = self.switch_account_internal(app, committed_key, true).await;
        } else {
          let snap = self.get_snapshot().await;
          self.emit_snapshot(app, &snap);
        }
      }
      MobileSessionEvent::UpdateProgress {
        stage,
        downloaded,
        total,
        percent,
        message,
      } => {
        let _ = app.emit(
          "update-progress",
          crate::updater::UpdateProgressEvent {
            stage,
            downloaded,
            total,
            percent,
            message,
          },
        );
      }
      MobileSessionEvent::SessionLoggedOut {
        provider_id,
        uuid,
        switch_seq,
        view_instance_id,
      } => {
        let _ = switch_seq;
        let Some(pid) = ProviderId::parse(&provider_id) else {
          return;
        };
        let Some(norm_uuid) = normalize_uuid(&uuid) else {
          return;
        };
        self
          .apply_logged_out_state(
            app,
            AccountKey {
              provider_id: pid,
              uuid: norm_uuid,
            },
            view_instance_id,
          )
          .await;
      }
    }
  }

  #[cfg(mobile)]
  async fn commit_mobile_session(
    &self,
    app: &AppHandle,
    key: AccountKey,
    view_instance_id: u64,
    expected_seq: u64,
    committed_url: Url,
  ) {
    if self.current_seq.load(Ordering::SeqCst) != expected_seq {
      return;
    }

    let snap_opt = {
      let mut locked = self.inner.lock().await;
      if self.current_seq.load(Ordering::SeqCst) != expected_seq {
        return;
      }
      let Some(spec) = resolve_provider(&key.provider_id) else {
        return;
      };
      if classify_top_level_url(&spec, &committed_url, false) != NavigationDecision::AllowHttps {
        return;
      }
      if !locked.conflicts.is_empty() || locked.store.pending_cleanup.contains(&key) {
        self
          .hide_foreground_slot(app, expected_seq, &mut locked)
          .await;
        return;
      }
      let Some(acc) = locked
        .store
        .accounts
        .iter()
        .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
        .cloned()
      else {
        self
          .hide_foreground_slot(app, expected_seq, &mut locked)
          .await;
        return;
      };

      let slot_matches = locked
        .active_slot
        .as_ref()
        .map(|s| {
          s.account_key == key
            && s.view_instance_id == view_instance_id
            && s.switch_seq == expected_seq
        })
        .unwrap_or(false);
      if !slot_matches {
        return;
      }

      let prev_last = locked.store.last_committed.clone();
      locked.store.last_committed = Some(key.clone());
      if let Err(err) = Self::persist_locked(&mut locked).await {
        locked.store.last_committed = prev_last;
        self
          .hide_foreground_slot(app, expected_seq, &mut locked)
          .await;
        locked.notice = Some(format!("提交账号记录写盘失败，已回退管理页: {err}"));
        Some(Self::snapshot_from_locked(&locked))
      } else {
        if let Some(slot) = &mut locked.active_slot {
          slot.committed = true;
        }
        let title = format!("{} · {}", spec.info.short_name, acc.alias);
        let _ = crate::platform::mobile::update_account_title(app, expected_seq, &title).await;
        Some(Self::snapshot_from_locked(&locked))
      }
    };

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
    }
  }
}

#[cfg(desktop)]
struct DesktopProbeResult {
  is_auth: bool,
  logged_out: bool,
  stale_session: bool,
  page_error: String,
  typed: String,
  email: String,
  display_name: String,
  avatar: String,
  quota_text: String,
  quota_pct: Option<u8>,
  quota_detail: Option<QuotaDetail>,
  pending_hotkey: Option<u8>,
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::storage::save_store_atomic;

  fn make_test_account(provider_id: ProviderId, uuid: &str, alias: &str) -> AccountRecord {
    AccountRecord {
      provider_id,
      uuid: uuid.to_string(),
      alias: alias.to_string(),
      created_at_ms: 1,
      auto_alias: false,
      proxy_url: None,
      timezone: None,
      avatar_url: None,
      quota_text: None,
      quota_percent: None,
      quota_detail: None,
      auth_state: AuthState::Unknown,
      cookie_expires_at_ms: None,
      cookie_session_only: false,
    }
  }

  #[tokio::test]
  async fn reorder_accounts_updates_only_target_provider_and_validates() {
    let temp_dir = std::env::temp_dir().join(format!("maam-reorder-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let store_path = temp_dir.join("accounts.json");

    let m1 = "00000000-0000-4000-8000-000000000001";
    let h1 = "00000000-0000-4000-8000-000000000002";
    let m2 = "00000000-0000-4000-8000-000000000003";
    let m3 = "00000000-0000-4000-8000-000000000004";
    let h2 = "00000000-0000-4000-8000-000000000005";

    let initial = AccountsStoreFile {
      version: crate::storage::STORE_VERSION,
      accounts: vec![
        make_test_account(ProviderId::muse(), m1, "m1"),
        make_test_account(ProviderId::hark(), h1, "h1"),
        make_test_account(ProviderId::muse(), m2, "m2"),
        make_test_account(ProviderId::muse(), m3, "m3"),
        make_test_account(ProviderId::hark(), h2, "h2"),
      ],
      last_committed: None,
      pending_cleanup: Vec::new(),
      group_chats: Vec::new(),
      external_agents: Vec::new(),
      settings: AppSettings::default(),
      unknown_accounts: Vec::new(),
    };
    save_store_atomic(&store_path, &initial).unwrap();

    let mgr = SessionManager::new(temp_dir.clone());
    let snap = mgr
      .reorder_accounts("muse", vec![m3.to_string(), m1.to_string(), m2.to_string()])
      .await
      .unwrap();
    let aliases: Vec<&str> = snap.accounts.iter().map(|a| a.alias.as_str()).collect();
    assert_eq!(aliases, vec!["m3", "h1", "m1", "m2", "h2"]);

    let reloaded = SessionManager::new(temp_dir.clone()).get_snapshot().await;
    let reloaded_aliases: Vec<&str> = reloaded.accounts.iter().map(|a| a.alias.as_str()).collect();
    assert_eq!(reloaded_aliases, vec!["m3", "h1", "m1", "m2", "h2"]);

    assert!(mgr
      .reorder_accounts("muse", vec![m1.to_string(), m2.to_string()])
      .await
      .is_err());
    assert!(mgr
      .reorder_accounts("muse", vec![m1.to_string(), m1.to_string(), m3.to_string()])
      .await
      .is_err());
    assert!(mgr
      .reorder_accounts("muse", vec![m1.to_string(), h1.to_string(), m3.to_string()])
      .await
      .is_err());

    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[test]
  fn builtin_provider_navigation_policy() {
    let muse = resolve_provider(&ProviderId::muse()).unwrap();
    let google_oauth = Url::parse("https://accounts.google.com/o/oauth2/v2/auth").unwrap();
    assert_eq!(
      classify_top_level_url(&muse, &google_oauth, false),
      NavigationDecision::ExternalGoogleOAuth
    );
    let http_url = Url::parse("http://muse.ai/").unwrap();
    assert_eq!(
      classify_top_level_url(&muse, &http_url, false),
      NavigationDecision::Deny
    );
    let blob_null = Url::parse("blob:null/12345678-90ab-cdef-1234-567890abcdef").unwrap();
    assert_eq!(
      classify_top_level_url(&muse, &blob_null, false),
      NavigationDecision::AllowBlank
    );
    let space_asset = Url::parse("https://x.s.metaaiusercontent.com/a").unwrap();
    assert_eq!(
      classify_top_level_url(&muse, &space_asset, false),
      NavigationDecision::AllowHttps
    );
    let space_worker = Url::parse("https://app.meta-agents-apps.workers.dev/").unwrap();
    assert_eq!(
      classify_top_level_url(&muse, &space_worker, false),
      NavigationDecision::AllowHttps
    );
  }

  #[tokio::test]
  async fn group_chat_eligibility_and_member_validation() {
    let temp_dir = std::env::temp_dir().join(format!("maam-group-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let store_path = temp_dir.join("accounts.json");

    let valid_uuid = "00000000-0000-4000-8000-000000000101";
    let expired_uuid = "00000000-0000-4000-8000-000000000102";
    let logged_out_uuid = "00000000-0000-4000-8000-000000000103";

    let mut valid_acc = make_test_account(ProviderId::muse(), valid_uuid, "Muse1");
    valid_acc.auth_state = AuthState::Authenticated;
    valid_acc.cookie_expires_at_ms = Some(now_ms() + 86_400_000);

    let mut expired_acc = make_test_account(ProviderId::hark(), expired_uuid, "HarkExpired");
    expired_acc.auth_state = AuthState::Authenticated;
    expired_acc.cookie_expires_at_ms = Some(1_000);

    let mut logged_out_acc = make_test_account(ProviderId::muse(), logged_out_uuid, "Out");
    logged_out_acc.auth_state = AuthState::LoggedOut;

    let initial = AccountsStoreFile {
      version: crate::storage::STORE_VERSION,
      accounts: vec![
        valid_acc.clone(),
        expired_acc.clone(),
        logged_out_acc.clone(),
      ],
      last_committed: None,
      pending_cleanup: Vec::new(),
      group_chats: Vec::new(),
      external_agents: Vec::new(),
      settings: AppSettings::default(),
      unknown_accounts: Vec::new(),
    };
    save_store_atomic(&store_path, &initial).unwrap();

    let mgr = SessionManager::new(temp_dir.clone());
    let locked = mgr.inner.lock().await;
    let valid_key = AccountKey {
      provider_id: ProviderId::muse(),
      uuid: valid_uuid.to_string(),
    };
    let expired_key = AccountKey {
      provider_id: ProviderId::hark(),
      uuid: expired_uuid.to_string(),
    };
    let out_key = AccountKey {
      provider_id: ProviderId::muse(),
      uuid: logged_out_uuid.to_string(),
    };

    let deduped = SessionManager::normalize_group_members(
      &locked,
      vec![valid_key.clone(), valid_key.clone()],
      None,
    )
    .unwrap();
    assert_eq!(deduped, vec![valid_key]);
    assert!(SessionManager::normalize_group_members(&locked, vec![expired_key], None).is_err());
    assert!(SessionManager::normalize_group_members(&locked, vec![out_key], None).is_err());
    assert!(SessionManager::normalize_group_members(&locked, vec![], None).is_err());

    drop(locked);
    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[test]
  fn parse_at_mentions_resolves_order_longest_match_and_all() {
    let k1 = AccountKey {
      provider_id: ProviderId::muse(),
      uuid: "00000000-0000-4000-8000-000000000001".to_string(),
    };
    let k2 = AccountKey {
      provider_id: ProviderId::muse(),
      uuid: "00000000-0000-4000-8000-000000000002".to_string(),
    };
    let k3 = AccountKey {
      provider_id: ProviderId::hark(),
      uuid: "00000000-0000-4000-8000-000000000003".to_string(),
    };
    let members = vec![
      (k1.clone(), "m1".to_string()),
      (k2.clone(), "m10".to_string()),
      (k3.clone(), "小脑袋".to_string()),
    ];

    assert_eq!(
      SessionManager::parse_at_mentions("test @m10 请和 @小脑袋帮我看看 @m1 @m10", &members),
      (false, vec![k2.clone(), k3.clone(), k1.clone()])
    );
    assert_eq!(
      SessionManager::parse_at_mentions("email test@m1.com 不应触发", &members),
      (false, vec![])
    );
    assert_eq!(
      SessionManager::parse_at_mentions("请 @全体成员 给出结论", &members),
      (true, vec![])
    );
    assert_eq!(
      SessionManager::parse_at_mentions("@all 总结", &members),
      (true, vec![])
    );
  }

  #[cfg(desktop)]
  #[tokio::test]
  async fn group_turn_prompt_differential_sync_relay_and_rename() {
    let temp_dir = std::env::temp_dir().join(format!("maam-group-prompt-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let store_path = temp_dir.join("accounts.json");

    let u1 = "00000000-0000-4000-8000-000000000201";
    let u2 = "00000000-0000-4000-8000-000000000202";
    let u_removed = "00000000-0000-4000-8000-000000000203";

    let mut a1 = make_test_account(ProviderId::muse(), u1, "架构师-小明");
    a1.auth_state = AuthState::Authenticated;
    let mut a2 = make_test_account(ProviderId::hark(), u2, "代码审查-小红");
    a2.auth_state = AuthState::Authenticated;

    let k1 = AccountKey {
      provider_id: ProviderId::muse(),
      uuid: u1.to_string(),
    };
    let k2 = AccountKey {
      provider_id: ProviderId::hark(),
      uuid: u2.to_string(),
    };
    let k_removed = AccountKey {
      provider_id: ProviderId::muse(),
      uuid: u_removed.to_string(),
    };

    let group_id = "grp-prompt-1".to_string();
    let group = GroupChatRecord {
      id: group_id.clone(),
      name: "架构评审群".to_string(),
      members: vec![k1.clone(), k2.clone()],
      created_at_ms: 100,
      shared_context: "核心约束：低内存，禁止写注释".to_string(),
      shared_skills: vec![
        GroupSharedSkill {
          id: "sk1".to_string(),
          name: "简洁输出".to_string(),
          description: String::new(),
          content: "只输出结论与风险".to_string(),
          enabled: true,
        },
        GroupSharedSkill {
          id: "sk2".to_string(),
          name: "已禁用".to_string(),
          description: String::new(),
          content: "不应注入此条技能".to_string(),
          enabled: false,
        },
      ],
      member_roles: vec![GroupMemberRole {
        member: k2.clone(),
        role: "审查并发与边界条件".to_string(),
      }],
      workflow_mode: GroupWorkflowMode::Parallel,
      max_relay_hops: 2,
      thread_bindings: Vec::new(),
      messages: vec![
        GroupChatMessage {
          id: "msg-old-removed".to_string(),
          role: "assistant".to_string(),
          sender: Some(k_removed),
          sender_name: "离职成员-旧名".to_string(),
          content: "历史保留正文内容".to_string(),
          created_at_ms: 90,
          status: GroupReplyStatus::Done,
          targets: Vec::new(),
          error: None,
          stage_label: None,
          hop_depth: 0,
          auto_routed_channel: None,
        },
        GroupChatMessage {
          id: "msg-u-1".to_string(),
          role: "user".to_string(),
          sender: None,
          sender_name: "我".to_string(),
          content: "@架构师-小明 @代码审查-小红 请评审".to_string(),
          created_at_ms: 100,
          status: GroupReplyStatus::Done,
          targets: vec![k1.clone(), k2.clone()],
          error: None,
          stage_label: None,
          hop_depth: 0,
          auto_routed_channel: None,
        },
        GroupChatMessage {
          id: "msg-a-1".to_string(),
          role: "assistant".to_string(),
          sender: Some(k1.clone()),
          sender_name: "架构师-小明".to_string(),
          content: "建议保持上限 2 并发以控制内存".to_string(),
          created_at_ms: 101,
          status: GroupReplyStatus::Done,
          targets: Vec::new(),
          error: None,
          stage_label: None,
          hop_depth: 0,
          auto_routed_channel: None,
        },
        GroupChatMessage {
          id: "msg-a-2".to_string(),
          role: "assistant".to_string(),
          sender: Some(k2.clone()),
          sender_name: "代码审查-小红".to_string(),
          content: String::new(),
          created_at_ms: 102,
          status: GroupReplyStatus::Queued,
          targets: Vec::new(),
          error: None,
          stage_label: None,
          hop_depth: 0,
          auto_routed_channel: None,
        },
      ],
    };

    let initial = AccountsStoreFile {
      version: crate::storage::STORE_VERSION,
      accounts: vec![a1, a2],
      last_committed: None,
      pending_cleanup: Vec::new(),
      group_chats: vec![group],
      external_agents: Vec::new(),
      settings: AppSettings::default(),
      unknown_accounts: Vec::new(),
    };
    save_store_atomic(&store_path, &initial).unwrap();

    let mgr = SessionManager::new(temp_dir.clone());

    let (_, _, p1, fp1) = mgr
      .build_turn_prompt_for_member(
        &group_id,
        &k2,
        11,
        "msg-a-2",
        "msg-u-1",
        "@架构师-小明 @代码审查-小红 请评审",
        None,
        0,
      )
      .await
      .unwrap();
    assert!(p1.contains("请侧重从「审查并发与边界条件」视角回答。"));
    assert!(p1.contains("参考背景：\n核心约束：低内存，禁止写注释"));
    assert!(p1.contains("回答要求：\n- 《简洁输出》：只输出结论与风险"));
    assert!(!p1.contains("不应注入此条技能"));
    assert!(!p1.contains("[上文纪要]"));
    assert!(p1.contains("问题：\n请评审"));
    assert!(!p1.contains("@架构师-小明"));
    assert!(!p1.contains("协同"));
    assert!(!p1.contains("分工"));

    let email_members = vec![
      (k1.clone(), "ldjx7@foxmail.com".to_string()),
      (k2.clone(), "ldjx.tc@gmail.com".to_string()),
    ];
    let stripped = SessionManager::strip_routing_mentions(
      "@ldjx7@foxmail.com @ldjx.tc@gmail.com 帮我分析一下市面上有哪些渠道可以免费试用DeepSeekv",
      &email_members,
    );
    assert_eq!(
      stripped,
      "帮我分析一下市面上有哪些渠道可以免费试用DeepSeekv"
    );
    assert_eq!(
      SessionManager::sanitize_history_assistant_text("仍在发送中\n空闲中"),
      None
    );
    assert_eq!(
      SessionManager::sanitize_history_assistant_text(
        "我不和其他账号的助手一起协作分工，但直接找我能查资料"
      ),
      Some("直接找我能查资料".to_string())
    );

    {
      let mut locked = mgr.inner.lock().await;
      locked
        .group_context_synced
        .insert((group_id.clone(), k2.clone()), (11, fp1));
    }
    let (_, _, p2, fp2) = mgr
      .build_turn_prompt_for_member(
        &group_id,
        &k2,
        11,
        "msg-a-2",
        "msg-u-1",
        "继续确认",
        None,
        0,
      )
      .await
      .unwrap();
    assert_eq!(fp1, fp2);
    assert_eq!(p2, "继续确认");

    let (_, _, p3, _) = mgr
      .build_turn_prompt_for_member(
        &group_id,
        &k2,
        22,
        "msg-a-2",
        "msg-u-1",
        "重连后继续",
        None,
        0,
      )
      .await
      .unwrap();
    assert!(p3.contains("参考背景：\n核心约束：低内存，禁止写注释"));

    let new_skill = vec![GroupSharedSkill {
      id: "sk1".to_string(),
      name: "简洁输出".to_string(),
      description: String::new(),
      content: "只输出结论与风险".to_string(),
      enabled: true,
    }];
    let new_roles = vec![GroupMemberRole {
      member: k2.clone(),
      role: "审查并发与边界条件".to_string(),
    }];
    mgr
      .update_group_knowledge(
        group_id.clone(),
        "新上下文：已通过压力测试".to_string(),
        new_skill,
        new_roles,
      )
      .await
      .unwrap();
    let (_, _, p4, _) = mgr
      .build_turn_prompt_for_member(
        &group_id,
        &k2,
        11,
        "msg-a-2",
        "msg-u-1",
        "检查新上下文",
        None,
        0,
      )
      .await
      .unwrap();
    assert!(p4.contains("参考背景：\n新上下文：已通过压力测试"));

    {
      let mut locked = mgr.inner.lock().await;
      locked.store.accounts[0].alias = "资深架构-大明".to_string();
    }
    let (_, _, p5, _) = mgr
      .build_turn_prompt_for_member(
        &group_id,
        &k2,
        11,
        "msg-a-2",
        "msg-u-1",
        "复核备注同步",
        Some("请做交叉评审"),
        0,
      )
      .await
      .unwrap();
    assert!(p5.contains("[阶段要求]\n请做交叉评审"));
    assert!(p5.contains("[背景资料]\n新上下文：已通过压力测试"));
    assert!(p5.contains("- 已知参考: 建议保持上限 2 并发以控制内存"));
    assert!(p5.contains("- 已知参考: 历史保留正文内容"));
    assert!(!p5.contains("资深架构-大明"));

    let (_, _, p5_muse, _) = mgr
      .build_turn_prompt_for_member(
        &group_id,
        &k1,
        11,
        "msg-a-2",
        "msg-u-1",
        "复核备注同步",
        Some("请做交叉评审"),
        0,
      )
      .await
      .unwrap();
    assert!(p5_muse.contains("- 参考（资深架构-大明）: 建议保持上限 2 并发以控制内存"));
    assert!(p5_muse.contains("- 参考（离职成员-旧名）: 历史保留正文内容"));

    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[cfg(desktop)]
  #[tokio::test]
  async fn hybrid_external_agents_a2a_anti_loop_and_thread_bindings() {
    let temp_dir = std::env::temp_dir().join(format!("maam-hybrid-group-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mgr = SessionManager::new(temp_dir.clone());

    let api_snap = mgr
      .upsert_external_agent(
        None,
        ExternalAgentKind::OpenaiApi,
        "DeepSeek-API".to_string(),
        "https://api.deepseek.com/v1".to_string(),
        "sk-demo".to_string(),
        "deepseek-chat".to_string(),
        String::new(),
        vec![],
        None,
        None,
        None,
      )
      .await
      .expect("create api agent");
    assert_eq!(api_snap.external_agents.len(), 1);
    let api_key = api_snap.external_agents[0].account_key();

    let cli_snap = mgr
      .upsert_external_agent(
        None,
        ExternalAgentKind::LocalCli,
        "ClaudeCLI".to_string(),
        String::new(),
        String::new(),
        String::new(),
        "claude".to_string(),
        vec!["-p".to_string()],
        None,
        None,
        None,
      )
      .await
      .expect("create cli agent");
    assert_eq!(cli_snap.external_agents.len(), 2);
    let cli_key = cli_snap.external_agents[1].account_key();

    let k3 = AccountKey {
      provider_id: ProviderId::muse(),
      uuid: "00000000-0000-4000-8000-000000000333".to_string(),
    };
    let members = vec![
      (api_key.clone(), "DeepSeek-API".to_string()),
      (cli_key.clone(), "ClaudeCLI".to_string()),
      (k3.clone(), "Muse-C".to_string()),
    ];
    let picked = SessionManager::select_a2a_relay_targets(
      "我先给出初稿，请 @DeepSeek-API 自查无意义，请 @ClaudeCLI 复核，也可问 @Muse-C",
      &api_key,
      &members,
      &[api_key.clone()],
      2,
    );
    assert_eq!(picked, vec![cli_key.clone(), k3.clone()]);

    let visited_blocked = SessionManager::select_a2a_relay_targets(
      "再交回 @DeepSeek-API 或 @ClaudeCLI",
      &k3,
      &members,
      &[api_key.clone(), cli_key.clone(), k3.clone()],
      2,
    );
    assert!(visited_blocked.is_empty());

    let mut acc = make_test_account(ProviderId::muse(), &k3.uuid, "Muse-C");
    acc.auth_state = AuthState::Authenticated;
    {
      let mut locked = mgr.inner.lock().await;
      locked.store.accounts.push(acc);
      locked.store.group_chats.push(GroupChatRecord {
        id: "g-hybrid-1".to_string(),
        name: "混合群".to_string(),
        members: vec![api_key.clone(), cli_key.clone(), k3.clone()],
        created_at_ms: 1,
        messages: vec![],
        shared_context: "背景约束：低功耗".to_string(),
        shared_skills: vec![],
        member_roles: vec![],
        workflow_mode: GroupWorkflowMode::A2aRelay,
        max_relay_hops: 2,
        thread_bindings: vec![GroupThreadBinding {
          member: k3.clone(),
          thread_url: "https://muse.ai/c/bound-1".to_string(),
          updated_at_ms: 1,
        }],
      });
    }

    let (sys, usr, full, _) = mgr
      .build_turn_prompt_for_member("g-hybrid-1", &api_key, 0, "m1", "u1", "开始评审", None, 0)
      .await
      .unwrap();
    assert!(sys.contains("[背景资料]\n背景约束：低功耗"));
    assert!(usr.contains("[接力标记说明]"));
    assert!(full.contains("ClaudeCLI"));

    let wf_snap = mgr
      .update_group_workflow("g-hybrid-1".to_string(), GroupWorkflowMode::Review, 3)
      .await
      .unwrap();
    assert_eq!(
      wf_snap.group_chats[0].workflow_mode,
      GroupWorkflowMode::Review
    );
    assert_eq!(wf_snap.group_chats[0].max_relay_hops, 3);

    let reset_snap = mgr
      .reset_group_thread_binding("g-hybrid-1".to_string(), k3.clone())
      .await
      .unwrap();
    assert!(reset_snap.group_chats[0].thread_bindings.is_empty());

    let del_snap = mgr.delete_external_agent(api_key.uuid).await.unwrap();
    assert_eq!(del_snap.external_agents.len(), 1);
    assert!(!del_snap.group_chats[0]
      .members
      .iter()
      .any(|m| m.provider_id.is_api()));

    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[cfg(desktop)]
  #[tokio::test]
  async fn external_agent_api_key_vault_roundtrip_edit_keeps_key_and_delete_clears_vault() {
    let temp_dir = std::env::temp_dir().join(format!("maam-vault-e2e-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mgr = SessionManager::new(temp_dir.clone());

    // 新建带 Key 的 API 成员：快照只给 hasApiKey，不含明文。
    let snap = mgr
      .upsert_external_agent(
        None,
        ExternalAgentKind::OpenaiApi,
        "DeepSeek-API".to_string(),
        "https://api.deepseek.com/v1".to_string(),
        "sk-live-123".to_string(),
        "deepseek-chat".to_string(),
        String::new(),
        vec![],
        None,
        None,
        None,
      )
      .await
      .expect("create api agent");
    assert_eq!(snap.external_agents.len(), 1);
    assert!(snap.external_agents[0].has_api_key);
    let on_disk = std::fs::read_to_string(temp_dir.join("accounts.json")).unwrap();
    assert!(!on_disk.contains("sk-live-123"), "磁盘配置不得含明文 Key");
    assert!(on_disk.contains("apiKeyRef"), "磁盘配置应存引用 id");

    // 重启后内存 Key 应从 vault 恢复（hasApiKey 为 true 即证明）。
    let mgr2 = SessionManager::new(temp_dir.clone());
    let snap2 = mgr2.get_snapshot().await;
    assert!(snap2.external_agents[0].has_api_key);
    let agent_id = snap2.external_agents[0].id.clone();

    // 编辑时 Key 留空：内存 Key 必须保留。
    mgr2
      .upsert_external_agent(
        Some(agent_id.clone()),
        ExternalAgentKind::OpenaiApi,
        "DeepSeek-API".to_string(),
        "https://api.deepseek.com/v1".to_string(),
        String::new(),
        "deepseek-chat".to_string(),
        String::new(),
        vec![],
        None,
        None,
        None,
      )
      .await
      .expect("edit keeps key");
    let kept = mgr2.inner.lock().await.store.external_agents[0]
      .api_key
      .clone();
    assert_eq!(kept, "sk-live-123", "留空编辑不得清空内存 Key");

    // 再次重启：Key 仍能从 vault 恢复。
    let mgr3 = SessionManager::new(temp_dir.clone());
    assert!(mgr3.get_snapshot().await.external_agents[0].has_api_key);

    // 删除成员：vault 里的 Key 同步清除。
    let key_ref = crate::key_vault::api_key_ref_for(&agent_id);
    mgr3
      .delete_external_agent(agent_id)
      .await
      .expect("delete agent");
    assert_eq!(
      crate::key_vault::ApiKeyVault::new(temp_dir.clone()).load_key(&key_ref),
      None,
      "删除后 vault 不得再含该 Key"
    );
    let vault_raw = std::fs::read_to_string(temp_dir.join("api-keys.json")).unwrap_or_default();
    assert!(
      !vault_raw.contains("sk-live-123"),
      "删除后回退文件不得再含该 Key"
    );
    let mgr4 = SessionManager::new(temp_dir.clone());
    assert!(mgr4.get_snapshot().await.external_agents.is_empty());

    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[cfg(desktop)]
  #[tokio::test]
  async fn external_agent_kind_change_to_local_cli_clears_orphan_vault_entry() {
    let temp_dir = std::env::temp_dir().join(format!("maam-vault-kind-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mgr = SessionManager::new(temp_dir.clone());

    let snap = mgr
      .upsert_external_agent(
        None,
        ExternalAgentKind::OpenaiApi,
        "DeepSeek-API".to_string(),
        "https://api.deepseek.com/v1".to_string(),
        "sk-live-456".to_string(),
        "deepseek-chat".to_string(),
        String::new(),
        vec![],
        None,
        None,
        None,
      )
      .await
      .expect("create api agent");
    let agent_id = snap.external_agents[0].id.clone();
    assert_eq!(
      crate::key_vault::ApiKeyVault::new(temp_dir.clone())
        .load_key(&crate::key_vault::api_key_ref_for(&agent_id))
        .as_deref(),
      Some("sk-live-456"),
      "新建后 vault 应含该 Key（与后端无关：keyring 或回退文件）"
    );

    // kind 改为 local_cli：vault 里的旧 Key 必须同步清除，不得成孤儿。
    mgr
      .upsert_external_agent(
        Some(agent_id),
        ExternalAgentKind::LocalCli,
        "DeepSeek-CLI".to_string(),
        String::new(),
        String::new(),
        String::new(),
        "deepseek-cli".to_string(),
        vec![],
        None,
        None,
        None,
      )
      .await
      .expect("change kind to local_cli");
    let vault_raw = std::fs::read_to_string(temp_dir.join("api-keys.json")).unwrap_or_default();
    assert!(
      !vault_raw.contains("sk-live-456"),
      "kind 改为 local_cli 后 vault 不得残留旧 Key"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[tokio::test]
  async fn evaluate_account_quota_handles_muse_extra_hark_caps_and_remaining_fallback() {
    let sanitized = SessionManager::sanitize_shared_context_for_prompt(
      "核心约束：低内存\n[本轮结论汇总 · 可以帮我做什么]\n- a@gmail.com: 回复一\n- b@x.com: 回复二\n- 备注: 保留",
    );
    assert_eq!(
      sanitized,
      "核心约束：低内存\n已有结论（议题：可以帮我做什么）\n- 观点1: 回复一\n- 观点2: 回复二\n- 备注: 保留"
    );
    let mut muse_healthy = make_test_account(
      ProviderId::muse(),
      "00000000-0000-4000-8000-000000000401",
      "Muse-Healthy",
    );
    muse_healthy.quota_detail = Some(QuotaDetail {
      plan_label: "5小时".to_string(),
      plan_value: "剩余 80%".to_string(),
      plan_subtext: None,
      plan_used_percent: Some(20),
      extra_label: None,
      extra_value: None,
      extra_subtext: None,
      extra_used_percent: None,
    });
    let eval_healthy = evaluate_account_quota(&muse_healthy);
    assert!(!eval_healthy.depleted);
    assert!(eval_healthy.sticky_ok);
    assert_eq!(eval_healthy.tier, 0);
    assert_eq!(eval_healthy.effective_used_pct, 20);

    let mut muse_using_extra = make_test_account(
      ProviderId::muse(),
      "00000000-0000-4000-8000-000000000402",
      "Muse-Extra",
    );
    muse_using_extra.quota_detail = Some(QuotaDetail {
      plan_label: "5小时".to_string(),
      plan_value: "用尽".to_string(),
      plan_subtext: None,
      plan_used_percent: Some(100),
      extra_label: Some("额外".to_string()),
      extra_value: Some("剩余 70%".to_string()),
      extra_subtext: None,
      extra_used_percent: Some(30),
    });
    let eval_extra = evaluate_account_quota(&muse_using_extra);
    assert!(!eval_extra.depleted);
    assert_eq!(eval_extra.tier, 1);
    assert_eq!(eval_extra.effective_used_pct, 30);

    let mut hark_monthly_full = make_test_account(
      ProviderId::hark(),
      "00000000-0000-4000-8000-000000000403",
      "Hark-MonthFull",
    );
    hark_monthly_full.quota_detail = Some(QuotaDetail {
      plan_label: "每日".to_string(),
      plan_value: "剩余 90%".to_string(),
      plan_subtext: None,
      plan_used_percent: Some(10),
      extra_label: Some("月上限".to_string()),
      extra_value: Some("已满".to_string()),
      extra_subtext: None,
      extra_used_percent: Some(100),
    });
    let eval_hark = evaluate_account_quota(&hark_monthly_full);
    assert!(eval_hark.depleted);
    assert!(!eval_hark.sticky_ok);

    let mut rem_fallback = make_test_account(
      ProviderId::muse(),
      "00000000-0000-4000-8000-000000000404",
      "Muse-RemFallback",
    );
    rem_fallback.quota_percent = Some(10);
    let eval_rem = evaluate_account_quota(&rem_fallback);
    assert!(!eval_rem.depleted);
    assert!(!eval_rem.sticky_ok);
    assert_eq!(eval_rem.effective_used_pct, 90);
    assert_eq!(eval_rem.tier, 1);
  }

  #[tokio::test]
  async fn auto_channel_routing_sticky_quota_switch_cooldown_and_failover() {
    let temp_dir = std::env::temp_dir().join(format!("maam-auto-route-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mgr = SessionManager::new(temp_dir.clone());

    let k1 = AccountKey {
      provider_id: ProviderId::muse(),
      uuid: "00000000-0000-4000-8000-000000000501".to_string(),
    };
    let k2 = AccountKey {
      provider_id: ProviderId::muse(),
      uuid: "00000000-0000-4000-8000-000000000502".to_string(),
    };
    let k3 = AccountKey {
      provider_id: ProviderId::muse(),
      uuid: "00000000-0000-4000-8000-000000000503".to_string(),
    };

    let mut a1 = make_test_account(ProviderId::muse(), &k1.uuid, "Muse-1");
    a1.auth_state = AuthState::Authenticated;
    a1.quota_percent = Some(60); // 40% used
    let mut a2 = make_test_account(ProviderId::muse(), &k2.uuid, "Muse-2");
    a2.auth_state = AuthState::Authenticated;
    a2.quota_percent = Some(90); // 10% used (highest remaining)
    let mut a3 = make_test_account(ProviderId::muse(), &k3.uuid, "Muse-3");
    a3.auth_state = AuthState::Authenticated;
    a3.quota_percent = Some(0); // depleted

    let mut locked = mgr.inner.lock().await;
    locked.store.accounts = vec![a1, a2, a3];
    locked.store.group_chats.push(GroupChatRecord {
      id: "g-auto-1".to_string(),
      name: "自动轮换群".to_string(),
      members: vec![k1.clone(), k2.clone(), k3.clone()],
      created_at_ms: 1,
      messages: Vec::new(),
      shared_context: String::new(),
      shared_skills: Vec::new(),
      member_roles: Vec::new(),
      workflow_mode: GroupWorkflowMode::Parallel,
      max_relay_hops: 2,
      thread_bindings: Vec::new(),
    });
    let eligible = SessionManager::collect_group_eligible_members_locked(&locked, 0);

    // 首次选择剩余额度最高的 Muse-2
    let (picked1, _) = SessionManager::select_auto_channel_account_locked(
      &locked,
      "g-auto-1",
      &ProviderId::muse(),
      &eligible,
      &[],
      1_000,
    )
    .unwrap();
    assert_eq!(picked1, k2);

    // 粘性复用：即使 Muse-2 用量涨到 70%（仍 <85%），且 Muse-1 更空闲，仍继续粘性复用 Muse-2
    locked
      .group_channel_last_account
      .insert(("g-auto-1".to_string(), ProviderId::muse()), k2.clone());
    locked.store.accounts[1].quota_percent = Some(30); // 70% used
    let (picked_sticky, _) = SessionManager::select_auto_channel_account_locked(
      &locked,
      "g-auto-1",
      &ProviderId::muse(),
      &eligible,
      &[],
      2_000,
    )
    .unwrap();
    assert_eq!(picked_sticky, k2);

    // 阈值切换：Muse-2 用量 >=85% 后自动切换到剩余最多的 Muse-1
    locked.store.accounts[1].quota_percent = Some(10); // 90% used
    let (picked_switch, _) = SessionManager::select_auto_channel_account_locked(
      &locked,
      "g-auto-1",
      &ProviderId::muse(),
      &eligible,
      &[],
      3_000,
    )
    .unwrap();
    assert_eq!(picked_switch, k1);

    // 冷却与故障转移排除：Muse-1 进入冷却后，即使 Muse-2 偏低也优先于冷却中的 Muse-1；且耗尽的 Muse-3 不会被选到
    locked.account_cooldown_until_ms.insert(k1.clone(), 50_000);
    let (picked_cooldown_avoid, _) = SessionManager::select_auto_channel_account_locked(
      &locked,
      "g-auto-1",
      &ProviderId::muse(),
      &eligible,
      &[],
      4_000,
    )
    .unwrap();
    assert_eq!(picked_cooldown_avoid, k2);

    // 仅剩耗尽账号时返回清晰错误
    let err = SessionManager::select_auto_channel_account_locked(
      &locked,
      "g-auto-1",
      &ProviderId::muse(),
      &eligible,
      &[k1, k2],
      5_000,
    )
    .unwrap_err();
    assert!(err.contains("耗尽") || err.contains("待授权"));

    drop(locked);
    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[tokio::test]
  async fn resolve_group_dispatch_targets_enforces_same_channel_mutual_exclusion() {
    let temp_dir = std::env::temp_dir().join(format!("maam-auto-mutex-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mgr = SessionManager::new(temp_dir.clone());

    let m1 = AccountKey {
      provider_id: ProviderId::muse(),
      uuid: "00000000-0000-4000-8000-000000000601".to_string(),
    };
    let m2 = AccountKey {
      provider_id: ProviderId::muse(),
      uuid: "00000000-0000-4000-8000-000000000602".to_string(),
    };
    let h1 = AccountKey {
      provider_id: ProviderId::hark(),
      uuid: "00000000-0000-4000-8000-000000000603".to_string(),
    };
    let h2 = AccountKey {
      provider_id: ProviderId::hark(),
      uuid: "00000000-0000-4000-8000-000000000604".to_string(),
    };

    let mut am1 = make_test_account(ProviderId::muse(), &m1.uuid, "Muse1");
    am1.auth_state = AuthState::Authenticated;
    am1.quota_percent = Some(50);
    let mut am2 = make_test_account(ProviderId::muse(), &m2.uuid, "Muse2");
    am2.auth_state = AuthState::Authenticated;
    am2.quota_percent = Some(90);
    let mut ah1 = make_test_account(ProviderId::hark(), &h1.uuid, "Hark1");
    ah1.auth_state = AuthState::Authenticated;
    ah1.quota_percent = Some(20);
    let mut ah2 = make_test_account(ProviderId::hark(), &h2.uuid, "Hark2");
    ah2.auth_state = AuthState::Authenticated;
    ah2.quota_percent = Some(80);

    let mut locked = mgr.inner.lock().await;
    locked.store.accounts = vec![am1, am2, ah1, ah2];
    locked.store.group_chats.push(GroupChatRecord {
      id: "g-mutex-1".to_string(),
      name: "互斥测试群".to_string(),
      members: vec![m1.clone(), m2.clone(), h1.clone(), h2.clone()],
      created_at_ms: 1,
      messages: Vec::new(),
      shared_context: String::new(),
      shared_skills: Vec::new(),
      member_roles: Vec::new(),
      workflow_mode: GroupWorkflowMode::Parallel,
      max_relay_hops: 2,
      thread_bindings: Vec::new(),
    });
    let eligible = SessionManager::collect_group_eligible_members_locked(&locked, 0);

    // 显式点名 @Muse1 + 同时传 auto_channels=["muse", "hark"]：
    // Muse 渠道与 @Muse1 互斥被过滤；跨渠道的 Hark 自动按最高剩余额度选中 Hark2
    let routed = SessionManager::resolve_group_dispatch_targets_locked(
      &mut locked,
      "g-mutex-1",
      "@Muse1 请看下",
      &eligible,
      Some(vec![m2.clone()]),
      Some(vec!["muse".to_string(), "hark".to_string()]),
      10_000,
    )
    .unwrap();
    assert_eq!(routed.len(), 2);
    assert_eq!(routed[0].0, m1);
    assert_eq!(routed[0].2, None);
    assert_eq!(routed[1].0, h2);
    assert_eq!(routed[1].2, Some(ProviderId::hark()));

    // 无正文 @，仅传 target_members=[Muse1] + auto_channels=["muse", "hark"]
    let routed_chip = SessionManager::resolve_group_dispatch_targets_locked(
      &mut locked,
      "g-mutex-1",
      "对比这两种实现",
      &eligible,
      Some(vec![m1.clone()]),
      Some(vec!["muse".to_string(), "hark".to_string()]),
      11_000,
    )
    .unwrap();
    assert_eq!(routed_chip.len(), 2);
    assert_eq!(routed_chip[0].0, m1);
    assert_eq!(routed_chip[0].2, None);
    assert_eq!(routed_chip[1].0, h2);
    assert_eq!(routed_chip[1].2, Some(ProviderId::hark()));

    drop(locked);
    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[test]
  fn clean_reply_text_and_hark_prompt_adaptation() {
    assert_eq!(
      SessionManager::clean_extracted_reply_text("Gemini 说\n收到！你好，有什么我可以帮你的吗？"),
      "收到！你好，有什么我可以帮你的吗？"
    );
    assert_eq!(
      SessionManager::clean_extracted_reply_text("Gemini said: Hello there"),
      "Hello there"
    );
    assert_eq!(SessionManager::clean_extracted_reply_text("Gemini 说"), "");
    assert_eq!(
      SessionManager::clean_extracted_reply_text("Gemini 说的是对的，确实如此。"),
      "Gemini 说的是对的，确实如此。"
    );

    assert_eq!(
      SessionManager::adapt_question_for_hark("各位好，收到请回复"),
      "你好，收到请回复"
    );
    assert_eq!(
      SessionManager::adapt_question_for_hark("大家觉得这个方案怎么样"),
      "你觉得这个方案怎么样"
    );
    assert_eq!(
      SessionManager::adapt_question_for_hark("大部分情况不受影响"),
      "大部分情况不受影响"
    );

    let hark_ctx = SessionManager::sanitize_shared_context_for_hark(
      "[本轮结论汇总 · API方案]\n- a@gmail.com: 使用 REST\n- 约束: 低内存",
    );
    assert!(hark_ctx.contains("背景要点（API方案）："));
    assert!(hark_ctx.contains("- 使用 REST"));
    assert!(!hark_ctx.contains("观点1"));
  }

  #[tokio::test]
  async fn page_error_state_tracks_excludes_group_members_and_clears_on_recovery_or_logout() {
    let temp_dir = std::env::temp_dir().join(format!("maam-page-err-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let store_path = temp_dir.join("accounts.json");

    let mut a1 = make_test_account(
      ProviderId::muse(),
      "00000000-0000-4000-8000-000000000901",
      "Muse-Err",
    );
    a1.auth_state = AuthState::Authenticated;
    let mut a2 = make_test_account(
      ProviderId::muse(),
      "00000000-0000-4000-8000-000000000902",
      "Muse-Ok",
    );
    a2.auth_state = AuthState::Authenticated;

    let k1 = AccountKey {
      provider_id: a1.provider_id.clone(),
      uuid: a1.uuid.clone(),
    };
    let k2 = AccountKey {
      provider_id: a2.provider_id.clone(),
      uuid: a2.uuid.clone(),
    };

    let initial = AccountsStoreFile {
      version: crate::storage::STORE_VERSION,
      accounts: vec![a1, a2],
      last_committed: None,
      pending_cleanup: Vec::new(),
      group_chats: vec![GroupChatRecord {
        id: "g-err-1".to_string(),
        name: "异常状态测试群".to_string(),
        members: vec![k1.clone(), k2.clone()],
        created_at_ms: 1,
        messages: vec![],
        shared_context: String::new(),
        shared_skills: vec![],
        member_roles: vec![],
        workflow_mode: GroupWorkflowMode::Parallel,
        max_relay_hops: 2,
        thread_bindings: vec![],
      }],
      external_agents: Vec::new(),
      settings: AppSettings::default(),
      unknown_accounts: Vec::new(),
    };
    save_store_atomic(&store_path, &initial).unwrap();

    let mgr = SessionManager::new(temp_dir.clone());
    let vid = 42u64;
    let mut locked = mgr.inner.lock().await;
    #[cfg(desktop)]
    locked.warm_pool.insert(
      k1.clone(),
      WarmDesktopEntry {
        account_key: k1.clone(),
        view_instance_id: vid,
        webview_label: "wv-err-test".to_string(),
        popup_label: None,
        paint_ready: true,
        staged_painted: true,
        navigation_reached: true,
        load_gen: Arc::new(AtomicU64::new(1)),
        last_url: None,
        captured_identity_hint: None,
        watcher_running: false,
        probe_notify: Arc::new(tokio::sync::Notify::new()),
        last_used_ms: 1,
        alive_flag: Arc::new(AtomicBool::new(true)),
      },
    );
    #[cfg(not(desktop))]
    {
      locked.active_slot = Some(ActiveSessionSlot {
        account_key: k1.clone(),
        view_instance_id: vid,
        switch_seq: 1,
        committed: true,
      });
    }

    let snap_err = SessionManager::apply_page_error_state_locked(
      &mut locked,
      k1.clone(),
      vid,
      "页面出错，请检查代理或清理重登",
    )
    .expect("should emit snapshot on page error");
    assert_eq!(snap_err.account_errors.len(), 1);
    assert_eq!(snap_err.account_errors[0].uuid, k1.uuid);
    assert_eq!(
      snap_err.account_errors[0].message,
      "页面出错，请检查代理或清理重登"
    );

    let eligible = SessionManager::collect_group_eligible_members_locked(&locked, 0);
    assert_eq!(eligible, vec![(k2.clone(), "Muse-Ok".to_string())]);

    let (snap_recovered, became_auth, _, _) = SessionManager::apply_detected_metadata_locked(
      &mut locked,
      &k1,
      vid,
      None,
      None,
      None,
      None,
      None,
      None,
    )
    .await;
    assert!(!became_auth);
    let snap_recovered = snap_recovered.expect("should emit snapshot when clearing page error");
    assert!(snap_recovered.account_errors.is_empty());
    assert_eq!(
      SessionManager::collect_group_eligible_members_locked(&locked, 0).len(),
      2
    );

    SessionManager::apply_page_error_state_locked(
      &mut locked,
      k1.clone(),
      vid,
      "登录态异常，请退出或清理重登",
    )
    .expect("re-insert page error");
    let snap_logged_out = SessionManager::apply_logged_out_state_locked(&mut locked, &k1, vid)
      .await
      .expect("should emit snapshot when transitioning to logged_out");
    assert!(snap_logged_out.account_errors.is_empty());
    assert_eq!(
      snap_logged_out
        .accounts
        .iter()
        .find(|a| a.uuid == k1.uuid)
        .unwrap()
        .auth_state,
      AuthState::LoggedOut
    );

    let _ = SessionManager::apply_page_error_state_locked(&mut locked, k1.clone(), vid, "二次异常")
      .expect("insert page error while already logged out");
    let snap_still_logged_out =
      SessionManager::apply_logged_out_state_locked(&mut locked, &k1, vid)
        .await
        .expect("clearing error on already logged_out account must still emit snapshot");
    assert!(snap_still_logged_out.account_errors.is_empty());

    drop(locked);
    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[tokio::test]
  async fn soft_identity_upgrades_to_strong_email_and_reverts_placeholder_on_logout() {
    let temp_dir = std::env::temp_dir().join(format!("maam-soft-id-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let store_path = temp_dir.join("accounts.json");

    let mut acc = make_test_account(
      ProviderId::muse(),
      "00000000-0000-4000-8000-000000000903",
      "待登录账号",
    );
    acc.auto_alias = true;
    acc.auth_state = AuthState::Unknown;

    let key = AccountKey {
      provider_id: acc.provider_id.clone(),
      uuid: acc.uuid.clone(),
    };

    let initial = AccountsStoreFile {
      version: crate::storage::STORE_VERSION,
      accounts: vec![acc],
      last_committed: None,
      pending_cleanup: Vec::new(),
      group_chats: Vec::new(),
      external_agents: Vec::new(),
      settings: AppSettings::default(),
      unknown_accounts: Vec::new(),
    };
    save_store_atomic(&store_path, &initial).unwrap();

    let mgr = SessionManager::new(temp_dir.clone());
    let vid = 77u64;
    let mut locked = mgr.inner.lock().await;
    #[cfg(desktop)]
    locked.warm_pool.insert(
      key.clone(),
      WarmDesktopEntry {
        account_key: key.clone(),
        view_instance_id: vid,
        webview_label: "wv-soft-test".to_string(),
        popup_label: None,
        paint_ready: true,
        staged_painted: true,
        navigation_reached: true,
        load_gen: Arc::new(AtomicU64::new(1)),
        last_url: None,
        captured_identity_hint: None,
        watcher_running: false,
        probe_notify: Arc::new(tokio::sync::Notify::new()),
        last_used_ms: 1,
        alive_flag: Arc::new(AtomicBool::new(true)),
      },
    );
    #[cfg(not(desktop))]
    {
      locked.active_slot = Some(ActiveSessionSlot {
        account_key: key.clone(),
        view_instance_id: vid,
        switch_seq: 1,
        committed: true,
      });
    }

    // 1. Soft fallback updates alias to "已登录账号" while keeping auto_alias = true.
    let (snap1, became_auth1, _, _) = SessionManager::apply_detected_metadata_locked(
      &mut locked,
      &key,
      vid,
      None,
      Some("已登录账号".to_string()),
      None,
      None,
      None,
      None,
    )
    .await;
    assert!(became_auth1);
    assert!(snap1.is_some());
    assert_eq!(locked.store.accounts[0].alias, "已登录账号");
    assert!(locked.store.accounts[0].auto_alias);

    // 2. Display name replaces generic "已登录账号" while still keeping auto_alias = true.
    let (snap2, _, _, _) = SessionManager::apply_detected_metadata_locked(
      &mut locked,
      &key,
      vid,
      None,
      Some("Alice Meta".to_string()),
      None,
      None,
      None,
      None,
    )
    .await;
    assert!(snap2.is_some());
    assert_eq!(locked.store.accounts[0].alias, "Alice Meta");
    assert!(locked.store.accounts[0].auto_alias);

    // 3. Strong email overrides soft identity and clears auto_alias.
    let (snap3, _, _, _) = SessionManager::apply_detected_metadata_locked(
      &mut locked,
      &key,
      vid,
      Some("alice@gmail.com".to_string()),
      None,
      None,
      None,
      None,
      None,
    )
    .await;
    assert!(snap3.is_some());
    assert_eq!(locked.store.accounts[0].alias, "alice@gmail.com");
    assert!(!locked.store.accounts[0].auto_alias);

    // 4. Logout on a soft-fallback placeholder reverts "已登录账号" -> "待登录账号".
    locked.store.accounts[0].alias = "已登录账号".to_string();
    locked.store.accounts[0].auto_alias = true;
    let snap_out = SessionManager::apply_logged_out_state_locked(&mut locked, &key, vid)
      .await
      .expect("logout should emit snapshot");
    let out_acc = &snap_out.accounts[0];
    assert_eq!(out_acc.auth_state, AuthState::LoggedOut);
    assert_eq!(out_acc.alias, "待登录账号");
    assert!(out_acc.auto_alias);

    drop(locked);
    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[test]
  fn append_auto_approve_audit_writes_jsonl_with_account_key_and_button() {
    let temp_dir = std::env::temp_dir().join(format!(
      "maam-audit-test-{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let key = AccountKey {
      provider_id: ProviderId::muse(),
      uuid: "uuid-1".to_string(),
    };
    let entries = vec![
      serde_json::json!({ "at": 1_700_000_000_000u64, "text": "Muse wants to read files", "button": "Allow once" }),
      serde_json::json!({ "at": 1_700_000_000_001u64, "text": "second card", "button": "Allow" }),
    ];
    SessionManager::append_auto_approve_audit(&temp_dir, &key, &entries);
    // 追加写：第二批不覆盖第一批。
    SessionManager::append_auto_approve_audit(&temp_dir, &key, &entries[..1]);
    let content =
      std::fs::read_to_string(temp_dir.join("auto-approve-audit.log")).expect("audit log written");
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 3);
    for line in &lines {
      let v: serde_json::Value = serde_json::from_str(line).expect("each line is json");
      assert_eq!(v["provider_id"], "muse");
      assert_eq!(v["uuid"], "uuid-1");
    }
    assert!(lines[0].contains("Allow once"));
    let second: serde_json::Value = serde_json::from_str(lines[1]).expect("second line is json");
    assert_eq!(second["button"], "Allow");
    assert_eq!(second["text"], "second card");
    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[tokio::test]
  async fn corrupt_config_blocks_later_persists() {
    let temp_dir = std::env::temp_dir().join(format!("maam-corrupt-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let store_path = temp_dir.join("accounts.json");
    let original = "{ broken json";
    std::fs::write(&store_path, original).unwrap();

    let mgr = SessionManager::new(temp_dir.clone());
    let snap = mgr.get_snapshot().await;
    assert!(snap.notice.is_some(), "损坏配置启动后应有提示");

    // 任何写盘尝试都必须失败，且磁盘文件字节不变（不能被空配置覆盖）。
    let res = mgr
      .resolve_uuid_conflict("muse", "00000000-0000-4000-8000-000000000001")
      .await;
    assert!(res.is_err(), "写盘被阻止时写盘操作应返回 Err");
    assert_eq!(std::fs::read_to_string(&store_path).unwrap(), original);
    let has_backup = std::fs::read_dir(&temp_dir)
      .unwrap()
      .filter_map(|e| e.ok())
      .any(|e| {
        e.file_name()
          .to_str()
          .is_some_and(|n| n.starts_with("accounts.json.corrupt-"))
      });
    assert!(has_backup, "损坏原文件应有 .corrupt-* 备份");
    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[tokio::test]
  async fn future_version_config_blocks_later_persists() {
    let temp_dir = std::env::temp_dir().join(format!("maam-future-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let store_path = temp_dir.join("accounts.json");
    let original = serde_json::to_string_pretty(&serde_json::json!({
      "version": 99,
      "accounts": [
        {
          "providerId": "muse",
          "uuid": "00000000-0000-4000-8000-000000000001",
          "alias": "m1"
        }
      ]
    }))
    .unwrap();
    std::fs::write(&store_path, &original).unwrap();

    let mgr = SessionManager::new(temp_dir.clone());
    let res = mgr
      .resolve_uuid_conflict("muse", "00000000-0000-4000-8000-000000000001")
      .await;
    assert!(res.is_err(), "只读配置的写盘操作应返回 Err");
    assert_eq!(std::fs::read_to_string(&store_path).unwrap(), original);
    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[tokio::test]
  async fn persist_blocked_returns_err_and_leaves_file_unchanged() {
    let temp_dir = std::env::temp_dir().join(format!("maam-persist-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let store_path = temp_dir.join("accounts.json");
    let original = "{ broken json";
    std::fs::write(&store_path, original).unwrap();

    let mgr = SessionManager::new(temp_dir.clone());
    let res = mgr.persist().await;
    assert!(res.is_err(), "persist_blocked 时 persist() 应返回 Err");
    assert_eq!(std::fs::read_to_string(&store_path).unwrap(), original);
    let snap = mgr.get_snapshot().await;
    assert!(snap.persist_blocked);
    assert!(
      snap
        .notice
        .as_deref()
        .unwrap_or_default()
        .contains("写盘已被阻止"),
      "应通过 notice 提示写盘被阻止"
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[tokio::test]
  async fn dismiss_notice_does_not_unblock_persist() {
    let temp_dir = std::env::temp_dir().join(format!("maam-dismiss-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let store_path = temp_dir.join("accounts.json");
    std::fs::write(&store_path, "{ broken json").unwrap();

    let mgr = SessionManager::new(temp_dir.clone());
    let _ = mgr.dismiss_notice().await;
    assert!(mgr.persist().await.is_err(), "只关掉提示不能解除写盘阻止");
    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[tokio::test]
  async fn continue_with_empty_config_unblocks_and_writes_empty_store() {
    let temp_dir = std::env::temp_dir().join(format!("maam-empty-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let store_path = temp_dir.join("accounts.json");
    std::fs::write(&store_path, "{ broken json").unwrap();

    let mgr = SessionManager::new(temp_dir.clone());
    let snap = mgr.continue_with_empty_config().await.unwrap();
    assert!(!snap.persist_blocked);
    assert!(snap.notice.is_none());
    let saved: serde_json::Value =
      serde_json::from_str(&std::fs::read_to_string(&store_path).unwrap()).unwrap();
    assert_eq!(
      saved
        .get("accounts")
        .and_then(|v| v.as_array())
        .unwrap()
        .len(),
      0
    );
    // 解除后写盘恢复正常，且损坏原文件的备份仍在。
    assert!(mgr.persist().await.is_ok());
    let has_backup = std::fs::read_dir(&temp_dir)
      .unwrap()
      .filter_map(|e| e.ok())
      .any(|e| {
        e.file_name()
          .to_str()
          .is_some_and(|n| n.starts_with("accounts.json.corrupt-"))
      });
    assert!(has_backup, "以空配置继续后 .corrupt-* 备份仍应保留");
    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[tokio::test]
  async fn restore_config_from_backup_recovers_last_good() {
    let temp_dir = std::env::temp_dir().join(format!("maam-restore-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let store_path = temp_dir.join("accounts.json");
    let initial = AccountsStoreFile {
      version: crate::storage::STORE_VERSION,
      accounts: vec![make_test_account(
        ProviderId::muse(),
        "00000000-0000-4000-8000-000000000001",
        "m1",
      )],
      last_committed: None,
      pending_cleanup: Vec::new(),
      group_chats: Vec::new(),
      external_agents: Vec::new(),
      settings: AppSettings::default(),
      unknown_accounts: Vec::new(),
    };
    save_store_atomic(&store_path, &initial).unwrap();
    // 第一次加载产生 accounts.json.bak（上一份可用配置）。
    drop(SessionManager::new(temp_dir.clone()));
    assert!(temp_dir.join("accounts.json.bak").exists());

    // 配置损坏后重新启动：写盘被阻止。
    std::fs::write(&store_path, "{ broken json").unwrap();
    let mgr = SessionManager::new(temp_dir.clone());
    assert!(mgr.get_snapshot().await.persist_blocked);

    let snap = mgr.restore_config_from_backup().await.unwrap();
    assert!(!snap.persist_blocked);
    assert_eq!(snap.accounts.len(), 1);
    assert_eq!(snap.accounts[0].alias, "m1");
    let saved: serde_json::Value =
      serde_json::from_str(&std::fs::read_to_string(&store_path).unwrap()).unwrap();
    assert_eq!(
      saved
        .get("accounts")
        .and_then(|v| v.as_array())
        .unwrap()
        .len(),
      1
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  #[tokio::test]
  async fn restore_config_from_backup_refused_when_not_blocked() {
    let temp_dir = std::env::temp_dir().join(format!("maam-refused-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let store_path = temp_dir.join("accounts.json");
    let initial = AccountsStoreFile {
      version: crate::storage::STORE_VERSION,
      accounts: vec![make_test_account(
        ProviderId::muse(),
        "00000000-0000-4000-8000-000000000001",
        "m1",
      )],
      last_committed: None,
      pending_cleanup: Vec::new(),
      group_chats: Vec::new(),
      external_agents: Vec::new(),
      settings: AppSettings::default(),
      unknown_accounts: Vec::new(),
    };
    save_store_atomic(&store_path, &initial).unwrap();

    let mgr = SessionManager::new(temp_dir.clone());
    let before = std::fs::read(&store_path).unwrap();
    let res = mgr.restore_config_from_backup().await;
    assert!(
      res.is_err(),
      "配置正常时不应允许从备份恢复（避免覆盖好配置）"
    );
    assert_eq!(std::fs::read(&store_path).unwrap(), before);
  }

  /// P1-8 防抖：10 次快速 `request_persist()` 只产生一次写盘；之后再请求一次，
  /// 写盘次数变为 2（写线程继续工作，不吞掉后续请求）。
  #[tokio::test]
  async fn debounced_persist_coalesces_rapid_requests_into_one_write() {
    let temp_dir = std::env::temp_dir().join(format!("maam-debounce-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mgr = std::sync::Arc::new(SessionManager::new(temp_dir.clone()));
    mgr.ensure_persist_writer_no_app();

    {
      let mut locked = mgr.inner.lock().await;
      locked.store.settings.minimize_to_tray = true;
    }
    for _ in 0..10 {
      mgr.request_persist();
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    wait_for_persist_writes(&mgr, 1).await;
    // 300ms 静默窗口已过，再等 500ms 确认没有第二次写盘。
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(mgr.persist_write_count.load(Ordering::SeqCst), 1);
    let reloaded = SessionManager::new(temp_dir.clone()).get_snapshot().await;
    assert!(reloaded.settings.minimize_to_tray);

    // 第二轮：再改一次并请求一次，应再写一次。
    {
      let mut locked = mgr.inner.lock().await;
      locked.store.settings.minimize_to_tray = false;
    }
    mgr.request_persist();
    wait_for_persist_writes(&mgr, 2).await;
    let reloaded = SessionManager::new(temp_dir.clone()).get_snapshot().await;
    assert!(!reloaded.settings.minimize_to_tray);

    let _ = std::fs::remove_dir_all(&temp_dir);
  }

  async fn wait_for_persist_writes(mgr: &SessionManager, expected: u64) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
      if mgr.persist_write_count.load(Ordering::SeqCst) == expected {
        return;
      }
      assert!(
        tokio::time::Instant::now() < deadline,
        "防抖写盘未在 10 秒内达到 {expected} 次"
      );
      tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
  }

  /// P1-8 防抖失败路径：写盘失败时写 notice「配置自动保存失败：…」，
  /// 且保留 dirty——恢复可写后下一次请求会重试并写盘成功。
  #[tokio::test]
  async fn debounced_persist_failure_writes_notice_and_retries() {
    let temp_dir = std::env::temp_dir().join(format!("maam-debounce-fail-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mgr = std::sync::Arc::new(SessionManager::new(temp_dir.clone()));
    mgr.ensure_persist_writer_no_app();

    // 先成功写一次，确认写线程在工作。
    {
      let mut locked = mgr.inner.lock().await;
      locked.store.settings.minimize_to_tray = true;
    }
    mgr.request_persist();
    wait_for_persist_writes(&mgr, 1).await;

    // 把 accounts.json 换成同名目录，写盘必然失败（rename 落不上去）。
    let store_path = temp_dir.join("accounts.json");
    std::fs::remove_file(&store_path).unwrap();
    std::fs::create_dir(&store_path).unwrap();
    {
      let mut locked = mgr.inner.lock().await;
      locked.store.settings.minimize_to_tray = false;
    }
    mgr.request_persist();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
      let notice = mgr.get_snapshot().await.notice.unwrap_or_default();
      if notice.contains("配置自动保存失败") {
        break;
      }
      assert!(
        tokio::time::Instant::now() < deadline,
        "写盘失败后 10 秒内未出现自动保存失败提示"
      );
      tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    // 失败不应计为一次成功写盘。
    assert_eq!(mgr.persist_write_count.load(Ordering::SeqCst), 1);

    // 恢复可写：删掉目录再请求一次，应重试成功。
    std::fs::remove_dir(&store_path).unwrap();
    mgr.request_persist();
    wait_for_persist_writes(&mgr, 2).await;
    let reloaded = SessionManager::new(temp_dir.clone()).get_snapshot().await;
    assert!(!reloaded.settings.minimize_to_tray);

    let _ = std::fs::remove_dir_all(&temp_dir);
  }
}
