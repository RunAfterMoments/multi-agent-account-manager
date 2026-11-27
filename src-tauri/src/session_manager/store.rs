//! Config store + persistence (P2-11 split).
//!
//! Moved verbatim from `session_manager.rs`: the store/persist unit —
//! `snapshot_from_locked`, `persist_locked` / `persist`, the P0-1 unblock
//! paths (`continue_with_empty_config`, `backup_candidates`,
//! `restore_config_from_backup`), snapshot emission, the debounced persist
//! writer (`request_persist` / `ensure_persist_writer` / `spawn_persist_writer`
//! / `persist_writer_loop`), and pending profile cleanup. No behavior change.
//! Methods are `pub(super)` (or `pub` where they already were) so the parent
//! module and sibling submodules can call them via `Self::` / `self.` —
//! methods belong to the type, not the file.

use super::{AccountPageError, InnerState, ManagerSnapshot, SessionManager};
use crate::platform::{self, current_platform_name};
use crate::providers::{all_provider_infos, resolve_provider, ProviderId};
use crate::storage::{
  migrate_agent_keys_to_vault, parse_store_str, save_store_atomic, AccountsStoreFile,
  ExternalAgentView,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

impl SessionManager {
  pub(super) fn snapshot_from_locked(locked: &InnerState) -> ManagerSnapshot {
    let (active_account, switching_account) = match &locked.active_slot {
      Some(slot) if slot.committed => (Some(slot.account_key.clone()), None),
      Some(slot) => (None, Some(slot.account_key.clone())),
      None => (None, None),
    };

    #[cfg(desktop)]
    let warm_accounts = locked
      .warm_pool
      .values()
      .filter(|e| e.paint_ready)
      .map(|e| e.account_key.clone())
      .collect::<Vec<_>>();

    #[cfg(not(desktop))]
    let warm_accounts = Vec::new();

    let providers = all_provider_infos();
    let account_errors = locked
      .account_page_errors
      .iter()
      .map(|(k, msg)| AccountPageError {
        provider_id: k.provider_id.clone(),
        uuid: k.uuid.clone(),
        message: msg.clone(),
      })
      .collect::<Vec<_>>();

    ManagerSnapshot {
      platform: current_platform_name(),
      providers,
      accounts: locked.store.accounts.clone(),
      selected_provider_id: locked.selected_provider_id.clone(),
      active_account,
      switching_account,
      warm_accounts,
      last_committed: locked.store.last_committed.clone(),
      pending_cleanup: locked.store.pending_cleanup.clone(),
      conflicts: locked.conflicts.clone(),
      notice: locked.notice.clone(),
      modal_active: locked.modal_active,
      effective_sidebar_collapsed: Self::is_sidebar_effectively_collapsed(locked),
      settings: locked.store.settings.clone(),
      global_shortcut_active: locked.global_shortcut_active,
      global_shortcut_error: locked.global_shortcut_error.clone(),
      group_chats: locked.store.group_chats.clone(),
      active_group_id: locked.active_group_id.clone(),
      external_agents: locked
        .store
        .external_agents
        .iter()
        .map(ExternalAgentView::from_record)
        .collect(),
      account_errors,
      persist_blocked: locked.persist_blocked,
      read_only: locked.read_only,
    }
  }

  /// 全库唯一的写盘收敛点（除加载器自身的迁移写盘外）。
  /// persist_blocked 时拒绝写盘并通过 notice 提示；写盘失败时写日志并提示。
  /// 调用方持有 `inner` 锁时用本方法；不持锁时用 `persist()`。
  /// P1-8：序列化与写盘在 `spawn_blocking` 中进行
  /// （`save_store_atomic_async`，临时文件 0600 落盘），不阻塞异步运行时；
  /// 但 `inner` 锁在写盘期间仍被持有——火后即忘的写盘请走防抖路径
  /// `request_persist()`，写线程短暂持锁克隆后释放锁再写盘。
  pub(super) async fn persist_locked(locked: &mut InnerState) -> Result<(), String> {
    if locked.persist_blocked {
      let reason = if locked.read_only {
        "配置版本高于当前版本，已设为只读，写盘已被阻止；请升级应用，或选择「以空配置继续」"
      } else {
        "配置加载失败，写盘已被阻止，请先选择「以空配置继续」或「从备份恢复」"
      };
      eprintln!("[叠号] 写盘被阻止: {reason}");
      // 启动提示已说明备份文件名与两个选择时不再覆盖，
      // 避免丢掉备份信息。
      if !locked
        .notice
        .as_deref()
        .is_some_and(|n| n.contains("写盘已被阻止"))
      {
        locked.notice = Some(reason.to_string());
      }
      return Err(reason.to_string());
    }
    let path = locked.store_path.clone();
    let store = locked.store.clone();
    match crate::storage::save_store_atomic_async(path, store).await {
      Ok(()) => Ok(()),
      Err(err) => {
        eprintln!("[叠号] 保存配置失败: {err}");
        locked.notice = Some(format!("保存配置失败: {err}"));
        Err(err)
      }
    }
  }

  /// 不持锁时的写盘入口，内部加锁后走 `persist_locked`。
  /// 库内调用方持锁时直接走 `persist_locked`；
  /// 本方法是对外暴露的唯一写盘入口，
  /// 供测试与后续不持锁的调用方使用。
  #[allow(dead_code)]
  pub async fn persist(&self) -> Result<(), String> {
    let mut locked = self.inner.lock().await;
    Self::persist_locked(&mut locked).await
  }

  /// 用户明确选择「以空配置继续」：
  /// 丢弃内存里的空配置状态并解除写盘阻止，
  /// 把空配置写盘（损坏原文件已有 .corrupt-* 备份）。
  /// 写盘失败则恢复阻止状态。
  pub async fn continue_with_empty_config(&self) -> Result<ManagerSnapshot, String> {
    let mut locked = self.inner.lock().await;
    if !locked.persist_blocked {
      return Ok(Self::snapshot_from_locked(&locked));
    }
    let was_read_only = locked.read_only;
    locked.store = AccountsStoreFile::default();
    locked.conflicts = Vec::new();
    locked.persist_blocked = false;
    locked.read_only = false;
    locked.notice = None;
    if let Err(err) = Self::persist_locked(&mut locked).await {
      locked.persist_blocked = true;
      locked.read_only = was_read_only;
      return Err(err);
    }
    Ok(Self::snapshot_from_locked(&locked))
  }

  /// 备份候选：先是最后一份可用配置 accounts.json.bak，再是最新
  /// 的 accounts.json.corrupt-*（按时间戳倒序）。
  pub(super) fn backup_candidates(store_path: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let Some(dir) = store_path.parent() else {
      return candidates;
    };
    let Some(file_name) = store_path
      .file_name()
      .map(|n| n.to_string_lossy().into_owned())
    else {
      return candidates;
    };
    let bak = dir.join(format!("{file_name}.bak"));
    if bak.exists() {
      candidates.push(bak);
    }
    let corrupt_prefix = format!("{file_name}.corrupt-");
    let mut corrupts: Vec<PathBuf> = std::fs::read_dir(dir)
      .map(|entries| {
        entries
          .filter_map(|e| e.ok())
          .map(|e| e.path())
          .filter(|p| {
            p.file_name()
              .and_then(|n| n.to_str())
              .is_some_and(|n| n.starts_with(&corrupt_prefix))
          })
          .collect()
      })
      .unwrap_or_default();
    corrupts.sort();
    corrupts.reverse();
    candidates.extend(corrupts);
    candidates
  }

  /// 用户明确选择「从备份恢复」：按 accounts.json.bak、最新
  /// accounts.json.corrupt-* 的顺序逐个尝试，首个能成功加载的胜出。
  /// 仅在写盘被阻止时可用；无备份或全部备份都无法加载时，
  /// 保持阻止状态。
  pub async fn restore_config_from_backup(&self) -> Result<ManagerSnapshot, String> {
    let (store_path, candidates) = {
      let locked = self.inner.lock().await;
      if !locked.persist_blocked {
        return Err("当前配置正常，无需从备份恢复".to_string());
      }
      (
        locked.store_path.clone(),
        Self::backup_candidates(&locked.store_path),
      )
    };
    if candidates.is_empty() {
      return Err(
        "未找到可用的配置备份（accounts.json.bak 或 accounts.json.corrupt-*）".to_string(),
      );
    }
    // 先校验再写回：候选只在内存里解析（parse_store_str 不触盘），
    // 通过才 save_store_atomic 写回 store_path（自带 0o600 + sync_all
    // + 原子替换）。之前是复制到同目录临时文件再 load_or_migrate_store，
    // 坏候选会在临时路径旁留下 accounts.json.restore-tmp.corrupt-* / .bak。
    let mut last_err = String::new();
    for candidate in candidates {
      let raw = match std::fs::read_to_string(&candidate) {
        Ok(raw) => raw,
        Err(err) => {
          last_err = format!("读取备份失败: {err}");
          continue;
        }
      };
      let mut loaded = match parse_store_str(&raw) {
        Ok(loaded) => loaded,
        Err(err) => {
          last_err = err;
          continue;
        }
      };
      // vault 迁移/回填（与启动加载一致）：v5/v6 备份的 Key 只在 vault
      // 里，须先取回内存，否则恢复后重启前全部 401；旧备份的明文 Key
      // 须先写入 vault，否则 save_store_atomic 剥离 api_key 后永久丢失。
      // 写入失败时不写盘，换下一个候选。
      if !migrate_agent_keys_to_vault(&mut loaded.store, &self.api_key_vault).await {
        last_err = "API Key 写入系统凭据库失败，已停止恢复该备份（原备份未动）".to_string();
        continue;
      }
      if let Err(err) = save_store_atomic(&store_path, &loaded.store) {
        last_err = err;
        continue;
      }
      let mut locked = self.inner.lock().await;
      locked.store = loaded.store;
      locked.conflicts = loaded.conflicts;
      locked.persist_blocked = false;
      locked.read_only = false;
      locked.notice = Some("已从备份恢复配置".to_string());
      locked.selected_provider_id = locked
        .store
        .last_committed
        .as_ref()
        .map(|k| k.provider_id.clone())
        .filter(|pid| resolve_provider(pid).is_some_and(|spec| spec.info.enabled))
        .unwrap_or_else(ProviderId::muse);
      return Ok(Self::snapshot_from_locked(&locked));
    }
    if last_err.is_empty() {
      last_err = "备份文件均无法读取".to_string();
    }
    Err(format!("备份恢复失败: {last_err}"))
  }

  pub(super) fn emit_snapshot(&self, app: &AppHandle, snap: &ManagerSnapshot) {
    // 只发给 manager webview：完整快照含账号列表与配置，账号页
    //（外站内容）不应收到。capabilities 已限制命令来源，这是纵深防御。
    let _ = app.emit_to("manager", "manager-state-changed", snap);
  }

  pub async fn get_snapshot(&self) -> ManagerSnapshot {
    let locked = self.inner.lock().await;
    Self::snapshot_from_locked(&locked)
  }

  /// 防抖写盘请求：只把 epoch +1 并唤醒写线程，不持锁、不写盘。
  /// 原先 `let _ = save_store_atomic(...)` 的调用点改为调用它。
  pub(super) fn request_persist(&self) {
    self.persist_epoch.fetch_add(1, Ordering::SeqCst);
    self.persist_notify.notify_one();
  }

  /// 启动防抖写线程（只启动一次）。生产在 `run_startup_tasks` 中调用。
  pub fn ensure_persist_writer(self: &Arc<Self>, app: &AppHandle) {
    self.spawn_persist_writer(Some(app.clone()));
  }

  /// 测试入口：不带 `AppHandle` 启动写线程（失败时只写 notice，不发快照）。
  /// `tauri::test::mock_app` 需要给 tauri 依赖加 `test` feature 才能用，
  /// 本任务不改构建配置，故测试走这条路。
  #[cfg(test)]
  pub(super) fn ensure_persist_writer_no_app(self: &Arc<Self>) {
    self.spawn_persist_writer(None);
  }

  pub(super) fn spawn_persist_writer(self: &Arc<Self>, app: Option<AppHandle>) {
    if self.persist_writer_started.swap(true, Ordering::SeqCst) {
      return;
    }
    let mgr = Arc::clone(self);
    // 用 `tokio::task::spawn` 而不用 `tauri::async_runtime::spawn`，
    // 使写线程在 `#[tokio::test]` 下同样能启动（生产跑在 tauri 的
    // tokio 运行时上，两者等价）。
    tokio::task::spawn(async move {
      mgr.persist_writer_loop(app).await;
    });
  }

  /// 防抖写线程：等到 300ms 内无新请求后，短暂持锁克隆 store、释放锁、
  /// `spawn_blocking` 写盘；失败时写 notice 并保留 dirty，等下一次
  /// 请求重试。行为变化：防抖路径的写盘不再同步发生；进程在最后一次
  /// 修改后 300ms 内退出可能丢最后一次自动保存（用户主动操作走同步
  /// `persist_locked`，不受影响）。
  /// P0-1：写盘被阻止（配置损坏/高版本只读）时跳过本次写盘，否则
  /// 防抖写盘会把内存里的空配置覆盖到损坏/高版本文件上。
  pub(super) async fn persist_writer_loop(self: &Arc<Self>, app: Option<AppHandle>) {
    loop {
      self.persist_notify.notified().await;
      loop {
        tokio::select! {
          _ = tokio::time::sleep(std::time::Duration::from_millis(300)) => break,
          _ = self.persist_notify.notified() => {}
        }
      }
      let (path, store, blocked) = {
        let locked = self.inner.lock().await;
        (
          locked.store_path.clone(),
          locked.store.clone(),
          locked.persist_blocked,
        )
      };
      if blocked {
        continue;
      }
      match crate::storage::save_store_atomic_async(path, store).await {
        Ok(()) => {
          #[cfg(test)]
          self.persist_write_count.fetch_add(1, Ordering::SeqCst);
        }
        Err(err) => {
          let snap = {
            let mut locked = self.inner.lock().await;
            locked.notice = Some(format!("配置自动保存失败：{err}"));
            Self::snapshot_from_locked(&locked)
          };
          if let Some(app) = &app {
            self.emit_snapshot(app, &snap);
          }
        }
      }
    }
  }

  pub(super) async fn retry_pending_cleanups(&self, app: &AppHandle) {
    let keys = {
      let locked = self.inner.lock().await;
      locked.store.pending_cleanup.clone()
    };
    if keys.is_empty() {
      return;
    }

    for key in keys {
      let deleted = self.delete_profile_for_key(app, &key.uuid).await;
      if deleted.is_ok() {
        let mut locked = self.inner.lock().await;
        locked.store.pending_cleanup.retain(|k| k != &key);
        self.request_persist();
      }
    }
  }

  pub(super) async fn delete_profile_for_key(
    &self,
    app: &AppHandle,
    uuid_str: &str,
  ) -> Result<(), String> {
    #[cfg(desktop)]
    {
      let app_data_dir = {
        let locked = self.inner.lock().await;
        locked.app_data_dir.clone()
      };
      platform::delete_desktop_profile(app, &app_data_dir, uuid_str).await
    }
    #[cfg(mobile)]
    {
      crate::platform::mobile::delete_profile(app, uuid_str).await
    }
    #[cfg(all(not(desktop), not(mobile)))]
    {
      let _ = (app, uuid_str);
      Ok(())
    }
  }
}
