use crate::storage::{
  default_agent_idle_timeout_secs, default_agent_timeout_secs, normalize_uuid, now_ms, AccountKey,
  ExternalAgentKind, ExternalAgentRecord,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use url::Url;
use uuid::Uuid;

#[cfg(desktop)]
use serde_json::Value;
#[cfg(desktop)]
use std::io::{Read, Write};
#[cfg(desktop)]
use std::process::{Command, Stdio};
#[cfg(desktop)]
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ResolvedMemberTransport {
  WebView { key: AccountKey },
  OpenAiApi { agent: ExternalAgentRecord },
  LocalCli { agent: ExternalAgentRecord },
}

#[allow(clippy::too_many_arguments)]
pub fn normalize_external_agent_input(
  id: Option<&str>,
  kind: ExternalAgentKind,
  name: &str,
  base_url: &str,
  api_key: &str,
  model: &str,
  command: &str,
  args: Vec<String>,
  workdir: Option<&str>,
  timeout_secs: Option<u64>,
  idle_timeout_secs: Option<u64>,
  created_at_ms: Option<u64>,
) -> Result<ExternalAgentRecord, String> {
  let clean_id = match id.map(str::trim).filter(|s| !s.is_empty()) {
    Some(raw) => normalize_uuid(raw).ok_or_else(|| "外部成员 ID 格式非法".to_string())?,
    None => Uuid::new_v4().hyphenated().to_string(),
  };
  let clean_name: String = name.trim().chars().take(40).collect();
  if clean_name.is_empty() {
    return Err("成员名称不能为空".to_string());
  }
  let clean_timeout_secs = timeout_secs.unwrap_or_else(default_agent_timeout_secs);
  if !(30..=3600).contains(&clean_timeout_secs) {
    return Err("总超时需在 30–3600 秒之间".to_string());
  }
  let clean_idle_timeout_secs = idle_timeout_secs.unwrap_or_else(default_agent_idle_timeout_secs);
  if !(10..=600).contains(&clean_idle_timeout_secs) {
    return Err("空闲超时需在 10–600 秒之间".to_string());
  }

  match kind {
    ExternalAgentKind::OpenaiApi => {
      let raw_url = base_url.trim();
      if raw_url.is_empty() {
        return Err("API Base URL 不能为空".to_string());
      }
      let candidate = if raw_url.contains("://") {
        raw_url.to_string()
      } else {
        format!("https://{raw_url}")
      };
      let parsed = Url::parse(&candidate).map_err(|_| "API Base URL 格式无效".to_string())?;
      let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
      let is_local_http = parsed.scheme() == "http"
        && matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]" | "::1");
      if parsed.scheme() != "https" && !is_local_http {
        return Err("API Base URL 仅支持 https:// 或本机 http://localhost".to_string());
      }
      if host.is_empty() {
        return Err("API Base URL 缺少有效主机名".to_string());
      }
      let clean_model: String = model.trim().chars().take(120).collect();
      if clean_model.is_empty() {
        return Err("请填写模型名称（Model）".to_string());
      }
      Ok(ExternalAgentRecord {
        id: clean_id.clone(),
        kind,
        name: clean_name,
        base_url: parsed.to_string().trim_end_matches('/').to_string(),
        api_key: api_key.trim().chars().take(512).collect(),
        api_key_ref: Some(crate::key_vault::api_key_ref_for(&clean_id)),
        model: clean_model,
        command: String::new(),
        args: Vec::new(),
        workdir: None,
        timeout_secs: clean_timeout_secs,
        idle_timeout_secs: clean_idle_timeout_secs,
        created_at_ms: created_at_ms.unwrap_or_else(now_ms),
      })
    }
    ExternalAgentKind::LocalCli => {
      let clean_cmd = command.trim();
      if clean_cmd.is_empty() {
        return Err("CLI 命令不能为空（例如 claude 或 codex）".to_string());
      }
      if clean_cmd.contains('\n') || clean_cmd.contains('\r') || clean_cmd.contains('\0') {
        return Err("CLI 命令包含非法控制字符".to_string());
      }
      if clean_cmd.chars().count() > 260 {
        return Err("CLI 命令长度不能超过 260 个字符".to_string());
      }
      let clean_args: Vec<String> = args
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(24)
        .collect();
      let clean_workdir = workdir
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(260).collect());
      Ok(ExternalAgentRecord {
        id: clean_id,
        kind,
        name: clean_name,
        base_url: String::new(),
        api_key: String::new(),
        api_key_ref: None,
        model: String::new(),
        command: clean_cmd.to_string(),
        args: clean_args,
        workdir: clean_workdir,
        timeout_secs: clean_timeout_secs,
        idle_timeout_secs: clean_idle_timeout_secs,
        created_at_ms: created_at_ms.unwrap_or_else(now_ms),
      })
    }
  }
}

#[cfg_attr(not(desktop), allow(dead_code))]
pub fn build_openai_endpoint(base_url: &str) -> String {
  let clean = base_url.trim().trim_end_matches('/');
  if clean.ends_with("/chat/completions") {
    clean.to_string()
  } else {
    format!("{clean}/chat/completions")
  }
}

#[cfg_attr(not(desktop), allow(dead_code))]
pub fn extract_openai_sse_delta(line: &str) -> Option<String> {
  #[cfg(desktop)]
  {
    let trimmed = line.trim();
    let payload = trimmed.strip_prefix("data:")?.trim();
    if payload.is_empty() || payload == "[DONE]" {
      return None;
    }
    let parsed: Value = serde_json::from_str(payload).ok()?;
    let choice = parsed.get("choices")?.get(0)?;
    if let Some(text) = choice
      .get("delta")
      .and_then(|d| d.get("content"))
      .and_then(|v| v.as_str())
    {
      if !text.is_empty() {
        return Some(text.to_string());
      }
    }
    if let Some(text) = choice
      .get("message")
      .and_then(|m| m.get("content"))
      .and_then(|v| v.as_str())
    {
      if !text.is_empty() {
        return Some(text.to_string());
      }
    }
    if let Some(text) = choice.get("text").and_then(|v| v.as_str()) {
      if !text.is_empty() {
        return Some(text.to_string());
      }
    }
    None
  }
  #[cfg(not(desktop))]
  {
    let _ = line;
    None
  }
}

#[cfg_attr(not(desktop), allow(dead_code))]
pub fn build_cli_args_and_stdin(args: &[String], prompt: &str) -> (Vec<String>, Option<String>) {
  let uses_placeholder = args.iter().any(|a| a.contains("{prompt}"));
  if uses_placeholder {
    let resolved = args
      .iter()
      .map(|a| a.replace("{prompt}", prompt))
      .collect::<Vec<_>>();
    (resolved, None)
  } else {
    (args.to_vec(), Some(prompt.to_string()))
  }
}

/// 超时或断流后：有累积文本就返回累积文本并标注「回复被截断」，
/// 没有才把原始错误透传。
fn finish_api_turn(
  result: Result<String, String>,
  shared_partial: &Arc<std::sync::Mutex<String>>,
) -> Result<String, String> {
  match result {
    Ok(text) => Ok(text),
    Err(err) => {
      let partial = shared_partial
        .lock()
        .map(|guard| guard.trim().to_string())
        .unwrap_or_default();
      if partial.is_empty() {
        Err(err)
      } else {
        Ok(format!("{partial}\n\n（回复被截断）"))
      }
    }
  }
}

fn sync_shared_partial(shared: &Arc<std::sync::Mutex<String>>, text: &str) {
  if let Ok(mut guard) = shared.lock() {
    *guard = text.to_string();
  }
}

#[cfg(desktop)]
pub async fn execute_openai_api_turn<F>(
  agent: &ExternalAgentRecord,
  system_prompt: &str,
  user_prompt: &str,
  cancel_flag: &Arc<AtomicBool>,
  mut on_progress: F,
) -> Result<String, String>
where
  F: FnMut(String),
{
  // 总超时取成员配置（默认 600 秒 = 10 分钟），只做整轮上限；
  // 空闲超时由 client 的 read_timeout(60s) 负责。
  // 旧实现用 Client::builder().timeout(120s)，覆盖整个响应体，
  // 长流式回复会在 120 秒被切断。
  let total_timeout = Duration::from_secs(agent.timeout_secs);
  let shared_partial: Arc<std::sync::Mutex<String>> =
    Arc::new(std::sync::Mutex::new(String::new()));
  let inner = execute_openai_api_turn_inner(
    agent,
    system_prompt,
    user_prompt,
    cancel_flag,
    &shared_partial,
    &mut on_progress,
  );
  match tokio::time::timeout(total_timeout, inner).await {
    Ok(result) => finish_api_turn(result, &shared_partial),
    Err(_) => {
      let partial = shared_partial
        .lock()
        .map(|guard| guard.trim().to_string())
        .unwrap_or_default();
      if partial.is_empty() {
        Err(format!(
          "模型 API 回复超时（{} 秒），未收到任何内容",
          total_timeout.as_secs()
        ))
      } else {
        Ok(format!("{partial}\n\n（回复被截断）"))
      }
    }
  }
}

#[cfg(desktop)]
async fn execute_openai_api_turn_inner<F>(
  agent: &ExternalAgentRecord,
  system_prompt: &str,
  user_prompt: &str,
  cancel_flag: &Arc<AtomicBool>,
  shared_partial: &Arc<std::sync::Mutex<String>>,
  on_progress: &mut F,
) -> Result<String, String>
where
  F: FnMut(String),
{
  let endpoint = build_openai_endpoint(&agent.base_url);
  let client = reqwest::Client::builder()
    .connect_timeout(Duration::from_secs(15))
    .read_timeout(Duration::from_secs(60))
    .build()
    .map_err(|e| format!("初始化 API HTTP 客户端失败: {e}"))?;

  let mut messages = Vec::new();
  if !system_prompt.trim().is_empty() {
    messages.push(serde_json::json!({
      "role": "system",
      "content": system_prompt.trim()
    }));
  }
  messages.push(serde_json::json!({
    "role": "user",
    "content": user_prompt
  }));

  let body = serde_json::json!({
    "model": agent.model,
    "stream": true,
    "messages": messages,
  });

  let body_bytes = serde_json::to_vec(&body).map_err(|e| format!("序列化 API 请求失败: {e}"))?;
  let mut req = client
    .post(&endpoint)
    .header(reqwest::header::CONTENT_TYPE, "application/json")
    .body(body_bytes);
  if !agent.api_key.trim().is_empty() {
    req = req.bearer_auth(agent.api_key.trim());
  }

  let mut resp = req
    .send()
    .await
    .map_err(|e| format!("请求模型 API 失败: {e}"))?;

  let status = resp.status();
  if !status.is_success() {
    let raw_err = resp.text().await.unwrap_or_default();
    let msg = serde_json::from_str::<Value>(&raw_err)
      .ok()
      .and_then(|v| {
        v.get("error")
          .and_then(|e| e.get("message"))
          .and_then(|m| m.as_str())
          .map(|s| s.to_string())
      })
      .unwrap_or_else(|| raw_err.chars().take(240).collect());
    return Err(format!("HTTP {}: {}", status.as_u16(), msg.trim()));
  }

  let content_type = resp
    .headers()
    .get(reqwest::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .unwrap_or("")
    .to_ascii_lowercase();

  if !content_type.contains("text/event-stream") && content_type.contains("json") {
    let raw_json = resp
      .text()
      .await
      .map_err(|e| format!("读取模型响应失败: {e}"))?;
    let parsed: Value =
      serde_json::from_str(&raw_json).map_err(|e| format!("解析模型 JSON 失败: {e}"))?;
    let reply = parsed
      .get("choices")
      .and_then(|c| c.get(0))
      .and_then(|c0| c0.get("message"))
      .and_then(|m| m.get("content"))
      .and_then(|v| v.as_str())
      .map(|s| s.trim().to_string())
      .filter(|s| !s.is_empty())
      .ok_or_else(|| "API 返回的 JSON 中缺少回复内容".to_string())?;
    return Ok(reply);
  }

  let mut accumulated = String::new();
  let mut line_buf: Vec<u8> = Vec::new();
  let mut raw_fallback: Vec<u8> = Vec::new();
  let mut last_emit = Instant::now() - Duration::from_secs(1);
  let mut cancelled = false;

  loop {
    // 取消令牌：chunk() 挂起时用 200ms 节拍轮询，最多 200ms 内注意到；
    // 流停滞超过 60 秒则由 read_timeout 触发断流（外层按截断返回已有文本）。
    if cancel_flag.load(Ordering::SeqCst) {
      cancelled = true;
      break;
    }
    let chunk = tokio::select! {
      res = resp.chunk() => match res {
        Ok(Some(chunk)) => chunk,
        Ok(None) => break,
        Err(e) => return Err(format!("流式读取 API 响应中断: {e}")),
      },
      _ = tokio::time::sleep(Duration::from_millis(200)) => continue,
    };
    if raw_fallback.len() < 16384 {
      let take = (16384 - raw_fallback.len()).min(chunk.len());
      raw_fallback.extend_from_slice(&chunk[..take]);
    }
    line_buf.extend_from_slice(&chunk);
    while let Some(pos) = line_buf.iter().position(|&b| b == b'\n') {
      let line_bytes: Vec<u8> = line_buf.drain(..=pos).collect();
      let line = String::from_utf8_lossy(&line_bytes);
      if let Some(delta) = extract_openai_sse_delta(&line) {
        accumulated.push_str(&delta);
      }
    }
    if !accumulated.is_empty() && last_emit.elapsed() >= Duration::from_millis(180) {
      last_emit = Instant::now();
      on_progress(accumulated.trim().to_string());
    }
    // 同步给外层的截断处理：总超时/断流时按此返回已有文本。
    sync_shared_partial(shared_partial, accumulated.trim());
  }

  if !line_buf.is_empty() {
    let line = String::from_utf8_lossy(&line_buf);
    if let Some(delta) = extract_openai_sse_delta(&line) {
      accumulated.push_str(&delta);
    }
  }

  if accumulated.trim().is_empty() && !raw_fallback.is_empty() {
    let text_fallback = String::from_utf8_lossy(&raw_fallback);
    if let Ok(parsed) = serde_json::from_str::<Value>(text_fallback.trim()) {
      if let Some(content) = parsed
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c0| c0.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
      {
        accumulated = content.to_string();
      }
    }
  }

  let final_text = accumulated.trim().to_string();
  sync_shared_partial(shared_partial, &final_text);
  if cancelled {
    // 手动停止：已有文本照常返回（群聊里保留已生成部分），没有才报错。
    if final_text.is_empty() {
      return Err("已手动停止模型 API，未获得回复".to_string());
    }
    return Ok(final_text);
  }
  if final_text.is_empty() {
    return Err("模型 API 未返回有效回复文本".to_string());
  }
  Ok(final_text)
}

/// macOS：用登录 shell 读 PATH（3 秒超时），结果缓存，失败返回 None。
/// Finder 启动的 GUI 应用拿不到 shell rc 里配的 PATH（如 nvm / volta /
/// bun），必须问一次登录 shell。仅 macOS 编译；Linux 上编译不到。
#[cfg(target_os = "macos")]
fn login_shell_path() -> Option<String> {
  static CACHED: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
  CACHED
    .get_or_init(|| {
      let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
      let mut child = std::process::Command::new(&shell)
        .args(["-ilc", "printf %s \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
      let deadline = std::time::Instant::now() + Duration::from_secs(3);
      loop {
        match child.try_wait() {
          Ok(Some(_)) => break,
          Ok(None) => {
            if std::time::Instant::now() >= deadline {
              let _ = child.kill();
              let _ = child.wait();
              return None;
            }
            std::thread::sleep(Duration::from_millis(50));
          }
          Err(_) => return None,
        }
      }
      let output = child.wait_with_output().ok()?;
      if !output.status.success() {
        return None;
      }
      let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
      if path.is_empty() {
        None
      } else {
        Some(path)
      }
    })
    .clone()
}

/// 合并 PATH：登录 shell 的 PATH 在最前（与登录 shell 的视角一致），
/// 再接当前进程的 PATH，最后补硬编码的常见目录；全程去重。
/// 与平台无关的纯函数，可在 Linux 上单测。
#[cfg(unix)]
fn merge_augmented_path(cur: &str, home: &str, shell_path: Option<&str>) -> String {
  let mut parts: Vec<String> = Vec::new();
  let mut push_unique = |part: &str| {
    let part = part.trim();
    if !part.is_empty() && !parts.iter().any(|p| p == part) {
      parts.push(part.to_string());
    }
  };
  if let Some(shell) = shell_path {
    for part in shell.split(':') {
      push_unique(part);
    }
  }
  for part in cur.split(':') {
    push_unique(part);
  }
  let mut extras = vec![
    "/opt/homebrew/bin".to_string(),
    "/usr/local/bin".to_string(),
    "/usr/bin".to_string(),
    "/bin".to_string(),
  ];
  if !home.is_empty() {
    extras.push(format!("{home}/.local/bin"));
    extras.push(format!("{home}/.npm-global/bin"));
    extras.push(format!("{home}/.cargo/bin"));
    // 登录 shell 读不到时（或 shell 本身没配）的兜底；nvm 的版本目录
    // 是通配符路径（~/.nvm/versions/node/*/bin），不硬编码，靠 shell PATH。
    extras.push(format!("{home}/.volta/bin"));
    extras.push(format!("{home}/.bun/bin"));
    extras.push(format!("{home}/Library/pnpm"));
    extras.push(format!("{home}/.claude/local/bin"));
  }
  for ex in extras {
    push_unique(&ex);
  }
  parts.join(":")
}

#[cfg(desktop)]
fn build_augmented_path() -> Option<String> {
  #[cfg(unix)]
  {
    let cur = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    #[cfg(target_os = "macos")]
    let shell_path = login_shell_path();
    #[cfg(not(target_os = "macos"))]
    let shell_path: Option<String> = None;
    Some(merge_augmented_path(&cur, &home, shell_path.as_deref()))
  }
  #[cfg(not(unix))]
  {
    None
  }
}

/// 增量 UTF-8 解码器：把任意切分的字节块拼成完整字符串。
/// 旧实现对每个 read 块直接 `String::from_utf8_lossy`，多字节字符（中文 3 字节）
/// 跨块时会被切成两半，变成 U+FFFD 乱码。这里只解出完整的有效前缀，
/// 把尾部不完整的多字节字符留在 `pending` 里等下一块；真正无效的字节
/// 才按单个 U+FFFD 输出并跳过。
#[derive(Debug, Default)]
pub struct Utf8ChunkDecoder {
  pending: Vec<u8>,
}

impl Utf8ChunkDecoder {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn push(&mut self, chunk: &[u8]) -> String {
    if chunk.is_empty() {
      return String::new();
    }
    self.pending.extend_from_slice(chunk);
    match std::str::from_utf8(&self.pending) {
      Ok(text) => {
        let out = text.to_string();
        self.pending.clear();
        out
      }
      Err(err) => {
        let valid = err.valid_up_to();
        let mut out = String::from_utf8_lossy(&self.pending[..valid]).into_owned();
        match err.error_len() {
          Some(len) => {
            // 确实无效的字节：输出一个 U+FFFD 并跳过该字节序列。
            out.push('\u{FFFD}');
            self.pending.drain(..valid + len);
          }
          None => {
            // 尾部是不完整的多字节字符，留给下一块补齐。
            self.pending.drain(..valid);
          }
        }
        out
      }
    }
  }

  pub fn finish(self) -> String {
    if self.pending.is_empty() {
      String::new()
    } else {
      String::from_utf8_lossy(&self.pending).into_owned()
    }
  }
}

#[cfg(desktop)]
enum CliStreamEvent {
  Stdout(Vec<u8>),
  Stderr(Vec<u8>),
  Finished(Result<Option<i32>, String>),
}

/// Unix：向整个子进程组发 SIGKILL。子进程以 `process_group(0)` 启动，
/// 自成组长，`sh -c` / `npx` 等包装脚本衍生的孙进程都在同一组内，
/// 只 `child.kill()` 杀不掉它们。
#[cfg(all(desktop, unix))]
pub(crate) fn kill_process_group(pid: u32) {
  unsafe {
    libc::killpg(pid as libc::pid_t, libc::SIGKILL);
  }
}

/// Windows：Job Object 句柄守卫。子进程挂入 Job 后，`TerminateJobObject`
/// 能一次杀掉整棵进程树；句柄随 watcher 线程结束自动关闭（不设
/// `KILL_ON_JOB_CLOSE`，正常退出不受影响）。
#[cfg(all(desktop, windows))]
struct CliJobGuard(windows::Win32::Foundation::HANDLE);

#[cfg(all(desktop, windows))]
impl Drop for CliJobGuard {
  fn drop(&mut self) {
    unsafe {
      let _ = windows::Win32::Foundation::CloseHandle(self.0);
    }
  }
}

/// 超时/取消错误里附带的部分输出：最多取前 500 字，避免错误消息过长。
fn format_timeout_partial(text: &str) -> String {
  let trimmed = text.trim();
  if trimmed.is_empty() {
    return String::new();
  }
  let taken: String = trimmed.chars().take(500).collect();
  if trimmed.chars().count() > 500 {
    format!("{taken}……")
  } else {
    taken
  }
}

/// 超时错误：附带已收集的部分输出，不再丢弃。
fn cli_timeout_error(partial: &str, limit: Duration, idle: bool) -> String {
  let secs = limit.as_secs();
  let collected = format_timeout_partial(partial);
  if collected.is_empty() {
    if idle {
      format!("本地 CLI 空闲超时（{secs} 秒无输出）")
    } else {
      format!("本地 CLI 执行超时（{secs} 秒）")
    }
  } else if idle {
    format!("本地 CLI 空闲超时（{secs} 秒无输出），已收集的部分输出：{collected}")
  } else {
    format!("本地 CLI 执行超时（{secs} 秒），已收集的部分输出：{collected}")
  }
}

/// 批处理垫片（.cmd/.bat）不能把 prompt 放进 argv：Rust ≥ 1.77 会拒绝
/// 含特殊字符（`"` / `%`）的批处理参数，prompt 只能走 stdin 或临时文件。
/// 与平台无关的纯检查，可在 Linux 上单测。
fn check_batch_prompt_conflict(is_batch: bool, uses_placeholder: bool) -> Result<(), String> {
  if is_batch && uses_placeholder {
    return Err(
      "该 CLI 是批处理垫片（.cmd/.bat），prompt 不能放在命令行参数里：\
      请去掉参数中的 {prompt}，改走 stdin（不带 {prompt} 时自动走 stdin）"
        .to_string(),
    );
  }
  Ok(())
}

/// Windows：解析 CLI 可执行文件。`Command::new("codex")` 只自动补 `.exe`，
/// npm 全局安装的 CLI 多为 `.cmd` 垫片，直接 spawn 会报找不到程序。
/// 这里自己遍历 PATH × PATHEXT，返回完整路径与是否为批处理垫片。
/// 仅 Windows 编译，Linux 上做不了单测，见 hardening-notes-cli.md 的手工步骤。
#[cfg(windows)]
fn resolve_cli_executable(command: &str) -> Result<(std::path::PathBuf, bool), String> {
  use std::path::{Path, PathBuf};

  fn is_batch(path: &Path) -> bool {
    matches!(
      path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref(),
      Some("cmd") | Some("bat")
    )
  }

  fn pathext() -> Vec<String> {
    std::env::var("PATHEXT")
      .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
      .split(';')
      .map(|s| s.trim().to_string())
      .filter(|s| !s.is_empty())
      .collect()
  }

  let has_separator = command.contains('/') || command.contains('\\');
  if has_separator {
    let path = PathBuf::from(command);
    if path.is_file() {
      let batch = is_batch(&path);
      return Ok((path, batch));
    }
    // 带路径但无扩展名时也按 PATHEXT 补全。
    if path.extension().is_none() {
      for ext in pathext() {
        let candidate = path.with_extension(ext.trim_start_matches('.'));
        if candidate.is_file() {
          let batch = is_batch(&candidate);
          return Ok((candidate, batch));
        }
      }
    }
    return Err(format!(
      "找不到本地 CLI「{command}」：请确认路径正确且文件存在"
    ));
  }

  let path_var = std::env::var("PATH").unwrap_or_default();
  for dir in path_var.split(';') {
    if dir.trim().is_empty() {
      continue;
    }
    let bare = Path::new(dir).join(command);
    if bare.is_file() {
      let batch = is_batch(&bare);
      return Ok((bare, batch));
    }
    for ext in pathext() {
      let candidate = Path::new(dir).join(format!("{command}{ext}"));
      if candidate.is_file() {
        let batch = is_batch(&candidate);
        return Ok((candidate, batch));
      }
    }
  }
  Err(format!(
    "找不到本地 CLI「{command}」：已按 PATH × PATHEXT 查找，请确认已安装并加入 PATH"
  ))
}

#[cfg(desktop)]
pub async fn execute_local_cli_turn<F>(
  agent: &ExternalAgentRecord,
  full_prompt: &str,
  cancel_flag: &Arc<AtomicBool>,
  mut on_progress: F,
) -> Result<String, String>
where
  F: FnMut(String),
{
  let (final_args, stdin_payload) = build_cli_args_and_stdin(&agent.args, full_prompt);
  // Windows：先解析可执行文件（.cmd 垫片需按 PATH × PATHEXT 查找），
  // 批处理垫片 + {prompt} 占位符时明确报错，提示改用 stdin。
  #[cfg(windows)]
  let (resolved_command, is_batch) = resolve_cli_executable(&agent.command)?;
  #[cfg(not(windows))]
  let (resolved_command, is_batch): (std::path::PathBuf, bool) =
    (std::path::PathBuf::from(&agent.command), false);
  let uses_placeholder = agent.args.iter().any(|a| a.contains("{prompt}"));
  check_batch_prompt_conflict(is_batch, uses_placeholder)?;
  let mut cmd = Command::new(&resolved_command);
  cmd.args(&final_args);
  if let Some(dir) = agent.workdir.as_deref().filter(|d| !d.trim().is_empty()) {
    cmd.current_dir(dir);
  }
  if let Some(path_val) = build_augmented_path() {
    cmd.env("PATH", path_val);
  }
  if stdin_payload.is_some() {
    cmd.stdin(Stdio::piped());
  } else {
    cmd.stdin(Stdio::null());
  }
  cmd.stdout(Stdio::piped());
  cmd.stderr(Stdio::piped());

  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW);
  }

  #[cfg(unix)]
  {
    // 自成进程组：超时/取消时 killpg 能杀掉包装脚本衍生的孙进程。
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
  }

  let mut child = cmd
    .spawn()
    .map_err(|e| format!("启动本地 CLI「{}」失败: {e}", agent.command))?;

  let child_pid = child.id();

  // Windows：启动后立即把子进程挂入 Job Object，超时/取消时一次杀整棵树。
  // 挂入失败（例如本进程已在某个 Job 里）时留空，杀灭时回退 taskkill /T /F。
  #[cfg(windows)]
  let job_guard: Option<CliJobGuard> = (|| {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};
    let job = unsafe { CreateJobObjectW(None, windows::core::PCWSTR::null()) }.ok()?;
    let proc = HANDLE(child.as_raw_handle() as isize);
    match unsafe { AssignProcessToJobObject(job, proc) } {
      Ok(()) => Some(CliJobGuard(job)),
      Err(_) => {
        unsafe {
          let _ = windows::Win32::Foundation::CloseHandle(job);
        }
        None
      }
    }
  })();

  if let (Some(mut stdin_handle), Some(input_text)) = (child.stdin.take(), stdin_payload) {
    std::thread::spawn(move || {
      let _ = stdin_handle.write_all(input_text.as_bytes());
      let _ = stdin_handle.flush();
    });
  }

  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<CliStreamEvent>();

  if let Some(mut stdout) = child.stdout.take() {
    let tx_out = tx.clone();
    std::thread::spawn(move || {
      let mut buf = [0u8; 4096];
      while let Ok(n) = stdout.read(&mut buf) {
        if n == 0 {
          break;
        }
        // 发原始字节：多字节字符可能跨块，解码由接收端增量完成。
        let _ = tx_out.send(CliStreamEvent::Stdout(buf[..n].to_vec()));
      }
    });
  }

  if let Some(mut stderr) = child.stderr.take() {
    let tx_err = tx.clone();
    std::thread::spawn(move || {
      let mut buf = [0u8; 2048];
      while let Ok(n) = stderr.read(&mut buf) {
        if n == 0 {
          break;
        }
        let _ = tx_err.send(CliStreamEvent::Stderr(buf[..n].to_vec()));
      }
    });
  }

  let (kill_tx, kill_rx) = std::sync::mpsc::channel::<()>();
  let tx_done = tx.clone();
  drop(tx);
  std::thread::spawn(move || loop {
    match child.try_wait() {
      Ok(Some(status)) => {
        let _ = tx_done.send(CliStreamEvent::Finished(Ok(status.code())));
        return;
      }
      Ok(None) => {
        if kill_rx.try_recv().is_ok() {
          #[cfg(unix)]
          kill_process_group(child_pid);
          #[cfg(windows)]
          {
            use windows::Win32::System::JobObjects::TerminateJobObject;
            match job_guard.as_ref() {
              Some(job) => unsafe {
                let _ = TerminateJobObject(job.0, 1);
              },
              None => {
                let _ = std::process::Command::new("taskkill")
                  .args(["/T", "/F", "/PID", &child_pid.to_string()])
                  .stdout(std::process::Stdio::null())
                  .stderr(std::process::Stdio::null())
                  .output();
              }
            }
          }
          let _ = child.kill();
          let _ = child.wait();
          let _ = tx_done.send(CliStreamEvent::Finished(Err(
            "本地 CLI 执行超时".to_string(),
          )));
          return;
        }
        std::thread::sleep(Duration::from_millis(80));
      }
      Err(err) => {
        let _ = tx_done.send(CliStreamEvent::Finished(Err(format!(
          "等待 CLI 进程退出失败: {err}"
        ))));
        return;
      }
    }
  });

  let mut stdout_acc = String::new();
  let mut stderr_acc = String::new();
  let mut stdout_decoder = Utf8ChunkDecoder::new();
  let mut stderr_decoder = Utf8ChunkDecoder::new();
  let mut exit_result: Option<Result<Option<i32>, String>> = None;
  let mut last_emit = Instant::now() - Duration::from_secs(1);
  // 总超时取成员配置（默认 600 秒）；另设空闲超时：N 秒无任何输出才判超时。
  // 范围已由 normalize / 迁移解析保证（总超时 30–3600 秒，空闲 10–600 秒）。
  let total_timeout = Duration::from_secs(agent.timeout_secs);
  let idle_timeout = Duration::from_secs(agent.idle_timeout_secs);
  let deadline = tokio::time::Instant::now() + total_timeout;
  let mut last_output = Instant::now();
  // 子进程退出后，读线程可能因继承了管道的孙进程（如 MCP server）
  // 迟迟不结束。收到 Finished 后只给约 500ms 排空，不再等管道关闭。
  let mut drain_deadline: Option<tokio::time::Instant> = None;
  let mut cancelled = false;

  loop {
    // 取消令牌：前端「停止」按钮置位后发信号杀整棵树，等 watcher 回报
    // Finished（最多 2 秒）再返回已有输出。
    if !cancelled && cancel_flag.load(Ordering::SeqCst) {
      cancelled = true;
      let _ = kill_tx.send(());
      drain_deadline = Some(tokio::time::Instant::now() + Duration::from_secs(2));
    }
    let now = tokio::time::Instant::now();
    if let Some(drain) = drain_deadline {
      if now >= drain {
        break;
      }
    }
    let remain = deadline.saturating_duration_since(now);
    if remain.is_zero() {
      let _ = kill_tx.send(());
      return Err(cli_timeout_error(&stdout_acc, total_timeout, false));
    }
    let idle_remain = idle_timeout.saturating_sub(last_output.elapsed());
    if idle_remain.is_zero() {
      let _ = kill_tx.send(());
      return Err(cli_timeout_error(&stdout_acc, idle_timeout, true));
    }
    // 等待取三者最小值，再封顶 200ms：取消令牌最多 200ms 内被注意到。
    let mut wait_for = remain.min(idle_remain);
    if let Some(drain) = drain_deadline {
      wait_for = wait_for.min(drain.saturating_duration_since(now));
    }
    wait_for = wait_for.min(Duration::from_millis(200));
    match tokio::time::timeout(wait_for, rx.recv()).await {
      Ok(Some(CliStreamEvent::Stdout(chunk))) => {
        last_output = Instant::now();
        stdout_acc.push_str(&stdout_decoder.push(&chunk));
        if last_emit.elapsed() >= Duration::from_millis(180) && !stdout_acc.trim().is_empty() {
          last_emit = Instant::now();
          on_progress(stdout_acc.trim().to_string());
        }
      }
      Ok(Some(CliStreamEvent::Stderr(chunk))) => {
        last_output = Instant::now();
        if stderr_acc.len() < 2048 {
          stderr_acc.push_str(&stderr_decoder.push(&chunk));
        }
      }
      Ok(Some(CliStreamEvent::Finished(res))) => {
        exit_result = Some(res);
        if cancelled {
          // 取消后杀灭已完成，直接返回已有输出，不再等排空。
          break;
        }
        if drain_deadline.is_none() {
          drain_deadline = Some(tokio::time::Instant::now() + Duration::from_millis(500));
        }
      }
      Ok(None) => break,
      Err(_) => {
        if drain_deadline.is_some() {
          // 排空窗口已过：子进程已退出，用已有输出返回。
          break;
        }
        // wait_for 到时：区分是总超时还是空闲超时（取消在循环顶部处理）。
        if last_output.elapsed() >= idle_timeout {
          let _ = kill_tx.send(());
          return Err(cli_timeout_error(&stdout_acc, idle_timeout, true));
        }
        if tokio::time::Instant::now() >= deadline {
          let _ = kill_tx.send(());
          return Err(cli_timeout_error(&stdout_acc, total_timeout, false));
        }
      }
    }
  }

  // 读线程结束时可能还留着尾部不完整的字节（子进程以半个多字节字符结尾），
  // 按 lossy 收尾，避免丢掉最后一个字符的前半部分。
  stdout_acc.push_str(&stdout_decoder.finish());
  stderr_acc.push_str(&stderr_decoder.finish());

  // 手动停止：已有输出照常返回（群聊里保留已生成部分），没有才报错。
  if cancelled {
    let partial = stdout_acc.trim().to_string();
    if partial.is_empty() {
      return Err("已手动停止本地 CLI，未获得输出".to_string());
    }
    return Ok(partial);
  }

  let final_out = stdout_acc.trim().to_string();
  match exit_result {
    Some(Err(timeout_err)) => Err(timeout_err),
    Some(Ok(Some(code))) if code != 0 && final_out.is_empty() => {
      let err_detail: String = stderr_acc.trim().chars().take(240).collect();
      if err_detail.is_empty() {
        Err(format!("本地 CLI 退出码 {code}，未返回内容"))
      } else {
        Err(format!("CLI 错误({code}): {err_detail}"))
      }
    }
    _ => {
      if final_out.is_empty() {
        let err_detail: String = stderr_acc.trim().chars().take(240).collect();
        if err_detail.is_empty() {
          Err("本地 CLI 未输出有效内容".to_string())
        } else {
          Err(err_detail)
        }
      } else {
        Ok(final_out)
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::{
    build_cli_args_and_stdin, build_openai_endpoint, extract_openai_sse_delta,
    normalize_external_agent_input, Utf8ChunkDecoder,
  };
  use crate::storage::ExternalAgentKind;

  #[test]
  fn utf8_chunk_decoder_reassembles_split_multibyte_chars() {
    // 「你好，世界」共 15 字节，中文每字 3 字节；切分点落在字中间时
    // 旧的逐块 from_utf8_lossy 会把半个字变成 U+FFFD。
    let text = "你好，世界";
    let bytes = text.as_bytes();
    assert_eq!(bytes.len(), 15);
    for split in 0..=bytes.len() {
      let mut decoder = Utf8ChunkDecoder::new();
      let mut out = decoder.push(&bytes[..split]);
      out.push_str(&decoder.push(&bytes[split..]));
      out.push_str(&decoder.finish());
      assert_eq!(out, text, "在字节 {split} 处切分后重组失败");
    }
  }

  fn test_cli_agent(
    command_args: &str,
    timeout_secs: u64,
    idle_timeout_secs: u64,
  ) -> crate::storage::ExternalAgentRecord {
    crate::storage::ExternalAgentRecord {
      id: uuid::Uuid::new_v4().hyphenated().to_string(),
      kind: ExternalAgentKind::LocalCli,
      name: "cli-test".to_string(),
      base_url: String::new(),
      api_key: String::new(),
      api_key_ref: None,
      model: String::new(),
      command: "sh".to_string(),
      args: vec!["-c".to_string(), command_args.to_string()],
      workdir: None,
      timeout_secs,
      idle_timeout_secs,
      created_at_ms: 0,
    }
  }

  fn fresh_cancel_flag() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
  }

  #[tokio::test]
  async fn cli_turn_returns_after_process_exit_despite_inherited_pipe() {
    // 子进程退出后，继承了 stdout 的后台孙进程仍占着管道：
    // 旧实现只有所有 sender drop（管道关闭）才退出循环，会等到
    // 120 秒总超时并丢弃已有输出。
    let agent = test_cli_agent("echo HELLO-DRAIN; sleep 5 &", 600, 60);
    let cancel = fresh_cancel_flag();
    let start = std::time::Instant::now();
    let result = super::execute_local_cli_turn(&agent, "hi", &cancel, |_| {}).await;
    let elapsed = start.elapsed();
    assert_eq!(result.as_deref(), Ok("HELLO-DRAIN"));
    assert!(
      elapsed < std::time::Duration::from_secs(10),
      "耗时 {elapsed:?}，仍在等管道关闭"
    );
  }

  #[tokio::test]
  async fn cli_turn_total_timeout_keeps_partial_output() {
    // 旧实现超时直接丢弃已收集输出；新实现超时错误里附带部分输出。
    let agent = test_cli_agent("echo PARTIAL-OUTPUT; sleep 30", 2, 60);
    let cancel = fresh_cancel_flag();
    let start = std::time::Instant::now();
    let result = super::execute_local_cli_turn(&agent, "hi", &cancel, |_| {}).await;
    let elapsed = start.elapsed();
    assert!(
      elapsed < std::time::Duration::from_secs(10),
      "耗时 {elapsed:?}，2 秒总超时未触发"
    );
    let err = result.expect_err("2 秒总超时应报错");
    assert!(err.contains("超时"), "{err}");
    assert!(
      err.contains("PARTIAL-OUTPUT"),
      "超时错误应附带已收集的部分输出：{err}"
    );
  }

  #[tokio::test]
  async fn cli_turn_idle_timeout_fires_without_output() {
    // 输出一行后 2 秒无输出：应判空闲超时，而不是等到 60 秒总超时。
    let agent = test_cli_agent("echo FIRST-LINE; sleep 30", 60, 2);
    let cancel = fresh_cancel_flag();
    let start = std::time::Instant::now();
    let result = super::execute_local_cli_turn(&agent, "hi", &cancel, |_| {}).await;
    let elapsed = start.elapsed();
    assert!(
      elapsed < std::time::Duration::from_secs(10),
      "耗时 {elapsed:?}，2 秒空闲超时未触发"
    );
    let err = result.expect_err("空闲超时应报错");
    assert!(err.contains("空闲超时"), "{err}");
    assert!(
      err.contains("FIRST-LINE"),
      "空闲超时错误应附带已有输出：{err}"
    );
  }

  #[tokio::test]
  async fn cli_turn_cancel_flag_kills_tree_and_returns_partial() {
    // 前端「停止」按钮置位取消令牌后，应杀掉整棵树并返回已有输出。
    let agent = test_cli_agent("echo BEFORE-STOP; sleep 30", 600, 60);
    let cancel = fresh_cancel_flag();
    let canceller = cancel.clone();
    tokio::spawn(async move {
      tokio::time::sleep(std::time::Duration::from_millis(500)).await;
      canceller.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let start = std::time::Instant::now();
    let result = super::execute_local_cli_turn(&agent, "hi", &cancel, |_| {}).await;
    let elapsed = start.elapsed();
    assert!(
      elapsed < std::time::Duration::from_secs(10),
      "耗时 {elapsed:?}，取消后未及时返回"
    );
    assert_eq!(result.as_deref(), Ok("BEFORE-STOP"));
  }

  #[cfg(unix)]
  #[test]
  fn process_group_kill_terminates_grandchild() {
    // 旧实现超时只 child.kill()，sh -c / npx 等包装脚本的孙进程会残留。
    // 新实现让子进程自成进程组，超时用 killpg 杀整个组。
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new("sh");
    cmd.args(["-c", "sleep 60 & echo $!; wait"]);
    cmd.process_group(0);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().expect("启动 sh");
    let pid = child.id();
    let stdout = child.stdout.take().expect("应有 stdout 管道");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("读取孙进程 pid");
    let grandchild: i32 = line.trim().parse().expect("孙进程 pid 应为数字");
    assert_eq!(unsafe { libc::kill(grandchild, 0) }, 0, "孙进程应已启动");
    super::kill_process_group(pid);
    let _ = child.kill();
    let _ = child.wait();
    // 孙进程被 killpg 杀死后可能短暂处于僵尸态（等 init 回收），轮询确认消失。
    let mut gone = false;
    for _ in 0..40 {
      if unsafe { libc::kill(grandchild, 0) } != 0 {
        gone = true;
        break;
      }
      std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(gone, "killpg 后孙进程 sleep 仍在运行（pid {grandchild}）");
  }

  #[test]
  fn utf8_chunk_decoder_handles_byte_by_byte_and_mixed_ascii() {
    let text = "abc你好def世界ghi";
    let mut decoder = Utf8ChunkDecoder::new();
    let mut out = String::new();
    for &b in text.as_bytes() {
      out.push_str(&decoder.push(&[b]));
    }
    out.push_str(&decoder.finish());
    assert_eq!(out, text);
  }

  #[test]
  fn openai_endpoint_and_sse_delta_parse_cleanly() {
    assert_eq!(
      build_openai_endpoint("https://api.deepseek.com/v1/"),
      "https://api.deepseek.com/v1/chat/completions"
    );
    assert_eq!(
      build_openai_endpoint("https://api.openai.com/v1/chat/completions"),
      "https://api.openai.com/v1/chat/completions"
    );

    let chunks = [
      "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}",
      "data: {\"choices\":[{\"delta\":{\"content\":\"Valkey-\"}}]}",
      "data: {\"choices\":[{\"delta\":{\"content\":\"9921\"}}]}",
      "data: [DONE]",
    ];
    let combined: String = chunks
      .iter()
      .filter_map(|line| extract_openai_sse_delta(line))
      .collect();
    assert_eq!(combined, "Valkey-9921");
  }

  #[test]
  fn cli_args_placeholder_or_stdin_fallback() {
    let (args1, stdin1) =
      build_cli_args_and_stdin(&["-p".into(), "{prompt}".into()], "hello world");
    assert_eq!(args1, vec!["-p", "hello world"]);
    assert_eq!(stdin1, None);

    let (args2, stdin2) = build_cli_args_and_stdin(&["exec".into()], "hello world");
    assert_eq!(args2, vec!["exec"]);
    assert_eq!(stdin2.as_deref(), Some("hello world"));
  }

  #[test]
  fn normalize_external_agent_validates_api_and_cli() {
    let bad_url = normalize_external_agent_input(
      None,
      ExternalAgentKind::OpenaiApi,
      "BadApi",
      "http://example.com/v1",
      "sk-1",
      "gpt-4o",
      "",
      vec![],
      None,
      None,
      None,
      None,
    );
    assert!(bad_url.is_err());

    let local_ok = normalize_external_agent_input(
      None,
      ExternalAgentKind::OpenaiApi,
      "LocalOllama",
      "http://127.0.0.1:11434/v1/",
      "",
      "qwen2.5",
      "",
      vec![],
      None,
      None,
      None,
      None,
    )
    .expect("localhost http should succeed");
    assert_eq!(local_ok.base_url, "http://127.0.0.1:11434/v1");

    let cli_ok = normalize_external_agent_input(
      None,
      ExternalAgentKind::LocalCli,
      "Claude Code",
      "",
      "",
      "",
      "claude",
      vec!["-p".into()],
      Some("/tmp"),
      None,
      None,
      None,
    )
    .expect("cli agent should succeed");
    assert_eq!(cli_ok.command, "claude");
    assert_eq!(cli_ok.account_key().provider_id.as_str(), "cli");
  }

  #[test]
  fn merge_augmented_path_prefers_login_shell_and_dedups() {
    let merged = super::merge_augmented_path(
      "/usr/bin:/bin:/opt/homebrew/bin",
      "/Users/li",
      Some("/Users/li/.nvm/versions/node/v22/bin:/opt/homebrew/bin:/usr/bin"),
    );
    let parts: Vec<&str> = merged.split(':').collect();
    // 登录 shell 的 PATH 在最前，去重后只出现一次。
    assert_eq!(parts[0], "/Users/li/.nvm/versions/node/v22/bin");
    assert_eq!(
      parts.iter().filter(|p| **p == "/opt/homebrew/bin").count(),
      1
    );
    assert_eq!(parts.iter().filter(|p| **p == "/usr/bin").count(), 1);
    // 硬编码补全仍在：volta / bun / pnpm / claude local。
    assert!(parts.contains(&"/Users/li/.volta/bin"));
    assert!(parts.contains(&"/Users/li/.bun/bin"));
    assert!(parts.contains(&"/Users/li/Library/pnpm"));
    assert!(parts.contains(&"/Users/li/.claude/local/bin"));
  }

  #[test]
  fn merge_augmented_path_without_shell_path_keeps_old_behavior() {
    let merged = super::merge_augmented_path("/usr/bin", "/home/u", None);
    assert!(merged.starts_with("/usr/bin:"));
    assert!(merged.contains("/home/u/.cargo/bin"));
    assert!(merged.contains("/opt/homebrew/bin"));
  }

  fn test_api_agent(base_url: &str, timeout_secs: u64) -> crate::storage::ExternalAgentRecord {
    crate::storage::ExternalAgentRecord {
      id: uuid::Uuid::new_v4().hyphenated().to_string(),
      kind: ExternalAgentKind::OpenaiApi,
      name: "api-test".to_string(),
      base_url: base_url.to_string(),
      api_key: String::new(),
      api_key_ref: None,
      model: "test-model".to_string(),
      command: String::new(),
      args: vec![],
      workdir: None,
      timeout_secs,
      idle_timeout_secs: 60,
      created_at_ms: 0,
    }
  }

  /// 起一个只发指定 SSE 首块、随后停顿的本地 HTTP 服务器（chunked 编码）。
  /// 返回 base_url。服务器线程在停顿结束后自行退出，不 join（随测试进程结束）。
  fn spawn_stalling_sse_server(first_payload: Option<String>) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定本地端口");
    let addr = listener.local_addr().expect("本地地址");
    std::thread::spawn(move || {
      let Ok((mut stream, _)) = listener.accept() else {
        return;
      };
      let mut buf = [0u8; 4096];
      let _ = stream.read(&mut buf);
      let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
      );
      if let Some(payload) = first_payload {
        let chunk = format!("{:x}\r\n{}\r\n", payload.len(), payload);
        let _ = stream.write_all(chunk.as_bytes());
        let _ = stream.flush();
      }
      // 停顿：触发客户端的总超时（或 read_timeout）。
      std::thread::sleep(std::time::Duration::from_secs(30));
    });
    format!("http://127.0.0.1:{}/v1", addr.port())
  }

  #[tokio::test]
  async fn api_turn_slow_stream_returns_partial_with_truncation_note() {
    // 旧实现：Client::builder().timeout(120s) 覆盖整个响应体，
    // 长流式回复会在 120 秒被切断且已累积文本随 Err 丢弃。
    // 新实现：2 秒总超时后返回已有文本，并标注「回复被截断」。
    let base_url = spawn_stalling_sse_server(Some(
      "data: {\"choices\":[{\"delta\":{\"content\":\"FIRST-CHUNK\"}}]}\n\n".to_string(),
    ));
    let agent = test_api_agent(&base_url, 2);
    let cancel = fresh_cancel_flag();
    let start = std::time::Instant::now();
    let result = super::execute_openai_api_turn(&agent, "", "hi", &cancel, |_| {}).await;
    let elapsed = start.elapsed();
    assert!(
      elapsed < std::time::Duration::from_secs(10),
      "耗时 {elapsed:?}，2 秒总超时未触发"
    );
    let text = result.expect("超时后应返回已累积文本，而不是报错");
    assert!(text.contains("FIRST-CHUNK"), "{text}");
    assert!(text.contains("回复被截断"), "{text}");
  }

  #[tokio::test]
  async fn api_turn_timeout_without_any_content_errors() {
    // 一点内容都没收到就超时：没有可返回的文本，应报错而非空回复。
    let base_url = spawn_stalling_sse_server(None);
    let agent = test_api_agent(&base_url, 2);
    let cancel = fresh_cancel_flag();
    let start = std::time::Instant::now();
    let result = super::execute_openai_api_turn(&agent, "", "hi", &cancel, |_| {}).await;
    let elapsed = start.elapsed();
    assert!(
      elapsed < std::time::Duration::from_secs(10),
      "耗时 {elapsed:?}，2 秒总超时未触发"
    );
    let err = result.expect_err("无任何内容时超时应报错");
    assert!(err.contains("超时"), "{err}");
  }

  #[test]
  fn batch_prompt_conflict_requires_stdin() {
    // 批处理垫片 + {prompt} 占位符：Rust 会拒绝含 " / % 的批处理参数，
    // 必须明确报错提示改用 stdin，而不是启动失败。
    let err = super::check_batch_prompt_conflict(true, true).expect_err("应拒绝");
    assert!(err.contains("stdin"), "{err}");
    assert!(err.contains("{prompt}"), "{err}");
    assert!(super::check_batch_prompt_conflict(true, false).is_ok());
    assert!(super::check_batch_prompt_conflict(false, true).is_ok());
  }

  #[cfg(windows)]
  #[test]
  fn resolve_cli_executable_finds_cmd_shim() {
    // 仅 Windows：临时目录放一个假 codex.cmd，挂到 PATH 前面，
    // 应被找到且识别为批处理垫片。
    let dir = std::env::temp_dir().join(format!("maam-resolve-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let shim = dir.join("maam-fake-cli.cmd");
    std::fs::write(&shim, "@echo off\r\necho hi\r\n").unwrap();
    let old_path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{};{old_path}", dir.display()));
    let found = super::resolve_cli_executable("maam-fake-cli");
    std::env::set_var("PATH", &old_path);
    let _ = std::fs::remove_dir_all(&dir);
    let (path, is_batch) = found.expect("应在 PATH 里找到 .cmd 垫片");
    assert!(is_batch);
    assert_eq!(path, shim);
    assert!(super::resolve_cli_executable("maam-definitely-not-installed-xyz").is_err());
  }

  #[test]
  fn normalize_external_agent_validates_timeouts() {
    let build = |timeout: Option<u64>, idle: Option<u64>| {
      normalize_external_agent_input(
        None,
        ExternalAgentKind::LocalCli,
        "CLI",
        "",
        "",
        "",
        "claude",
        vec![],
        None,
        timeout,
        idle,
        None,
      )
    };
    let defaults = build(None, None).expect("默认超时应通过");
    assert_eq!(defaults.timeout_secs, 600);
    assert_eq!(defaults.idle_timeout_secs, 60);
    let custom = build(Some(300), Some(30)).expect("合法超时应通过");
    assert_eq!(custom.timeout_secs, 300);
    assert_eq!(custom.idle_timeout_secs, 30);
    assert!(build(Some(5), None).is_err(), "总超时小于 30 秒应拒绝");
    assert!(build(Some(3601), None).is_err(), "总超时大于 3600 秒应拒绝");
    assert!(build(None, Some(5)).is_err(), "空闲超时小于 10 秒应拒绝");
    assert!(build(None, Some(601)).is_err(), "空闲超时大于 600 秒应拒绝");
  }
}
