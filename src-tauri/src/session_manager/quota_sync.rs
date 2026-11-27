//! Quota sync + identity probing (P2-11 split).
//!
//! Moved verbatim from `session_manager.rs`: the quota-sync unit —
//! `probe_meta_account_center_email`, `spawn_desktop_identity_watcher`,
//! `eval_identity_probe`, `refresh_account_cookie_expiry`,
//! `apply_cookie_expiry_summary`, `apply_detected_metadata_locked` /
//! `apply_detected_metadata`, `apply_logged_out_state_locked` /
//! `apply_logged_out_state`, `apply_page_error_state_locked` /
//! `apply_page_error_state`, and `notify_external_auth_policy`. No behavior
//! change. Methods are `pub(super)` (or `pub` where they already were) so
//! the parent module and sibling submodules keep calling them via `Self::` /
//! `self.` — methods belong to the type, not the file. The pure quota
//! evaluation helpers live in `quota.rs`; this module is the sync/probe side.

#[cfg(desktop)]
use super::PendingExternalAuth;
#[cfg(desktop)]
use super::{DesktopProbeResult, IDENTITY_PROBE_JS};
use super::{InnerState, ManagerSnapshot, SessionManager};
#[cfg(desktop)]
use crate::platform;
#[cfg(desktop)]
use crate::providers::is_verified_callback_origin;
use crate::providers::resolve_provider;
#[cfg(desktop)]
use crate::storage::now_ms;
use crate::storage::{normalize_avatar_url, AccountKey, AuthState, QuotaDetail};
#[cfg(desktop)]
use std::sync::atomic::Ordering;
#[cfg(desktop)]
use std::sync::Arc;
use tauri::AppHandle;
#[cfg(desktop)]
use tauri::Manager;
#[cfg(desktop)]
use url::Url;
#[cfg(desktop)]
use uuid::Uuid;

impl SessionManager {
  #[cfg(desktop)]
  pub(super) async fn notify_external_auth_policy(
    &self,
    app: &AppHandle,
    key: AccountKey,
    view_instance_id: u64,
  ) {
    let snap_opt = {
      let mut locked = self.inner.lock().await;
      let matches = locked
        .active_slot
        .as_ref()
        .map(|s| s.account_key == key && s.view_instance_id == view_instance_id)
        .unwrap_or(false);
      if !matches {
        return;
      }
      let cur_seq = self.current_seq.load(Ordering::SeqCst);
      let entry_ok = resolve_provider(&key.provider_id)
        .and_then(|spec| {
          Url::parse(&spec.info.entry_url)
            .ok()
            .map(|u| is_verified_callback_origin(&spec, &u))
        })
        .unwrap_or(false);
      locked.pending_external_auth = Some(PendingExternalAuth {
        account_key: key.clone(),
        switch_seq: cur_seq,
        nonce: format!("{}-{}", key.provider_id.as_str(), Uuid::new_v4().simple()),
      });
      if !entry_ok || locked.pending_external_auth.is_some() {
        locked.pending_external_auth = None;
      }
      locked.notice = Some(
        "Google OAuth 按政策禁止内嵌 WebView，请使用邮箱、手机号登录，或点击账号设置使用「导入 Cookie」载入系统浏览器登录态"
          .to_string(),
      );
      Some(Self::snapshot_from_locked(&locked))
    };

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
    }
  }

  #[cfg(desktop)]
  pub(super) async fn probe_meta_account_center_email(
    self: Arc<Self>,
    app: AppHandle,
    key: AccountKey,
    view_instance_id: u64,
  ) {
    let Ok(meta_url) = Url::parse("https://accountscenter.meta.com/") else {
      return;
    };
    let Ok(parsed_uuid) = Uuid::parse_str(&key.uuid) else {
      return;
    };

    let (proxy_raw, profile_dir) = {
      let locked = self.inner.lock().await;
      if !Self::is_view_instance_valid(&locked, &key, view_instance_id) {
        return;
      }
      let Some(acc) = locked
        .store
        .accounts
        .iter()
        .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid && a.auto_alias)
      else {
        return;
      };
      let proxy_str = acc.proxy_url.clone().unwrap_or_default();
      let p_dir = locked.app_data_dir.join("profiles").join(&key.uuid);
      (proxy_str, p_dir)
    };

    // 1A：Windows Cookie 读取在锁外执行（原先在锁内 await）。
    // 代理与热池 label 在这里短锁重读；与上一个短锁之间账号几乎
    // 不可能被改，即使变了也只影响这次尽力而为的 Cookie 读取。
    #[cfg(windows)]
    let has_win_cookies = {
      let (app_data_dir, warm_label, proxy_url_opt) = {
        let locked = self.inner.lock().await;
        (
          locked.app_data_dir.clone(),
          locked
            .warm_pool
            .get(&key)
            .map(|entry| entry.webview_label.clone()),
          locked
            .store
            .accounts
            .iter()
            .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
            .and_then(|a| a.proxy_url.clone()),
        )
      };
      self
        .read_windows_cookies_for_url(
          &app,
          &app_data_dir,
          warm_label,
          proxy_url_opt,
          &key,
          meta_url.clone(),
        )
        .await
        .is_ok_and(|(hdr, _)| !hdr.trim().is_empty())
    };
    #[cfg(not(windows))]
    let has_win_cookies = true;

    #[cfg(target_os = "macos")]
    {
      let Ok((hdr, _)) =
        platform::read_macos_cookies_for_url(&app, parsed_uuid, meta_url.clone()).await
      else {
        return;
      };
      if hdr.trim().is_empty() {
        return;
      }
    }
    #[cfg(windows)]
    {
      if !has_win_cookies {
        return;
      }
    }
    #[cfg(not(windows))]
    let _ = has_win_cookies;

    let Ok(_proxy) = platform::parse_account_proxy(&proxy_raw) else {
      return;
    };
    let Some(window) = app.get_window("main") else {
      return;
    };
    let instance_id = self.view_counter.fetch_add(1, Ordering::SeqCst);
    #[allow(unused_mut)]
    let mut builder = tauri::webview::WebviewBuilder::new(
      format!("meta-id-{instance_id}"),
      tauri::WebviewUrl::External("about:blank".parse().expect("valid blank url")),
    )
    .focused(false)
    .on_navigation(|nav_url| {
      if nav_url.as_str() == "about:blank" {
        return true;
      }
      if nav_url.scheme() != "https" {
        return false;
      }
      nav_url
        .host_str()
        .map(|h| {
          let low = h.to_ascii_lowercase();
          low == "meta.com" || low.ends_with(".meta.com")
        })
        .unwrap_or(false)
    });

    #[cfg(target_os = "macos")]
    {
      builder = builder
        .data_store_identifier(*parsed_uuid.as_bytes())
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15");
    }
    #[cfg(not(target_os = "macos"))]
    {
      builder = builder.data_directory(profile_dir.clone());
    }
    #[cfg(windows)]
    {
      let Ok(b) =
        platform::with_windows_account_proxy(builder, _proxy.as_ref(), &app.config().identifier)
      else {
        return;
      };
      builder = b;
    }

    let (position, size) = Self::initial_hidden_child_rect(&window);
    let Ok(temp_wv) = window.add_child(builder, position, size) else {
      return;
    };
    let _ = temp_wv.hide();

    let run_probe = async {
      if platform::verify_desktop_profile_binding(&temp_wv, parsed_uuid, &profile_dir)
        .await
        .is_err()
      {
        return;
      }
      if temp_wv.navigate(meta_url).is_err() {
        return;
      }
      for attempt in 0..16 {
        tokio::time::sleep(std::time::Duration::from_millis(if attempt == 0 {
          800
        } else {
          600
        }))
        .await;
        let still_needed = {
          let locked = self.inner.lock().await;
          Self::is_view_instance_valid(&locked, &key, view_instance_id)
            && locked
              .store
              .accounts
              .iter()
              .any(|a| a.provider_id == key.provider_id && a.uuid == key.uuid && a.auto_alias)
        };
        if !still_needed {
          break;
        }
        let Some(probe) = Self::eval_identity_probe(&temp_wv).await else {
          continue;
        };
        if probe.logged_out {
          break;
        }
        if !probe.email.is_empty() {
          let _ = self
            .apply_detected_metadata(
              &app,
              key.clone(),
              view_instance_id,
              Some(probe.email),
              None,
              None,
              None,
              None,
              None,
            )
            .await;
          break;
        }
      }
    };

    run_probe.await;
    let _ = temp_wv.close();
  }

  #[cfg(desktop)]
  pub(super) fn spawn_desktop_identity_watcher(
    self: &Arc<Self>,
    app: AppHandle,
    key: AccountKey,
    view_instance_id: u64,
    probe_notify: Arc<tokio::sync::Notify>,
  ) {
    let mgr = Arc::clone(self);
    tauri::async_runtime::spawn(async move {
      let mut tick_count: u32 = 0;
      let mut consecutive_auth_ticks: u32 = 0;
      let mut consecutive_logged_out: u8 = 0;
      let mut consecutive_page_error: u8 = 0;
      let mut stale_session_reset_done = false;
      let mut meta_center_probed = false;
      let mut next_delay_ms: Option<u64> = None;
      loop {
        if let Some(delay_ms) = next_delay_ms.take() {
          if consecutive_logged_out == 1 || consecutive_page_error == 1 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
          } else {
            tokio::select! {
              _ = tokio::time::sleep(std::time::Duration::from_millis(delay_ms)) => {},
              _ = probe_notify.notified() => {},
            }
          }
        }
        tick_count = tick_count.saturating_add(1);

        let labels_opt = {
          let locked = mgr.inner.lock().await;
          let account_exists = locked
            .store
            .accounts
            .iter()
            .any(|a| a.provider_id == key.provider_id && a.uuid == key.uuid);
          if !account_exists {
            None
          } else {
            locked.warm_pool.get(&key).and_then(|e| {
              if e.view_instance_id == view_instance_id && e.alive_flag.load(Ordering::SeqCst) {
                Some((e.webview_label.clone(), e.popup_label.clone()))
              } else {
                None
              }
            })
          }
        };

        let Some((wv_label, popup_label)) = labels_opt else {
          let mut locked = mgr.inner.lock().await;
          if let Some(entry) = locked.warm_pool.get_mut(&key) {
            if entry.view_instance_id == view_instance_id {
              entry.watcher_running = false;
            }
          }
          break;
        };

        if let Some(p_label) = popup_label {
          if let Some(p_wv) = app.get_webview(&p_label) {
            if let Some(probe) = Self::eval_identity_probe(&p_wv).await {
              let popup_hint = if !probe.email.is_empty() {
                Some(probe.email)
              } else if !probe.typed.is_empty() {
                Some(probe.typed)
              } else {
                None
              };
              if let Some(hint_val) = popup_hint {
                let mut locked = mgr.inner.lock().await;
                if let Some(entry) = locked.warm_pool.get_mut(&key) {
                  if entry.view_instance_id == view_instance_id {
                    entry.captured_identity_hint = Some(hint_val);
                  }
                }
              }
            }
          }
        }

        let Some(main_wv) = app.get_webview(&wv_label) else {
          let mut locked = mgr.inner.lock().await;
          if let Some(entry) = locked.warm_pool.get_mut(&key) {
            if entry.view_instance_id == view_instance_id {
              entry.watcher_running = false;
            }
          }
          break;
        };
        let Some(probe) = Self::eval_identity_probe(&main_wv).await else {
          next_delay_ms = Some(if tick_count < 6 { 800 } else { 6500 });
          continue;
        };

        if let Some(idx) = probe.pending_hotkey {
          let mgr_hotkey = Arc::clone(&mgr);
          let app_hotkey = app.clone();
          tauri::async_runtime::spawn(async move {
            let _ = mgr_hotkey.switch_account_by_index(&app_hotkey, idx).await;
          });
        }

        let (resolved_identity, soft_identity) = {
          let mut locked = mgr.inner.lock().await;
          let cookie_hint = locked.cookie_identity_hints.get(&key).cloned();
          if let Some(entry) = locked.warm_pool.get_mut(&key) {
            if entry.view_instance_id == view_instance_id {
              if !probe.typed.is_empty() {
                entry.captured_identity_hint = Some(probe.typed);
              }
              if probe.is_auth {
                let strong = if !probe.email.is_empty() {
                  Some(probe.email)
                } else if entry.captured_identity_hint.is_some() {
                  entry.captured_identity_hint.clone()
                } else {
                  cookie_hint
                };
                let soft = if strong.is_none() {
                  if !probe.display_name.is_empty() {
                    Some(probe.display_name)
                  } else if consecutive_auth_ticks >= 3 {
                    Some("已登录账号".to_string())
                  } else {
                    None
                  }
                } else {
                  None
                };
                (strong, soft)
              } else {
                (None, None)
              }
            } else {
              (None, None)
            }
          } else {
            (None, None)
          }
        };

        if probe.is_auth {
          consecutive_auth_ticks = consecutive_auth_ticks.saturating_add(1);
          consecutive_logged_out = 0;
          consecutive_page_error = 0;
          stale_session_reset_done = false;
          if key.provider_id.is_muse() && resolved_identity.is_none() && !meta_center_probed {
            let needs_meta_probe = {
              let locked = mgr.inner.lock().await;
              locked
                .store
                .accounts
                .iter()
                .any(|a| a.provider_id == key.provider_id && a.uuid == key.uuid && a.auto_alias)
            };
            if needs_meta_probe {
              meta_center_probed = true;
              tauri::async_runtime::spawn(Arc::clone(&mgr).probe_meta_account_center_email(
                app.clone(),
                key.clone(),
                view_instance_id,
              ));
            }
          }
          let (just_authenticated, needs_cookie_check) = mgr
            .apply_detected_metadata(
              &app,
              key.clone(),
              view_instance_id,
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
          if just_authenticated || needs_cookie_check {
            let mgr_cookie = Arc::clone(&mgr);
            let app_cookie = app.clone();
            let key_cookie = key.clone();
            tauri::async_runtime::spawn(async move {
              mgr_cookie
                .refresh_account_cookie_expiry(&app_cookie, key_cookie, just_authenticated)
                .await;
            });
          }
          next_delay_ms = Some(if tick_count < 6 { 950 } else { 6500 });
        } else if probe.logged_out {
          consecutive_auth_ticks = 0;
          meta_center_probed = false;
          consecutive_page_error = 0;
          consecutive_logged_out = consecutive_logged_out.saturating_add(1);
          if consecutive_logged_out >= 2 {
            mgr
              .apply_logged_out_state(&app, key.clone(), view_instance_id)
              .await;
            if probe.stale_session && !stale_session_reset_done {
              stale_session_reset_done = true;
              if key.provider_id.is_muse() {
                if let Ok(u) = Url::parse("https://muse.ai/api/logout") {
                  let _ = main_wv.navigate(u);
                }
              } else if key.provider_id.is_hark() {
                if let Ok(u) = Url::parse("https://app.hark.com/login") {
                  let _ = main_wv.navigate(u);
                }
              }
            }
            next_delay_ms = Some(if tick_count < 6 { 950 } else { 6500 });
          } else {
            next_delay_ms = Some(300);
          }
        } else if !probe.page_error.is_empty() {
          consecutive_auth_ticks = 0;
          consecutive_logged_out = 0;
          consecutive_page_error = consecutive_page_error.saturating_add(1);
          if consecutive_page_error >= 2 {
            mgr
              .apply_page_error_state(&app, key.clone(), view_instance_id, probe.page_error)
              .await;
            next_delay_ms = Some(if tick_count < 6 { 950 } else { 6500 });
          } else {
            next_delay_ms = Some(400);
          }
        } else {
          consecutive_auth_ticks = 0;
          consecutive_logged_out = 0;
          consecutive_page_error = 0;
          next_delay_ms = Some(if tick_count < 6 { 500 } else { 6500 });
        }
      }
    });
  }

  #[cfg(desktop)]
  pub(super) async fn eval_identity_probe(webview: &tauri::Webview) -> Option<DesktopProbeResult> {
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx_cell = Arc::new(std::sync::Mutex::new(Some(tx)));
    let tx_inner = Arc::clone(&tx_cell);
    webview
      .eval_with_callback(IDENTITY_PROBE_JS, move |raw| {
        if let Some(sender) = tx_inner.lock().unwrap_or_else(|e| e.into_inner()).take() {
          let _ = sender.send(raw);
        }
      })
      .ok()?;

    let raw = tokio::time::timeout(std::time::Duration::from_millis(1200), rx)
      .await
      .ok()?
      .ok()?;

    let inner_json = serde_json::from_str::<String>(&raw).ok()?;
    let val = serde_json::from_str::<serde_json::Value>(&inner_json).ok()?;
    let is_auth = val.get("auth").and_then(|v| v.as_bool()).unwrap_or(false);
    let logged_out = val
      .get("loggedOut")
      .and_then(|v| v.as_bool())
      .unwrap_or(false);
    let stale_session = val
      .get("staleSession")
      .and_then(|v| v.as_bool())
      .unwrap_or(false);
    let page_error = val
      .get("pageError")
      .and_then(|v| v.as_str())
      .unwrap_or("")
      .trim()
      .to_string();
    let typed = val
      .get("typed")
      .and_then(|v| v.as_str())
      .unwrap_or("")
      .trim()
      .to_string();
    let email = val
      .get("email")
      .and_then(|v| v.as_str())
      .unwrap_or("")
      .trim()
      .to_string();
    let display_name = val
      .get("displayName")
      .and_then(|v| v.as_str())
      .unwrap_or("")
      .trim()
      .to_string();
    let avatar = val
      .get("avatar")
      .and_then(|v| v.as_str())
      .unwrap_or("")
      .trim()
      .to_string();
    let quota_text = val
      .get("quotaText")
      .and_then(|v| v.as_str())
      .unwrap_or("")
      .trim()
      .to_string();
    let quota_pct = val
      .get("quotaPct")
      .and_then(|v| v.as_u64())
      .map(|n| n.min(100) as u8);
    let quota_detail = val
      .get("quotaDetail")
      .filter(|v| !v.is_null())
      .and_then(|v| serde_json::from_value::<QuotaDetail>(v.clone()).ok())
      .filter(|d| !d.plan_label.trim().is_empty() || !d.plan_value.trim().is_empty());
    let pending_hotkey = val
      .get("pendingHotkey")
      .and_then(|v| v.as_u64())
      .map(|n| n as u8);

    Some(DesktopProbeResult {
      is_auth,
      logged_out,
      stale_session,
      page_error,
      typed,
      email,
      display_name,
      avatar,
      quota_text,
      quota_pct,
      quota_detail,
      pending_hotkey,
    })
  }

  #[cfg_attr(not(desktop), allow(dead_code))]
  pub(super) fn apply_page_error_state_locked(
    locked: &mut InnerState,
    key: AccountKey,
    view_instance_id: u64,
    message: &str,
  ) -> Option<ManagerSnapshot> {
    let trimmed = message.trim().to_string();
    if trimmed.is_empty() || !Self::is_view_instance_valid(locked, &key, view_instance_id) {
      return None;
    }
    if !locked
      .store
      .accounts
      .iter()
      .any(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
    {
      return None;
    }
    if locked.account_page_errors.get(&key) == Some(&trimmed) {
      return None;
    }
    locked.account_page_errors.insert(key, trimmed);
    Some(Self::snapshot_from_locked(locked))
  }

  #[cfg_attr(not(desktop), allow(dead_code))]
  pub(super) async fn apply_page_error_state(
    &self,
    app: &AppHandle,
    key: AccountKey,
    view_instance_id: u64,
    message: String,
  ) {
    let snap_opt = {
      let mut locked = self.inner.lock().await;
      Self::apply_page_error_state_locked(&mut locked, key, view_instance_id, &message)
    };

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
    }
  }

  // 注意：本函数体内已无 .await，但保持 async——session_manager.rs
  // 的测试直接以 `.await` 调用它，改同步 fn 会破坏测试编译。
  pub(super) async fn apply_logged_out_state_locked(
    locked: &mut InnerState,
    key: &AccountKey,
    view_instance_id: u64,
  ) -> Option<ManagerSnapshot> {
    if !Self::is_view_instance_valid(locked, key, view_instance_id) {
      return None;
    }

    #[cfg(desktop)]
    locked.cookie_checked_at.remove(key);
    locked.cookie_identity_hints.remove(key);
    let cleared_err = locked.account_page_errors.remove(key).is_some();

    let target = locked
      .store
      .accounts
      .iter_mut()
      .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)?;

    let revert_placeholder = target.auto_alias && target.alias == "已登录账号";
    let store_changed = revert_placeholder
      || !(target.auth_state == AuthState::LoggedOut
        && target.quota_detail.is_none()
        && target.quota_text.is_none()
        && target.quota_percent.is_none()
        && target.cookie_expires_at_ms.is_none()
        && !target.cookie_session_only);

    if !store_changed && !cleared_err {
      return None;
    }

    if store_changed {
      target.auth_state = AuthState::LoggedOut;
      if revert_placeholder {
        target.alias = "待登录账号".to_string();
      }
      target.quota_detail = None;
      target.quota_text = None;
      target.quota_percent = None;
      target.cookie_expires_at_ms = None;
      target.cookie_session_only = false;
      // 1A：锁内不再同步写盘。调用方（apply_logged_out_state）在锁
      // 释放后对返回的快照 request_persist；写盘失败时写线程立
      // notice 并保留重试，快照照常返回。
    }

    Some(Self::snapshot_from_locked(locked))
  }

  pub(super) async fn apply_logged_out_state(
    &self,
    app: &AppHandle,
    key: AccountKey,
    view_instance_id: u64,
  ) {
    let snap_opt = {
      let mut locked = self.inner.lock().await;
      Self::apply_logged_out_state_locked(&mut locked, &key, view_instance_id).await
    };

    // 1A：锁内不再同步写盘；有快照即内存有改动，走防抖写盘。
    // （仅清页面错误、无 store 改动时也会多一次内容不变的写盘，无害。）
    if snap_opt.is_some() {
      self.request_persist();
    }

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
    }
  }

  #[cfg(desktop)]
  #[allow(dead_code)]
  pub(super) async fn apply_cookie_expiry_summary(
    &self,
    app: &AppHandle,
    key: &AccountKey,
    summary: platform::CookieExpirySummary,
  ) {
    let checked_at = now_ms();
    let snap_opt = {
      let mut locked = self.inner.lock().await;
      let Some(idx) = locked
        .store
        .accounts
        .iter()
        .position(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
      else {
        return;
      };
      if locked.store.accounts[idx].auth_state == AuthState::LoggedOut {
        return;
      }
      if summary.expires_at_ms.is_some() || summary.session_only {
        locked.cookie_checked_at.insert(key.clone(), checked_at);
      }
      let target = &mut locked.store.accounts[idx];
      if target.cookie_expires_at_ms == summary.expires_at_ms
        && target.cookie_session_only == summary.session_only
      {
        return;
      }
      target.cookie_expires_at_ms = summary.expires_at_ms;
      target.cookie_session_only = summary.session_only;
      // 1A：后台同步路径，锁内不再同步写盘，改走防抖；快照照常返回。
      self.request_persist();
      Some(Self::snapshot_from_locked(&locked))
    };

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
    }
  }

  #[cfg(desktop)]
  #[allow(dead_code)]
  pub(super) async fn refresh_account_cookie_expiry(
    self: &Arc<Self>,
    app: &AppHandle,
    key: AccountKey,
    force: bool,
  ) {
    const COOKIE_CHECK_COOLDOWN_MS: u64 = 10 * 60 * 1000;
    let now = now_ms();

    #[cfg(not(any(target_os = "macos", windows)))]
    {
      let _ = (app, key, force, now, COOKIE_CHECK_COOLDOWN_MS);
    }

    #[cfg(any(target_os = "macos", windows))]
    {
      let url_opt = {
        let locked = self.inner.lock().await;
        let Some(account) = locked
          .store
          .accounts
          .iter()
          .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
        else {
          return;
        };
        if account.auth_state == AuthState::LoggedOut {
          return;
        }
        if !force {
          if let Some(&last_checked) = locked.cookie_checked_at.get(&key) {
            if now.saturating_sub(last_checked) < COOKIE_CHECK_COOLDOWN_MS {
              return;
            }
          }
        }
        let Some(spec) = resolve_provider(&key.provider_id) else {
          return;
        };
        if !spec.info.enabled {
          return;
        }
        let Ok(url) = Url::parse(&spec.info.entry_url) else {
          return;
        };

        #[cfg(target_os = "macos")]
        let platform_target = Uuid::parse_str(&key.uuid).ok().map(|uuid| (url, uuid));

        #[cfg(windows)]
        let platform_target = locked
          .warm_pool
          .get(&key)
          .and_then(|entry| app.get_webview(&entry.webview_label))
          .map(|webview| (url, webview));

        platform_target
      };

      let Some((url, handle)) = url_opt else {
        return;
      };

      #[cfg(target_os = "macos")]
      let summary_res = platform::read_macos_cookies_for_url(app, handle, url)
        .await
        .map(|(_, summary)| summary);

      #[cfg(windows)]
      let summary_res = Self::read_windows_webview_cookies(handle, url)
        .await
        .map(|(_, summary)| summary);

      if let Ok(summary) = summary_res {
        self.apply_cookie_expiry_summary(app, &key, summary).await;
      }
    }
  }

  #[allow(clippy::too_many_arguments)]
  pub(super) async fn apply_detected_metadata_locked(
    locked: &mut InnerState,
    key: &AccountKey,
    view_instance_id: u64,
    identity_opt: Option<String>,
    soft_identity_opt: Option<String>,
    avatar_opt: Option<String>,
    quota_text_opt: Option<String>,
    quota_pct_opt: Option<u8>,
    quota_detail_opt: Option<QuotaDetail>,
  ) -> (Option<ManagerSnapshot>, bool, bool, Option<String>) {
    if !Self::is_view_instance_valid(locked, key, view_instance_id) {
      return (None, false, false, None);
    }

    let cleared_err = locked.account_page_errors.remove(key).is_some();

    let Some(target) = locked
      .store
      .accounts
      .iter_mut()
      .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
    else {
      return (None, false, false, None);
    };

    let mut changed = false;
    let mut became_authenticated = false;
    let mut alias_updated = None;

    if target.auth_state != AuthState::Authenticated {
      target.auth_state = AuthState::Authenticated;
      became_authenticated = true;
      changed = true;
    }

    if target.auto_alias {
      if let Some(raw_id) = identity_opt {
        let cleaned = raw_id.trim();
        if !cleaned.is_empty() && cleaned.len() <= 80 {
          if target.alias != cleaned {
            target.alias = cleaned.to_string();
            alias_updated = Some(cleaned.to_string());
          }
          target.auto_alias = false;
          changed = true;
        }
      } else if let Some(raw_soft) = soft_identity_opt {
        let cleaned = raw_soft.trim();
        if !cleaned.is_empty() && cleaned.len() <= 80 {
          let should_apply_soft =
            target.alias == "待登录账号" || (cleaned != "已登录账号" && target.alias != cleaned);
          if should_apply_soft {
            target.alias = cleaned.to_string();
            alias_updated = Some(cleaned.to_string());
            changed = true;
          }
        }
      }
    }

    if let Some(av) = avatar_opt {
      // MAAM-M-10：实时探测的头像只入库 data: 位图（现有行为：http(s) 不入库），
      // 且须过白名单——SVG data URL（base64 与非 base64）一律拒收，静默保持原头像。
      if let Some(av) = normalize_avatar_url(&av) {
        if av.starts_with("data:") && av.len() < 80_000 && target.avatar_url.as_ref() != Some(&av) {
          target.avatar_url = Some(av);
          changed = true;
        }
      }
    }

    if let Some(detail) = quota_detail_opt {
      if target.quota_detail.as_ref() != Some(&detail) {
        target.quota_detail = Some(detail);
        changed = true;
      }
    }

    if let Some(qt) = quota_text_opt {
      let cleaned_qt = qt.trim();
      if !cleaned_qt.is_empty() && cleaned_qt.len() <= 60 {
        let is_placeholder = cleaned_qt == "额度充足" || cleaned_qt == "正在同步额度…";
        let should_overwrite = match (&target.quota_text, is_placeholder) {
          (Some(existing), true) if existing != "额度充足" && existing != "正在同步额度…" => {
            false
          }
          (Some(existing), _)
            if existing == cleaned_qt && target.quota_percent == quota_pct_opt =>
          {
            false
          }
          _ => true,
        };
        if should_overwrite {
          target.quota_text = Some(cleaned_qt.to_string());
          target.quota_percent = quota_pct_opt;
          changed = true;
        }
      }
    }

    let needs_cookie_check = became_authenticated
      || (target.cookie_expires_at_ms.is_none() && !target.cookie_session_only);

    // 1A：锁内不再同步写盘。调用方（apply_detected_metadata）在锁释放
    // 后对返回的快照 request_persist；写盘失败时写线程立 notice 并
    // 保留重试，快照照常返回。
    // 注意：本函数体内已无 .await，但保持 async——session_manager.rs
    // 的测试直接以 `.await` 调用它，改同步 fn 会破坏测试编译。
    let snap_opt = if changed || cleared_err {
      Some(Self::snapshot_from_locked(locked))
    } else {
      None
    };

    let fg_title = if snap_opt.is_some() {
      alias_updated.and_then(|new_alias| {
        let is_fg = locked
          .active_slot
          .as_ref()
          .map(|s| s.committed && &s.account_key == key)
          .unwrap_or(false);
        if is_fg {
          let short_name = resolve_provider(&key.provider_id)
            .map(|s| s.info.short_name)
            .unwrap_or_else(|| key.provider_id.as_str().to_string());
          Some(format!("{short_name} · {new_alias}"))
        } else {
          None
        }
      })
    } else {
      None
    };

    (snap_opt, became_authenticated, needs_cookie_check, fg_title)
  }

  #[allow(clippy::too_many_arguments)]
  pub(super) async fn apply_detected_metadata(
    &self,
    app: &AppHandle,
    key: AccountKey,
    view_instance_id: u64,
    identity_opt: Option<String>,
    soft_identity_opt: Option<String>,
    avatar_opt: Option<String>,
    quota_text_opt: Option<String>,
    quota_pct_opt: Option<u8>,
    quota_detail_opt: Option<QuotaDetail>,
  ) -> (bool, bool) {
    let (snap_opt, became_authenticated, needs_cookie_check, fg_title) = {
      let mut locked = self.inner.lock().await;
      Self::apply_detected_metadata_locked(
        &mut locked,
        &key,
        view_instance_id,
        identity_opt,
        soft_identity_opt,
        avatar_opt,
        quota_text_opt,
        quota_pct_opt,
        quota_detail_opt,
      )
      .await
    };

    // 1A：锁内不再同步写盘；有快照即内存有改动，走防抖写盘。
    // （仅清页面错误、无 store 改动时也会多一次内容不变的写盘，无害。）
    if snap_opt.is_some() {
      self.request_persist();
    }

    if let Some(title) = fg_title {
      #[cfg(desktop)]
      if let Some(main_win) = app.get_window("main") {
        let _ = main_win.set_title(&title);
      }
      #[cfg(mobile)]
      {
        let seq_opt = {
          let locked = self.inner.lock().await;
          locked.active_slot.as_ref().map(|s| s.switch_seq)
        };
        if let Some(seq) = seq_opt {
          let _ = crate::platform::mobile::update_account_title(app, seq, &title).await;
        }
      }
    }

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
    }

    (became_authenticated, needs_cookie_check)
  }
}
