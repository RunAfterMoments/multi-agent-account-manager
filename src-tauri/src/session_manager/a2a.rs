//! A2A relay + review pipeline (P2-11 split).
//!
//! Moved verbatim from `session_manager.rs`: the agent-to-agent relay chain
//! (`select_a2a_relay_targets`, `run_group_a2a_chain`) and the review
//! pipeline (`run_group_review_pipeline`) that chains group members so each
//! next member builds on the previous replies. No behavior change. Methods
//! are `pub(super)` so the parent module (group-chat engine) keeps calling
//! them via `Self::` / `self.` — methods belong to the type, not the file.

use super::{
  GroupAttachment, SessionManager, REVIEW_FALLBACK_CHAR_LIMIT, REVIEW_INLINE_CHAR_LIMIT,
};
use crate::providers::{resolve_provider, ProviderId};
use crate::storage::{
  now_ms, AccountKey, AuthState, GroupChatMessage, GroupReplyStatus, GroupWorkflowMode,
};
use std::sync::Arc;
use tauri::AppHandle;
use uuid::Uuid;

impl SessionManager {
  #[cfg_attr(not(desktop), allow(dead_code))]
  pub(super) fn select_a2a_relay_targets(
    reply_text: &str,
    speaker: &AccountKey,
    eligible_members: &[(AccountKey, String)],
    visited: &[AccountKey],
    max_per_hop: usize,
  ) -> Vec<AccountKey> {
    let (mention_all, mentioned) = Self::parse_at_mentions(reply_text, eligible_members);
    let candidates: Vec<AccountKey> = if mention_all {
      eligible_members.iter().map(|(k, _)| k.clone()).collect()
    } else {
      mentioned
    };
    candidates
      .into_iter()
      .filter(|k| k != speaker && !visited.contains(k))
      .take(max_per_hop)
      .collect()
  }

  #[cfg(desktop)]
  pub(super) async fn run_group_review_pipeline(
    self: Arc<Self>,
    app: AppHandle,
    group_id: String,
    user_msg_id: String,
    raw_user_content: String,
    jobs: Vec<(
      usize,
      AccountKey,
      String,
      Option<String>,
      Option<ProviderId>,
    )>,
  ) {
    let mut completed: Vec<(String, String)> = Vec::new();
    for (pos, (idx, member_key, msg_id, stage_label, auto_ch)) in jobs.iter().enumerate() {
      let is_hark = member_key.provider_id.is_hark();
      let is_external = member_key.provider_id.is_external();
      let (stage_inst, fallback_stage_inst, attachment) = if *idx == 0 {
        let s = if is_hark {
          "请针对「当前问题」给出清晰完整的初始方案与关键依据，请直接在回复正文完整输出内容，不要省略。".to_string()
        } else {
          "请针对「当前问题」给出清晰完整的初始方案与关键依据。".to_string()
        };
        (s, None, None)
      } else {
        let full_prev_summary = completed
          .iter()
          .enumerate()
          .map(|(i, (speaker, text))| {
            let clean_label = if is_hark {
              format!("草稿要点{}", i + 1)
            } else if speaker.contains('@') {
              format!("阶段{}初稿", i + 1)
            } else {
              format!("前稿（{speaker}）")
            };
            format!("### {clean_label}\n{}", text.trim())
          })
          .collect::<Vec<_>>()
          .join("\n\n");

        let format_with_body = |body: &str| -> String {
          if is_hark {
            if stage_label.as_deref() == Some("综合定稿") {
              format!(
                "请结合以下待完善方案与修改建议，为我整理输出最终可直接落地的完整结论，请直接在回复正文完整输出内容，不要省略：\n{body}"
              )
            } else {
              format!(
                "以下是一份待完善的方案要点，请直接为我指出其中的漏洞、边界风险与遗漏并给出具体修改建议，请直接在回复正文完整输出内容，不要省略：\n{body}"
              )
            }
          } else if stage_label.as_deref() == Some("综合定稿") {
            format!("请结合以下初稿与审阅意见，整合输出最终可直接落地的完整结论：\n{body}")
          } else {
            format!("请检查以下初稿中的漏洞、边界风险与遗漏，并给出具体修正建议：\n{body}")
          }
        };

        let char_count = full_prev_summary.chars().count();
        if is_external || char_count <= REVIEW_INLINE_CHAR_LIMIT {
          (format_with_body(&full_prev_summary), None, None)
        } else {
          let filename = format!("review-stage-{pos}.md");
          let attachment_note = if is_hark {
            if stage_label.as_deref() == Some("综合定稿") {
              format!(
                "请完整阅读我附带的 `{filename}` 文档（内含前期方案与修改建议），为我整理输出最终可直接落地的完整结论，请直接在回复正文完整输出内容，不要省略。"
              )
            } else {
              format!(
                "请完整阅读我附带的 `{filename}` 文档（内含待完善的方案要点），直接为我指出其中的漏洞、边界风险与遗漏并给出具体修改建议，请直接在回复正文完整输出内容，不要省略。"
              )
            }
          } else if stage_label.as_deref() == Some("综合定稿") {
            format!(
              "请完整阅读附件 `{filename}` 中的初稿与审阅意见，整合输出最终可直接落地的完整结论。"
            )
          } else {
            format!(
              "请完整阅读附件 `{filename}` 中的前期方案内容，检查其中的漏洞、边界风险与遗漏，并给出具体修正建议。"
            )
          };
          let truncated_body: String = full_prev_summary
            .chars()
            .take(REVIEW_FALLBACK_CHAR_LIMIT)
            .collect();
          let fallback_body = if char_count > REVIEW_FALLBACK_CHAR_LIMIT {
            format!("{truncated_body}\n...(后续超长内容已略)")
          } else {
            truncated_body
          };
          (
            attachment_note,
            Some(format_with_body(&fallback_body)),
            Some(GroupAttachment {
              filename,
              mime_type: "text/markdown".to_string(),
              content: full_prev_summary,
            }),
          )
        }
      };

      let reply_opt = self
        .run_group_member_turn_with_failover(
          &app,
          &group_id,
          member_key,
          msg_id,
          &user_msg_id,
          &raw_user_content,
          Some(&stage_inst),
          fallback_stage_inst.as_deref(),
          attachment.as_ref(),
          0,
          auto_ch.as_ref(),
        )
        .await;

      if let Some((actual_key, reply_text)) = reply_opt {
        let alias = {
          let locked = self.inner.lock().await;
          Self::resolve_member_alias_locked(&locked, &actual_key)
            .unwrap_or_else(|| actual_key.uuid.chars().take(6).collect())
        };
        completed.push((alias, reply_text));
      } else {
        for (_, _, rem_msg_id, _, _) in &jobs[pos + 1..] {
          self
            .update_group_message_state(
              &app,
              &group_id,
              rem_msg_id,
              GroupReplyStatus::Error,
              None,
              Some(Some("前序评审阶段未完成，已中止后续流程".to_string())),
              true,
            )
            .await;
        }
        break;
      }
    }
  }

  #[cfg(desktop)]
  #[allow(clippy::too_many_arguments)]
  pub(super) fn run_group_a2a_chain(
    self: Arc<Self>,
    app: AppHandle,
    group_id: String,
    member_key: AccountKey,
    msg_id: String,
    user_msg_id: String,
    raw_user_content: String,
    hop_depth: u8,
    visited: Vec<AccountKey>,
  ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
      let stage_hint = if hop_depth > 0 {
        Some(format!("A2A 接力第 {hop_depth} 跳"))
      } else {
        None
      };
      let auto_ch = if hop_depth == 0 {
        let locked = self.inner.lock().await;
        locked
          .store
          .group_chats
          .iter()
          .find(|g| g.id == group_id)
          .and_then(|g| g.messages.iter().find(|m| m.id == msg_id))
          .and_then(|m| m.auto_routed_channel.as_deref())
          .and_then(ProviderId::parse)
      } else {
        None
      };
      let Some((actual_key, reply_text)) = self
        .run_group_member_turn_with_failover(
          &app,
          &group_id,
          &member_key,
          &msg_id,
          &user_msg_id,
          &raw_user_content,
          stage_hint.as_deref(),
          None,
          None,
          hop_depth,
          auto_ch.as_ref(),
        )
        .await
      else {
        return;
      };

      if reply_text.trim().is_empty() {
        return;
      }

      let mut active_visited = visited;
      if !active_visited.contains(&actual_key) {
        active_visited.push(actual_key.clone());
      }

      let next_hops = {
        let mut locked = self.inner.lock().await;
        let Some(g_idx) = locked
          .store
          .group_chats
          .iter()
          .position(|g| g.id == group_id)
        else {
          return;
        };
        if locked.store.group_chats[g_idx].workflow_mode != GroupWorkflowMode::A2aRelay
          || hop_depth >= locked.store.group_chats[g_idx].max_relay_hops.clamp(1, 3)
        {
          return;
        }
        let eligible_members: Vec<(AccountKey, String)> = locked.store.group_chats[g_idx]
          .members
          .iter()
          .filter_map(|m| {
            if m.provider_id.is_external() {
              locked
                .store
                .external_agents
                .iter()
                .find(|e| &e.account_key() == m)
                .map(|e| (m.clone(), e.name.clone()))
            } else {
              let now = now_ms();
              locked
                .store
                .accounts
                .iter()
                .find(|a| a.provider_id == m.provider_id && a.uuid == m.uuid)
                .filter(|a| {
                  a.auth_state != AuthState::LoggedOut
                    && a.cookie_expires_at_ms.is_none_or(|exp| exp > now)
                })
                .filter(|a| resolve_provider(&a.provider_id).is_some_and(|s| s.info.enabled))
                .map(|a| (m.clone(), a.alias.clone()))
            }
          })
          .collect();

        let next_targets = Self::select_a2a_relay_targets(
          &reply_text,
          &actual_key,
          &eligible_members,
          &active_visited,
          2,
        );
        if next_targets.is_empty() {
          return;
        }

        let speaker_alias = Self::resolve_member_alias_locked(&locked, &actual_key)
          .unwrap_or_else(|| actual_key.uuid.chars().take(6).collect());
        let next_hop = hop_depth + 1;
        let stage_label = format!("A2A 接力 #{next_hop}");
        let mut next_visited = active_visited.clone();
        for k in &next_targets {
          if !next_visited.contains(k) {
            next_visited.push(k.clone());
          }
        }

        let ts = now_ms();
        let snippet: String = reply_text.trim().chars().take(600).collect();
        let relay_prompt = format!("请接力回应上一跳「{speaker_alias}」的协作结果：\n{snippet}");
        let mut spawned = Vec::with_capacity(next_targets.len());
        for (offset, target_key) in next_targets.into_iter().enumerate() {
          let alias = Self::resolve_member_alias_locked(&locked, &target_key)
            .unwrap_or_else(|| target_key.uuid.chars().take(6).collect());
          let reply_msg_id = Uuid::new_v4().hyphenated().to_string();
          locked.store.group_chats[g_idx]
            .messages
            .push(GroupChatMessage {
              id: reply_msg_id.clone(),
              role: "assistant".to_string(),
              sender: Some(target_key.clone()),
              sender_name: alias,
              content: String::new(),
              created_at_ms: ts + (offset as u64) + 1,
              status: GroupReplyStatus::Queued,
              targets: Vec::new(),
              error: None,
              stage_label: Some(stage_label.clone()),
              hop_depth: next_hop,
              auto_routed_channel: None,
            });
          spawned.push((
            target_key,
            reply_msg_id,
            relay_prompt.clone(),
            next_visited.clone(),
          ));
        }
        if Self::persist_locked(&mut locked).await.is_err() {
          return;
        }
        let snap = Self::snapshot_from_locked(&locked);
        Some((snap, spawned, next_hop))
      };

      if let Some((snap, spawned, next_hop)) = next_hops {
        self.emit_snapshot(&app, &snap);
        for (target_key, new_msg_id, relay_prompt, next_visited) in spawned {
          let mgr = Arc::clone(&self);
          let app_h = app.clone();
          let gid = group_id.clone();
          let uid = user_msg_id.clone();
          tauri::async_runtime::spawn(mgr.run_group_a2a_chain(
            app_h,
            gid,
            target_key,
            new_msg_id,
            uid,
            relay_prompt,
            next_hop,
            next_visited,
          ));
        }
      }
    })
  }
}
