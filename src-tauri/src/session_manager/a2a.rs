//! A2A relay + review pipeline (P2-11 split).
//!
//! Moved verbatim from `session_manager.rs`: the agent-to-agent relay chain
//! (`select_a2a_relay_targets`, `run_group_a2a_chain`) and the review
//! pipeline (`run_group_review_pipeline`) that chains group members so each
//! next member builds on the previous replies. No behavior change. Methods
//! are `pub(super)` so the parent module (group-chat engine) keeps calling
//! them via `Self::` / `self.` — methods belong to the type, not the file.

use super::SessionManager;
#[cfg(desktop)]
use super::{GroupAttachment, REVIEW_FALLBACK_CHAR_LIMIT, REVIEW_INLINE_CHAR_LIMIT};
use crate::providers::{resolve_provider, ProviderId};
use crate::storage::{
  now_ms, AccountKey, AuthState, ExternalAgentKind, GroupChatMessage, GroupReplyStatus,
  GroupWorkflowMode,
};
use std::sync::Arc;
use tauri::AppHandle;
use uuid::Uuid;

/// A2A 接力在途登记 Drop 守卫：子链结束（正常、提前返回、panic）
/// 都从 `a2a_relay_in_flight` 释放该目标。复用 group_chat 的
/// `cleanup_inner_on_drop`（try_lock 快路径，抢不到锁时派生异步清理）。
#[cfg(desktop)]
struct A2aInFlightGuard {
  mgr: Arc<SessionManager>,
  group_id: String,
  target: AccountKey,
}

#[cfg(desktop)]
impl Drop for A2aInFlightGuard {
  fn drop(&mut self) {
    let group_id = self.group_id.clone();
    let target = self.target.clone();
    super::group_chat::cleanup_inner_on_drop(&self.mgr, move |locked| {
      if let Some(set) = locked.a2a_relay_in_flight.get_mut(&group_id) {
        set.remove(&target);
      }
    });
  }
}

/// 评审管线一个阶段的任务：(阶段序号, 成员, 消息 id, 阶段名, 自动通道)。
/// 别名只为压住 clippy::type_complexity（元组本身不复杂到值得拆 struct）。
/// 仅桌面评审管线使用（run_group_review_pipeline 是 #[cfg(desktop)]）。
#[cfg(desktop)]
pub(super) type ReviewJob = (
  usize,
  AccountKey,
  String,
  Option<String>,
  Option<ProviderId>,
);

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
    jobs: Vec<ReviewJob>,
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
        // 截断体（外部成员与附件兜底共用）：超长时截到 4500 字并标明已略。
        let truncated_body: String = full_prev_summary
          .chars()
          .take(REVIEW_FALLBACK_CHAR_LIMIT)
          .collect();
        let fallback_body = if char_count > REVIEW_FALLBACK_CHAR_LIMIT {
          format!("{truncated_body}\n...(后续超长内容已略)")
        } else {
          truncated_body
        };
        if char_count <= REVIEW_INLINE_CHAR_LIMIT {
          (format_with_body(&full_prev_summary), None, None)
        } else if is_external {
          // 外部成员读不到附件：不再无上限内联，截断后直接放进正文。
          (format_with_body(&fallback_body), None, None)
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

      // 评审管线点名本地 CLI 成员时默认不自动执行：前序阶段的回复是
      // 模型生成文本，可能含注入内容；本地命令执行需用户确认。
      // 该阶段以 Error 状态挂起并中止后续阶段，用户点「重试」即确认执行。
      // 第 0 阶段（方案初稿）除外：输入完全是用户自己写的，没有前序
      // AI 输出可供注入，用户把 LocalCli 排第一棒时不应整条评审报错。
      let is_local_cli = {
        let locked = self.inner.lock().await;
        locked
          .store
          .external_agents
          .iter()
          .any(|e| e.account_key() == *member_key && e.kind == ExternalAgentKind::LocalCli)
      };
      if *idx > 0 && is_local_cli {
        let alias = {
          let locked = self.inner.lock().await;
          Self::resolve_member_alias_locked(&locked, member_key)
            .unwrap_or_else(|| member_key.uuid.chars().take(6).collect())
        };
        {
          let mut locked = self.inner.lock().await;
          locked.notice =
            Some("评审已暂停：有本地 CLI 成员需你确认后点「重试」才会执行本地命令".to_string());
        }
        self
          .update_group_message_state(
            &app,
            &group_id,
            msg_id,
            GroupReplyStatus::Error,
            None,
            Some(Some(format!(
              "评审点名了本地 CLI 成员「{alias}」，为安全起见未自动执行本地命令。确认无误后点「重试」才会运行。"
            ))),
            true,
          )
          .await;
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
        // 跨链去重：并行跑着的两条接力链可能同时点名同一成员；已在
        // 途中的目标不再重复扇出。这里只过滤、不登记——登记放在下面
        // 与扇出成对发生，且只登记真正进入 spawned 的非 LocalCli
        // 目标（LocalCli 在下面 continue 掉、未进 spawned）。
        let next_targets: Vec<AccountKey> = {
          let in_flight = locked
            .a2a_relay_in_flight
            .entry(group_id.clone())
            .or_default();
          next_targets
            .into_iter()
            .filter(|k| !in_flight.contains(k))
            .collect()
        };
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
        let mut held_cli = 0usize;
        for (offset, target_key) in next_targets.into_iter().enumerate() {
          let alias = Self::resolve_member_alias_locked(&locked, &target_key)
            .unwrap_or_else(|| target_key.uuid.chars().take(6).collect());
          let reply_msg_id = Uuid::new_v4().hyphenated().to_string();
          // A2A 接力点名本地 CLI 成员时默认不自动执行：上一跳的回复是
          // 模型生成文本，可能含注入的 @标记；本地命令执行需用户确认。
          // 消息以 Error 状态挂起，用户点「重试」即确认执行（走
          // retry_group_message，对本轮原始提问重新跑该成员）。
          let is_local_cli = locked
            .store
            .external_agents
            .iter()
            .any(|e| e.account_key() == target_key && e.kind == ExternalAgentKind::LocalCli);
          if is_local_cli {
            held_cli += 1;
            locked.store.group_chats[g_idx].messages.push(GroupChatMessage {
              id: reply_msg_id,
              role: "assistant".to_string(),
              sender: Some(target_key.clone()),
              sender_name: alias.clone(),
              content: String::new(),
              created_at_ms: ts + (offset as u64) + 1,
              status: GroupReplyStatus::Error,
              targets: Vec::new(),
              error: Some(format!(
                "A2A 接力点名了本地 CLI 成员「{alias}」，为安全起见未自动执行本地命令。确认无误后点「重试」才会运行。"
              )),
              stage_label: Some(stage_label.clone()),
              hop_depth: next_hop,
              auto_routed_channel: None,
            });
            continue;
          }
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
        if held_cli > 0 {
          locked.notice =
            Some("A2A 接力已暂停：有本地 CLI 成员需你确认后点「重试」才会执行本地命令".to_string());
        }
        // 1A：接力链是后台任务，无调用方等写盘结果；锁内不再同步写盘，
        // 改走防抖。写盘失败不再提前返回——登记与扇出总是成对发生，
        // 不存在“已登记却未扇出”的泄漏；写线程失败会立 notice 并在
        // 下次请求时重试，内存里的接力消息不受影响。
        self.request_persist();
        // 登记在途目标：只登记真正进入 spawned 的（LocalCli 已在
        // 上面 continue 掉）。登记与下面的扇出成对发生，写盘走防抖
        // 不再阻塞本路径，故不再以写盘成功为前提。
        {
          let in_flight = locked
            .a2a_relay_in_flight
            .entry(group_id.clone())
            .or_default();
          for (target_key, _, _, _) in &spawned {
            in_flight.insert(target_key.clone());
          }
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
          tauri::async_runtime::spawn(async move {
            // Drop 守卫：回合结束（正常返回、提前返回与 panic）都释放
            // 该目标，避免在途登记泄漏导致该成员之后不再被接力点名。
            let _in_flight_guard = A2aInFlightGuard {
              mgr: Arc::clone(&mgr),
              group_id: gid.clone(),
              target: target_key.clone(),
            };
            mgr
              .run_group_a2a_chain(
                app_h,
                gid,
                target_key,
                new_msg_id,
                uid,
                relay_prompt,
                next_hop,
                next_visited,
              )
              .await;
          });
        }
      }
    })
  }
}
