//! Group-chat engine (P2-11 split).
//!
//! Moved verbatim from `session_manager.rs`: group CRUD, member dispatch and
//! auto-routing, the member-turn pipeline (`run_group_member_turn`,
//! `run_group_member_turn_with_failover`, `execute_external_member_turn`,
//! `execute_webview_member_turn`), thread bindings, the auto-approve audit,
//! and the prompt/extract/sanitize helpers. No behavior change. Methods are
//! `pub(super)` (or `pub` where they already were) so the parent module and
//! sibling submodules keep calling them via `Self::` / `self.` — methods
//! belong to the type, not the file. Calls into the parent's webview
//! lifecycle (`apply_desktop_layout_locked`, `close_warm_entry`,
//! `hide_foreground_slot`) and the A2A module resolve through the type.

#[cfg(desktop)]
use super::{
  build_group_attach_file_js, build_group_check_attached_js, build_group_poll_js,
  build_group_send_js, build_group_sync_latest_js, build_hark_click_preview_card_js,
  build_hark_close_expanded_js, build_hark_read_expanded_js, build_new_chat_click_js,
  GroupAttachment,
};
use super::{
  evaluate_account_quota, AccountQuotaEval, InnerState, ManagerSnapshot, SessionManager, APP_TITLE,
  AUTO_ROUTE_COOLDOWN_MS, MAX_GROUP_MEMBERS,
};
use crate::group_transport;
#[cfg(desktop)]
use crate::group_transport::ResolvedMemberTransport;
#[cfg(target_os = "macos")]
use crate::platform;
use crate::providers::{
  classify_top_level_url, is_verified_callback_origin, resolve_provider, NavigationDecision,
  ProviderId,
};
#[cfg(desktop)]
use crate::site_adapters::SiteAdapterConfig;
use crate::site_adapters::{self, SiteAdaptersSnapshot};
#[cfg(desktop)]
use crate::storage::GroupChatMessage;
#[cfg(any(desktop, test))]
use crate::storage::GroupThreadBinding;
use crate::storage::{
  normalize_uuid, now_ms, save_store_atomic, AccountKey, AccountRecord, AuthState,
  ExternalAgentKind, ExternalAgentRecord, GroupChatRecord, GroupMemberRole, GroupReplyStatus,
  GroupSharedSkill, GroupWorkflowMode,
};
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::AppHandle;
#[cfg(desktop)]
use tauri::Manager;
use url::Url;
use uuid::Uuid;

impl SessionManager {
  #[cfg(any(desktop, test))]
  pub(super) fn resolve_member_alias_locked(
    locked: &InnerState,
    key: &AccountKey,
  ) -> Option<String> {
    if key.provider_id.is_external() {
      locked
        .store
        .external_agents
        .iter()
        .find(|e| e.account_key() == *key)
        .map(|e| e.name.clone())
    } else {
      locked
        .store
        .accounts
        .iter()
        .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
        .map(|a| a.alias.clone())
    }
  }

  pub(super) fn is_account_group_eligible(acc: &AccountRecord, now: u64) -> bool {
    resolve_provider(&acc.provider_id).is_some_and(|s| s.info.enabled)
      && acc.auth_state == AuthState::Authenticated
      && acc.cookie_expires_at_ms.is_none_or(|exp| exp > now)
  }

  pub(super) fn normalize_group_members(
    locked: &InnerState,
    raw_members: Vec<AccountKey>,
    allowed_existing: Option<&[AccountKey]>,
  ) -> Result<Vec<AccountKey>, String> {
    let now = now_ms();
    let mut members = Vec::new();
    for raw in raw_members {
      let Some(uuid) = normalize_uuid(&raw.uuid) else {
        continue;
      };
      let key = AccountKey {
        provider_id: raw.provider_id,
        uuid,
      };
      if members.contains(&key) {
        continue;
      }
      if key.provider_id.is_external() {
        if !locked
          .store
          .external_agents
          .iter()
          .any(|e| e.account_key() == key)
        {
          return Err("所选 API/CLI 成员不存在或已被移除".to_string());
        }
        members.push(key);
        continue;
      }
      let Some(acc) = locked
        .store
        .accounts
        .iter()
        .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
      else {
        return Err("所选账号不存在或已被移除".to_string());
      };
      if acc.auth_state == AuthState::LoggedOut {
        return Err(format!("账号「{}」已退出登录，不能加入群聊", acc.alias));
      }
      let already_member = allowed_existing.is_some_and(|ex| ex.contains(&key));
      if !already_member && !Self::is_account_group_eligible(acc, now) {
        return Err(format!(
          "账号「{}」尚未完成有效登录（或登录态已过期）",
          acc.alias
        ));
      }
      members.push(key);
    }

    if members.is_empty() {
      return Err("请至少选择 1 个可用成员".to_string());
    }
    if members.len() > MAX_GROUP_MEMBERS {
      return Err(format!("单个群聊最多支持 {} 个成员", MAX_GROUP_MEMBERS));
    }
    Ok(members)
  }

  pub(super) fn set_group_window_title(app: &AppHandle, group_name: Option<&str>) {
    #[cfg(desktop)]
    if let Some(main_win) = app.get_window("main") {
      let title = match group_name {
        Some(name) if !name.trim().is_empty() => format!("{APP_TITLE} · 群聊 · {}", name.trim()),
        _ => format!("{APP_TITLE} · 群聊"),
      };
      let _ = main_win.set_title(&title);
    }
    #[cfg(not(desktop))]
    let _ = (app, group_name);
  }

  pub(super) fn starts_with_ignore_case(haystack: &str, needle: &str) -> Option<usize> {
    let n_chars: Vec<char> = needle.chars().flat_map(|c| c.to_lowercase()).collect();
    if n_chars.is_empty() {
      return None;
    }
    let mut matched_idx = 0usize;
    for (byte_idx, ch) in haystack.char_indices() {
      for lc in ch.to_lowercase() {
        if matched_idx >= n_chars.len() || lc != n_chars[matched_idx] {
          return None;
        }
        matched_idx += 1;
      }
      if matched_idx == n_chars.len() {
        return Some(byte_idx + ch.len_utf8());
      }
    }
    None
  }

  #[cfg_attr(not(desktop), allow(dead_code))]
  pub(super) fn parse_at_mentions(
    content: &str,
    members: &[(AccountKey, String)],
  ) -> (bool, Vec<AccountKey>) {
    let mut sorted_aliases: Vec<(&AccountKey, &str)> = members
      .iter()
      .filter_map(|(k, alias)| {
        let trimmed = alias.trim();
        if trimmed.is_empty() {
          None
        } else {
          Some((k, trimmed))
        }
      })
      .collect();
    sorted_aliases.sort_by_key(|(_, alias)| std::cmp::Reverse(alias.chars().count()));

    let mut mentioned = Vec::new();
    for (idx, ch) in content.char_indices() {
      if ch != '@' {
        continue;
      }
      let after = &content[idx + 1..];
      // 邮箱抑制优先：@ 前是字母数字说明它嵌在邮箱地址里
      //（如 a.b@all.com、x@全体成员），不是 @指令；广播关键词与成员
      // 别名都受此保护。真正的 @all / @全体成员是独立词元（@ 前为
      // 开头、空白或标点），不受影响。
      if idx > 0
        && content[..idx]
          .chars()
          .next_back()
          .is_some_and(|prev| prev.is_ascii_alphanumeric())
      {
        continue;
      }
      if after.starts_with("全体成员") || after.starts_with("全体") || after.starts_with("所有人")
      {
        return (true, Vec::new());
      }
      if let Some(end) = Self::starts_with_ignore_case(after, "all") {
        if !after[end..]
          .chars()
          .next()
          .is_some_and(|next| next.is_ascii_alphanumeric() || next == '_' || next == '-')
        {
          return (true, Vec::new());
        }
      }
      for (key, alias) in &sorted_aliases {
        if let Some(end) = Self::starts_with_ignore_case(after, alias) {
          let needs_ascii_boundary = alias
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric());
          if needs_ascii_boundary
            && after[end..]
              .chars()
              .next()
              .is_some_and(|next| next.is_ascii_alphanumeric())
          {
            continue;
          }
          if !mentioned.contains(*key) {
            mentioned.push((*key).clone());
          }
          break;
        }
      }
    }
    (false, mentioned)
  }

  #[cfg_attr(not(desktop), allow(dead_code))]
  pub(super) fn strip_routing_mentions(content: &str, members: &[(AccountKey, String)]) -> String {
    let mut sorted_aliases: Vec<&str> = members
      .iter()
      .map(|(_, alias)| alias.trim())
      .filter(|a| !a.is_empty())
      .collect();
    sorted_aliases.sort_by_key(|alias| std::cmp::Reverse(alias.chars().count()));

    let mut out = String::with_capacity(content.len());
    let mut cursor = 0usize;
    while cursor < content.len() {
      let rest = &content[cursor..];
      let Some(ch) = rest.chars().next() else {
        break;
      };
      if ch != '@' {
        out.push(ch);
        cursor += ch.len_utf8();
        continue;
      }
      let preceded_by_alnum = cursor > 0
        && content[..cursor]
          .chars()
          .next_back()
          .is_some_and(|prev| prev.is_ascii_alphanumeric());
      if preceded_by_alnum {
        out.push(ch);
        cursor += ch.len_utf8();
        continue;
      }

      let after = &content[cursor + 1..];
      let mut matched_len: Option<usize> = None;
      for kw in ["全体成员", "所有人", "全体"] {
        if after.starts_with(kw) {
          matched_len = Some(kw.len());
          break;
        }
      }
      if matched_len.is_none() {
        if let Some(end) = Self::starts_with_ignore_case(after, "all") {
          if !after[end..]
            .chars()
            .next()
            .is_some_and(|next| next.is_ascii_alphanumeric() || next == '_' || next == '-')
          {
            matched_len = Some(end);
          }
        }
      }
      if matched_len.is_none() {
        for alias in &sorted_aliases {
          if let Some(end) = Self::starts_with_ignore_case(after, alias) {
            let needs_ascii_boundary = alias
              .chars()
              .last()
              .is_some_and(|c| c.is_ascii_alphanumeric());
            if needs_ascii_boundary
              && after[end..]
                .chars()
                .next()
                .is_some_and(|next| next.is_ascii_alphanumeric())
            {
              continue;
            }
            matched_len = Some(end);
            break;
          }
        }
      }

      if let Some(m_len) = matched_len {
        cursor += 1 + m_len;
        while let Some(sep) = content[cursor..].chars().next() {
          if sep == ' ' || sep == '\t' || sep == '、' {
            cursor += sep.len_utf8();
          } else {
            break;
          }
        }
      } else {
        out.push(ch);
        cursor += ch.len_utf8();
      }
    }

    let cleaned = out
      .trim_start_matches(|c: char| {
        c.is_whitespace() || matches!(c, '，' | ',' | '、' | '：' | ':')
      })
      .trim()
      .to_string();
    if cleaned.is_empty() {
      content.trim().to_string()
    } else {
      cleaned
    }
  }

  #[cfg(desktop)]
  pub(super) fn compute_group_knowledge_fingerprint(
    group: &GroupChatRecord,
    member: &AccountKey,
  ) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    group.shared_context.hash(&mut hasher);
    for skill in &group.shared_skills {
      if skill.enabled {
        skill.id.hash(&mut hasher);
        skill.name.hash(&mut hasher);
        skill.content.hash(&mut hasher);
      }
    }
    if let Some(role_entry) = group.member_roles.iter().find(|r| &r.member == member) {
      role_entry.role.hash(&mut hasher);
    }
    hasher.finish()
  }

  pub async fn select_group_chat(
    self: &Arc<Self>,
    app: &AppHandle,
    group_id: Option<String>,
  ) -> Result<ManagerSnapshot, String> {
    let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;
    let (snap, group_name) = {
      let mut locked = self.inner.lock().await;
      let cleaned_id = group_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
      let resolved = match cleaned_id {
        Some(ref gid) => {
          let found = locked
            .store
            .group_chats
            .iter()
            .find(|g| &g.id == gid)
            .cloned()
            .ok_or_else(|| "未找到对应的群聊".to_string())?;
          Some(found)
        }
        None => locked.store.group_chats.first().cloned(),
      };

      self.hide_foreground_slot(app, next_seq, &mut locked).await;
      locked.notice = None;
      locked.active_group_id = resolved.as_ref().map(|g| g.id.clone());
      #[cfg(desktop)]
      Self::apply_desktop_layout_locked(app, &locked);
      (
        Self::snapshot_from_locked(&locked),
        resolved.map(|g| g.name),
      )
    };

    Self::set_group_window_title(app, group_name.as_deref());
    Ok(snap)
  }

  pub async fn create_group_chat(
    self: &Arc<Self>,
    app: &AppHandle,
    name: String,
    members: Vec<AccountKey>,
  ) -> Result<ManagerSnapshot, String> {
    let clean_name = name.trim();
    if clean_name.is_empty() {
      return Err("群聊名称不能为空".to_string());
    }
    if clean_name.chars().count() > 32 {
      return Err("群聊名称不能超过 32 个字".to_string());
    }

    let next_seq = self.current_seq.fetch_add(1, Ordering::SeqCst) + 1;
    let (snap, final_name) = {
      let mut locked = self.inner.lock().await;
      let valid_members = Self::normalize_group_members(&locked, members, None)?;
      let record = GroupChatRecord {
        id: Uuid::new_v4().hyphenated().to_string(),
        name: clean_name.to_string(),
        members: valid_members,
        created_at_ms: now_ms(),
        messages: Vec::new(),
        shared_context: String::new(),
        shared_skills: Vec::new(),
        member_roles: Vec::new(),
        workflow_mode: GroupWorkflowMode::Parallel,
        max_relay_hops: 2,
        thread_bindings: Vec::new(),
      };
      let new_id = record.id.clone();
      locked.store.group_chats.push(record);
      if let Err(err) = Self::persist_locked(&mut locked).await {
        locked.store.group_chats.pop();
        return Err(err);
      }

      self.hide_foreground_slot(app, next_seq, &mut locked).await;
      locked.active_group_id = Some(new_id);
      locked.notice = None;
      #[cfg(desktop)]
      Self::apply_desktop_layout_locked(app, &locked);
      (Self::snapshot_from_locked(&locked), clean_name.to_string())
    };

    Self::set_group_window_title(app, Some(&final_name));
    Ok(snap)
  }

  pub async fn update_group_chat(
    self: &Arc<Self>,
    app: &AppHandle,
    group_id: String,
    name: String,
    members: Vec<AccountKey>,
  ) -> Result<ManagerSnapshot, String> {
    let clean_name = name.trim();
    if clean_name.is_empty() {
      return Err("群聊名称不能为空".to_string());
    }
    if clean_name.chars().count() > 32 {
      return Err("群聊名称不能超过 32 个字".to_string());
    }

    let (snap, active_name) = {
      let mut locked = self.inner.lock().await;
      let idx = locked
        .store
        .group_chats
        .iter()
        .position(|g| g.id == group_id)
        .ok_or_else(|| "未找到对应的群聊".to_string())?;
      let existing_members = locked.store.group_chats[idx].members.clone();
      let valid_members = Self::normalize_group_members(&locked, members, Some(&existing_members))?;

      let prev_name = locked.store.group_chats[idx].name.clone();
      let prev_roles = locked.store.group_chats[idx].member_roles.clone();
      let prev_bindings = locked.store.group_chats[idx].thread_bindings.clone();
      locked.store.group_chats[idx].name = clean_name.to_string();
      locked.store.group_chats[idx].members = valid_members.clone();
      locked.store.group_chats[idx]
        .member_roles
        .retain(|mr| valid_members.contains(&mr.member));
      locked.store.group_chats[idx]
        .thread_bindings
        .retain(|tb| valid_members.contains(&tb.member));
      if let Err(err) = Self::persist_locked(&mut locked).await {
        locked.store.group_chats[idx].name = prev_name;
        locked.store.group_chats[idx].members = existing_members;
        locked.store.group_chats[idx].member_roles = prev_roles;
        locked.store.group_chats[idx].thread_bindings = prev_bindings;
        return Err(err);
      }

      #[cfg(desktop)]
      locked
        .group_context_synced
        .retain(|(gid, k), _| gid != &group_id || valid_members.contains(k));

      let is_active = locked.active_group_id.as_deref() == Some(group_id.as_str());
      (
        Self::snapshot_from_locked(&locked),
        if is_active {
          Some(clean_name.to_string())
        } else {
          None
        },
      )
    };

    if let Some(active_name) = active_name {
      Self::set_group_window_title(app, Some(&active_name));
    }
    Ok(snap)
  }

  pub async fn update_group_knowledge(
    &self,
    group_id: String,
    shared_context: String,
    shared_skills: Vec<GroupSharedSkill>,
    member_roles: Vec<GroupMemberRole>,
  ) -> Result<ManagerSnapshot, String> {
    let clean_context: String = shared_context.trim().chars().take(8000).collect();

    let mut clean_skills = Vec::new();
    let mut seen_ids = HashSet::new();
    for sk in shared_skills {
      let name: String = sk.name.trim().chars().take(40).collect();
      let content: String = sk.content.trim().chars().take(2000).collect();
      if name.is_empty() || content.is_empty() {
        continue;
      }
      if clean_skills.len() >= 30 {
        return Err("单个群聊最多支持 30 个共享技能".to_string());
      }
      let id = if sk.id.trim().is_empty() || seen_ids.contains(sk.id.trim()) {
        Uuid::new_v4().hyphenated().to_string()
      } else {
        sk.id.trim().to_string()
      };
      seen_ids.insert(id.clone());
      clean_skills.push(GroupSharedSkill {
        id,
        name,
        description: sk.description.trim().chars().take(120).collect(),
        content,
        enabled: sk.enabled,
      });
    }

    let mut locked = self.inner.lock().await;
    let idx = locked
      .store
      .group_chats
      .iter()
      .position(|g| g.id == group_id)
      .ok_or_else(|| "未找到对应的群聊".to_string())?;
    let members = locked.store.group_chats[idx].members.clone();

    let mut clean_roles: Vec<GroupMemberRole> = Vec::new();
    for mr in member_roles {
      let Some(uuid) = normalize_uuid(&mr.member.uuid) else {
        continue;
      };
      let key = AccountKey {
        provider_id: mr.member.provider_id,
        uuid,
      };
      let role_str: String = mr.role.trim().chars().take(120).collect();
      if role_str.is_empty()
        || !members.contains(&key)
        || clean_roles.iter().any(|existing| existing.member == key)
      {
        continue;
      }
      clean_roles.push(GroupMemberRole {
        member: key,
        role: role_str,
      });
    }

    let prev_ctx = locked.store.group_chats[idx].shared_context.clone();
    let prev_skills = locked.store.group_chats[idx].shared_skills.clone();
    let prev_roles = locked.store.group_chats[idx].member_roles.clone();
    locked.store.group_chats[idx].shared_context = clean_context;
    locked.store.group_chats[idx].shared_skills = clean_skills;
    locked.store.group_chats[idx].member_roles = clean_roles;
    if let Err(err) = Self::persist_locked(&mut locked).await {
      locked.store.group_chats[idx].shared_context = prev_ctx;
      locked.store.group_chats[idx].shared_skills = prev_skills;
      locked.store.group_chats[idx].member_roles = prev_roles;
      return Err(err);
    }

    #[cfg(desktop)]
    locked
      .group_context_synced
      .retain(|(gid, _), _| gid != &group_id);

    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn append_group_shared_context(
    &self,
    group_id: String,
    snippet: String,
  ) -> Result<ManagerSnapshot, String> {
    let clean_snippet = snippet.trim();
    if clean_snippet.is_empty() {
      return Err("沉淀内容不能为空".to_string());
    }

    let mut locked = self.inner.lock().await;
    let idx = locked
      .store
      .group_chats
      .iter()
      .position(|g| g.id == group_id)
      .ok_or_else(|| "未找到对应的群聊".to_string())?;

    let prev = locked.store.group_chats[idx].shared_context.clone();
    let combined = if prev.trim().is_empty() {
      clean_snippet.to_string()
    } else {
      format!("{}\n\n{}", prev.trim(), clean_snippet)
    };
    let final_ctx: String = if combined.chars().count() > 8000 {
      let skip = combined.chars().count() - 8000;
      combined.chars().skip(skip).collect()
    } else {
      combined
    };

    locked.store.group_chats[idx].shared_context = final_ctx;
    if let Err(err) = Self::persist_locked(&mut locked).await {
      locked.store.group_chats[idx].shared_context = prev;
      return Err(err);
    }

    #[cfg(desktop)]
    locked
      .group_context_synced
      .retain(|(gid, _), _| gid != &group_id);

    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn delete_group_chat(
    &self,
    app: &AppHandle,
    group_id: String,
  ) -> Result<ManagerSnapshot, String> {
    let (snap, next_title, is_group_view) = {
      let mut locked = self.inner.lock().await;
      let idx = locked
        .store
        .group_chats
        .iter()
        .position(|g| g.id == group_id)
        .ok_or_else(|| "未找到对应的群聊".to_string())?;
      let removed = locked.store.group_chats.remove(idx);
      if let Err(err) = Self::persist_locked(&mut locked).await {
        locked.store.group_chats.insert(idx, removed);
        return Err(err);
      }
      #[cfg(desktop)]
      locked
        .group_context_synced
        .retain(|(gid, _), _| gid != &group_id);
      let was_active = locked.active_group_id.as_deref() == Some(group_id.as_str());
      if was_active {
        locked.active_group_id = locked.store.group_chats.first().map(|g| g.id.clone());
      }
      let next_name = locked
        .active_group_id
        .as_ref()
        .and_then(|gid| locked.store.group_chats.iter().find(|g| &g.id == gid))
        .map(|g| g.name.clone());
      (Self::snapshot_from_locked(&locked), next_name, was_active)
    };

    if is_group_view {
      Self::set_group_window_title(app, next_title.as_deref());
    }
    Ok(snap)
  }

  #[allow(clippy::too_many_arguments)]
  pub async fn upsert_external_agent(
    &self,
    id: Option<String>,
    kind: ExternalAgentKind,
    name: String,
    base_url: String,
    api_key: String,
    model: String,
    command: String,
    args: Vec<String>,
    workdir: Option<String>,
    timeout_secs: Option<u64>,
    idle_timeout_secs: Option<u64>,
  ) -> Result<ManagerSnapshot, String> {
    let mut locked = self.inner.lock().await;
    let existing_pos = id.as_deref().and_then(normalize_uuid).and_then(|norm_id| {
      locked
        .store
        .external_agents
        .iter()
        .position(|e| e.id == norm_id)
    });
    let created_at = existing_pos.map(|pos| locked.store.external_agents[pos].created_at_ms);

    let mut record = group_transport::normalize_external_agent_input(
      id.as_deref(),
      kind,
      &name,
      &base_url,
      &api_key,
      &model,
      &command,
      args,
      workdir.as_deref(),
      timeout_secs,
      idle_timeout_secs,
      created_at,
    )?;

    // P1-7 缺陷修复：先记下编辑前的旧 Key 与旧 Key 引用——写盘失败时回滚 vault；
    // kind 改为 local_cli 后，旧 vault 条目成孤儿，保存成功后清理。
    let prev_api_key: Option<String> =
      existing_pos.map(|pos| locked.store.external_agents[pos].api_key.clone());
    let prev_key_ref: Option<String> =
      existing_pos.and_then(|pos| locked.store.external_agents[pos].api_key_ref.clone());
    let record_key_ref = record.api_key_ref.clone();

    // P1-7：编辑时 Key 留空表示不修改，沿用内存里的旧 Key；
    // 新 Key 或沿用的旧 Key：落盘前必须先进 vault，否则重启后丢失。
    // vault 写入失败直接返回，不动内存与磁盘。
    if kind == ExternalAgentKind::OpenaiApi {
      if api_key.trim().is_empty() {
        if let Some(pos) = existing_pos {
          record.api_key = locked.store.external_agents[pos].api_key.clone();
        }
      }
      if !record.api_key.is_empty() {
        if let Some(key_ref) = record.api_key_ref.clone() {
          let vault = &self.api_key_vault;
          vault
            .store_key(&key_ref, &record.api_key)
            .await
            .map_err(|e| format!("保存 API Key 到系统凭据库失败：{e}"))?;
        }
      }
    };

    // P1-7 缺陷修复：本次是否真的写了 vault（写盘失败回滚时只回滚写过的）。
    let vault_written = kind == ExternalAgentKind::OpenaiApi
      && !record.api_key.is_empty()
      && record.api_key_ref.is_some();

    let prev_agents = locked.store.external_agents.clone();
    let prev_groups = locked.store.group_chats.clone();
    if let Some(pos) = existing_pos {
      let old_key = locked.store.external_agents[pos].account_key();
      let new_key = record.account_key();
      locked.store.external_agents[pos] = record;
      if old_key != new_key {
        for group in &mut locked.store.group_chats {
          for m in &mut group.members {
            if *m == old_key {
              *m = new_key.clone();
            }
          }
          for mr in &mut group.member_roles {
            if mr.member == old_key {
              mr.member = new_key.clone();
            }
          }
          for tb in &mut group.thread_bindings {
            if tb.member == old_key {
              tb.member = new_key.clone();
            }
          }
        }
      }
    } else {
      locked.store.external_agents.push(record);
    }

    if let Err(err) = Self::persist_locked(&mut locked).await {
      locked.store.external_agents = prev_agents;
      locked.store.group_chats = prev_groups;
      // P1-7 缺陷修复：写盘失败回滚 vault，否则新 Key 残留在 vault 里，
      // 与内存/磁盘的旧记录不一致，下次启动会用“编辑失败”的新 Key。
      if vault_written {
        if let Some(key_ref) = record_key_ref {
          let vault = &self.api_key_vault;
          match prev_api_key {
            Some(old) if !old.is_empty() => {
              let _ = vault.store_key(&key_ref, &old).await;
            }
            _ => {
              vault.delete_key(&key_ref).await;
            }
          }
        }
      }
      return Err(err);
    }

    // P1-7 缺陷修复：kind 改为 local_cli 后旧 vault 条目成孤儿，保存成功后清理。
    if kind != ExternalAgentKind::OpenaiApi {
      if let Some(key_ref) = prev_key_ref {
        self.api_key_vault.delete_key(&key_ref).await;
      }
    }

    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn delete_external_agent(&self, id: String) -> Result<ManagerSnapshot, String> {
    let norm_id = normalize_uuid(&id).ok_or_else(|| "外部成员 ID 格式非法".to_string())?;
    let mut locked = self.inner.lock().await;
    let pos = locked
      .store
      .external_agents
      .iter()
      .position(|e| e.id == norm_id)
      .ok_or_else(|| "未找到对应的外部成员".to_string())?;
    let removed_key = locked.store.external_agents[pos].account_key();
    let removed_key_ref = locked.store.external_agents[pos].api_key_ref.clone();
    let prev_agents = locked.store.external_agents.clone();
    let prev_groups = locked.store.group_chats.clone();

    locked.store.external_agents.remove(pos);
    for group in &mut locked.store.group_chats {
      group.members.retain(|m| m != &removed_key);
      group.member_roles.retain(|mr| mr.member != removed_key);
      group.thread_bindings.retain(|tb| tb.member != removed_key);
    }

    if let Err(err) = Self::persist_locked(&mut locked).await {
      locked.store.external_agents = prev_agents;
      locked.store.group_chats = prev_groups;
      return Err(err);
    }

    // P1-7：保存成功后同步删除 vault 里的 Key，失败忽略，不阻塞删除。
    if let Some(key_ref) = removed_key_ref {
      self.api_key_vault.delete_key(&key_ref).await;
    }

    #[cfg(desktop)]
    locked
      .group_context_synced
      .retain(|(_, k), _| k != &removed_key);

    Ok(Self::snapshot_from_locked(&locked))
  }

  /// 前端「停止」按钮：置位指定群聊消息的外部成员取消令牌。
  /// 返回 true 表示该轮确实在执行中（令牌已登记）；false 表示
  /// 该消息当前没有在跑的 API/CLI 轮次（已结束或不是外部成员）。
  #[cfg(desktop)]
  pub async fn cancel_external_member_turn(&self, group_id: String, msg_id: String) -> bool {
    let locked = self.inner.lock().await;
    match locked.external_turn_cancel.get(&(group_id, msg_id)) {
      Some(flag) => {
        flag.store(true, Ordering::SeqCst);
        true
      }
      None => false,
    }
  }

  pub async fn update_group_workflow(
    &self,
    group_id: String,
    workflow_mode: GroupWorkflowMode,
    max_relay_hops: u8,
  ) -> Result<ManagerSnapshot, String> {
    let clamped_hops = max_relay_hops.clamp(1, 3);
    let mut locked = self.inner.lock().await;
    let idx = locked
      .store
      .group_chats
      .iter()
      .position(|g| g.id == group_id)
      .ok_or_else(|| "未找到对应的群聊".to_string())?;
    let prev_mode = locked.store.group_chats[idx].workflow_mode;
    let prev_hops = locked.store.group_chats[idx].max_relay_hops;
    locked.store.group_chats[idx].workflow_mode = workflow_mode;
    locked.store.group_chats[idx].max_relay_hops = clamped_hops;

    if let Err(err) = Self::persist_locked(&mut locked).await {
      locked.store.group_chats[idx].workflow_mode = prev_mode;
      locked.store.group_chats[idx].max_relay_hops = prev_hops;
      return Err(err);
    }

    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn reset_group_thread_binding(
    &self,
    group_id: String,
    member: AccountKey,
  ) -> Result<ManagerSnapshot, String> {
    let norm_uuid = normalize_uuid(&member.uuid).ok_or_else(|| "成员标识格式非法".to_string())?;
    let target_key = AccountKey {
      provider_id: member.provider_id,
      uuid: norm_uuid,
    };
    let mut locked = self.inner.lock().await;
    let idx = locked
      .store
      .group_chats
      .iter()
      .position(|g| g.id == group_id)
      .ok_or_else(|| "未找到对应的群聊".to_string())?;
    let prev_bindings = locked.store.group_chats[idx].thread_bindings.clone();
    locked.store.group_chats[idx]
      .thread_bindings
      .retain(|tb| tb.member != target_key);
    if let Err(err) = Self::persist_locked(&mut locked).await {
      locked.store.group_chats[idx].thread_bindings = prev_bindings;
      return Err(err);
    }
    #[cfg(desktop)]
    locked.group_context_synced.remove(&(group_id, target_key));
    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn get_site_adapters(&self) -> SiteAdaptersSnapshot {
    let app_data_dir = {
      let locked = self.inner.lock().await;
      locked.app_data_dir.clone()
    };
    site_adapters::get_adapters_snapshot(&app_data_dir)
  }

  pub async fn save_site_adapter_overrides(
    &self,
    raw_json: String,
  ) -> Result<SiteAdaptersSnapshot, String> {
    let app_data_dir = {
      let locked = self.inner.lock().await;
      locked.app_data_dir.clone()
    };
    site_adapters::validate_and_save_overrides(&app_data_dir, &raw_json)
  }

  pub async fn reset_site_adapter_overrides(&self) -> Result<SiteAdaptersSnapshot, String> {
    let app_data_dir = {
      let locked = self.inner.lock().await;
      locked.app_data_dir.clone()
    };
    site_adapters::reset_overrides(&app_data_dir)
  }

  pub async fn clear_group_messages(&self, group_id: String) -> Result<ManagerSnapshot, String> {
    let mut locked = self.inner.lock().await;
    let idx = locked
      .store
      .group_chats
      .iter()
      .position(|g| g.id == group_id)
      .ok_or_else(|| "未找到对应的群聊".to_string())?;
    let prev = std::mem::take(&mut locked.store.group_chats[idx].messages);
    if let Err(err) = Self::persist_locked(&mut locked).await {
      locked.store.group_chats[idx].messages = prev;
      return Err(err);
    }
    #[cfg(desktop)]
    locked
      .group_context_synced
      .retain(|(gid, _), _| gid != &group_id);
    Ok(Self::snapshot_from_locked(&locked))
  }

  pub async fn start_group_new_topic(
    &self,
    group_id: String,
    keep_shared_context: bool,
  ) -> Result<ManagerSnapshot, String> {
    let mut locked = self.inner.lock().await;
    let idx = locked
      .store
      .group_chats
      .iter()
      .position(|g| g.id == group_id)
      .ok_or_else(|| "未找到对应的群聊".to_string())?;
    if locked.store.group_chats[idx].messages.iter().any(|m| {
      matches!(
        m.status,
        GroupReplyStatus::Queued | GroupReplyStatus::Sending | GroupReplyStatus::Streaming
      )
    }) {
      return Err("有成员正在回复，请等待完成后再开启新话题".to_string());
    }
    let prev_msgs = std::mem::take(&mut locked.store.group_chats[idx].messages);
    let prev_bindings = std::mem::take(&mut locked.store.group_chats[idx].thread_bindings);
    let prev_ctx = locked.store.group_chats[idx].shared_context.clone();
    if !keep_shared_context {
      locked.store.group_chats[idx].shared_context.clear();
    }
    if let Err(err) = Self::persist_locked(&mut locked).await {
      let g = &mut locked.store.group_chats[idx];
      g.messages = prev_msgs;
      g.thread_bindings = prev_bindings;
      g.shared_context = prev_ctx;
      return Err(err);
    }
    #[cfg(desktop)]
    locked
      .group_context_synced
      .retain(|(gid, _), _| gid != &group_id);
    locked
      .group_channel_last_account
      .retain(|(gid, _), _| gid != &group_id);
    Ok(Self::snapshot_from_locked(&locked))
  }

  #[cfg_attr(not(desktop), allow(dead_code))]
  pub(super) fn collect_group_eligible_members_locked(
    locked: &InnerState,
    g_idx: usize,
  ) -> Vec<(AccountKey, String)> {
    let mut eligible_members = Vec::new();
    for member in &locked.store.group_chats[g_idx].members {
      if member.provider_id.is_external() {
        if let Some(ext) = locked
          .store
          .external_agents
          .iter()
          .find(|e| &e.account_key() == member)
        {
          eligible_members.push((member.clone(), ext.name.clone()));
        }
      } else if let Some(acc) = locked
        .store
        .accounts
        .iter()
        .find(|a| a.provider_id == member.provider_id && a.uuid == member.uuid)
      {
        if acc.auth_state != AuthState::LoggedOut
          && resolve_provider(&acc.provider_id).is_some_and(|s| s.info.enabled)
          && !locked.account_page_errors.contains_key(member)
        {
          eligible_members.push((member.clone(), acc.alias.clone()));
        }
      }
    }
    eligible_members
  }

  #[cfg_attr(not(desktop), allow(dead_code))]
  pub(super) fn channel_display_label_locked(
    _locked: &InnerState,
    channel_pid: &ProviderId,
  ) -> String {
    if channel_pid.is_api() {
      return "OpenAI API".to_string();
    }
    if channel_pid.is_cli() {
      return "本地 CLI".to_string();
    }
    resolve_provider(channel_pid)
      .map(|s| s.info.short_name)
      .unwrap_or_else(|| channel_pid.as_str().to_string())
  }

  #[cfg_attr(not(desktop), allow(dead_code))]
  pub(super) fn select_auto_channel_account_locked(
    locked: &InnerState,
    group_id: &str,
    channel_pid: &ProviderId,
    eligible_members: &[(AccountKey, String)],
    excluded: &[AccountKey],
    now: u64,
  ) -> Result<(AccountKey, String), String> {
    let channel_label = Self::channel_display_label_locked(locked, channel_pid);
    let channel_members: Vec<(AccountKey, String)> = eligible_members
      .iter()
      .filter(|(k, _)| &k.provider_id == channel_pid && !excluded.contains(k))
      .cloned()
      .collect();

    if channel_members.is_empty() {
      return Err(format!(
        "渠道「{channel_label}」下暂无可用且未登录失效的成员"
      ));
    }

    let group_messages = locked
      .store
      .group_chats
      .iter()
      .find(|g| g.id == group_id)
      .map(|g| g.messages.as_slice())
      .unwrap_or(&[]);

    #[derive(Clone)]
    struct Candidate {
      key: AccountKey,
      alias: String,
      quota: AccountQuotaEval,
      is_busy: bool,
      in_cooldown: bool,
      last_assigned: u64,
    }

    let mut usable: Vec<Candidate> = Vec::new();
    for (key, alias) in channel_members {
      let latest_msg = group_messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant" && m.sender.as_ref() == Some(&key));
      if latest_msg.is_some_and(|m| m.status == GroupReplyStatus::NeedsAuth) {
        continue;
      }

      let quota = if key.provider_id.is_external() {
        AccountQuotaEval {
          depleted: false,
          sticky_ok: true,
          tier: 3,
          effective_used_pct: 50,
        }
      } else if let Some(acc) = locked
        .store
        .accounts
        .iter()
        .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
      {
        evaluate_account_quota(acc)
      } else {
        continue;
      };

      if quota.depleted {
        continue;
      }

      #[cfg(desktop)]
      let in_flight = locked
        .in_flight_group_members
        .get(&key)
        .copied()
        .unwrap_or(0)
        > 0;
      #[cfg(not(desktop))]
      let in_flight = false;

      let msg_busy = group_messages.iter().any(|m| {
        m.sender.as_ref() == Some(&key)
          && matches!(
            m.status,
            GroupReplyStatus::Queued | GroupReplyStatus::Sending | GroupReplyStatus::Streaming
          )
      });
      let is_busy = in_flight || msg_busy;
      let in_cooldown = locked
        .account_cooldown_until_ms
        .get(&key)
        .is_some_and(|&until| until > now);
      let last_assigned = locked
        .account_last_assigned_ms
        .get(&key)
        .copied()
        .unwrap_or(0);

      usable.push(Candidate {
        key,
        alias,
        quota,
        is_busy,
        in_cooldown,
        last_assigned,
      });
    }

    if usable.is_empty() {
      return Err(format!(
        "渠道「{channel_label}」下所有账号额度已耗尽或待授权，请切换其他渠道或处理授权"
      ));
    }

    if let Some(last_key) = locked
      .group_channel_last_account
      .get(&(group_id.to_string(), channel_pid.clone()))
    {
      if let Some(sticky) = usable.iter().find(|c| &c.key == last_key) {
        if !sticky.is_busy && !sticky.in_cooldown && sticky.quota.sticky_ok {
          return Ok((sticky.key.clone(), sticky.alias.clone()));
        }
      }
    }

    usable.sort_by_key(|c| {
      (
        c.in_cooldown,
        c.is_busy,
        c.quota.tier,
        c.quota.effective_used_pct,
        c.last_assigned,
      )
    });
    let best = &usable[0];
    Ok((best.key.clone(), best.alias.clone()))
  }

  #[cfg_attr(not(desktop), allow(dead_code))]
  pub(super) fn resolve_group_dispatch_targets_locked(
    locked: &mut InnerState,
    group_id: &str,
    trimmed_content: &str,
    eligible_members: &[(AccountKey, String)],
    target_members: Option<Vec<AccountKey>>,
    auto_channels: Option<Vec<String>>,
    now: u64,
  ) -> Result<Vec<(AccountKey, String, Option<ProviderId>)>, String> {
    let (mention_all, mentioned_keys) = Self::parse_at_mentions(trimmed_content, eligible_members);
    if mention_all {
      return Ok(
        eligible_members
          .iter()
          .map(|(k, a)| (k.clone(), a.clone(), None))
          .collect(),
      );
    }

    let requested_keys: Vec<AccountKey> = target_members
      .unwrap_or_default()
      .into_iter()
      .filter_map(|k| {
        normalize_uuid(&k.uuid).map(|uuid| AccountKey {
          provider_id: k.provider_id,
          uuid,
        })
      })
      .collect();

    let has_inline_mentions = !mentioned_keys.is_empty();
    let explicit_targets: Vec<(AccountKey, String)> = if has_inline_mentions {
      mentioned_keys
        .into_iter()
        .filter_map(|mk| eligible_members.iter().find(|(ek, _)| ek == &mk).cloned())
        .collect()
    } else if !requested_keys.is_empty() {
      let mut seen = Vec::new();
      requested_keys
        .iter()
        .filter_map(|rk| {
          if seen.contains(&rk) {
            return None;
          }
          let found = eligible_members.iter().find(|(ek, _)| ek == rk).cloned()?;
          seen.push(rk);
          Some(found)
        })
        .collect()
    } else {
      Vec::new()
    };

    if has_inline_mentions && explicit_targets.is_empty() {
      return Err("指定的 @成员当前未登录或不可用，请重新选择".to_string());
    }

    let mut parsed_channels: Vec<ProviderId> = Vec::new();
    for raw_ch in auto_channels.unwrap_or_default() {
      if let Some(pid) = ProviderId::parse(&raw_ch) {
        if !parsed_channels.contains(&pid) {
          parsed_channels.push(pid);
        }
      }
    }

    let explicit_providers: std::collections::HashSet<ProviderId> = explicit_targets
      .iter()
      .map(|(k, _)| k.provider_id.clone())
      .collect();

    let mut auto_targets: Vec<(AccountKey, String, Option<ProviderId>)> = Vec::new();
    let mut excluded: Vec<AccountKey> = explicit_targets.iter().map(|(k, _)| k.clone()).collect();
    for channel_pid in parsed_channels {
      if explicit_providers.contains(&channel_pid) {
        continue;
      }
      let (picked_key, picked_alias) = Self::select_auto_channel_account_locked(
        locked,
        group_id,
        &channel_pid,
        eligible_members,
        &excluded,
        now,
      )?;
      locked
        .account_last_assigned_ms
        .insert(picked_key.clone(), now);
      excluded.push(picked_key.clone());
      auto_targets.push((picked_key, picked_alias, Some(channel_pid)));
    }

    if explicit_targets.is_empty() && auto_targets.is_empty() {
      if !requested_keys.is_empty() {
        return Err("指定的 @成员当前未登录或不可用，请重新选择".to_string());
      }
      return Ok(
        eligible_members
          .iter()
          .map(|(k, a)| (k.clone(), a.clone(), None))
          .collect(),
      );
    }

    let mut combined: Vec<(AccountKey, String, Option<ProviderId>)> = explicit_targets
      .into_iter()
      .map(|(k, a)| (k, a, None))
      .collect();
    combined.extend(auto_targets);
    Ok(combined)
  }

  pub async fn send_group_message(
    self: &Arc<Self>,
    app: &AppHandle,
    group_id: String,
    content: String,
    target_members: Option<Vec<AccountKey>>,
    auto_channels: Option<Vec<String>>,
  ) -> Result<ManagerSnapshot, String> {
    #[cfg(not(desktop))]
    {
      let _ = (app, group_id, content, target_members, auto_channels);
      Err("群聊功能暂仅支持桌面端".to_string())
    }

    #[cfg(desktop)]
    {
      let trimmed = content.trim().to_string();
      if trimmed.is_empty() {
        return Err("发送内容不能为空".to_string());
      }
      if trimmed.chars().count() > 4000 {
        return Err("单条群聊消息不能超过 4000 字".to_string());
      }

      let (snap, user_msg_id, workflow_mode, dispatch_jobs) = {
        let mut locked = self.inner.lock().await;
        if !locked.conflicts.is_empty() {
          return Err("存在 UUID 归属冲突，请先处理冲突后再发消息".to_string());
        }
        let g_idx = locked
          .store
          .group_chats
          .iter()
          .position(|g| g.id == group_id)
          .ok_or_else(|| "未找到对应的群聊".to_string())?;

        let eligible_members = Self::collect_group_eligible_members_locked(&locked, g_idx);
        if eligible_members.is_empty() {
          return Err("没有可接收消息的可用成员（请检查成员登录或配置状态）".to_string());
        }

        let ts = now_ms();
        let target_accounts = Self::resolve_group_dispatch_targets_locked(
          &mut locked,
          &group_id,
          &trimmed,
          &eligible_members,
          target_members,
          auto_channels,
          ts,
        )?;

        let workflow_mode = locked.store.group_chats[g_idx].workflow_mode;
        let user_id = Uuid::new_v4().hyphenated().to_string();
        let target_keys: Vec<AccountKey> =
          target_accounts.iter().map(|(k, _, _)| k.clone()).collect();
        locked.store.group_chats[g_idx]
          .messages
          .push(GroupChatMessage {
            id: user_id.clone(),
            role: "user".to_string(),
            sender: None,
            sender_name: "我".to_string(),
            content: trimmed.clone(),
            created_at_ms: ts,
            status: GroupReplyStatus::Done,
            targets: target_keys,
            error: None,
            stage_label: None,
            hop_depth: 0,
            auto_routed_channel: None,
          });

        let total_targets = target_accounts.len();
        let mut jobs = Vec::with_capacity(total_targets);
        for (offset, (member_key, alias, auto_ch)) in target_accounts.into_iter().enumerate() {
          let stage_label = if workflow_mode == GroupWorkflowMode::Review && total_targets >= 2 {
            if offset == 0 {
              Some("方案初稿".to_string())
            } else if offset + 1 == total_targets && total_targets >= 3 {
              Some("综合定稿".to_string())
            } else {
              Some("交叉评审".to_string())
            }
          } else {
            None
          };
          let reply_msg_id = Uuid::new_v4().hyphenated().to_string();
          locked.store.group_chats[g_idx]
            .messages
            .push(GroupChatMessage {
              id: reply_msg_id.clone(),
              role: "assistant".to_string(),
              sender: Some(member_key.clone()),
              sender_name: alias,
              content: String::new(),
              created_at_ms: ts + (offset as u64) + 1,
              status: GroupReplyStatus::Queued,
              targets: Vec::new(),
              error: None,
              stage_label: stage_label.clone(),
              hop_depth: 0,
              auto_routed_channel: auto_ch.as_ref().map(|p| p.as_str().to_string()),
            });
          jobs.push((offset, member_key, reply_msg_id, stage_label, auto_ch));
        }

        let msgs = &mut locked.store.group_chats[g_idx].messages;
        if msgs.len() > 120 {
          let excess = msgs.len() - 120;
          msgs.drain(..excess);
        }

        Self::persist_locked(&mut locked).await?;
        (
          Self::snapshot_from_locked(&locked),
          user_id,
          workflow_mode,
          jobs,
        )
      };

      self.emit_snapshot(app, &snap);

      match workflow_mode {
        GroupWorkflowMode::Review if dispatch_jobs.len() >= 2 => {
          let mgr = Arc::clone(self);
          let app_handle = app.clone();
          let gid = group_id.clone();
          let uid = user_msg_id.clone();
          let raw_content = trimmed.clone();
          tauri::async_runtime::spawn(async move {
            mgr
              .run_group_review_pipeline(app_handle, gid, uid, raw_content, dispatch_jobs)
              .await;
          });
        }
        GroupWorkflowMode::A2aRelay => {
          let initial_visited: Vec<AccountKey> = dispatch_jobs
            .iter()
            .map(|(_, k, _, _, _)| k.clone())
            .collect();
          for (order_idx, member_key, msg_id, _, _) in dispatch_jobs {
            let mgr = Arc::clone(self);
            let app_handle = app.clone();
            let gid = group_id.clone();
            let uid = user_msg_id.clone();
            let raw_content = trimmed.clone();
            let visited = initial_visited.clone();
            tauri::async_runtime::spawn(async move {
              if order_idx > 0 {
                tokio::time::sleep(std::time::Duration::from_millis((order_idx as u64) * 40)).await;
              }
              mgr
                .run_group_a2a_chain(
                  app_handle,
                  gid,
                  member_key,
                  msg_id,
                  uid,
                  raw_content,
                  0,
                  visited,
                )
                .await;
            });
          }
        }
        _ => {
          for (order_idx, member_key, msg_id, stage_label, auto_ch) in dispatch_jobs {
            let mgr = Arc::clone(self);
            let app_handle = app.clone();
            let gid = group_id.clone();
            let uid = user_msg_id.clone();
            let raw_content = trimmed.clone();
            tauri::async_runtime::spawn(async move {
              if order_idx > 0 {
                tokio::time::sleep(std::time::Duration::from_millis((order_idx as u64) * 40)).await;
              }
              let _ = mgr
                .run_group_member_turn_with_failover(
                  &app_handle,
                  &gid,
                  &member_key,
                  &msg_id,
                  &uid,
                  &raw_content,
                  stage_label.as_deref(),
                  None,
                  None,
                  0,
                  auto_ch.as_ref(),
                )
                .await;
            });
          }
        }
      }

      Ok(snap)
    }
  }

  #[cfg(desktop)]
  #[allow(clippy::too_many_arguments)]
  pub(super) async fn run_group_member_turn_with_failover(
    self: &Arc<Self>,
    app: &AppHandle,
    group_id: &str,
    member_key: &AccountKey,
    msg_id: &str,
    user_msg_id: &str,
    raw_user_content: &str,
    stage_instruction: Option<&str>,
    fallback_stage_instruction: Option<&str>,
    attachment: Option<&GroupAttachment>,
    hop_depth: u8,
    auto_channel: Option<&ProviderId>,
  ) -> Option<(AccountKey, String)> {
    if let Some(reply) = self
      .run_group_member_turn(
        app,
        group_id,
        member_key,
        msg_id,
        user_msg_id,
        raw_user_content,
        stage_instruction,
        fallback_stage_instruction,
        attachment,
        hop_depth,
      )
      .await
    {
      let mut locked = self.inner.lock().await;
      locked.group_channel_last_account.insert(
        (group_id.to_string(), member_key.provider_id.clone()),
        member_key.clone(),
      );
      locked.account_cooldown_until_ms.remove(member_key);
      return Some((member_key.clone(), reply));
    }

    let now = now_ms();
    let failover_snap = {
      let mut locked = self.inner.lock().await;
      locked
        .account_cooldown_until_ms
        .insert(member_key.clone(), now + AUTO_ROUTE_COOLDOWN_MS);
      if let Some(channel_pid) = auto_channel {
        let g_idx = locked
          .store
          .group_chats
          .iter()
          .position(|g| g.id == group_id)?;
        let can_failover = locked.store.group_chats[g_idx]
          .messages
          .iter()
          .find(|m| m.id == msg_id)
          .is_some_and(|m| m.content.trim().is_empty());
        if !can_failover {
          return None;
        }
        let eligible_members = Self::collect_group_eligible_members_locked(&locked, g_idx);
        let excluded = [member_key.clone()];
        let Ok((next_key, next_alias)) = Self::select_auto_channel_account_locked(
          &locked,
          group_id,
          channel_pid,
          &eligible_members,
          &excluded,
          now,
        ) else {
          return None;
        };

        // 自动失败切换不得切到本地 CLI：切过去等于在用户不知情时
        // 自动执行本地命令。从 LocalCli 失败切走不受影响（只拦切入）。
        if locked
          .store
          .external_agents
          .iter()
          .any(|e| e.account_key() == next_key && e.kind == ExternalAgentKind::LocalCli)
        {
          return None;
        }

        locked
          .account_last_assigned_ms
          .insert(next_key.clone(), now);
        if let Some(m) = locked.store.group_chats[g_idx]
          .messages
          .iter_mut()
          .find(|m| m.id == msg_id)
        {
          m.sender = Some(next_key.clone());
          m.sender_name = next_alias;
          m.status = GroupReplyStatus::Queued;
          m.error = None;
          m.content.clear();
        }
        if let Some(um) = locked.store.group_chats[g_idx]
          .messages
          .iter_mut()
          .find(|m| m.id == user_msg_id && m.role == "user")
        {
          if let Some(slot) = um.targets.iter_mut().find(|t| *t == member_key) {
            *slot = next_key.clone();
          }
        }
        self.request_persist();
        Some((next_key, Self::snapshot_from_locked(&locked)))
      } else {
        None
      }
    };

    if let Some((next_key, snap)) = failover_snap {
      self.emit_snapshot(app, &snap);
      if let Some(reply) = self
        .run_group_member_turn(
          app,
          group_id,
          &next_key,
          msg_id,
          user_msg_id,
          raw_user_content,
          stage_instruction,
          fallback_stage_instruction,
          attachment,
          hop_depth,
        )
        .await
      {
        let mut locked = self.inner.lock().await;
        locked.group_channel_last_account.insert(
          (group_id.to_string(), next_key.provider_id.clone()),
          next_key.clone(),
        );
        locked.account_cooldown_until_ms.remove(&next_key);
        return Some((next_key, reply));
      } else {
        let mut locked = self.inner.lock().await;
        locked
          .account_cooldown_until_ms
          .insert(next_key, now_ms() + AUTO_ROUTE_COOLDOWN_MS);
      }
    }

    None
  }

  pub async fn retry_group_message(
    self: &Arc<Self>,
    app: &AppHandle,
    group_id: String,
    msg_id: String,
  ) -> Result<ManagerSnapshot, String> {
    #[cfg(not(desktop))]
    {
      let _ = (app, group_id, msg_id);
      Err("群聊功能暂仅支持桌面端".to_string())
    }

    #[cfg(desktop)]
    {
      let (snap, member_key, user_msg_id, raw_content, stage_label, hop_depth, auto_ch) = {
        let mut locked = self.inner.lock().await;
        if !locked.conflicts.is_empty() {
          return Err("存在 UUID 归属冲突，请先处理冲突后再重试".to_string());
        }
        let g_idx = locked
          .store
          .group_chats
          .iter()
          .position(|g| g.id == group_id)
          .ok_or_else(|| "未找到对应的群聊".to_string())?;
        let msg_idx = locked.store.group_chats[g_idx]
          .messages
          .iter()
          .position(|m| m.id == msg_id && m.role == "assistant")
          .ok_or_else(|| "未找到可重试的回复消息".to_string())?;

        let target_msg = &locked.store.group_chats[g_idx].messages[msg_idx];
        if !matches!(
          target_msg.status,
          GroupReplyStatus::Error | GroupReplyStatus::NeedsAuth
        ) {
          return Err("仅支持对失败或待授权的消息点击重试".to_string());
        }
        let member_key = target_msg
          .sender
          .clone()
          .ok_or_else(|| "找不到该消息对应的成员".to_string())?;
        let stage_label = target_msg.stage_label.clone();
        let hop_depth = target_msg.hop_depth;
        let auto_ch = target_msg
          .auto_routed_channel
          .as_deref()
          .and_then(ProviderId::parse);

        let member_available = if member_key.provider_id.is_external() {
          locked
            .store
            .external_agents
            .iter()
            .any(|e| e.account_key() == member_key)
        } else {
          locked
            .store
            .accounts
            .iter()
            .find(|a| a.provider_id == member_key.provider_id && a.uuid == member_key.uuid)
            .is_some_and(|acc| {
              acc.auth_state != AuthState::LoggedOut
                && resolve_provider(&acc.provider_id).is_some_and(|s| s.info.enabled)
            })
        };
        if !member_available {
          return Err("该成员当前未登录或配置不可用，请先检查成员状态".to_string());
        }

        let (user_msg_id, raw_content) = locked.store.group_chats[g_idx].messages[..msg_idx]
          .iter()
          .rev()
          .find(|m| m.role == "user" && !m.content.trim().is_empty())
          .map(|m| (m.id.clone(), m.content.clone()))
          .ok_or_else(|| "找不到对应的原始提问消息，请重新发送".to_string())?;

        let msg_mut = &mut locked.store.group_chats[g_idx].messages[msg_idx];
        let prev_status = msg_mut.status;
        let prev_content = std::mem::take(&mut msg_mut.content);
        let prev_error = msg_mut.error.take();
        msg_mut.status = GroupReplyStatus::Queued;
        if let Err(err) = Self::persist_locked(&mut locked).await {
          let msg_revert = &mut locked.store.group_chats[g_idx].messages[msg_idx];
          msg_revert.status = prev_status;
          msg_revert.content = prev_content;
          msg_revert.error = prev_error;
          return Err(err);
        }

        (
          Self::snapshot_from_locked(&locked),
          member_key,
          user_msg_id,
          raw_content,
          stage_label,
          hop_depth,
          auto_ch,
        )
      };

      self.emit_snapshot(app, &snap);

      let mgr = Arc::clone(self);
      let app_handle = app.clone();
      let gid = group_id.clone();
      let mid = msg_id.clone();
      tauri::async_runtime::spawn(async move {
        let _ = mgr
          .run_group_member_turn_with_failover(
            &app_handle,
            &gid,
            &member_key,
            &mid,
            &user_msg_id,
            &raw_content,
            stage_label.as_deref(),
            None,
            None,
            hop_depth,
            auto_ch.as_ref(),
          )
          .await;
      });

      Ok(snap)
    }
  }

  pub async fn sync_group_message_reply(
    self: &Arc<Self>,
    app: &AppHandle,
    group_id: String,
    msg_id: String,
  ) -> Result<ManagerSnapshot, String> {
    #[cfg(not(desktop))]
    {
      let _ = (app, group_id, msg_id);
      Err("群聊功能暂仅支持桌面端".to_string())
    }

    #[cfg(desktop)]
    {
      let (member_key, raw_user_content, app_data_dir) = {
        let locked = self.inner.lock().await;
        if !locked.conflicts.is_empty() {
          return Err("存在 UUID 归属冲突，请先处理冲突后再同步".to_string());
        }
        let g_idx = locked
          .store
          .group_chats
          .iter()
          .position(|g| g.id == group_id)
          .ok_or_else(|| "未找到对应的群聊".to_string())?;
        let msg_idx = locked.store.group_chats[g_idx]
          .messages
          .iter()
          .position(|m| m.id == msg_id && m.role == "assistant")
          .ok_or_else(|| "未找到可同步的回复消息".to_string())?;

        let target_msg = &locked.store.group_chats[g_idx].messages[msg_idx];
        if matches!(
          target_msg.status,
          GroupReplyStatus::Queued | GroupReplyStatus::Sending | GroupReplyStatus::Streaming
        ) {
          return Err("该消息仍在生成中，请稍候".to_string());
        }
        let member_key = target_msg
          .sender
          .clone()
          .ok_or_else(|| "找不到该消息对应的成员".to_string())?;
        if member_key.provider_id.is_external() {
          return Err("API/CLI 成员不支持从页面同步，请使用重试".to_string());
        }
        let member_available = locked
          .store
          .accounts
          .iter()
          .find(|a| a.provider_id == member_key.provider_id && a.uuid == member_key.uuid)
          .is_some_and(|acc| {
            acc.auth_state != AuthState::LoggedOut
              && resolve_provider(&acc.provider_id).is_some_and(|s| s.info.enabled)
          });
        if !member_available {
          return Err("该成员当前未登录或配置不可用，请先检查成员状态".to_string());
        }
        let raw_user_content = locked.store.group_chats[g_idx].messages[..msg_idx]
          .iter()
          .rev()
          .find(|m| m.role == "user" && !m.content.trim().is_empty())
          .map(|m| m.content.clone())
          .unwrap_or_default();

        (member_key, raw_user_content, locked.app_data_dir.clone())
      };

      let (wv, _) = self
        .ensure_group_member_webview_ready(app, &member_key)
        .await?;
      Self::wake_and_settle_group_webview(&wv).await;

      let host = wv
        .url()
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_ascii_lowercase()))
        .unwrap_or_default();
      let adapter = site_adapters::resolve_adapter_for_host(&app_data_dir, &host);
      let is_hark = host.contains("hark.com");

      let sync_js = build_group_sync_latest_js(&adapter, &raw_user_content);
      let res = Self::eval_json_callback(&wv, &sync_js)
        .await
        .ok_or_else(|| "读取该账号页面内容超时，请前往原生页面确认后重试".to_string())?;
      let raw_text = res
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
      let mut final_text = Self::clean_extracted_reply_text(&raw_text);
      if is_hark && !final_text.trim().is_empty() {
        final_text = self
          .extract_hark_expanded_card_if_needed(&wv, &final_text)
          .await;
        final_text = Self::clean_extracted_reply_text(&final_text);
      }
      if final_text.trim().is_empty() {
        let err_hint = res
          .get("error")
          .and_then(|v| v.as_str())
          .filter(|s| !s.trim().is_empty())
          .unwrap_or("暂未从该账号原生页面读取到有效回复，请前往页面确认后再同步");
        return Err(err_hint.to_string());
      }

      let snap = {
        let mut locked = self.inner.lock().await;
        let g = locked
          .store
          .group_chats
          .iter_mut()
          .find(|g| g.id == group_id)
          .ok_or_else(|| "未找到对应的群聊".to_string())?;
        let m = g
          .messages
          .iter_mut()
          .find(|m| m.id == msg_id && m.role == "assistant")
          .ok_or_else(|| "未找到可同步的回复消息".to_string())?;
        let prev_status = m.status;
        let prev_content = std::mem::replace(&mut m.content, final_text);
        let prev_error = m.error.take();
        m.status = GroupReplyStatus::Done;
        if let Err(err) = Self::persist_locked(&mut locked).await {
          if let Some(g_rev) = locked
            .store
            .group_chats
            .iter_mut()
            .find(|g| g.id == group_id)
          {
            if let Some(m_rev) = g_rev.messages.iter_mut().find(|m| m.id == msg_id) {
              m_rev.status = prev_status;
              m_rev.content = prev_content;
              m_rev.error = prev_error;
            }
          }
          return Err(err);
        }
        Self::snapshot_from_locked(&locked)
      };

      self.emit_snapshot(app, &snap);
      Ok(snap)
    }
  }

  #[cfg(desktop)]
  pub(super) async fn eval_json_callback(
    webview: &tauri::Webview,
    js: &str,
  ) -> Option<serde_json::Value> {
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx_cell = Arc::new(std::sync::Mutex::new(Some(tx)));
    let tx_inner = Arc::clone(&tx_cell);
    webview
      .eval_with_callback(js, move |raw| {
        if let Some(sender) = tx_inner.lock().unwrap_or_else(|e| e.into_inner()).take() {
          let _ = sender.send(raw);
        }
      })
      .ok()?;
    let raw = tokio::time::timeout(std::time::Duration::from_millis(5000), rx)
      .await
      .ok()?
      .ok()?;
    if let Ok(inner_json) = serde_json::from_str::<String>(&raw) {
      if let Ok(val) = serde_json::from_str::<serde_json::Value>(&inner_json) {
        return Some(val);
      }
    }
    serde_json::from_str::<serde_json::Value>(&raw).ok()
  }

  /// 把一次自动点击审计行追加写到应用数据目录下的 `auto-approve-audit.log`
  ///（JSONL，每行含时间、账号 key、卡片文本前 140 字、被点击的按钮文案）。
  /// 写盘失败只丢弃本次审计行，不影响主流程。
  #[cfg(desktop)]
  pub(super) fn append_auto_approve_audit(
    app_data_dir: &Path,
    key: &AccountKey,
    entries: &[serde_json::Value],
  ) {
    use std::io::Write;
    let path = app_data_dir.join("auto-approve-audit.log");
    // 按大小轮转：超过 1MB 时把旧日志移到 .1（只保留一份旧日志），
    // 避免长期高频点击让审计日志无限增长。轮转失败也不影响主流程。
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1_048_576) {
      let rotated = app_data_dir.join("auto-approve-audit.log.1");
      let _ = std::fs::remove_file(&rotated);
      let _ = std::fs::rename(&path, &rotated);
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
      .create(true)
      .append(true)
      .open(&path)
    else {
      return;
    };
    // 审计日志含账号 key 与审批卡片文本：仅所有者可读写。
    // 每次追加时都设置，之前已存在的 0o644 旧日志也会被收紧。
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    for entry in entries {
      // 单条截断：审计文本来自页面上报，恶意页面可用超大文本撑日志；
      // 卡片文本 1000 字、按钮文本 128 字足够覆盖真实内容。
      let text: String = entry
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .take(1000)
        .collect();
      let button: String = entry
        .get("button")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .take(128)
        .collect();
      let line = serde_json::json!({
        "at_ms": entry.get("at").and_then(|v| v.as_u64()).unwrap_or(0),
        "provider_id": key.provider_id.as_str(),
        "uuid": key.uuid,
        "text": text,
        "button": button,
      });
      let _ = writeln!(file, "{line}");
    }
  }

  /// 从保活 webview 读回自动审批的上报（drain 语义，读走即清）：
  /// 审计条目、熔断标志、需要手动处理的标记。
  /// 熔断时持久化关闭自动审批总开关并向 UI 发 notice；
  /// 有手动处理标记时同样发 notice（同一张卡 60 秒内 JS 侧只记一次）。
  /// 在静默同步的每个保活账号任务末尾调用；页面未加载上报函数时静默跳过。
  #[cfg(desktop)]
  pub(super) async fn drain_auto_approve_report(
    &self,
    app: &AppHandle,
    key: &AccountKey,
    webview: &tauri::Webview,
  ) {
    let Some(report) = Self::eval_json_callback(
      webview,
      "(() => { try { return typeof window.__maamAutoApproveReport === 'function' ? window.__maamAutoApproveReport() : null; } catch (_) { return null; } })()",
    )
    .await
    else {
      return;
    };
    let Some(entries) = report.get("entries").and_then(|v| v.as_array()) else {
      return;
    };
    if !entries.is_empty() {
      let app_data_dir = {
        let locked = self.inner.lock().await;
        locked.app_data_dir.clone()
      };
      Self::append_auto_approve_audit(&app_data_dir, key, entries);
    }
    let tripped = report
      .get("tripped")
      .and_then(|v| v.as_bool())
      .unwrap_or(false);
    let needs_manual = report
      .get("needsManual")
      .cloned()
      .unwrap_or(serde_json::Value::Null);
    if !tripped && needs_manual.is_null() {
      return;
    }
    let mut locked = self.inner.lock().await;
    let mut changed = false;
    if tripped && locked.store.settings.auto_approve {
      locked.store.settings.auto_approve = false;
      self.auto_approve.store(false, Ordering::SeqCst);
      locked.notice = Some(
        "自动审批在 60 秒内自动点击超过 10 次，已自动关闭，请检查该账号页面的审批弹窗".to_string(),
      );
      changed = true;
    }
    if !needs_manual.is_null() {
      let reason = needs_manual
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("");
      let text = needs_manual
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("");
      let what = match reason {
        "command" => "命令执行类审批",
        _ => "仅有「始终允许」的审批",
      };
      locked.notice = Some(format!(
        "检测到{what}，自动审批已跳过，请前往该账号页面手动处理：{text}"
      ));
      changed = true;
    }
    if !changed {
      return;
    }
    // 熔断后把关闭持久化，避免下次启动或新建 webview 又重新开启。
    // P0-1：写盘被阻止时不直写，否则会覆盖损坏/高版本配置文件。
    if tripped && !locked.persist_blocked {
      if let Err(err) = save_store_atomic(&locked.store_path, &locked.store) {
        locked.notice = Some(format!("自动审批已熔断关闭，但写盘失败：{err}"));
      }
    }
    let allow_commands = locked.store.settings.auto_approve_commands;
    let warm_labels: Vec<String> = locked
      .warm_pool
      .values()
      .map(|e| e.webview_label.clone())
      .collect();
    let snap = Self::snapshot_from_locked(&locked);
    drop(locked);
    self.emit_snapshot(app, &snap);
    // 熔断后立即把关闭广播到所有保活 webview：之前只持久化了开关，
    // 已加载页面里 window.__maamAutoApprove 仍是 true，会继续自动点击
    // 直到页面重载。只在 tripped 时广播（needsManual 不改开关）。
    if tripped {
      let sync_js = Self::auto_approve_sync_js(false, allow_commands);
      for label in warm_labels {
        if let Some(wv) = app.get_webview(&label) {
          let _ = wv.eval(&sync_js);
        }
      }
    }
  }

  #[cfg(desktop)]
  #[allow(clippy::too_many_arguments)]
  pub(super) async fn update_group_message_state(
    &self,
    app: &AppHandle,
    group_id: &str,
    msg_id: &str,
    status: GroupReplyStatus,
    content_opt: Option<String>,
    error_opt: Option<Option<String>>,
    persist: bool,
  ) -> bool {
    let snap_opt = {
      let mut locked = self.inner.lock().await;
      let Some(g) = locked
        .store
        .group_chats
        .iter_mut()
        .find(|g| g.id == group_id)
      else {
        return false;
      };
      let Some(m) = g.messages.iter_mut().find(|m| m.id == msg_id) else {
        return false;
      };
      let mut changed = m.status != status;
      m.status = status;
      if let Some(content) = content_opt {
        if m.content != content {
          m.content = content;
          changed = true;
        }
      }
      if let Some(err) = error_opt {
        if m.error != err {
          m.error = err;
          changed = true;
        }
      }
      if !changed && !persist {
        return true;
      }
      if persist {
        self.request_persist();
      }
      Some(Self::snapshot_from_locked(&locked))
    };

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
      true
    } else {
      false
    }
  }

  #[cfg(desktop)]
  pub(super) async fn ensure_group_member_webview_ready(
    self: &Arc<Self>,
    app: &AppHandle,
    key: &AccountKey,
  ) -> Result<(tauri::Webview, u64), String> {
    let account_to_start = {
      let mut locked = self.inner.lock().await;
      if let Some(entry) = locked.warm_pool.get_mut(key) {
        if entry.paint_ready {
          entry.last_used_ms = now_ms();
          let vid = entry.view_instance_id;
          let label = entry.webview_label.clone();
          if let Some(wv) = app.get_webview(&label) {
            return Ok((wv, vid));
          }
        }
        None
      } else {
        let Some(acc) = locked
          .store
          .accounts
          .iter()
          .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
          .cloned()
        else {
          return Err("账号不存在或已被移除".to_string());
        };
        if acc.auth_state == AuthState::LoggedOut {
          return Err("该账号当前处于未登录状态，请先前往登录".to_string());
        }
        Some(acc)
      }
    };

    if let Some(acc) = account_to_start {
      self
        .prewarm_desktop_account_with_limit(app, acc, true)
        .await?;
    }

    for _ in 0..200 {
      {
        let mut locked = self.inner.lock().await;
        let Some(acc) = locked
          .store
          .accounts
          .iter()
          .find(|a| a.provider_id == key.provider_id && a.uuid == key.uuid)
        else {
          return Err("账号已被移除".to_string());
        };
        if acc.auth_state == AuthState::LoggedOut {
          return Err("该账号未登录或会话已失效，请前往原生页面重新登录".to_string());
        }
        if let Some(entry) = locked.warm_pool.get_mut(key) {
          if entry.paint_ready {
            entry.last_used_ms = now_ms();
            let vid = entry.view_instance_id;
            let label = entry.webview_label.clone();
            if let Some(wv) = app.get_webview(&label) {
              return Ok((wv, vid));
            }
          }
        }
      }
      tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    Err("成员页面加载超时，请切换至该账号页确认状态".to_string())
  }

  #[cfg(desktop)]
  pub(super) fn normalize_url_for_thread(url: &Url) -> String {
    let scheme = url.scheme().to_ascii_lowercase();
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let path = url.path().trim_end_matches('/');
    match url.query() {
      Some(q) if !q.is_empty() => format!("{scheme}://{host}{path}?{q}"),
      _ => format!("{scheme}://{host}{path}"),
    }
  }

  #[cfg(desktop)]
  pub(super) async fn wait_webview_settled(wv: &tauri::Webview, max_ticks: usize) {
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    for _ in 0..max_ticks {
      #[cfg(target_os = "macos")]
      let loading = platform::is_macos_webview_loading(wv).await;
      #[cfg(not(target_os = "macos"))]
      let loading = false;
      if !loading {
        if let Some(val) = Self::eval_json_callback(wv, "document.readyState === 'complete'").await
        {
          if val.as_bool() == Some(true) {
            return;
          }
        }
      }
      tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
  }

  #[cfg(desktop)]
  // edition 2021 不允许 let-chain，两个嵌套 if（if-let + if-bool）无法合并，保留嵌套。
  #[allow(clippy::collapsible_if)]
  pub(super) async fn prepare_group_member_thread(
    &self,
    group_id: &str,
    member_key: &AccountKey,
    wv: &tauri::Webview,
  ) {
    let Ok(cur_url) = wv.url() else {
      return;
    };
    let (app_data_dir, spec_opt, bound_url) = {
      let locked = self.inner.lock().await;
      let spec = resolve_provider(&member_key.provider_id);
      let bound = locked
        .store
        .group_chats
        .iter()
        .find(|g| g.id == group_id)
        .and_then(|g| g.thread_bindings.iter().find(|tb| &tb.member == member_key))
        .map(|tb| tb.thread_url.clone());
      (locked.app_data_dir.clone(), spec, bound)
    };
    let Some(spec) = spec_opt else {
      return;
    };
    let cur_host = cur_url.host_str().unwrap_or("");
    let adapter = site_adapters::resolve_adapter_for_host(&app_data_dir, cur_host);
    if adapter.thread_url_patterns.is_empty() {
      return;
    }

    if let Some(bound_str) = bound_url {
      if let Ok(target_url) = Url::parse(&bound_str) {
        // 绑定的 thread_url 只允许跳回本站（callback_origins）：之前用
        // classify_top_level_url 的 AllowHttps 会放行任意 allowed_host
        //（CDN、facebook.com 等），会话页会带着账号 Cookie 跳过去。
        if is_verified_callback_origin(&spec, &target_url)
          && Self::normalize_url_for_thread(&target_url) != Self::normalize_url_for_thread(&cur_url)
        {
          if wv.navigate(target_url).is_ok() {
            Self::wait_webview_settled(wv, 24).await;
          }
        }
      }
      return;
    }

    let cur_str = cur_url.as_str();
    let on_existing_thread = adapter
      .thread_url_patterns
      .iter()
      .any(|p| !p.is_empty() && cur_str.contains(p));
    if !on_existing_thread {
      return;
    }

    if !adapter.new_chat_selectors.is_empty() {
      let click_js = build_new_chat_click_js(&adapter);
      if Self::eval_json_callback(wv, &click_js)
        .await
        .and_then(|v| v.get("clicked").and_then(|b| b.as_bool()))
        .unwrap_or(false)
      {
        Self::wait_webview_settled(wv, 14).await;
        return;
      }
    }

    if let Some(new_url) = adapter
      .new_chat_url
      .as_deref()
      .and_then(|s| Url::parse(s).ok())
    {
      // new_chat_url 同样只允许本站：适配器覆盖（用户可编辑的 JSON）
      // 里写一个 CDN/外站地址就能让会话页带着 Cookie 跳过去。
      if is_verified_callback_origin(&spec, &new_url)
        && Self::normalize_url_for_thread(&new_url) != Self::normalize_url_for_thread(&cur_url)
      {
        if wv.navigate(new_url).is_ok() {
          Self::wait_webview_settled(wv, 20).await;
        }
      }
    }
  }

  #[cfg(desktop)]
  pub(super) async fn persist_group_thread_binding(
    &self,
    app: &AppHandle,
    group_id: &str,
    member_key: &AccountKey,
    _pre_send_norm: Option<&str>,
    wv: &tauri::Webview,
    adapter: &SiteAdapterConfig,
  ) {
    if adapter.thread_url_patterns.is_empty() {
      return;
    }
    let Ok(cur_url) = wv.url() else {
      return;
    };
    let cur_str = cur_url.as_str().to_string();
    let matches_pat = adapter
      .thread_url_patterns
      .iter()
      .any(|p| !p.is_empty() && cur_str.contains(p));
    if !matches_pat {
      return;
    }

    let snap_opt = {
      let mut locked = self.inner.lock().await;
      let valid_url = resolve_provider(&member_key.provider_id).is_some_and(|spec| {
        matches!(
          classify_top_level_url(&spec, &cur_url, false),
          NavigationDecision::AllowHttps
        )
      });
      if !valid_url {
        return;
      }
      let Some(g) = locked
        .store
        .group_chats
        .iter_mut()
        .find(|g| g.id == group_id)
      else {
        return;
      };
      if let Some(existing) = g
        .thread_bindings
        .iter_mut()
        .find(|tb| &tb.member == member_key)
      {
        if existing.thread_url == cur_str {
          return;
        }
        existing.thread_url = cur_str;
        existing.updated_at_ms = now_ms();
      } else {
        g.thread_bindings.push(GroupThreadBinding {
          member: member_key.clone(),
          thread_url: cur_str,
          updated_at_ms: now_ms(),
        });
      }
      self.request_persist();
      Some(Self::snapshot_from_locked(&locked))
    };

    if let Some(snap) = snap_opt {
      self.emit_snapshot(app, &snap);
    }
  }

  #[cfg(desktop)]
  pub(super) async fn resolve_member_transport_locked(
    locked: &InnerState,
    key: &AccountKey,
  ) -> Result<ResolvedMemberTransport, String> {
    if key.provider_id.is_external() {
      let ext = locked
        .store
        .external_agents
        .iter()
        .find(|e| e.account_key() == *key)
        .cloned()
        .ok_or_else(|| "API/CLI 成员不存在或已被移除".to_string())?;
      return Ok(match ext.kind {
        ExternalAgentKind::OpenaiApi => ResolvedMemberTransport::OpenAiApi { agent: ext },
        ExternalAgentKind::LocalCli => ResolvedMemberTransport::LocalCli { agent: ext },
      });
    }
    Ok(ResolvedMemberTransport::WebView { key: key.clone() })
  }

  #[cfg(desktop)]
  pub(super) fn sanitize_history_assistant_text(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    if s.is_empty() {
      return None;
    }
    for prefix in [
      "我不和其他账号的助手一起协作分工，但",
      "我不和其他账号的助手一起协作分工,",
      "我不和其他账号的助手一起协作分工。",
      "群聊里替某个账号发言我做不了，不过",
      "群聊里替某个账号发言我做不了，",
      "群聊里替某个账号发言我做不了。",
    ] {
      if let Some(rest) = s.strip_prefix(prefix) {
        s = rest.trim();
      }
    }
    if s.is_empty() {
      return None;
    }
    if s.contains("2项预定任务") && s.contains("查看所有预定任务") {
      return None;
    }
    let lines: Vec<&str> = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
      return None;
    }
    if lines.len() <= 3 && s.chars().count() <= 80 {
      let all_transient = lines.iter().all(|l| {
        matches!(
          *l,
          "Muse"
            | "Hark"
            | "空闲中"
            | "已连接"
            | "连接中"
            | "正在连接"
            | "仍在发送中"
            | "发送中"
            | "努力工作中"
            | "工作中"
            | "努力思考中"
            | "正在思考"
            | "思考中"
            | "正在搜索"
            | "正在搜索网页"
            | "正在联网搜索"
            | "搜索中"
            | "检索中"
            | "正在检索"
            | "加载中"
            | "待命中"
            | "Researching channels"
        ) || l.starts_with("Searching for ")
          || l.starts_with("Researching ")
      });
      if all_transient {
        return None;
      }
    }
    Some(s.chars().take(240).collect())
  }

  #[cfg(desktop)]
  #[allow(clippy::too_many_arguments)]
  #[cfg_attr(not(desktop), allow(dead_code))]
  pub(super) fn sanitize_shared_context_for_prompt(raw: &str) -> String {
    let mut out_lines: Vec<String> = Vec::new();
    let mut idx = 0usize;
    for line in raw.trim().lines() {
      let t = line.trim();
      if let Some(inner) = t
        .strip_prefix("[本轮结论汇总")
        .and_then(|s| s.strip_suffix(']'))
      {
        idx = 0;
        let topic = inner.trim_start_matches([' ', '·']).trim();
        out_lines.push(if topic.is_empty() {
          "已有结论".to_string()
        } else {
          format!("已有结论（议题：{topic}）")
        });
        continue;
      }
      if let Some(rest) = t.strip_prefix("- ") {
        if let Some(pos) = rest.find(": ") {
          if rest[..pos].contains('@') {
            idx += 1;
            out_lines.push(format!("- 观点{idx}: {}", &rest[pos + 2..]));
            continue;
          }
        }
      }
      out_lines.push(line.to_string());
    }
    out_lines.join("\n").chars().take(3000).collect()
  }

  #[cfg_attr(not(desktop), allow(dead_code))]
  pub(super) fn clean_extracted_reply_text(raw: &str) -> String {
    const SPEAKERS: [&str; 6] = ["Gemini", "ChatGPT", "Claude", "DeepSeek", "Grok", "Kimi"];
    const VERBS: [&str; 3] = ["说", "said", "says"];

    let mut s = raw.trim();
    loop {
      let mut stripped = None;
      'outer: for sp in SPEAKERS {
        if let Some(after_sp) = Self::starts_with_ignore_case(s, sp).map(|len| &s[len..]) {
          let after_sp_ws = after_sp.trim_start_matches([' ', '\t']);
          for verb in VERBS {
            if let Some(v_len) = Self::starts_with_ignore_case(after_sp_ws, verb) {
              let tail = &after_sp_ws[v_len..];
              let tail_ws = tail.trim_start_matches([' ', '\t']);
              if tail_ws.is_empty() {
                return String::new();
              }
              if let Some(after_colon) = tail_ws
                .strip_prefix(':')
                .or_else(|| tail_ws.strip_prefix('：'))
              {
                stripped = Some(after_colon.trim_start());
                break 'outer;
              }
              if tail_ws.starts_with('\n') || tail_ws.starts_with('\r') {
                stripped = Some(tail_ws.trim_start());
                break 'outer;
              }
            }
          }
        }
      }
      match stripped {
        Some(next) => s = next,
        None => break,
      }
    }
    s.trim().to_string()
  }

  #[cfg_attr(not(desktop), allow(dead_code))]
  pub(super) fn adapt_question_for_hark(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    for (pat, rep) in [
      ("群里的各位", "你"),
      ("群里各位", "你"),
      ("群里大家", "你"),
      ("各位好", "你好"),
      ("大家好", "你好"),
      ("请各位", "请你"),
      ("请大家", "请你"),
      ("各位成员", "你"),
      ("各位助手", "你"),
      ("你们分别", "请你"),
      ("你们各自", "请你"),
      ("你们几个", "你"),
      ("你们", "你"),
    ] {
      s = s.replace(pat, rep);
    }

    for action in [
      "觉得", "怎么", "看看", "看下", "看", "帮忙", "帮我", "帮", "来", "说说", "说下", "说",
      "回答", "回复", "回", "能否", "能", "可以", "一起", "各自", "都", "有", "给",
    ] {
      s = s.replace(&format!("各位{action}"), &format!("你{action}"));
      s = s.replace(&format!("大家{action}"), &format!("你{action}"));
    }

    for prefix in [
      "各位，",
      "各位,",
      "各位：",
      "各位:",
      "各位、",
      "各位 ",
      "大家，",
      "大家,",
      "大家：",
      "大家:",
      "大家、",
      "大家 ",
    ] {
      if let Some(rest) = s.strip_prefix(prefix) {
        let rest_trim = rest.trim();
        if !rest_trim.is_empty() {
          s = rest_trim.to_string();
          break;
        }
      }
    }

    let cleaned = s.trim().to_string();
    if cleaned.is_empty() {
      raw.trim().to_string()
    } else {
      cleaned
    }
  }

  #[cfg(desktop)]
  pub(super) fn sanitize_shared_context_for_hark(raw: &str) -> String {
    let base = Self::sanitize_shared_context_for_prompt(raw);
    let mut out_lines: Vec<String> = Vec::new();
    for line in base.lines() {
      let t = line.trim();
      if t == "已有结论" {
        out_lines.push("背景要点：".to_string());
        continue;
      }
      if let Some(inner) = t
        .strip_prefix("已有结论（议题：")
        .and_then(|s| s.strip_suffix('）'))
      {
        out_lines.push(format!("背景要点（{inner}）："));
        continue;
      }
      if let Some(rest) = t.strip_prefix("- 观点") {
        if let Some(pos) = rest.find(": ") {
          if rest[..pos].chars().all(|c| c.is_ascii_digit()) {
            out_lines.push(format!("- {}", &rest[pos + 2..]));
            continue;
          }
        }
      }
      out_lines.push(line.to_string());
    }
    out_lines.join("\n").chars().take(3000).collect()
  }

  #[cfg(desktop)]
  #[allow(clippy::too_many_arguments)]
  pub(super) async fn build_turn_prompt_for_member(
    &self,
    group_id: &str,
    member_key: &AccountKey,
    view_instance_id: u64,
    msg_id: &str,
    user_msg_id: &str,
    raw_user_content: &str,
    stage_instruction: Option<&str>,
    hop_depth: u8,
  ) -> Option<(String, String, String, u64)> {
    let locked = self.inner.lock().await;
    let group = locked.store.group_chats.iter().find(|g| g.id == group_id)?;

    let fingerprint = Self::compute_group_knowledge_fingerprint(group, member_key);
    let synced_key = (group_id.to_string(), member_key.clone());
    let needs_full_sync = member_key.provider_id.is_external()
      || locked.group_context_synced.get(&synced_key) != Some(&(view_instance_id, fingerprint));

    let member_pairs: Vec<(AccountKey, String)> = group
      .members
      .iter()
      .filter_map(|m| Self::resolve_member_alias_locked(&locked, m).map(|a| (m.clone(), a)))
      .collect();

    let is_hark = member_key.provider_id.is_hark();
    let stripped_question = Self::strip_routing_mentions(raw_user_content, &member_pairs);
    let clean_question = if is_hark {
      Self::adapt_question_for_hark(&stripped_question)
    } else {
      stripped_question
    };

    let my_role = group
      .member_roles
      .iter()
      .find(|r| &r.member == member_key)
      .map(|r| r.role.trim())
      .filter(|r| !r.is_empty());

    let enabled_skills: Vec<&GroupSharedSkill> =
      group.shared_skills.iter().filter(|s| s.enabled).collect();
    let has_shared_ctx = !group.shared_context.trim().is_empty();
    let stage_clean = stage_instruction.map(str::trim).filter(|s| !s.is_empty());

    let format_skill_lines = |max_chars: usize| -> Vec<String> {
      let mut skill_lines = Vec::new();
      let mut total_chars = 0usize;
      for sk in &enabled_skills {
        let line = format!("- 《{}》：{}", sk.name.trim(), sk.content.trim());
        let count = line.chars().count();
        if total_chars + count > max_chars && !skill_lines.is_empty() {
          break;
        }
        total_chars += count;
        skill_lines.push(line);
      }
      skill_lines
    };

    let is_webview_parallel = !member_key.provider_id.is_external()
      && group.workflow_mode == GroupWorkflowMode::Parallel
      && stage_clean.is_none();

    if is_webview_parallel {
      let mut sys_sections: Vec<String> = Vec::new();
      if needs_full_sync {
        if let Some(role) = my_role {
          sys_sections.push(format!("请侧重从「{role}」视角回答。"));
        }
        if has_shared_ctx {
          let ctx_snip = if is_hark {
            Self::sanitize_shared_context_for_hark(&group.shared_context)
          } else {
            Self::sanitize_shared_context_for_prompt(&group.shared_context)
          };
          sys_sections.push(format!("参考背景：\n{ctx_snip}"));
        }
        if !enabled_skills.is_empty() {
          let skill_lines = format_skill_lines(4000);
          if !skill_lines.is_empty() {
            sys_sections.push(format!("回答要求：\n{}", skill_lines.join("\n")));
          }
        }
      }

      if sys_sections.is_empty() {
        return Some((
          String::new(),
          clean_question.clone(),
          clean_question,
          fingerprint,
        ));
      }

      let system_section = sys_sections.join("\n\n");
      let user_section = format!("问题：\n{clean_question}");
      let full_prompt = format!("{system_section}\n\n{user_section}");
      return Some((system_section, user_section, full_prompt, fingerprint));
    }

    let mut cutoff = group
      .messages
      .iter()
      .position(|m| m.id == msg_id)
      .unwrap_or(group.messages.len());
    if group.workflow_mode == GroupWorkflowMode::Review && stage_clean.is_some() {
      if let Some(user_pos) = group.messages.iter().position(|m| m.id == user_msg_id) {
        cutoff = cutoff.min(user_pos);
      }
    }
    let history_lines = group.messages[..cutoff]
      .iter()
      .rev()
      .filter(|m| {
        m.id != user_msg_id && m.status == GroupReplyStatus::Done && !m.content.trim().is_empty()
      })
      .filter_map(|m| {
        if m.role == "user" {
          let q = Self::strip_routing_mentions(&m.content, &member_pairs);
          let snip: String = q.trim().chars().take(200).collect();
          if snip.is_empty() {
            None
          } else {
            Some(format!("- 提问: {snip}"))
          }
        } else {
          let snip = Self::sanitize_history_assistant_text(&m.content)?;
          let raw_speaker = m
            .sender
            .as_ref()
            .and_then(|sk| Self::resolve_member_alias_locked(&locked, sk))
            .unwrap_or_else(|| m.sender_name.clone());
          let label = if is_hark || raw_speaker.contains('@') {
            "已知参考".to_string()
          } else {
            format!("参考（{raw_speaker}）")
          };
          Some(format!("- {label}: {snip}"))
        }
      })
      .take(6)
      .collect::<Vec<_>>()
      .into_iter()
      .rev()
      .collect::<Vec<_>>();

    let is_a2a_active = !is_hark
      && group.workflow_mode == GroupWorkflowMode::A2aRelay
      && hop_depth < group.max_relay_hops;

    let mut sys_sections: Vec<String> = Vec::new();
    if needs_full_sync {
      if let Some(role) = my_role {
        sys_sections.push(format!(
          "[回答视角偏好]\n请重点从「{role}」视角进行分析与解答。"
        ));
      }
      if has_shared_ctx {
        let ctx_snip = if is_hark {
          Self::sanitize_shared_context_for_hark(&group.shared_context)
        } else {
          Self::sanitize_shared_context_for_prompt(&group.shared_context)
        };
        sys_sections.push(format!("[背景资料]\n{ctx_snip}"));
      }
      if !enabled_skills.is_empty() {
        let skill_lines = format_skill_lines(4000);
        if !skill_lines.is_empty() {
          sys_sections.push(format!("[输出规范]\n{}", skill_lines.join("\n")));
        }
      }
    } else if has_shared_ctx || !enabled_skills.is_empty() || my_role.is_some() {
      sys_sections.push("（请继续遵循本会话已提供的背景资料与输出规范）".to_string());
    }

    let mut user_sections: Vec<String> = Vec::new();
    if !history_lines.is_empty() {
      user_sections.push(format!("[上文纪要]\n{}", history_lines.join("\n")));
    }

    if is_a2a_active {
      let other_names = group
        .members
        .iter()
        .filter(|m| *m != member_key)
        .filter_map(|m| Self::resolve_member_alias_locked(&locked, m))
        .collect::<Vec<_>>()
        .join("、");
      if !other_names.is_empty() {
        user_sections.push(format!(
          "[接力标记说明]\n如需补充复核，可在回答末尾附上 @标记（可选：{other_names}）；无需接力则不要写 @。"
        ));
      }
    }

    // 阶段要求放进 user 段：它含前序成员的回复原文（不可信内容），
    // 绝不能进 system 段，否则会被模型当作系统指令执行（MAAM-H-01）。
    if let Some(stage_inst) = stage_clean {
      user_sections.push(format!("[阶段要求]\n{stage_inst}"));
    }

    if sys_sections.is_empty() && user_sections.is_empty() {
      return Some((
        String::new(),
        clean_question.clone(),
        clean_question,
        fingerprint,
      ));
    }

    user_sections.push(format!(
      "[当前问题]\n{}\n（请直接回答「当前问题」，不要复述上方方括号标题）",
      clean_question
    ));

    let system_section = sys_sections.join("\n\n");
    let user_section = user_sections.join("\n\n");
    let full_prompt = if system_section.is_empty() {
      user_section.clone()
    } else {
      format!("{system_section}\n\n{user_section}")
    };
    Some((system_section, user_section, full_prompt, fingerprint))
  }
}

/// 在 Drop 里清理需要 `inner` 锁的记账：优先 try_lock 同步做完；
/// 抢不到锁时派生异步任务做（运行时存活期间可用；进程退出时记账
/// 随内存一起消失，无需清理）。供下面的两个 Drop 守卫共用。
#[cfg(desktop)]
pub(super) fn cleanup_inner_on_drop<F>(mgr: &Arc<SessionManager>, f: F)
where
  F: FnOnce(&mut InnerState) + Send + 'static,
{
  match mgr.inner.try_lock() {
    Ok(mut locked) => f(&mut locked),
    Err(_) => {
      let mgr = Arc::clone(mgr);
      tauri::async_runtime::spawn(async move {
        let mut locked = mgr.inner.lock().await;
        f(&mut locked);
      });
    }
  }
}

/// 回合记账 Drop 守卫：in_flight 计数与 warm-pool 时间戳在回合结束时
/// 统一扣减/更新。正常路径、提前返回与 panic 都走这里，避免计数泄漏
/// 导致 warm-pool 永久认为该成员在跑而不回收。
#[cfg(desktop)]
struct InFlightGuard {
  mgr: Arc<SessionManager>,
  app: AppHandle,
  key: AccountKey,
}

#[cfg(desktop)]
impl Drop for InFlightGuard {
  fn drop(&mut self) {
    let app = self.app.clone();
    let key = self.key.clone();
    cleanup_inner_on_drop(&self.mgr, move |locked| {
      if let Some(cnt) = locked.in_flight_group_members.get_mut(&key) {
        *cnt = cnt.saturating_sub(1);
        if *cnt == 0 {
          locked.in_flight_group_members.remove(&key);
        }
      }
      if let Some(entry) = locked.warm_pool.get_mut(&key) {
        entry.last_used_ms = now_ms();
      }
      SessionManager::apply_desktop_layout_locked(&app, locked);
    });
  }
}

/// 外部回合取消令牌 Drop 守卫：登记后无论正常结束、提前返回还是
/// panic 都注销，避免「停止」按钮把已结束的回合当成还在跑。
#[cfg(desktop)]
struct ExternalTurnCancelGuard {
  mgr: Arc<SessionManager>,
  group_id: String,
  msg_id: String,
}

#[cfg(desktop)]
impl Drop for ExternalTurnCancelGuard {
  fn drop(&mut self) {
    let group_id = self.group_id.clone();
    let msg_id = self.msg_id.clone();
    cleanup_inner_on_drop(&self.mgr, move |locked| {
      locked.external_turn_cancel.remove(&(group_id, msg_id));
    });
  }
}

impl SessionManager {
  #[cfg(desktop)]
  #[allow(clippy::too_many_arguments)]
  pub(super) async fn run_group_member_turn(
    self: &Arc<Self>,
    app: &AppHandle,
    group_id: &str,
    member_key: &AccountKey,
    msg_id: &str,
    user_msg_id: &str,
    raw_user_content: &str,
    stage_instruction: Option<&str>,
    fallback_stage_instruction: Option<&str>,
    attachment: Option<&GroupAttachment>,
    hop_depth: u8,
  ) -> Option<String> {
    let transport = {
      let locked = self.inner.lock().await;
      Self::resolve_member_transport_locked(&locked, member_key).await
    };

    let transport = match transport {
      Ok(t) => t,
      Err(err) => {
        self
          .update_group_message_state(
            app,
            group_id,
            msg_id,
            GroupReplyStatus::Error,
            None,
            Some(Some(err)),
            true,
          )
          .await;
        return None;
      }
    };

    // 并发锁：同一成员的回合必须串行。之前只有 webview 成员拿了
    // group_member_locks，外部成员（API/CLI）没拿——两个回合可能
    // 同时跑同一个本地 CLI，stdin/输出/文件互相交错。锁由
    // _member_guard（Drop 即释放）持有整个回合；in_flight 计数
    // 由下面的 InFlightGuard 在函数尾统一扣减（正常、提前返回、
    // panic 都走守卫）。
    let member_lock = {
      let mut locked = self.inner.lock().await;
      *locked
        .in_flight_group_members
        .entry(member_key.clone())
        .or_insert(0) += 1;
      Self::apply_desktop_layout_locked(app, &locked);
      Arc::clone(
        locked
          .group_member_locks
          .entry(member_key.clone())
          .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
      )
    };
    // InFlightGuard 在等锁之前构造：上面刚 +1 的计数，若等锁期间任务
    // 被取消，仍由守卫的 Drop 扣减，不泄漏。声明顺序上它先于
    // _member_guard（后声明的先释放），即成员锁先放、计数后扣——新回合
    // 只能在旧回合 Future 完全结束（全部局部守卫已释放）后才开始，
    // 瞬时计数多 1 不影响正确性。
    let _in_flight_guard = InFlightGuard {
      mgr: Arc::clone(self),
      app: app.clone(),
      key: member_key.clone(),
    };
    let _member_guard = member_lock.lock_owned().await;
    let reply = match transport {
      ResolvedMemberTransport::WebView { .. } => {
        if let Ok(_permit) = Arc::clone(&self.group_worker_semaphore)
          .acquire_owned()
          .await
        {
          self
            .execute_webview_member_turn(
              app,
              group_id,
              member_key,
              msg_id,
              user_msg_id,
              raw_user_content,
              stage_instruction,
              fallback_stage_instruction,
              attachment,
              hop_depth,
            )
            .await
        } else {
          None
        }
      }
      ResolvedMemberTransport::OpenAiApi { agent }
      | ResolvedMemberTransport::LocalCli { agent } => {
        self
          .execute_external_member_turn(
            app,
            group_id,
            member_key,
            &agent,
            msg_id,
            user_msg_id,
            raw_user_content,
            stage_instruction,
            hop_depth,
          )
          .await
      }
    };
    reply
  }

  #[cfg(desktop)]
  #[allow(clippy::too_many_arguments)]
  pub(super) async fn execute_external_member_turn(
    self: &Arc<Self>,
    app: &AppHandle,
    group_id: &str,
    member_key: &AccountKey,
    agent: &ExternalAgentRecord,
    msg_id: &str,
    user_msg_id: &str,
    raw_user_content: &str,
    stage_instruction: Option<&str>,
    hop_depth: u8,
  ) -> Option<String> {
    if !self
      .update_group_message_state(
        app,
        group_id,
        msg_id,
        GroupReplyStatus::Sending,
        None,
        Some(None),
        false,
      )
      .await
    {
      return None;
    }

    let (system_section, user_section, full_prompt, _fingerprint) = self
      .build_turn_prompt_for_member(
        group_id,
        member_key,
        0,
        msg_id,
        user_msg_id,
        raw_user_content,
        stage_instruction,
        hop_depth,
      )
      .await?;

    if !self
      .update_group_message_state(
        app,
        group_id,
        msg_id,
        GroupReplyStatus::Streaming,
        None,
        Some(None),
        false,
      )
      .await
    {
      return None;
    }

    let (prog_tx, mut prog_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mgr_prog = Arc::clone(self);
    let app_prog = app.clone();
    let gid_prog = group_id.to_string();
    let mid_prog = msg_id.to_string();
    let fwd_handle = tauri::async_runtime::spawn(async move {
      while let Some(partial) = prog_rx.recv().await {
        let _ = mgr_prog
          .update_group_message_state(
            &app_prog,
            &gid_prog,
            &mid_prog,
            GroupReplyStatus::Streaming,
            Some(partial),
            None,
            false,
          )
          .await;
      }
    });

    // 取消令牌：本轮执行期间登记，结束后注销，供前端「停止」按钮置位。
    // 注销由守卫接管：函数尾（正常、提前返回、panic）都注销。
    let cancel_flag = Arc::new(AtomicBool::new(false));
    {
      let mut locked = self.inner.lock().await;
      locked.external_turn_cancel.insert(
        (group_id.to_string(), msg_id.to_string()),
        Arc::clone(&cancel_flag),
      );
    }
    let _cancel_guard = ExternalTurnCancelGuard {
      mgr: Arc::clone(self),
      group_id: group_id.to_string(),
      msg_id: msg_id.to_string(),
    };

    let result = match agent.kind {
      ExternalAgentKind::OpenaiApi => {
        group_transport::execute_openai_api_turn(
          agent,
          &system_section,
          &user_section,
          &cancel_flag,
          |partial| {
            let _ = prog_tx.send(partial);
          },
        )
        .await
      }
      ExternalAgentKind::LocalCli => {
        group_transport::execute_local_cli_turn(agent, &full_prompt, &cancel_flag, |partial| {
          let _ = prog_tx.send(partial);
        })
        .await
      }
    };

    drop(prog_tx);
    let _ = fwd_handle.await;

    match result {
      Ok(final_text) => {
        self
          .update_group_message_state(
            app,
            group_id,
            msg_id,
            GroupReplyStatus::Done,
            Some(final_text.clone()),
            Some(None),
            true,
          )
          .await;
        Some(final_text)
      }
      Err(err_msg) => {
        self
          .update_group_message_state(
            app,
            group_id,
            msg_id,
            GroupReplyStatus::Error,
            None,
            Some(Some(err_msg)),
            true,
          )
          .await;
        None
      }
    }
  }

  #[cfg(desktop)]
  pub(super) async fn wake_and_settle_group_webview(wv: &tauri::Webview) {
    let wake_js = r#"(() => {
      try {
        window.dispatchEvent(new Event('focus'));
        document.dispatchEvent(new Event('visibilitychange'));
        window.dispatchEvent(new PageTransitionEvent('pageshow', { persisted: true }));
        const txt = (document.body && document.body.innerText) || '';
        const connecting = /(?:正在连接|连接中|Connecting|Reconnecting|离线|重连中)/i.test(txt.slice(0, 600));
        return JSON.stringify({ connecting });
      } catch (_) {
        return JSON.stringify({ connecting: false });
      }
    })()"#;
    let first = Self::eval_json_callback(wv, wake_js).await;
    tokio::time::sleep(std::time::Duration::from_millis(380)).await;
    if first
      .and_then(|v| v.get("connecting").and_then(|b| b.as_bool()))
      .unwrap_or(false)
    {
      for _ in 0..7 {
        tokio::time::sleep(std::time::Duration::from_millis(450)).await;
        let still = Self::eval_json_callback(wv, wake_js)
          .await
          .and_then(|v| v.get("connecting").and_then(|b| b.as_bool()))
          .unwrap_or(false);
        if !still {
          break;
        }
      }
    }
  }

  #[cfg(desktop)]
  pub(super) async fn discard_hung_group_webview(
    &self,
    app: &AppHandle,
    group_id: &str,
    key: &AccountKey,
  ) {
    let mut locked = self.inner.lock().await;
    locked
      .group_context_synced
      .remove(&(group_id.to_string(), key.clone()));
    if locked.active_slot.as_ref().map(|s| &s.account_key) != Some(key) {
      if let Some(entry) = locked.warm_pool.remove(key) {
        Self::close_warm_entry(app, entry);
      }
    }
  }

  #[cfg(desktop)]
  pub(super) async fn try_attach_file_in_webview(
    wv: &tauri::Webview,
    adapter: &SiteAdapterConfig,
    attachment: &GroupAttachment,
  ) -> bool {
    let attach_js = build_group_attach_file_js(adapter, attachment);
    let dispatched = Self::eval_json_callback(wv, &attach_js)
      .await
      .and_then(|v| v.get("dispatched").and_then(|b| b.as_bool()))
      .unwrap_or(false);
    if !dispatched {
      return false;
    }
    let check_js = build_group_check_attached_js(adapter, &attachment.filename);
    for _ in 0..6 {
      tokio::time::sleep(std::time::Duration::from_millis(400)).await;
      if Self::eval_json_callback(wv, &check_js)
        .await
        .and_then(|v| v.get("ready").and_then(|b| b.as_bool()))
        .unwrap_or(false)
      {
        return true;
      }
    }
    false
  }

  #[cfg(desktop)]
  pub(super) async fn extract_hark_expanded_card_if_needed(
    &self,
    wv: &tauri::Webview,
    preview_text: &str,
  ) -> String {
    let click_js = build_hark_click_preview_card_js(preview_text);
    let Some(click_res) = Self::eval_json_callback(wv, &click_js).await else {
      return preview_text.to_string();
    };
    if !click_res
      .get("clicked")
      .and_then(|v| v.as_bool())
      .unwrap_or(false)
    {
      return preview_text.to_string();
    }

    let intro_prefix = click_res
      .get("introPrefix")
      .and_then(|v| v.as_str())
      .unwrap_or("")
      .trim()
      .to_string();
    let card_preview = click_res
      .get("cardPreview")
      .and_then(|v| v.as_str())
      .unwrap_or(preview_text)
      .trim()
      .to_string();

    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    let read_js = build_hark_read_expanded_js(&card_preview);
    let mut best_expanded = String::new();
    let mut stable_hits = 0usize;
    for _ in 0..8 {
      if let Some(res) = Self::eval_json_callback(wv, &read_js).await {
        let cur = res
          .get("text")
          .and_then(|v| v.as_str())
          .map(Self::clean_extracted_reply_text)
          .unwrap_or_default();
        if cur.chars().count() > card_preview.chars().count() + 24 {
          if cur == best_expanded {
            stable_hits += 1;
            if stable_hits >= 1 {
              break;
            }
          } else {
            best_expanded = cur;
            stable_hits = 0;
          }
        }
      }
      tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    }

    let _ = Self::eval_json_callback(wv, &build_hark_close_expanded_js()).await;
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;

    if best_expanded.chars().count() <= card_preview.chars().count() + 24 {
      return preview_text.to_string();
    }
    if intro_prefix.is_empty() || best_expanded.starts_with(&intro_prefix) {
      best_expanded
    } else {
      format!("{intro_prefix}\n\n{best_expanded}")
    }
  }

  #[cfg(desktop)]
  #[allow(clippy::too_many_arguments)]
  pub(super) async fn execute_webview_member_turn(
    self: &Arc<Self>,
    app: &AppHandle,
    group_id: &str,
    member_key: &AccountKey,
    msg_id: &str,
    user_msg_id: &str,
    raw_user_content: &str,
    stage_instruction: Option<&str>,
    fallback_stage_instruction: Option<&str>,
    attachment: Option<&GroupAttachment>,
    hop_depth: u8,
  ) -> Option<String> {
    if !self
      .update_group_message_state(
        app,
        group_id,
        msg_id,
        GroupReplyStatus::Sending,
        None,
        Some(None),
        false,
      )
      .await
    {
      return None;
    }

    let (mut wv, mut view_instance_id) = match self
      .ensure_group_member_webview_ready(app, member_key)
      .await
    {
      Ok(pair) => pair,
      Err(err) => {
        self
          .update_group_message_state(
            app,
            group_id,
            msg_id,
            GroupReplyStatus::Error,
            Some(String::new()),
            Some(Some(err)),
            true,
          )
          .await;
        return None;
      }
    };

    self
      .prepare_group_member_thread(group_id, member_key, &wv)
      .await;

    for retry_round in 0..2usize {
      Self::wake_and_settle_group_webview(&wv).await;

      let (_, _, prompt_text, fingerprint) = self
        .build_turn_prompt_for_member(
          group_id,
          member_key,
          view_instance_id,
          msg_id,
          user_msg_id,
          raw_user_content,
          stage_instruction,
          hop_depth,
        )
        .await?;

      let (adapter, pre_send_norm) = {
        let locked = self.inner.lock().await;
        let app_data_dir = locked.app_data_dir.clone();
        let url_opt = wv.url().ok();
        let host = url_opt
          .as_ref()
          .and_then(|u| u.host_str())
          .unwrap_or("")
          .to_string();
        let pre = url_opt.as_ref().map(Self::normalize_url_for_thread);
        (
          site_adapters::resolve_adapter_for_host(&app_data_dir, &host),
          pre,
        )
      };

      let mut effective_stage_instruction = stage_instruction;
      let mut effective_prompt_text = prompt_text;
      if let Some(att) = attachment {
        let attached_ok = Self::try_attach_file_in_webview(&wv, &adapter, att).await;
        if !attached_ok {
          if let Some(fb_stage) = fallback_stage_instruction {
            effective_stage_instruction = Some(fb_stage);
            if let Some((_, _, fb_prompt, _)) = self
              .build_turn_prompt_for_member(
                group_id,
                member_key,
                view_instance_id,
                msg_id,
                user_msg_id,
                raw_user_content,
                effective_stage_instruction,
                hop_depth,
              )
              .await
            {
              effective_prompt_text = fb_prompt;
            }
          }
        }
      }

      let mut send_js = build_group_send_js(&adapter, msg_id, &effective_prompt_text);
      let mut send_res_opt = None;
      let mut send_eval_misses = 0usize;
      let mut recovered_dead_wv = false;
      for attempt in 0..24 {
        let send_res = match Self::eval_json_callback(&wv, &send_js).await {
          Some(v) => {
            send_eval_misses = 0;
            v
          }
          None => {
            send_eval_misses += 1;
            if !recovered_dead_wv
              && (app.get_webview(wv.label()).is_none() || send_eval_misses >= 4)
            {
              recovered_dead_wv = true;
              send_eval_misses = 0;
              self
                .discard_hung_group_webview(app, group_id, member_key)
                .await;
              if let Ok((new_wv, new_vid)) = self
                .ensure_group_member_webview_ready(app, member_key)
                .await
              {
                wv = new_wv;
                view_instance_id = new_vid;
                Self::wake_and_settle_group_webview(&wv).await;
                if let Some((_, _, rebuilt, _)) = self
                  .build_turn_prompt_for_member(
                    group_id,
                    member_key,
                    view_instance_id,
                    msg_id,
                    user_msg_id,
                    raw_user_content,
                    effective_stage_instruction,
                    hop_depth,
                  )
                  .await
                {
                  send_js = build_group_send_js(&adapter, msg_id, &rebuilt);
                }
                continue;
              }
            }
            if app.get_webview(wv.label()).is_none() || send_eval_misses >= 6 || attempt == 23 {
              self
                .update_group_message_state(
                  app,
                  group_id,
                  msg_id,
                  GroupReplyStatus::Error,
                  Some(String::new()),
                  Some(Some("无法向该账号页面注入消息脚本".to_string())),
                  true,
                )
                .await;
              return None;
            }
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            continue;
          }
        };
        let ok = send_res
          .get("ok")
          .and_then(|v| v.as_bool())
          .unwrap_or(false);
        let status_str = send_res
          .get("status")
          .and_then(|v| v.as_str())
          .unwrap_or("error");
        if ok || status_str != "no_composer" || attempt == 23 {
          send_res_opt = Some(send_res);
          break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
      }
      let send_res = send_res_opt?;

      if !send_res
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
      {
        let status_str = send_res
          .get("status")
          .and_then(|v| v.as_str())
          .unwrap_or("error");
        let err_msg = send_res
          .get("error")
          .and_then(|v| v.as_str())
          .unwrap_or("向该账号发送消息失败")
          .to_string();
        let final_status = if status_str == "logged_out" || status_str == "needs_auth" {
          self
            .apply_logged_out_state(app, member_key.clone(), view_instance_id)
            .await;
          GroupReplyStatus::NeedsAuth
        } else if status_str == "page_error" {
          self
            .apply_page_error_state(app, member_key.clone(), view_instance_id, err_msg.clone())
            .await;
          GroupReplyStatus::Error
        } else {
          GroupReplyStatus::Error
        };
        self
          .update_group_message_state(
            app,
            group_id,
            msg_id,
            final_status,
            Some(String::new()),
            Some(Some(err_msg)),
            true,
          )
          .await;
        return None;
      }

      {
        let mut locked = self.inner.lock().await;
        locked.group_context_synced.insert(
          (group_id.to_string(), member_key.clone()),
          (view_instance_id, fingerprint),
        );
      }

      if !self
        .update_group_message_state(
          app,
          group_id,
          msg_id,
          GroupReplyStatus::Streaming,
          Some(String::new()),
          Some(None),
          false,
        )
        .await
      {
        return None;
      }

      let poll_js = build_group_poll_js(&adapter, msg_id);
      let mut last_pushed_text = String::new();
      let mut last_pushed_is_transient = false;
      let mut poll_eval_misses = 0usize;
      let mut trigger_reload_retry = false;
      for poll_round in 0..420 {
        tokio::time::sleep(std::time::Duration::from_millis(450)).await;
        // 群聊轮询期间定期拉取自动审批上报（约每 9 秒一次）：熔断在
        // 群聊进行中触发时，不必等保活同步周期才关闭已加载页面的自动点击。
        // drain 本身只做一次 JS eval，无上报时立即返回，开销可忽略。
        if poll_round % 20 == 0 {
          self.drain_auto_approve_report(app, member_key, &wv).await;
        }
        let res = match Self::eval_json_callback(&wv, &poll_js).await {
          Some(v) => {
            poll_eval_misses = 0;
            v
          }
          None => {
            poll_eval_misses += 1;
            if app.get_webview(wv.label()).is_some() && poll_eval_misses < 6 {
              continue;
            }
            if retry_round == 0 && last_pushed_text.is_empty() {
              trigger_reload_retry = true;
              break;
            }
            self
              .update_group_message_state(
                app,
                group_id,
                msg_id,
                GroupReplyStatus::Error,
                Some(if last_pushed_is_transient {
                  String::new()
                } else {
                  last_pushed_text
                }),
                Some(Some("页面视图已关闭或刷新，连接中断".to_string())),
                true,
              )
              .await;
            return None;
          }
        };

        let state = res
          .get("state")
          .and_then(|v| v.as_str())
          .unwrap_or("streaming");
        let is_transient = res
          .get("transient")
          .and_then(|v| v.as_bool())
          .unwrap_or(false);
        let raw_text = res
          .get("text")
          .and_then(|v| v.as_str())
          .unwrap_or("")
          .trim()
          .to_string();
        let text = if is_transient {
          raw_text
        } else {
          Self::clean_extracted_reply_text(&raw_text)
        };
        let err_opt = res
          .get("error")
          .and_then(|v| v.as_str())
          .map(|s| s.trim().to_string())
          .filter(|s| !s.is_empty());

        match state {
          "stalled_retry" if retry_round == 0 => {
            trigger_reload_retry = true;
            break;
          }
          "stalled_retry" => {
            self
              .update_group_message_state(
                app,
                group_id,
                msg_id,
                GroupReplyStatus::Error,
                Some(String::new()),
                Some(Some(
                  "发送后页面仍在发送中未发出，请切换至该账号页面检查网络后重试".to_string(),
                )),
                true,
              )
              .await;
            return None;
          }
          "done" => {
            let mut final_text = if !text.is_empty() {
              text
            } else if !last_pushed_is_transient {
              last_pushed_text
            } else {
              String::new()
            };
            if adapter.id == "hark" && !final_text.trim().is_empty() {
              final_text = self
                .extract_hark_expanded_card_if_needed(&wv, &final_text)
                .await;
            }
            self
              .update_group_message_state(
                app,
                group_id,
                msg_id,
                GroupReplyStatus::Done,
                Some(final_text.clone()),
                Some(None),
                true,
              )
              .await;
            self
              .persist_group_thread_binding(
                app,
                group_id,
                member_key,
                pre_send_norm.as_deref(),
                &wv,
                &adapter,
              )
              .await;
            return Some(final_text);
          }
          "needs_auth" => {
            let final_text = if !text.is_empty() {
              text
            } else if !last_pushed_is_transient {
              last_pushed_text
            } else {
              String::new()
            };
            self
              .update_group_message_state(
                app,
                group_id,
                msg_id,
                GroupReplyStatus::NeedsAuth,
                Some(final_text),
                Some(Some(err_opt.unwrap_or_else(|| {
                  "检测到授权确认弹窗，请前往该账号页面点击允许".to_string()
                }))),
                true,
              )
              .await;
            return None;
          }
          "error" => {
            let final_text = if !text.is_empty() {
              text
            } else if !last_pushed_is_transient {
              last_pushed_text
            } else {
              String::new()
            };
            self
              .update_group_message_state(
                app,
                group_id,
                msg_id,
                GroupReplyStatus::Error,
                Some(final_text),
                Some(Some(err_opt.unwrap_or_else(|| "生成回复失败".to_string()))),
                true,
              )
              .await;
            return None;
          }
          _ => {
            if !is_transient && !text.is_empty() && text != last_pushed_text {
              last_pushed_text = text.clone();
              last_pushed_is_transient = false;
              if !self
                .update_group_message_state(
                  app,
                  group_id,
                  msg_id,
                  GroupReplyStatus::Streaming,
                  Some(text),
                  None,
                  false,
                )
                .await
              {
                return None;
              }
            }
          }
        }
      }

      if trigger_reload_retry {
        {
          let mut locked = self.inner.lock().await;
          locked
            .group_context_synced
            .remove(&(group_id.to_string(), member_key.clone()));
        }
        let _ = self
          .update_group_message_state(
            app,
            group_id,
            msg_id,
            GroupReplyStatus::Sending,
            Some(String::new()),
            Some(None),
            false,
          )
          .await;
        if app.get_webview(wv.label()).is_some() {
          let _ = wv.eval("window.location.reload()");
          Self::wait_webview_settled(&wv, 18).await;
          tokio::time::sleep(std::time::Duration::from_millis(900)).await;
        } else {
          self
            .discard_hung_group_webview(app, group_id, member_key)
            .await;
          if let Ok((new_wv, new_vid)) = self
            .ensure_group_member_webview_ready(app, member_key)
            .await
          {
            wv = new_wv;
            view_instance_id = new_vid;
          } else {
            break;
          }
        }
        continue;
      }

      if !last_pushed_text.is_empty() && !last_pushed_is_transient {
        let final_text = if adapter.id == "hark" {
          self
            .extract_hark_expanded_card_if_needed(&wv, &last_pushed_text)
            .await
        } else {
          last_pushed_text
        };
        self
          .update_group_message_state(
            app,
            group_id,
            msg_id,
            GroupReplyStatus::Done,
            Some(final_text.clone()),
            Some(None),
            true,
          )
          .await;
        self
          .persist_group_thread_binding(
            app,
            group_id,
            member_key,
            pre_send_norm.as_deref(),
            &wv,
            &adapter,
          )
          .await;
        return Some(final_text);
      }
      break;
    }

    self
      .update_group_message_state(
        app,
        group_id,
        msg_id,
        GroupReplyStatus::Error,
        Some(String::new()),
        Some(Some("等待回复超时，请切换至该账号页面查看详情".to_string())),
        true,
      )
      .await;
    None
  }
}
