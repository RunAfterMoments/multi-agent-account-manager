//! Desktop webview lifecycle (P2-11 split).
//!
//! Moved verbatim from `session_manager.rs`: the warm-pool helpers
//! (`close_warm_entry`, `group_needs_auth_members`, `evict_warm_pool_if_needed`,
//! `reclaim_idle_desktop_webviews`, `detach_foreground_slot_locked` / `execute_hide_slot`,
//! `set_right_webview_input_frozen`, `apply_desktop_layout_locked`,
//! `recompute_desktop_bounds`), the account-webview builders
//! (`build_desktop_account_webview`, `attach_desktop_account_webview`,
//! `navigate_desktop_account`, `open_desktop_session`, `eval_paint_ready`,
//! `wait_paint_and_finalize_desktop`, `fail_initial_proxy_load`,
//! `initial_hidden_child_rect`, `discard_desktop_view_locked`), prewarming
//! (`prewarm_desktop_account`, `prewarm_next_cold_account` incl. the
//! non-desktop stub), `register_desktop_popup`, and `is_view_instance_valid`.
//! No behavior change. Methods are `pub(super)` (or `pub` where they already
//! were) so the parent module and sibling submodules keep calling them via
//! `Self::` / `self.` — methods belong to the type, not the file. The
//! identity watcher (`spawn_desktop_identity_watcher`) still lives in the
//! parent and calls into this module.

#[cfg(desktop)]
use super::{ActiveSessionSlot, ManagerSnapshot, APP_TITLE};
use super::{InnerState, SessionManager};
#[cfg(desktop)]
use super::{
  WarmDesktopEntry, AUTO_APPROVE_INIT_JS, BACKGROUND_RAF_INIT_JS, DOM_PAINT_READY_JS,
  GROUP_IDLE_RECLAIM_MS, LEFT_PANE_WIDTH, LOGIN_CAPTURE_INIT_JS, MAX_CONCURRENT_PREWARMS,
  MAX_WARM_DESKTOP_VIEWS, SERVICE_RAIL_WIDTH,
};
#[cfg(desktop)]
use crate::platform;
#[cfg(desktop)]
use crate::providers::ProviderSpec;
#[cfg(desktop)]
use crate::providers::{classify_top_level_url, resolve_provider, NavigationDecision};
use crate::storage::AccountKey;
#[cfg(desktop)]
use crate::storage::{now_ms, AccountRecord, GroupReplyStatus};
#[cfg(desktop)]
use std::collections::{HashMap, HashSet, VecDeque};
#[cfg(desktop)]
use std::path::PathBuf;
use std::sync::atomic::Ordering;
#[cfg(desktop)]
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;
use tauri::AppHandle;
#[cfg(desktop)]
use tauri::Manager;
#[cfg(desktop)]
use url::Url;
#[cfg(desktop)]
use uuid::Uuid;

/// 移动端在锁外关闭当前活动会话时携带的参数（仅需 switch_seq）。
/// 桌面端在 detach 里已同步完成隐藏，execute 无事可做。
pub(super) struct HideSlotOp {
  next_seq: u64,
}

impl SessionManager {
  #[cfg(desktop)]
  pub(super) fn close_warm_entry(app: &AppHandle, entry: WarmDesktopEntry) {
    entry.alive_flag.store(false, Ordering::SeqCst);
    entry.probe_notify.notify_one();
    if let Some(popup_label) = entry.popup_label {
      if let Some(popup_win) = app.get_webview_window(&popup_label) {
        let _ = popup_win.close();
      }
    }
    if let Some(wv) = app.get_webview(&entry.webview_label) {
      let _ = wv.hide();
      let _ = wv.close();
    }
  }

  #[cfg(desktop)]
  pub(super) fn group_needs_auth_members(locked: &InnerState) -> HashSet<AccountKey> {
    locked
      .active_group_id
      .as_ref()
      .and_then(|gid| locked.store.group_chats.iter().find(|g| &g.id == gid))
      .map(|g| {
        g.messages
          .iter()
          .filter(|m| m.status == GroupReplyStatus::NeedsAuth)
          .filter_map(|m| m.sender.clone())
          .collect()
      })
      .unwrap_or_default()
  }

  #[cfg(desktop)]
  pub(super) fn evict_warm_pool_if_needed(
    app: &AppHandle,
    locked: &mut InnerState,
    protect: Option<&AccountKey>,
    max_len: usize,
  ) {
    let needs_auth_members = Self::group_needs_auth_members(locked);

    while locked.warm_pool.len() > max_len {
      let victim_key = locked
        .warm_pool
        .iter()
        .filter(|(k, _)| {
          Some(*k) != protect
            && locked.active_slot.as_ref().map(|s| &s.account_key) != Some(*k)
            && locked.in_flight_group_members.get(*k).copied().unwrap_or(0) == 0
            && !needs_auth_members.contains(*k)
        })
        .min_by_key(|(_, entry)| (entry.last_used_ms, entry.view_instance_id))
        .map(|(k, _)| k.clone());
      let Some(victim) = victim_key else {
        break;
      };
      if let Some(entry) = locked.warm_pool.remove(&victim) {
        Self::close_warm_entry(app, entry);
      }
    }
  }

  #[cfg(desktop)]
  pub(super) async fn reclaim_idle_desktop_webviews(&self, app: &AppHandle) {
    let now = now_ms();
    let snap_opt = {
      let mut locked = self.inner.lock().await;
      let needs_auth_members = Self::group_needs_auth_members(&locked);
      let in_group = locked.active_group_id.is_some();

      let mut candidates: Vec<(AccountKey, u64)> = locked
        .warm_pool
        .iter()
        .filter(|(k, entry)| {
          entry.paint_ready
            && entry.last_used_ms > 0
            && now.saturating_sub(entry.last_used_ms) >= GROUP_IDLE_RECLAIM_MS
            && locked.active_slot.as_ref().map(|s| &s.account_key) != Some(*k)
            && locked.in_flight_group_members.get(*k).copied().unwrap_or(0) == 0
            && !needs_auth_members.contains(*k)
        })
        .map(|(k, entry)| (k.clone(), entry.last_used_ms))
        .collect();

      if candidates.is_empty() {
        return;
      }

      if !in_group && locked.warm_pool.len() <= 2 {
        return;
      }

      candidates.sort_by_key(|(_, last_used)| *last_used);
      let keep_recent = if in_group {
        0
      } else {
        2.min(locked.warm_pool.len())
      };
      let mut closed_any = false;
      for (victim, _) in candidates {
        if !in_group && locked.warm_pool.len() <= keep_recent {
          break;
        }
        if let Some(entry) = locked.warm_pool.remove(&victim) {
          Self::close_warm_entry(app, entry);
          closed_any = true;
        }
      }

      if closed_any {
        Some(Self::snapshot_from_locked(&locked))
      } else {
        None
      }
    };

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
    }
  }

  /// 1A：旧 `hide_foreground_slot` 的锁内纯内存步——清
  /// `pending_external_auth`、复位 `active_view_id`、取出旧槽位；
  /// 桌面端同步完成 popup 关闭 / webview 隐藏 / 标题与布局。
  /// 移动端若有旧槽位则返回 `Some(op)`，调用方释放 `inner` 锁后
  /// 再调 `execute_hide_slot`；无旧槽位返回 `None`，不用执行第二步。
  pub(super) fn detach_foreground_slot_locked(
    &self,
    app: &AppHandle,
    next_seq: u64,
    locked: &mut InnerState,
  ) -> Option<HideSlotOp> {
    // 移动端不用 app（关闭会话在锁外的 execute_hide_slot 里做）；
    // 显式读一次，避免移动端目标报 app 从未读取的警告。
    #[cfg(mobile)]
    let _ = app;
    locked.pending_external_auth = None;
    self.active_view_id.store(0, Ordering::SeqCst);
    let had_slot = if let Some(old_slot) = locked.active_slot.take() {
      #[cfg(desktop)]
      {
        let _ = next_seq;
        if let Some(entry) = locked.warm_pool.get_mut(&old_slot.account_key) {
          if let Some(popup_label) = entry.popup_label.take() {
            if let Some(popup_win) = app.get_webview_window(&popup_label) {
              let _ = popup_win.close();
            }
          }
          if let Some(wv) = app.get_webview(&entry.webview_label) {
            let _ = wv.hide();
          }
        }
      }
      #[cfg(mobile)]
      {
        let _ = old_slot;
      }
      true
    } else {
      false
    };
    #[cfg(desktop)]
    {
      if let Some(main_win) = app.get_window("main") {
        let _ = main_win.set_title(APP_TITLE);
      }
      Self::apply_desktop_layout_locked(app, locked);
    }
    #[cfg(mobile)]
    {
      if had_slot {
        Some(HideSlotOp { next_seq })
      } else {
        None
      }
    }
    #[cfg(not(mobile))]
    {
      let _ = had_slot;
      None
    }
  }

  /// 1A：旧 `hide_foreground_slot` 的锁外执行步——移动端关闭当前活动
  /// 会话（插件 IPC，需 await，必须在释放 `inner` 锁后调用）。
  /// 桌面端在 detach 里已同步完成，调用后直接返回。
  pub(super) async fn execute_hide_slot(&self, app: &AppHandle, op: HideSlotOp) {
    #[cfg(mobile)]
    {
      let _ = crate::platform::mobile::close_active_session(app, op.next_seq).await;
    }
    #[cfg(not(mobile))]
    {
      // 桌面端无事可做；显式读一次字段，避免非移动端目标报
      // `next_seq` 从未读取的 dead_code 警告。
      let _ = app;
      let _ = op.next_seq;
    }
  }

  #[cfg(desktop)]
  pub(super) fn set_right_webview_input_frozen(webview: &tauri::Webview, frozen: bool) {
    #[cfg(windows)]
    {
      let _ = webview.with_webview(move |pwv| unsafe {
        let mut hwnd = windows::Win32::Foundation::HWND::default();
        if pwv.controller().ParentWindow(&mut hwnd).is_ok() && !hwnd.0.is_null() {
          let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(hwnd, !frozen);
        }
      });
    }

    if frozen {
      let _ = webview.eval(
        r#"(() => {
          try {
            document.documentElement.inert = true;
            document.activeElement?.blur?.();
            if (!document.getElementById('__mgr_freeze_shield__')) {
              const s = document.createElement('div');
              s.id = '__mgr_freeze_shield__';
              s.style.cssText = 'position:fixed;inset:0;z-index:2147483647;background:transparent;cursor:not-allowed;';
              for (const ev of ['mousedown','mouseup','click','keydown','keyup','keypress','focusin','wheel','touchstart']) {
                s.addEventListener(ev, (e) => { e.stopImmediatePropagation(); e.preventDefault(); }, true);
              }
              document.documentElement.appendChild(s);
            }
          } catch (_) {}
        })()"#,
      );
    } else {
      let _ = webview.eval(
        r#"(() => {
          try {
            document.documentElement.inert = false;
            document.getElementById('__mgr_freeze_shield__')?.remove();
          } catch (_) {}
        })()"#,
      );
    }
  }

  /// 当前生效的折叠态：用户偏好 + 弹窗/锁死/冲突强制展开。
  /// 弹窗开时临时展开（set_modal_active 本来就会调布局重算，关弹窗
  /// 后自动缩回）；锁死或 UUID 冲突时强制展开，不隐藏恢复按钮。
  /// 前后端单一真相源：snapshot_from_locked 也用它算
  /// `effective_sidebar_collapsed`。
  pub(super) fn is_sidebar_effectively_collapsed(locked: &InnerState) -> bool {
    locked.store.settings.sidebar_collapsed
      && !locked.modal_active
      && !locked.persist_blocked
      && locked.conflicts.is_empty()
  }

  #[cfg(desktop)]
  pub(super) fn apply_desktop_layout_locked(app: &AppHandle, locked: &InnerState) {
    let Some(main_win) = app.get_window("main") else {
      return;
    };
    let Ok(phys_size) = main_win.inner_size() else {
      return;
    };
    let Ok(scale) = main_win.scale_factor() else {
      return;
    };

    // 折叠态只留 68px 服务栏（与 styles.css 的 .service-rail 同宽），
    // 账号 WebView 多出 312px。manager 与账号 WebView 的 bounds、
    // 群聊后台 keep_group_behind 全部走 left_phys，一处改完就全端联动。
    // 注意不用 CSS 过渡动画：原生子 WebView 的 set_bounds 是瞬时跳变，
    // CSS 动画会造成 200ms 白缝或挤压抖动，前端用 display:none 瞬切。
    let left_logical = if Self::is_sidebar_effectively_collapsed(locked) {
      SERVICE_RAIL_WIDTH
    } else {
      LEFT_PANE_WIDTH
    };
    let (left_phys, right_phys, height_phys) =
      Self::desktop_right_rect(phys_size.width, phys_size.height, scale, left_logical);

    let active_committed_label = locked.active_slot.as_ref().and_then(|slot| {
      if slot.committed {
        locked
          .warm_pool
          .get(&slot.account_key)
          .map(|e| e.webview_label.clone())
      } else {
        None
      }
    });

    let active_loading_label = locked.active_slot.as_ref().and_then(|slot| {
      if !slot.committed {
        locked
          .warm_pool
          .get(&slot.account_key)
          .map(|e| e.webview_label.clone())
      } else {
        None
      }
    });

    let stage_loading_behind = cfg!(target_os = "macos")
      && active_committed_label.is_none()
      && active_loading_label.is_some()
      && right_phys > 1;
    let group_work_behind =
      cfg!(target_os = "macos") && active_committed_label.is_none() && right_phys > 1;

    if group_work_behind {
      if let Some(mgr_wv) = app.get_webview("manager") {
        let _ = mgr_wv.set_bounds(tauri::Rect {
          position: tauri::PhysicalPosition::new(0, 0).into(),
          size: tauri::PhysicalSize::new(phys_size.width, phys_size.height).into(),
        });
        #[cfg(target_os = "macos")]
        platform::raise_macos_webview_to_front(&mgr_wv);
      }
    }

    // 切号黑闪修复：先显新号、再藏旧号。旧顺序是循环里先 hide 旧
    // WebView、最后才 show 新号，中间几帧右侧裸露出下层主窗口；
    // 现在先把新号 set_bounds + show，再去藏旧号——任何一帧都至少
    // 有一个 WebView 盖住右侧，不留空窗（z 序不影响：旧号盖在上面
    // 时画面仍是旧号内容，藏掉后即露出已就位的新号）。
    if let Some(active_label) = active_committed_label.as_ref() {
      if let Some(mgr_wv) = app.get_webview("manager") {
        let _ = mgr_wv.set_bounds(tauri::Rect {
          position: tauri::PhysicalPosition::new(0, 0).into(),
          size: tauri::PhysicalSize::new(left_phys, height_phys).into(),
        });
      }

      if let Some(acc_wv) = app.get_webview(active_label) {
        if right_phys <= 1 {
          let _ = acc_wv.hide();
        } else {
          let _ = acc_wv.set_bounds(tauri::Rect {
            position: tauri::PhysicalPosition::new(left_phys as i32, 0).into(),
            size: tauri::PhysicalSize::new(right_phys, height_phys).into(),
          });
          let _ = acc_wv.show();
        }
        Self::set_right_webview_input_frozen(&acc_wv, locked.modal_active);
        if locked.modal_active {
          if let Some(mgr_wv) = app.get_webview("manager") {
            let _ = mgr_wv.set_focus();
          }
        }
      }
    } else if let Some(mgr_wv) = app.get_webview("manager") {
      let _ = mgr_wv.set_bounds(tauri::Rect {
        position: tauri::PhysicalPosition::new(0, 0).into(),
        size: tauri::PhysicalSize::new(phys_size.width, phys_size.height).into(),
      });

      #[cfg(target_os = "macos")]
      {
        platform::raise_macos_webview_to_front(&mgr_wv);
        // 注意用 as_ref().filter(...)：直接 filter 会 move 掉
        // active_loading_label，后面的循环还要借用它（macOS 专属，
        // Linux/Windows 编不到这段，CI 的 macOS job 抓出来的）。
        if let Some(loading_label) = active_loading_label.as_ref().filter(|_| right_phys > 1) {
          if let Some(loading_wv) = app.get_webview(loading_label) {
            let _ = loading_wv.set_bounds(tauri::Rect {
              position: tauri::PhysicalPosition::new(left_phys as i32, 0).into(),
              size: tauri::PhysicalSize::new(right_phys, height_phys).into(),
            });
            let _ = loading_wv.show();
            platform::raise_macos_webview_to_front(&mgr_wv);
          }
        }
      }
    }

    // 切号黑闪修复：只要右侧矩形变化，就把 warm_pool 里所有隐藏保活
    // 的 WebView 也同步 set_bounds 到最新右侧尺寸（保持 hide 状态），
    // 切过去之前 Chromium 早就按正确宽度排好版，不再现算。
    // 矩形未变时跳过，避免 Moved 等事件重复下发（set_bounds 本身不贵，
    // 但隐藏页收到新尺寸会触发 Chromium 重排，没必要每事件都做）。
    let rect_changed =
      locked.last_desktop_right_rect.get() != Some((left_phys, right_phys, height_phys));
    if rect_changed {
      locked
        .last_desktop_right_rect
        .set(Some((left_phys, right_phys, height_phys)));
    }

    for entry in locked.warm_pool.values() {
      let keep_visible = Some(&entry.webview_label) == active_committed_label.as_ref()
        || (stage_loading_behind && Some(&entry.webview_label) == active_loading_label.as_ref());
      let keep_group_behind = group_work_behind
        && locked
          .in_flight_group_members
          .contains_key(&entry.account_key);
      if keep_group_behind {
        if let Some(wv) = app.get_webview(&entry.webview_label) {
          let _ = wv.set_bounds(tauri::Rect {
            position: tauri::PhysicalPosition::new(left_phys as i32, 0).into(),
            size: tauri::PhysicalSize::new(right_phys, height_phys).into(),
          });
          let _ = wv.show();
        }
      } else if !keep_visible {
        if let Some(wv) = app.get_webview(&entry.webview_label) {
          if rect_changed {
            let _ = wv.set_bounds(tauri::Rect {
              position: tauri::PhysicalPosition::new(left_phys as i32, 0).into(),
              size: tauri::PhysicalSize::new(right_phys, height_phys).into(),
            });
          }
          let _ = wv.hide();
        }
      }
    }

    #[cfg(target_os = "macos")]
    if group_work_behind {
      if let Some(mgr_wv) = app.get_webview("manager") {
        platform::raise_macos_webview_to_front(&mgr_wv);
      }
    }

    // 切号后聚焦新账号 WebView（联网恢复）：账号页在热池里隐藏期间被
    // Chromium 后台节流，断开的 SSE/心跳多半只在 window focus 时重连；
    // 切号只 show 不 focus，焦点一直留在管理页，页面收不到 focus、
    // 一直显示离线，点一下页面（获得焦点）才恢复。只在已提交槽位
    // 变化时聚焦一次，避免窗口缩放/折叠等纯布局重算抢走管理页的焦点；
    // 弹窗打开时不抢，管理页保持焦点。label 照常记入缓存，关弹窗后
    // 不再补聚焦；无已提交槽位时清空缓存，下次切号重新聚焦。
    // 放循环之后：先藏旧号再聚焦，避免藏旧号扰动焦点归属。
    match active_committed_label.as_ref() {
      Some(label) => {
        let prev = locked
          .last_focused_account_label
          .replace(Some(label.clone()));
        if prev.as_deref() != Some(label.as_str()) && !locked.modal_active {
          if let Some(acc_wv) = app.get_webview(label) {
            let _ = acc_wv.set_focus();
          }
        }
      }
      None => locked.last_focused_account_label.set(None),
    }
  }

  #[cfg(desktop)]
  pub async fn recompute_desktop_bounds(&self, app: &AppHandle) {
    let locked = self.inner.lock().await;
    Self::apply_desktop_layout_locked(app, &locked);
  }

  #[cfg(desktop)]
  #[allow(clippy::too_many_arguments)]
  pub(super) fn build_desktop_account_webview(
    self: &Arc<Self>,
    app: &AppHandle,
    key: AccountKey,
    provider_spec: ProviderSpec,
    parsed_uuid: Uuid,
    profile_dir: &std::path::Path,
    proxy_url: Option<&Url>,
    timezone: Option<&str>,
    webview_label: &str,
    view_instance_id: u64,
    allow_blank: Arc<AtomicBool>,
    alive_flag: Arc<AtomicBool>,
    load_gen: Arc<AtomicU64>,
  ) -> tauri::webview::WebviewBuilder<tauri::Wry> {
    let alive_nav = Arc::clone(&alive_flag);
    let mgr_for_nav = Arc::clone(self);
    let app_for_nav = app.clone();
    let allow_blank_nav = Arc::clone(&allow_blank);
    let spec_for_nav = provider_spec.clone();
    let key_for_nav = key.clone();

    let alive_popup = Arc::clone(&alive_flag);
    let active_vid_popup = Arc::clone(&self.active_view_id);
    let mgr_for_popup = Arc::clone(self);
    let app_for_popup = app.clone();
    let spec_for_popup = provider_spec.clone();
    let key_for_popup = key.clone();
    let popup_counter = Arc::new(AtomicU64::new(1));

    let alive_load = Arc::clone(&alive_flag);
    let load_gen_for_load = Arc::clone(&load_gen);
    let mgr_for_load = Arc::clone(self);
    let app_for_load = app.clone();
    let key_for_load = key;
    let handle_page_load = !cfg!(windows) || proxy_url.is_none();

    // 待下载队列：别名只为压住 clippy::type_complexity。
    type PendingDownloads = Arc<std::sync::Mutex<HashMap<String, VecDeque<(u64, PathBuf)>>>>;
    let pending_downloads: PendingDownloads = Arc::new(std::sync::Mutex::new(HashMap::new()));
    let mgr_for_dl = Arc::clone(self);
    let app_for_dl = app.clone();
    let pending_for_dl = Arc::clone(&pending_downloads);
    let mgr_for_popup_dl = Arc::clone(self);
    let app_for_popup_dl = app.clone();
    let pending_for_popup_dl = Arc::clone(&pending_downloads);

    #[allow(unused_mut)]
    let mut builder = tauri::webview::WebviewBuilder::new(
      webview_label,
      tauri::WebviewUrl::External("about:blank".parse().expect("valid blank url")),
    )
    .focused(false)
    .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
    .initialization_script(format!(
      "window.__maamAutoApprove = {}; window.__maamAutoApproveAllowCommands = {}; window.__maamAutoApproveOrigins = {};",
      if self.auto_approve.load(Ordering::SeqCst) {
        "true"
      } else {
        "false"
      },
      if self.auto_approve_commands.load(Ordering::SeqCst) {
        "true"
      } else {
        "false"
      },
      // auto_approve.js 的门控用：只在这些本站 origin 下运行。
      serde_json::to_string(&provider_spec.info.callback_origins)
        .unwrap_or_else(|_| "[]".to_string())
    ))
    .initialization_script(LOGIN_CAPTURE_INIT_JS)
    .initialization_script(AUTO_APPROVE_INIT_JS)
    .initialization_script(BACKGROUND_RAF_INIT_JS)
    .initialization_script(crate::site_adapters::NET_HOOK_INIT_JS);

    let timezone_js = timezone.map(platform::build_timezone_override_init_js);
    #[cfg(not(windows))]
    if let Some(ref tz_script) = timezone_js {
      builder = builder.initialization_script(tz_script);
    }
    let popup_timezone_js = timezone_js;

    let mut builder = builder
      .on_navigation(move |url| {
        if !alive_nav.load(Ordering::SeqCst) {
          return false;
        }
        let can_blank = allow_blank_nav.load(Ordering::SeqCst);
        match classify_top_level_url(&spec_for_nav, url, can_blank) {
          NavigationDecision::AllowBlank => true,
          NavigationDecision::AllowHttps => true,
          NavigationDecision::ExternalGoogleOAuth => {
            let mgr_ext = Arc::clone(&mgr_for_nav);
            let app_ext = app_for_nav.clone();
            let key_ext = key_for_nav.clone();
            tauri::async_runtime::spawn(async move {
              mgr_ext
                .notify_external_auth_policy(&app_ext, key_ext, view_instance_id)
                .await;
            });
            false
          }
          NavigationDecision::Deny => false,
        }
      })
      .on_new_window(move |url, features| {
        if !alive_popup.load(Ordering::SeqCst)
          || active_vid_popup.load(Ordering::SeqCst) != view_instance_id
        {
          return tauri::webview::NewWindowResponse::Deny;
        }
        let decision = classify_top_level_url(&spec_for_popup, &url, true);
        match decision {
          NavigationDecision::ExternalGoogleOAuth => {
            let mgr_ext = Arc::clone(&mgr_for_popup);
            let app_ext = app_for_popup.clone();
            let key_ext = key_for_popup.clone();
            tauri::async_runtime::spawn(async move {
              mgr_ext
                .notify_external_auth_policy(&app_ext, key_ext, view_instance_id)
                .await;
            });
            return tauri::webview::NewWindowResponse::Deny;
          }
          NavigationDecision::Deny => {
            return tauri::webview::NewWindowResponse::Deny;
          }
          NavigationDecision::AllowBlank | NavigationDecision::AllowHttps => {}
        }

        let popup_idx = popup_counter.fetch_add(1, Ordering::SeqCst);
        let popup_label = format!("auth-popup-{view_instance_id}-{popup_idx}");
        let popup_label_for_close = popup_label.clone();
        let popup_label_for_slot = popup_label.clone();

        let alive_popup_nav = Arc::clone(&alive_popup);
        let active_vid_popup_nav = Arc::clone(&active_vid_popup);
        let app_popup_nav = app_for_popup.clone();
        let mgr_popup_nav = Arc::clone(&mgr_for_popup);
        let spec_popup_nav = spec_for_popup.clone();
        let key_popup_nav = key_for_popup.clone();

        let popup_title = url
          .host_str()
          .map(|h| format!("登录认证 · {h}"))
          .unwrap_or_else(|| "登录认证".to_string());

        let popup_mgr_dl = Arc::clone(&mgr_for_popup_dl);
        let popup_app_dl = app_for_popup_dl.clone();
        let popup_pending_dl = Arc::clone(&pending_for_popup_dl);
        let popup_tz_js = popup_timezone_js.clone();

        let popup_builder = tauri::WebviewWindowBuilder::new(
          &app_for_popup,
          &popup_label,
          tauri::WebviewUrl::External("about:blank".parse().expect("valid blank url")),
        )
        .window_features(features)
        .initialization_script(LOGIN_CAPTURE_INIT_JS);
        let popup_builder = if let Some(ref tz_script) = popup_tz_js {
          popup_builder.initialization_script(tz_script)
        } else {
          popup_builder
        };
        let popup_builder = popup_builder
          .title(&popup_title)
          .on_download(move |_wv, event| {
            popup_mgr_dl.handle_desktop_download_event(&popup_app_dl, &popup_pending_dl, event)
          })
          .on_navigation(move |nav_url| {
            if !alive_popup_nav.load(Ordering::SeqCst)
              || active_vid_popup_nav.load(Ordering::SeqCst) != view_instance_id
            {
              if let Some(pw) = app_popup_nav.get_webview_window(&popup_label_for_close) {
                let _ = pw.close();
              }
              return false;
            }
            match classify_top_level_url(&spec_popup_nav, nav_url, true) {
              NavigationDecision::AllowBlank => true,
              NavigationDecision::AllowHttps => {
                if let Some(host) = nav_url.host_str() {
                  if let Some(pw) = app_popup_nav.get_webview_window(&popup_label_for_close) {
                    let _ = pw.set_title(&format!("登录认证 · {host}"));
                  }
                }
                true
              }
              NavigationDecision::ExternalGoogleOAuth => {
                if let Some(pw) = app_popup_nav.get_webview_window(&popup_label_for_close) {
                  let _ = pw.close();
                }
                let mgr_ext = Arc::clone(&mgr_popup_nav);
                let app_ext = app_popup_nav.clone();
                let key_ext = key_popup_nav.clone();
                tauri::async_runtime::spawn(async move {
                  mgr_ext
                    .notify_external_auth_policy(&app_ext, key_ext, view_instance_id)
                    .await;
                });
                false
              }
              NavigationDecision::Deny => false,
            }
          });

        match popup_builder.build() {
          Ok(popup_window) => {
            let mgr_slot = Arc::clone(&mgr_for_popup);
            let app_slot = app_for_popup.clone();
            let key_slot = key_for_popup.clone();
            tauri::async_runtime::spawn(async move {
              mgr_slot
                .register_desktop_popup(&app_slot, key_slot, view_instance_id, popup_label_for_slot)
                .await;
            });
            tauri::webview::NewWindowResponse::Create {
              window: popup_window,
            }
          }
          Err(_) => tauri::webview::NewWindowResponse::Deny,
        }
      })
      .on_download(move |_wv, event| {
        mgr_for_dl.handle_desktop_download_event(&app_for_dl, &pending_for_dl, event)
      })
      .on_page_load(move |_wv, payload| {
        if payload.url().as_str() == "about:blank" {
          return;
        }
        if !alive_load.load(Ordering::SeqCst) {
          return;
        }
        match payload.event() {
          tauri::webview::PageLoadEvent::Started => {
            load_gen_for_load.fetch_add(1, Ordering::SeqCst);
          }
          tauri::webview::PageLoadEvent::Finished => {
            if !handle_page_load {
              return;
            }
            let expected_gen = load_gen_for_load.fetch_add(1, Ordering::SeqCst) + 1;
            let loaded_url = payload.url().clone();
            let mgr_commit = Arc::clone(&mgr_for_load);
            let app_commit = app_for_load.clone();
            let key_commit = key_for_load.clone();
            tauri::async_runtime::spawn(async move {
              mgr_commit
                .wait_paint_and_finalize_desktop(
                  &app_commit,
                  key_commit,
                  view_instance_id,
                  loaded_url,
                  expected_gen,
                  false,
                )
                .await;
            });
          }
        }
      });

    #[cfg(target_os = "macos")]
    {
      let _ = profile_dir;
      builder = builder
        .data_store_identifier(*parsed_uuid.as_bytes())
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15");
    }

    #[cfg(not(target_os = "macos"))]
    {
      let _ = parsed_uuid;
      builder = builder.data_directory(profile_dir.to_path_buf());
    }

    if provider_spec.info.id.is_hark() {
      builder = builder.background_color(tauri::webview::Color(23, 29, 47, 255));
    } else {
      // 账号页本身是浅色站（Muse/Claude 等）：未绘区域给白底，
      // 切号瞬间不再露出 WebView2 默认黑底。Hark 站本身是深色，
      // 保持上面的深色底。
      builder = builder.background_color(tauri::webview::Color(255, 255, 255, 255));
    }

    builder
  }

  /// 桌面右侧区域物理矩形（纯函数，便于单测）：
  /// (left_phys, right_phys, height_phys)。
  #[cfg(desktop)]
  pub(super) fn desktop_right_rect(
    phys_width: u32,
    phys_height: u32,
    scale: f64,
    left_logical: f64,
  ) -> (u32, u32, u32) {
    let left_phys = ((left_logical * scale).round().max(0.0) as u32).min(phys_width);
    let right_phys = phys_width.saturating_sub(left_phys);
    (left_phys, right_phys, phys_height)
  }

  /// 隐藏子 WebView 初始尺寸（纯函数，便于单测）：
  /// 宽 = 窗口宽 - 当前真实左栏宽（下限 600），高 = 窗口高（下限 400）。
  /// 左栏宽必须取调用时真实的折叠态：折叠时 68px，展开时 380px；
  /// 写死 LEFT_PANE_WIDTH 会让折叠态下创建的 WebView 窄 312px，
  /// 切号时才 set_bounds + show，Chromium 重绘前露出黑底（切号黑闪主因）。
  #[cfg(desktop)]
  fn hidden_child_size(win_w: f64, win_h: f64, left_logical: f64) -> (f64, f64) {
    let left_w = win_w.min(left_logical);
    let right_w = (win_w - left_w).max(600.0);
    (right_w, win_h.max(400.0))
  }

  #[cfg(desktop)]
  pub(super) fn initial_hidden_child_rect(
    main_win: &tauri::Window,
    left_logical: f64,
  ) -> (tauri::LogicalPosition<f64>, tauri::LogicalSize<f64>) {
    let (win_w, win_h) = match (main_win.inner_size(), main_win.scale_factor()) {
      (Ok(phys), Ok(scale)) if scale > 0.0 => {
        ((phys.width as f64) / scale, (phys.height as f64) / scale)
      }
      _ => (1280.0, 800.0),
    };
    let (right_w, height) = Self::hidden_child_size(win_w, win_h, left_logical);
    (
      tauri::LogicalPosition::new(win_w.max(2000.0) + 1000.0, 0.0),
      tauri::LogicalSize::new(right_w, height),
    )
  }

  #[cfg(desktop)]
  pub(super) fn discard_desktop_view_locked(
    &self,
    app: &AppHandle,
    locked: &mut InnerState,
    key: &AccountKey,
    view_instance_id: u64,
    notice: String,
  ) -> bool {
    if !Self::is_view_instance_valid(locked, key, view_instance_id) {
      return false;
    }
    if let Some(entry) = locked.warm_pool.remove(key) {
      Self::close_warm_entry(app, entry);
    }
    if locked
      .active_slot
      .as_ref()
      .is_some_and(|slot| slot.view_instance_id == view_instance_id)
    {
      locked.active_slot = None;
      self.active_view_id.store(0, Ordering::SeqCst);
      locked.notice = Some(notice);
      Self::apply_desktop_layout_locked(app, locked);
    }
    true
  }

  #[cfg(desktop)]
  #[allow(clippy::too_many_arguments)]
  pub(super) async fn attach_desktop_account_webview(
    self: &Arc<Self>,
    app: &AppHandle,
    key: &AccountKey,
    parsed_uuid: Uuid,
    profile_dir: &std::path::Path,
    webview_label: &str,
    view_instance_id: u64,
    alive_flag: &Arc<AtomicBool>,
    load_gen: &Arc<AtomicU64>,
  ) -> Result<(), String> {
    let main_win = app
      .get_window("main")
      .ok_or_else(|| "主窗口不存在".to_string())?;
    // 创建时就按当前真实折叠态算宽：折叠 68px，展开 380px。
    // 写死 LEFT_PANE_WIDTH 会让折叠态下创建的 WebView 窄 312px，
    // 切号时才放大，Chromium 重绘前露出黑底。
    let left_logical = {
      let locked = self.inner.lock().await;
      if Self::is_sidebar_effectively_collapsed(&locked) {
        SERVICE_RAIL_WIDTH
      } else {
        LEFT_PANE_WIDTH
      }
    };
    let (init_pos, init_size) = Self::initial_hidden_child_rect(&main_win, left_logical);
    #[cfg(windows)]
    let retry_until = tokio::time::Instant::now() + std::time::Duration::from_secs(5);

    // Windows 上旧 WebView2 会话未退出时会重试（见下面 windows 分支）；
    // Linux/macOS 上循环体恒定 return，clippy 在当前目标上会报 never_loop。
    #[allow(clippy::never_loop)]
    loop {
      let (proxy_url, timezone, provider_spec) = {
        let locked = self.inner.lock().await;
        if !Self::is_view_instance_valid(&locked, key, view_instance_id) {
          return Ok(());
        }
        let spec =
          resolve_provider(&key.provider_id).ok_or_else(|| "未找到对应的服务配置".to_string())?;
        let account = locked
          .store
          .accounts
          .iter()
          .find(|account| account.provider_id == key.provider_id && account.uuid == key.uuid)
          .ok_or_else(|| "账号不存在，已阻止加载".to_string())?;
        (
          platform::parse_account_proxy(account.proxy_url.as_deref().unwrap_or_default())?,
          account.timezone.clone(),
          spec,
        )
      };
      let builder = self.build_desktop_account_webview(
        app,
        key.clone(),
        provider_spec,
        parsed_uuid,
        profile_dir,
        proxy_url.as_ref(),
        timezone.as_deref(),
        webview_label,
        view_instance_id,
        Arc::new(AtomicBool::new(true)),
        Arc::clone(alive_flag),
        Arc::clone(load_gen),
      );
      #[cfg(windows)]
      let builder = platform::with_windows_account_proxy(
        builder,
        proxy_url.as_ref(),
        &app.config().identifier,
      )?;
      let result = main_win.add_child(builder, init_pos, init_size);

      match result {
        Ok(webview) => {
          let keep_behind = {
            let locked = self.inner.lock().await;
            cfg!(target_os = "macos")
              && locked.active_slot.as_ref().is_none_or(|s| !s.committed)
              && locked.in_flight_group_members.contains_key(key)
          };
          if keep_behind {
            #[cfg(target_os = "macos")]
            if let Some(mgr_wv) = app.get_webview("manager") {
              platform::raise_macos_webview_to_front(&mgr_wv);
            }
          } else {
            let _ = webview.hide();
          }
          return self
            .navigate_desktop_account(app, &webview, key, view_instance_id, load_gen)
            .await;
        }
        #[cfg(windows)]
        Err(err) if platform::windows_webview_options_in_use(&err) => {
          if tokio::time::Instant::now() >= retry_until {
            return Err("该账号的旧 WebView2 会话尚未退出，请稍后重新打开账号".to_string());
          }
          tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
        Err(err) => return Err(format!("创建独立账号视图失败: {err}")),
      }
    }
  }

  #[cfg(desktop)]
  pub(super) async fn navigate_desktop_account(
    self: &Arc<Self>,
    app: &AppHandle,
    webview: &tauri::Webview,
    key: &AccountKey,
    view_instance_id: u64,
    load_gen: &Arc<AtomicU64>,
  ) -> Result<(), String> {
    let _ = load_gen;
    let (proxy_url, timezone, uuid, profile_dir, entry_url_str) = {
      let locked = self.inner.lock().await;
      if !Self::is_view_instance_valid(&locked, key, view_instance_id) {
        drop(locked);
        let _ = webview.close();
        return Ok(());
      }
      let spec =
        resolve_provider(&key.provider_id).ok_or_else(|| "未找到对应的服务配置".to_string())?;
      let account = locked
        .store
        .accounts
        .iter()
        .find(|account| account.provider_id == key.provider_id && account.uuid == key.uuid)
        .ok_or_else(|| "账号不存在，已阻止加载".to_string())?;
      let proxy_url =
        platform::parse_account_proxy(account.proxy_url.as_deref().unwrap_or_default())?;
      let timezone = account.timezone.clone();
      let uuid = Uuid::parse_str(&key.uuid).map_err(|err| format!("非法 UUID: {err}"))?;
      let profile_dir = locked.app_data_dir.join("profiles").join(&key.uuid);
      (proxy_url, timezone, uuid, profile_dir, spec.info.entry_url)
    };
    let uses_proxy = proxy_url.is_some();

    platform::verify_desktop_profile_binding(webview, uuid, &profile_dir).await?;
    #[cfg(windows)]
    platform::configure_windows_timezone(webview, timezone.as_deref()).await?;
    #[cfg(not(windows))]
    let _ = timezone;
    #[cfg(target_os = "macos")]
    platform::configure_desktop_proxy(webview, proxy_url).await?;
    #[cfg(target_os = "macos")]
    if uses_proxy {
      let mgr = Arc::clone(self);
      let app = app.clone();
      let key = key.clone();
      platform::on_macos_navigation_failed(webview, move || {
        let mgr = Arc::clone(&mgr);
        let app = app.clone();
        let key = key.clone();
        tauri::async_runtime::spawn(async move {
          mgr
            .fail_initial_proxy_load(
              &app,
              key,
              view_instance_id,
              "页面加载失败，请检查该账号的代理地址和连接",
            )
            .await;
        });
      })
      .await?;
    }
    #[cfg(windows)]
    if uses_proxy {
      let mgr = Arc::clone(self);
      let app = app.clone();
      let key = key.clone();
      let load_gen_win = Arc::clone(load_gen);
      platform::on_windows_navigation_completed(webview, move |result| {
        let mgr = Arc::clone(&mgr);
        let app = app.clone();
        let key = key.clone();
        let load_gen_cb = Arc::clone(&load_gen_win);
        tauri::async_runtime::spawn(async move {
          match result {
            Ok(url) => {
              let expected_gen = load_gen_cb.fetch_add(1, Ordering::SeqCst) + 1;
              mgr
                .wait_paint_and_finalize_desktop(
                  &app,
                  key,
                  view_instance_id,
                  url,
                  expected_gen,
                  false,
                )
                .await;
            }
            Err(()) => {
              mgr
                .fail_initial_proxy_load(
                  &app,
                  key,
                  view_instance_id,
                  "页面加载失败，请检查该账号的代理地址和连接",
                )
                .await;
            }
          }
        });
      })
      .await?;
    }

    let entry_url = entry_url_str
      .parse()
      .map_err(|err| format!("服务入口地址错误: {err}"))?;
    {
      let locked = self.inner.lock().await;
      if !Self::is_view_instance_valid(&locked, key, view_instance_id) {
        drop(locked);
        let _ = webview.close();
        return Ok(());
      }
    }
    webview
      .navigate(entry_url)
      .map_err(|err| format!("导航到服务入口失败: {err}"))?;

    if uses_proxy {
      let mgr = Arc::clone(self);
      let app = app.clone();
      let key = key.clone();
      tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        mgr
          .fail_initial_proxy_load(
            &app,
            key,
            view_instance_id,
            "页面加载超时，请检查该账号的代理地址和连接",
          )
          .await;
      });
    }
    Ok(())
  }

  #[cfg(desktop)]
  pub(super) async fn fail_initial_proxy_load(
    &self,
    app: &AppHandle,
    key: AccountKey,
    view_instance_id: u64,
    notice: &str,
  ) {
    let mut locked = self.inner.lock().await;
    if locked
      .warm_pool
      .get(&key)
      .is_some_and(|entry| entry.paint_ready || entry.navigation_reached)
    {
      return;
    }
    if self.discard_desktop_view_locked(app, &mut locked, &key, view_instance_id, notice.to_owned())
    {
      locked.prewarm_failed.insert(key);
      self.emit_snapshot(app, &Self::snapshot_from_locked(&locked));
    }
  }

  #[cfg(desktop)]
  pub(super) async fn open_desktop_session(
    self: &Arc<Self>,
    app: &AppHandle,
    account: AccountRecord,
    app_data_dir: PathBuf,
    expected_seq: u64,
    view_instance_id: u64,
    is_cold_start: bool,
  ) -> Result<ManagerSnapshot, String> {
    let parsed_uuid = Uuid::parse_str(&account.uuid).map_err(|e| format!("非法 UUID: {e}"))?;
    let profile_dir = app_data_dir.join("profiles").join(&account.uuid);

    #[cfg(windows)]
    if is_cold_start && !profile_dir.exists() {
      let mut locked = self.inner.lock().await;
      locked.notice =
        Some("上次已提交账号的 Windows WebView2 资料目录缺失，已停留在管理页".to_string());
      return Ok(Self::snapshot_from_locked(&locked));
    }
    #[cfg(not(windows))]
    let _ = is_cold_start;

    #[cfg(windows)]
    std::fs::create_dir_all(&profile_dir).map_err(|e| format!("创建资料目录失败: {e}"))?;

    let webview_label = format!("account-{view_instance_id}");
    let alive_flag = Arc::new(AtomicBool::new(true));
    let load_gen = Arc::new(AtomicU64::new(0));
    let key = AccountKey {
      provider_id: account.provider_id,
      uuid: account.uuid.clone(),
    };

    let loading_snap = {
      let mut locked = self.inner.lock().await;
      if self.current_seq.load(Ordering::SeqCst) != expected_seq {
        return Ok(Self::snapshot_from_locked(&locked));
      }
      if let Some(entry) = locked.warm_pool.remove(&key) {
        Self::close_warm_entry(app, entry);
      }
      locked.active_slot = Some(ActiveSessionSlot {
        account_key: key.clone(),
        view_instance_id,
        switch_seq: expected_seq,
        committed: false,
      });
      self
        .active_view_id
        .store(view_instance_id, Ordering::SeqCst);
      locked.warm_pool.insert(
        key.clone(),
        WarmDesktopEntry {
          account_key: key.clone(),
          view_instance_id,
          webview_label: webview_label.clone(),
          popup_label: None,
          paint_ready: false,
          staged_painted: false,
          navigation_reached: false,
          load_gen: Arc::clone(&load_gen),
          last_url: None,
          captured_identity_hint: None,
          watcher_running: false,
          probe_notify: Arc::new(tokio::sync::Notify::new()),
          last_used_ms: now_ms(),
          alive_flag: Arc::clone(&alive_flag),
        },
      );
      Self::evict_warm_pool_if_needed(app, &mut locked, Some(&key), MAX_WARM_DESKTOP_VIEWS);
      Self::apply_desktop_layout_locked(app, &locked);
      Self::snapshot_from_locked(&locked)
    };
    self.emit_snapshot(app, &loading_snap);

    let result = self
      .attach_desktop_account_webview(
        app,
        &key,
        parsed_uuid,
        &profile_dir,
        &webview_label,
        view_instance_id,
        &alive_flag,
        &load_gen,
      )
      .await;
    if let Err(err) = result {
      let mut locked = self.inner.lock().await;
      self.discard_desktop_view_locked(app, &mut locked, &key, view_instance_id, err);
      return Ok(Self::snapshot_from_locked(&locked));
    }

    {
      let locked = self.inner.lock().await;
      if locked.active_slot.as_ref().is_some_and(|slot| {
        slot.account_key == key && slot.view_instance_id == view_instance_id && !slot.committed
      }) {
        Self::apply_desktop_layout_locked(app, &locked);
      }
    }

    Ok(self.get_snapshot().await)
  }

  #[cfg(desktop)]
  pub(super) async fn eval_paint_ready(webview: &tauri::Webview) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx_cell = Arc::new(std::sync::Mutex::new(Some(tx)));
    let tx_inner = Arc::clone(&tx_cell);
    if webview
      .eval_with_callback(DOM_PAINT_READY_JS, move |raw| {
        if let Some(sender) = tx_inner.lock().unwrap_or_else(|e| e.into_inner()).take() {
          let _ = sender.send(raw);
        }
      })
      .is_err()
    {
      return false;
    }

    match tokio::time::timeout(std::time::Duration::from_millis(600), rx).await {
      Ok(Ok(raw)) => raw.trim() == "true",
      _ => false,
    }
  }

  #[cfg(desktop)]
  pub(super) async fn wait_paint_and_finalize_desktop(
    self: &Arc<Self>,
    app: &AppHandle,
    key: AccountKey,
    view_instance_id: u64,
    loaded_url: Url,
    expected_load_gen: u64,
    reset_paint_state: bool,
  ) {
    let wait_params = {
      let mut locked = self.inner.lock().await;
      let Some(spec) = resolve_provider(&key.provider_id) else {
        return;
      };
      if classify_top_level_url(&spec, &loaded_url, false) != NavigationDecision::AllowHttps {
        return;
      }
      locked.warm_pool.get_mut(&key).and_then(|e| {
        if e.view_instance_id == view_instance_id
          && e.load_gen.load(Ordering::SeqCst) == expected_load_gen
        {
          e.navigation_reached = true;
          Some((
            e.webview_label.clone(),
            e.paint_ready,
            Arc::clone(&e.alive_flag),
            Arc::clone(&e.load_gen),
          ))
        } else {
          None
        }
      })
    };

    let Some((wv_label, already_ready, alive_flag, load_gen)) = wait_params else {
      return;
    };

    if !already_ready || reset_paint_state {
      if reset_paint_state {
        if let Some(wv) = app.get_webview(&wv_label) {
          let _ = wv.eval("window.__maamReadySince = 0; window.__maamReadyFrames = 0;");
        }
      }
      for tick in 0..100 {
        if !alive_flag.load(Ordering::SeqCst)
          || load_gen.load(Ordering::SeqCst) != expected_load_gen
        {
          return;
        }
        let Some(wv) = app.get_webview(&wv_label) else {
          return;
        };
        #[cfg(target_os = "macos")]
        if platform::is_macos_webview_loading(&wv).await {
          tokio::time::sleep(std::time::Duration::from_millis(if tick < 20 {
            80
          } else {
            150
          }))
          .await;
          continue;
        }
        if Self::eval_paint_ready(&wv).await {
          tokio::time::sleep(std::time::Duration::from_millis(60)).await;
          if !alive_flag.load(Ordering::SeqCst)
            || load_gen.load(Ordering::SeqCst) != expected_load_gen
          {
            return;
          }
          #[cfg(target_os = "macos")]
          if platform::is_macos_webview_loading(&wv).await {
            continue;
          }
          break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(if tick < 20 {
          80
        } else {
          150
        }))
        .await;
      }
    }

    let check_params = {
      let locked = self.inner.lock().await;
      locked.warm_pool.get(&key).and_then(|e| {
        if e.view_instance_id == view_instance_id {
          Some((
            e.webview_label.clone(),
            locked.app_data_dir.join("profiles").join(&key.uuid),
          ))
        } else {
          None
        }
      })
    };

    let Some((wv_label, expected_dir)) = check_params else {
      return;
    };
    let Ok(parsed_uuid) = Uuid::parse_str(&key.uuid) else {
      return;
    };
    let Some(wv) = app.get_webview(&wv_label) else {
      return;
    };

    if let Err(err) =
      platform::verify_desktop_profile_binding(&wv, parsed_uuid, &expected_dir).await
    {
      let snap = {
        let mut locked = self.inner.lock().await;
        if !self.discard_desktop_view_locked(app, &mut locked, &key, view_instance_id, err) {
          return;
        }
        Self::snapshot_from_locked(&locked)
      };
      self.emit_snapshot(app, &snap);
      return;
    }

    // 1A：前台提交的写盘不再在锁内同步做。锁内只记 last_committed 并
    // 备好提交所需数据，释放锁后 persist_now，成功再重锁提交、失败
    // 再重锁回退。persist 期间用户可能已切走，故重锁后先校验 slot
    // 仍是本轮等待的前台位才动它。
    enum ForegroundCommit {
      // Done variant 含整份 ManagerSnapshot，体积远大于 NeedsPersist；
      // 装箱避免 clippy::large_enum_variant（快照本身已是堆上数据为主，
      // 这里多一层指针无实际开销）。
      Done(
        Box<(
          Option<ManagerSnapshot>,
          bool,
          Option<Arc<tokio::sync::Notify>>,
        )>,
      ),
      NeedsPersist {
        prev_last: Option<AccountKey>,
        need_watcher: bool,
        probe_notify: Arc<tokio::sync::Notify>,
        title: String,
      },
    }

    let outcome = {
      let mut locked = self.inner.lock().await;
      if !locked.conflicts.is_empty() || locked.store.pending_cleanup.contains(&key) {
        if let Some(entry) = locked.warm_pool.remove(&key) {
          Self::close_warm_entry(app, entry);
        }
        return;
      }
      let Some(acc) = locked
        .store
        .accounts
        .iter()
        .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
        .cloned()
      else {
        if let Some(entry) = locked.warm_pool.remove(&key) {
          Self::close_warm_entry(app, entry);
        }
        return;
      };

      let is_foreground_waiting = locked
        .active_slot
        .as_ref()
        .map(|s| s.account_key == key && s.view_instance_id == view_instance_id && !s.committed)
        .unwrap_or(false);

      let mut need_watcher = false;
      let probe_notify;
      if let Some(entry) = locked.warm_pool.get_mut(&key) {
        if entry.view_instance_id != view_instance_id
          || entry.load_gen.load(Ordering::SeqCst) != expected_load_gen
        {
          return;
        }
        entry.paint_ready = true;
        if is_foreground_waiting {
          entry.staged_painted = true;
        }
        entry.last_url = Some(loaded_url);
        if !entry.watcher_running {
          entry.watcher_running = true;
          need_watcher = true;
        }
        probe_notify = Arc::clone(&entry.probe_notify);
      } else {
        return;
      }

      if is_foreground_waiting {
        let prev_last = locked.store.last_committed.clone();
        locked.store.last_committed = Some(key.clone());
        let short_name = resolve_provider(&key.provider_id)
          .map(|s| s.info.short_name)
          .unwrap_or_else(|| key.provider_id.as_str().to_string());
        ForegroundCommit::NeedsPersist {
          prev_last,
          need_watcher,
          probe_notify,
          title: format!("{short_name} · {}", acc.alias),
        }
      } else {
        ForegroundCommit::Done(Box::new((
          Some(Self::snapshot_from_locked(&locked)),
          need_watcher,
          Some(probe_notify),
        )))
      }
    };

    // 用户主动前台提交：调用方依赖“写盘成功才提交”，故走强同步
    // persist_now（已释放锁），失败语义按原逻辑保留（回退 + notice）。
    let (snap_opt, start_watcher, probe_notify_opt) = match outcome {
      ForegroundCommit::Done(tuple) => *tuple,
      ForegroundCommit::NeedsPersist {
        prev_last,
        need_watcher,
        probe_notify,
        title,
      } => match self.persist_now().await {
        Ok(()) => {
          let mut locked = self.inner.lock().await;
          // 只有 slot 仍是本轮等待的前台位才提交：persist 期间用户
          // 可能已切到别的账号，提交错位 slot 会 corrupt 状态。
          // watcher 与提交无关（按 entry 走），两种情况都照常返回。
          if locked.active_slot.as_ref().is_some_and(|s| {
            s.account_key == key && s.view_instance_id == view_instance_id && !s.committed
          }) {
            if let Some(slot) = &mut locked.active_slot {
              slot.committed = true;
            }
            if let Some(entry) = locked.warm_pool.get_mut(&key) {
              entry.last_used_ms = now_ms();
            }
            locked
              .last_active_per_provider
              .insert(key.provider_id.clone(), key.clone());
            if let Some(main_win) = app.get_window("main") {
              let _ = main_win.set_title(&title);
            }
            Self::apply_desktop_layout_locked(app, &locked);
          }
          (
            Some(Self::snapshot_from_locked(&locked)),
            need_watcher,
            Some(probe_notify),
          )
        }
        Err(err) => {
          let mut locked = self.inner.lock().await;
          // 只回退仍属于本轮的位：persist 期间用户已切走时不许动新
          // slot；last_committed 只有仍是本账号时才回退，避免覆盖
          // 后来者的提交。
          if locked
            .active_slot
            .as_ref()
            .is_some_and(|s| s.account_key == key && s.view_instance_id == view_instance_id)
          {
            locked.store.last_committed = prev_last;
            locked.active_slot = None;
            self.active_view_id.store(0, Ordering::SeqCst);
          } else if locked.store.last_committed.as_ref() == Some(&key) {
            locked.store.last_committed = prev_last;
          }
          locked.notice = Some(format!("提交账号记录写盘失败，已回退管理页: {err}"));
          Self::apply_desktop_layout_locked(app, &locked);
          (Some(Self::snapshot_from_locked(&locked)), false, None)
        }
      },
    };

    if let Some(probe_notify) = probe_notify_opt {
      if start_watcher {
        self.spawn_desktop_identity_watcher(app.clone(), key, view_instance_id, probe_notify);
      } else {
        probe_notify.notify_one();
      }
    }

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
    }

    let mgr_next = Arc::clone(self);
    let app_next = app.clone();
    tauri::async_runtime::spawn(async move {
      tokio::time::sleep(std::time::Duration::from_millis(350)).await;
      mgr_next.prewarm_next_cold_account(&app_next).await;
    });
  }

  #[cfg(desktop)]
  pub(super) async fn prewarm_desktop_account(
    self: &Arc<Self>,
    app: &AppHandle,
    account: AccountRecord,
  ) -> Result<(), String> {
    self
      .prewarm_desktop_account_with_limit(app, account, false)
      .await
  }

  #[cfg(desktop)]
  pub(super) async fn prewarm_desktop_account_with_limit(
    self: &Arc<Self>,
    app: &AppHandle,
    account: AccountRecord,
    allow_evict_for_group: bool,
  ) -> Result<(), String> {
    let key = AccountKey {
      provider_id: account.provider_id,
      uuid: account.uuid.clone(),
    };
    let app_data_dir = {
      let mut locked = self.inner.lock().await;
      if locked.warm_pool.contains_key(&key) {
        return Ok(());
      }
      if locked.store.pending_cleanup.contains(&key) {
        return Err("账号资料正在清理".to_string());
      }
      if allow_evict_for_group && locked.warm_pool.len() >= MAX_WARM_DESKTOP_VIEWS {
        Self::evict_warm_pool_if_needed(
          app,
          &mut locked,
          Some(&key),
          MAX_WARM_DESKTOP_VIEWS.saturating_sub(1),
        );
      }
      if locked.warm_pool.len() >= MAX_WARM_DESKTOP_VIEWS {
        return if allow_evict_for_group {
          Err("后台常驻窗口数已达上限，请减少当前群聊或活跃会话后重试".to_string())
        } else {
          Ok(())
        };
      }
      locked.app_data_dir.clone()
    };

    let parsed_uuid = Uuid::parse_str(&account.uuid).map_err(|e| format!("非法 UUID: {e}"))?;
    let profile_dir = app_data_dir.join("profiles").join(&account.uuid);

    #[cfg(windows)]
    std::fs::create_dir_all(&profile_dir).map_err(|e| format!("创建资料目录失败: {e}"))?;

    let view_instance_id = self.view_counter.fetch_add(1, Ordering::SeqCst);
    let webview_label = format!("account-{view_instance_id}");
    let alive_flag = Arc::new(AtomicBool::new(true));
    let load_gen = Arc::new(AtomicU64::new(0));

    {
      let mut locked = self.inner.lock().await;
      if locked.warm_pool.contains_key(&key)
        || locked.store.pending_cleanup.contains(&key)
        || locked.warm_pool.len() >= MAX_WARM_DESKTOP_VIEWS
        || !locked
          .store
          .accounts
          .iter()
          .any(|record| record.provider_id == key.provider_id && record.uuid == key.uuid)
      {
        return Ok(());
      }
      locked.warm_pool.insert(
        key.clone(),
        WarmDesktopEntry {
          account_key: key.clone(),
          view_instance_id,
          webview_label: webview_label.clone(),
          popup_label: None,
          paint_ready: false,
          staged_painted: false,
          navigation_reached: false,
          load_gen: Arc::clone(&load_gen),
          last_url: None,
          captured_identity_hint: None,
          watcher_running: false,
          probe_notify: Arc::new(tokio::sync::Notify::new()),
          last_used_ms: 0,
          alive_flag: Arc::clone(&alive_flag),
        },
      );
    }

    let result = self
      .attach_desktop_account_webview(
        app,
        &key,
        parsed_uuid,
        &profile_dir,
        &webview_label,
        view_instance_id,
        &alive_flag,
        &load_gen,
      )
      .await;
    if let Err(err) = result {
      let mut locked = self.inner.lock().await;
      if self.discard_desktop_view_locked(app, &mut locked, &key, view_instance_id, err.clone()) {
        return Err(err);
      }
    }

    {
      let locked = self.inner.lock().await;
      if locked.in_flight_group_members.contains_key(&key) {
        Self::apply_desktop_layout_locked(app, &locked);
      }
    }

    Ok(())
  }

  #[cfg(desktop)]
  pub fn prewarm_next_cold_account(
    self: &Arc<Self>,
    app: &AppHandle,
  ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    let mgr = Arc::clone(self);
    let app_handle = app.clone();
    Box::pin(async move {
      let next_cold = {
        let locked = mgr.inner.lock().await;
        let in_flight = locked.warm_pool.values().filter(|e| !e.paint_ready).count();
        if locked.active_group_id.is_some()
          || locked.warm_pool.len() >= MAX_WARM_DESKTOP_VIEWS
          || in_flight >= MAX_CONCURRENT_PREWARMS
        {
          None
        } else {
          let is_eligible_cold = |a: &&AccountRecord| {
            if !resolve_provider(&a.provider_id).is_some_and(|s| s.info.enabled) {
              return false;
            }
            let k = AccountKey {
              provider_id: a.provider_id.clone(),
              uuid: a.uuid.clone(),
            };
            !locked.warm_pool.contains_key(&k)
              && !locked.store.pending_cleanup.contains(&k)
              && !locked.prewarm_failed.contains(&k)
          };
          locked
            .store
            .accounts
            .iter()
            .filter(is_eligible_cold)
            .find(|a| a.provider_id == locked.selected_provider_id)
            .or_else(|| locked.store.accounts.iter().find(is_eligible_cold))
            .cloned()
        }
      };

      if let Some(account) = next_cold {
        let failed_key = AccountKey {
          provider_id: account.provider_id.clone(),
          uuid: account.uuid.clone(),
        };
        if mgr
          .prewarm_desktop_account(&app_handle, account)
          .await
          .is_err()
        {
          {
            let mut locked = mgr.inner.lock().await;
            locked.prewarm_failed.insert(failed_key);
          }
          tokio::time::sleep(std::time::Duration::from_millis(400)).await;
          mgr.prewarm_next_cold_account(&app_handle).await;
        }
      }
    })
  }

  #[cfg(not(desktop))]
  pub fn prewarm_next_cold_account(
    self: &Arc<Self>,
    _app: &AppHandle,
  ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {})
  }

  #[cfg(desktop)]
  pub(super) async fn register_desktop_popup(
    &self,
    app: &AppHandle,
    key: AccountKey,
    view_instance_id: u64,
    popup_label: String,
  ) {
    let mut locked = self.inner.lock().await;
    if self.active_view_id.load(Ordering::SeqCst) != view_instance_id {
      if let Some(pw) = app.get_webview_window(&popup_label) {
        let _ = pw.close();
      }
      return;
    }
    let Some(entry) = locked.warm_pool.get_mut(&key) else {
      if let Some(pw) = app.get_webview_window(&popup_label) {
        let _ = pw.close();
      }
      return;
    };
    if entry.view_instance_id != view_instance_id {
      if let Some(pw) = app.get_webview_window(&popup_label) {
        let _ = pw.close();
      }
      return;
    }
    if let Some(prev_popup) = entry.popup_label.replace(popup_label) {
      if let Some(pw) = app.get_webview_window(&prev_popup) {
        let _ = pw.close();
      }
    }
  }

  pub(super) fn is_view_instance_valid(
    locked: &InnerState,
    key: &AccountKey,
    view_instance_id: u64,
  ) -> bool {
    #[cfg(desktop)]
    {
      locked
        .warm_pool
        .get(key)
        .map(|e| e.view_instance_id == view_instance_id)
        .unwrap_or(false)
    }
    #[cfg(not(desktop))]
    {
      locked
        .active_slot
        .as_ref()
        .map(|s| s.account_key == *key && s.view_instance_id == view_instance_id)
        .unwrap_or(false)
    }
  }
}

#[cfg(test)]
#[cfg(desktop)]
mod tests {
  use super::*;

  #[test]
  fn desktop_right_rect_expanded_collapsed_and_narrow() {
    // 1920x1200 逻辑 @125%（2400x1500 物理）：展开左栏 380 -> 475，
    // 折叠 68 -> 85；高度取物理高。
    assert_eq!(
      SessionManager::desktop_right_rect(2400, 1500, 1.25, LEFT_PANE_WIDTH),
      (475, 1925, 1500)
    );
    assert_eq!(
      SessionManager::desktop_right_rect(2400, 1500, 1.25, SERVICE_RAIL_WIDTH),
      (85, 2315, 1500)
    );
    // 100% 下 1280x800：展开右宽 900。
    assert_eq!(
      SessionManager::desktop_right_rect(1280, 800, 1.0, LEFT_PANE_WIDTH),
      (380, 900, 800)
    );
    // 窗口比左栏还窄：左栏钳到窗口宽，右宽 0（不下溢）。
    assert_eq!(
      SessionManager::desktop_right_rect(300, 800, 1.0, LEFT_PANE_WIDTH),
      (300, 0, 800)
    );
  }

  #[test]
  fn hidden_child_size_uses_real_left_width() {
    // 1920x1200：展开右宽 1540；折叠右宽 1852（旧代码写死 380 时
    // 折叠态只有 1540，窄 312px——切号黑闪主因）。
    assert_eq!(
      SessionManager::hidden_child_size(1920.0, 1200.0, LEFT_PANE_WIDTH),
      (1540.0, 1200.0)
    );
    assert_eq!(
      SessionManager::hidden_child_size(1920.0, 1200.0, SERVICE_RAIL_WIDTH),
      (1852.0, 1200.0)
    );
    // 窄窗口宽下限 600、矮窗口高下限 400。
    assert_eq!(
      SessionManager::hidden_child_size(700.0, 800.0, LEFT_PANE_WIDTH),
      (600.0, 800.0)
    );
    assert_eq!(
      SessionManager::hidden_child_size(1280.0, 300.0, LEFT_PANE_WIDTH),
      (900.0, 400.0)
    );
  }
}
