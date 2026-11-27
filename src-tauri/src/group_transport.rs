use crate::storage::{
  default_agent_idle_timeout_secs, default_agent_timeout_secs, normalize_uuid, now_ms, AccountKey,
  ExternalAgentKind, ExternalAgentRecord,
};
#[cfg(desktop)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use url::Url;
use uuid::Uuid;
use zeroize::Zeroizing;

#[cfg(desktop)]
use serde_json::Value;
#[cfg(desktop)]
use std::io::{Read, Write};
#[cfg(desktop)]
use std::net::{IpAddr, ToSocketAddrs};
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
        api_key: Zeroizing::new(api_key.trim().chars().take(512).collect()),
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
      // 白名单：只允许已知的 AI 编程助手 CLI，通用解释器一律不允许。
      validate_local_cli_command(clean_cmd)?;
      let clean_args: Vec<String> = args
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(24)
        .collect();
      // prompt 不进 argv：参数里出现 {prompt} 即拒绝，改走 stdin。
      if clean_args.iter().any(|a| a.contains("{prompt}")) {
        return Err(
          "参数里不能写 {prompt}：prompt 改走 stdin（参数里去掉 {prompt} 后自动走 stdin），避免提问内容出现在进程参数里或被当成命令行开关解析"
            .to_string(),
        );
      }
      let clean_workdir = match workdir.map(str::trim).filter(|s| !s.is_empty()) {
        Some(dir) => {
          let capped: String = dir.chars().take(260).collect();
          // 工作目录必须真实存在且是目录：canonicalize 解析符号链接，
          // 防止保存后目录被换成别处；不存在或不是目录直接拒绝保存。
          // 存的仍是用户原样输入（显示友好），执行时再 canonicalize 一次。
          let canonical =
            std::fs::canonicalize(&capped).map_err(|e| format!("工作目录不存在或无法访问: {e}"))?;
          if !canonical.is_dir() {
            return Err("工作目录不是一个目录".to_string());
          }
          Some(capped)
        }
        None => None,
      };
      Ok(ExternalAgentRecord {
        id: clean_id,
        kind,
        name: clean_name,
        base_url: String::new(),
        api_key: Zeroizing::new(String::new()),
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

/// 模型 API 响应字节上限（复审定值）：恶意/异常服务端用超大行或无限流
/// 撑爆内存。SSE 单行超限直接报错；SSE 累计 / 非流式 JSON 超限分别
/// 截断或报错；错误体只读 16 KiB（只用来拼错误信息）。
#[cfg(desktop)]
const MAX_API_SSE_LINE_BYTES: usize = 256 * 1024;
#[cfg(desktop)]
const MAX_API_SSE_ACCUMULATED_BYTES: usize = 2 * 1024 * 1024;
#[cfg(desktop)]
const MAX_API_JSON_BODY_BYTES: usize = 4 * 1024 * 1024;
#[cfg(desktop)]
const MAX_API_ERROR_BYTES: usize = 16 * 1024;

/// 稳定 Rust 上 `IpAddr::is_global()` 仍是不稳定 API（E0658，见 #27709），
/// 这里按 IANA 特殊用途地址表手写等价判断：凡属以下任一范围即非公网。
/// 宁可误拒一段公网地址（安全侧），也不放过一段特殊地址。
#[cfg(desktop)]
fn is_global_ip(ip: &IpAddr) -> bool {
  match ip {
    IpAddr::V4(v4) => {
      let o = v4.octets();
      !matches!(o[0], 0 | 10 | 127)
        && !(o[0] == 100 && (64..=127).contains(&o[1])) // 100.64/10 共享地址空间
        && !(o[0] == 169 && o[1] == 254) // 链路本地 169.254/16
        && !(o[0] == 172 && (16..=31).contains(&o[1])) // 私有 172.16/12
        && !(o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24（含 192.0.0.0/29）
        && !(o[0] == 192 && o[1] == 0 && o[2] == 2) // 文档 192.0.2/24
        && !(o[0] == 192 && o[1] == 168) // 私有 192.168/16
        && !(o[0] == 198 && (18..=19).contains(&o[1])) // 基准测试 198.18/15
        && !(o[0] == 198 && o[1] == 51 && o[2] == 100) // 文档 198.51.100/24
        && !(o[0] == 203 && o[1] == 0 && o[2] == 113) // 文档 203.0.113/24
        && !(224..=255).contains(&o[0]) // 组播 224/4 + 保留 240/4（含广播）
    }
    IpAddr::V6(v6) => {
      let s = v6.segments();
      !(s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0)
        // ::/96（含 :: 与 ::1，IPv4 兼容已废弃；::ffff: 映射已在调用前还原成 IPv4）
        && !(s[0] & 0xfe00 == 0xfc00) // 唯一本地 fc00::/7
        && !(s[0] & 0xffc0 == 0xfe80) // 链路本地 fe80::/10
        && !(s[0] & 0xffc0 == 0xfec0) // 站点本地 fec0::/10（已废弃）
        && !(s[0] & 0xff00 == 0xff00) // 组播 ff00::/8
        && !(s[0] == 0x2001 && s[1] == 0x0db8) // 文档 2001:db8::/32
        && !(s[0] == 0x2001 && s[1] == 0x0000) // Teredo 2001::/32
        && !(s[0] == 0x2001 && s[1] == 0x0002) // 基准测试 2001:2::/48
        && !(s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0) // 丢弃 100::/64
        && !(s[0] == 0x2002) // 6to4 2002::/16（内嵌 IPv4，可借此触及内网）
        && !(s[0] == 0x0064 && s[1] == 0xff9b)
      // NAT64 前缀 64:ff9b::/32：含知名 64:ff9b::/96 与本地用 64:ff9b:1::/48
      //（内嵌 IPv4，可借此触及内网）
    }
  }
}

/// 校验 OpenAI 兼容端点的解析后地址（SSRF 防护）：域名先解析再过滤，
/// 解析出的任一地址为内网/保留地址即拒绝；仅放行公网 IP 与 loopback
///（本机 Ollama 如 127.0.0.1:11434 是文档化的合法用法）。
/// 返回（主机名，已校验的地址列表）：调用方须把地址列表经
/// `resolve_to_addrs` 钉进 reqwest 客户端，否则 reqwest 连接时会重新
/// 解析，解析时检查与连接时解析之间存在 TOCTOU（DNS 重绑定）。
/// 配合 Policy::none()（禁重定向跟随）使用。
#[cfg(desktop)]
fn validate_openai_endpoint_host(
  endpoint: &str,
) -> Result<(String, Vec<std::net::SocketAddr>), String> {
  let url = Url::parse(endpoint).map_err(|_| "模型 API 地址不是有效 URL".to_string())?;
  if url.scheme() != "https" && url.scheme() != "http" {
    return Err("模型 API 地址只支持 http(s)".to_string());
  }
  let host_raw = url
    .host_str()
    .ok_or_else(|| "模型 API 地址缺少主机名".to_string())?;
  // url crate 的 host_str() 对 IPv6 带方括号（"[::1]"）：to_socket_addrs
  // 与 IpAddr 解析都认不出，先去掉方括号再用。
  let host = host_raw.trim_start_matches('[').trim_end_matches(']');
  let port = url.port_or_known_default().unwrap_or(443);
  // IP 字面量不走 DNS，直接用；域名才解析。
  let addrs: Vec<std::net::SocketAddr> = if let Ok(ip) = host.parse::<IpAddr>() {
    vec![std::net::SocketAddr::new(ip, port)]
  } else {
    (host, port)
      .to_socket_addrs()
      .map_err(|e| format!("无法解析模型 API 主机 {host}: {e}"))?
      .collect()
  };
  if addrs.is_empty() {
    return Err(format!("模型 API 主机 {host} 未解析到任何地址"));
  }
  for addr in &addrs {
    // IPv4 映射的 IPv6（如 ::ffff:10.0.0.1）先还原成 IPv4 再判断，
    // 否则内网地址可能绕过全局地址检查。
    let ip = match addr.ip() {
      IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
        Some(v4) => IpAddr::V4(v4),
        None => IpAddr::V6(v6),
      },
      v4 => v4,
    };
    if ip.is_loopback() {
      continue;
    }
    if !is_global_ip(&ip) {
      return Err(format!(
        "模型 API 主机 {host} 解析到内网/保留地址 {ip}，已拒绝"
      ));
    }
  }
  Ok((host.to_string(), addrs))
}

/// 流式读响应体，超过 cap 即停止（truncated=true），避免恶意服务端
/// 用超大响应体撑爆内存；读中途出错返回 Err。
#[cfg(desktop)]
async fn read_response_body_capped(
  resp: &mut reqwest::Response,
  cap: usize,
) -> Result<(Vec<u8>, bool), String> {
  let mut buf = Vec::new();
  loop {
    match resp.chunk().await {
      Ok(Some(chunk)) => {
        let room = cap.saturating_sub(buf.len());
        if room == 0 {
          return Ok((buf, true));
        }
        let take = room.min(chunk.len());
        buf.extend_from_slice(&chunk[..take]);
        if take < chunk.len() {
          return Ok((buf, true));
        }
      }
      Ok(None) => break,
      Err(e) => return Err(format!("读取模型响应失败: {e}")),
    }
  }
  Ok((buf, false))
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
/// 本地 CLI 可执行文件白名单：只允许已知的 AI 编程助手 CLI。
/// 命令可以是裸名（走 PATH 查找）或绝对路径，但最终执行的文件名
///（去掉 .exe/.cmd/.bat 后缀）必须在白名单里；`sh`、`powershell`、
///`python` 等通用解释器一律不允许，避免群聊输入变成任意命令执行。
/// 白名单故意收得很窄：新 CLI 请提需求评审后再加。
pub const LOCAL_CLI_ALLOWED_COMMANDS: &[&str] = &["claude", "codex", "gemini", "qwen", "opencode"];

/// 校验本地 CLI 命令是否在白名单内（保存时与执行时各查一次）。
/// 纯逻辑，全平台可跑（normalize 在非桌面端也会编译）。
pub fn validate_local_cli_command(command: &str) -> Result<(), String> {
  let stem = std::path::Path::new(command.trim())
    .file_stem()
    .and_then(|s| s.to_str())
    .unwrap_or("")
    .to_ascii_lowercase();
  if LOCAL_CLI_ALLOWED_COMMANDS.contains(&stem.as_str()) {
    Ok(())
  } else {
    Err(format!(
      "本地 CLI 只允许运行白名单里的命令（{}），当前命令「{}」不在白名单内",
      LOCAL_CLI_ALLOWED_COMMANDS.join("、"),
      command.trim()
    ))
  }
}

/// 组装 CLI 参数与 stdin：prompt 一律走 stdin，绝不放进 argv。
/// 原因：argv 会出现在进程列表（ps / 任务管理器）里，提问内容可能
/// 含敏感信息；且以 `-` 开头的提问会被 CLI 当成开关解析（flag 注入）。
/// 参数里出现 `{prompt}` 即报错，提示去掉它（历史配置需手动改一次）。
#[cfg_attr(not(desktop), allow(dead_code))]
pub fn build_cli_args_and_stdin(
  args: &[String],
  prompt: &str,
) -> Result<(Vec<String>, String), String> {
  if args.iter().any(|a| a.contains("{prompt}")) {
    return Err(
      "参数里不能写 {prompt}：prompt 改走 stdin（参数里去掉 {prompt} 后自动走 stdin），避免提问内容出现在进程参数里或被当成命令行开关解析"
        .to_string(),
    );
  }
  Ok((args.to_vec(), prompt.to_string()))
}

/// 超时或断流后：有累积文本就返回累积文本并标注「回复被截断」，
/// 没有才把原始错误透传。
#[cfg_attr(not(desktop), allow(dead_code))]
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

#[cfg_attr(not(desktop), allow(dead_code))]
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
  // 空闲超时由 client 的 read_timeout（成员 idle_timeout_secs，默认 60 秒）负责。
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
  // SSRF 防护：先解析后过滤，内网/保留地址拒绝；再禁重定向跟随，
  // 否则服务端一个 302 就能把带着 Authorization 的请求带到内网。
  // 最后把校验过的地址钉进客户端（resolve_to_addrs），reqwest 连接时
  // 不再重新解析，关闭 DNS 重绑定 TOCTOU。IP 字面量不走 DNS，不用钉。
  let (endpoint_host, pinned_addrs) = validate_openai_endpoint_host(&endpoint)?;
  let mut client_builder = reqwest::Client::builder()
    .redirect(reqwest::redirect::Policy::none())
    .connect_timeout(Duration::from_secs(15))
    .read_timeout(Duration::from_secs(agent.idle_timeout_secs.clamp(10, 600)));
  if endpoint_host.parse::<std::net::IpAddr>().is_err() {
    client_builder = client_builder.resolve_to_addrs(&endpoint_host, &pinned_addrs);
  }
  let client = client_builder
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
    // 错误体同样封顶读取：旧实现 resp.text() 无上限，恶意服务端
    // 可用超大错误体撑内存；只取前 16 KiB 解析错误信息。
    let (raw_bytes, _) = read_response_body_capped(&mut resp, MAX_API_ERROR_BYTES)
      .await
      .unwrap_or_default();
    let raw_err = String::from_utf8_lossy(&raw_bytes).into_owned();
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
    let (raw_bytes, truncated) =
      read_response_body_capped(&mut resp, MAX_API_JSON_BODY_BYTES).await?;
    if truncated {
      return Err(format!(
        "模型 API 返回的 JSON 超过 {} 字节上限",
        MAX_API_JSON_BODY_BYTES
      ));
    }
    let raw_json = String::from_utf8_lossy(&raw_bytes).into_owned();
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
  let mut capped = false;

  loop {
    // 取消令牌：chunk() 挂起时用 200ms 节拍轮询，最多 200ms 内注意到；
    // 流停滞超过成员空闲超时（默认 60 秒）则由 read_timeout 触发断流
    //（外层按截断返回已有文本）。
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
    // 单行无上限增长会撑爆内存：服务端一直不发 \n 时直接报错。
    if line_buf.len() > MAX_API_SSE_LINE_BYTES {
      return Err(format!(
        "模型 API 流式响应单行超过 {} 字节，疑似异常响应",
        MAX_API_SSE_LINE_BYTES
      ));
    }
    while let Some(pos) = line_buf.iter().position(|&b| b == b'\n') {
      let line_bytes: Vec<u8> = line_buf.drain(..=pos).collect();
      let line = String::from_utf8_lossy(&line_bytes);
      if let Some(delta) = extract_openai_sse_delta(&line) {
        accumulated.push_str(&delta);
        if accumulated.len() > MAX_API_SSE_ACCUMULATED_BYTES {
          capped = true;
          break;
        }
      }
    }
    if capped {
      break;
    }
    if !accumulated.is_empty() && last_emit.elapsed() >= Duration::from_millis(180) {
      last_emit = Instant::now();
      on_progress(accumulated.trim().to_string());
    }
    // 同步给外层的截断处理：总超时/断流时按此返回已有文本。
    sync_shared_partial(shared_partial, accumulated.trim());
  }

  if capped {
    // 累计文本超限：按截断返回已有内容（与总超时行为一致）。
    let text = format!(
      "{}\n\n（回复被截断：超过 {} 字节上限）",
      accumulated.trim(),
      MAX_API_SSE_ACCUMULATED_BYTES
    );
    sync_shared_partial(shared_partial, &text);
    return Ok(text);
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
#[cfg_attr(not(desktop), allow(dead_code))]
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

/// 本地 CLI 子进程的最小环境：清空继承的环境变量（不把宿主的密钥、
/// 代理、token 透传给子进程，避免提示词注入的代码借此外泄），只给
/// CLI 运行必需的几项。PATH 沿用 build_augmented_path 的补全结果；
/// HOME 让 CLI 找到自己的配置文件与授权；其余一律不透传。
/// 注意：需要代理的 CLI 请在它自己的配置文件里配代理，不要依赖环境变量。
#[cfg(desktop)]
fn apply_minimal_cli_env(cmd: &mut std::process::Command) {
  cmd.env_clear();
  match build_augmented_path() {
    Some(path_val) => {
      cmd.env("PATH", path_val);
    }
    None => {
      if let Ok(v) = std::env::var("PATH") {
        cmd.env("PATH", v);
      }
    }
  }
  #[cfg(unix)]
  for key in [
    "HOME",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "USER",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
  ] {
    if let Ok(v) = std::env::var(key) {
      cmd.env(key, v);
    }
  }
  #[cfg(windows)]
  for key in [
    "SystemRoot",
    "USERPROFILE",
    "TEMP",
    "TMP",
    // .cmd 垫片（npm 全局装的 codex/gemini/claude）与凭据读取所需：
    // ComSpec/PATHEXT 决定 .cmd 能否被解析执行，APPDATA/LOCALAPPDATA
    // 是 CLI 找登录态的常规位置，SystemDrive 是其衍生路径的根。
    // 均不含第三方 API Key。
    "ComSpec",
    "PATHEXT",
    "SystemDrive",
    "APPDATA",
    "LOCALAPPDATA",
  ] {
    if let Ok(v) = std::env::var(key) {
      cmd.env(key, v);
    }
  }
}

/// 本地 CLI stdout 累计上限：超过即杀进程并报错，避免异常 CLI
/// 用无限输出撑爆内存。stderr 另有 2048 字截断（接收端）。
#[cfg(desktop)]
const MAX_CLI_STDOUT_BYTES: usize = 2 * 1024 * 1024;

/// 增量 UTF-8 解码器：把任意切分的字节块拼成完整字符串。
/// 旧实现对每个 read 块直接 `String::from_utf8_lossy`，多字节字符（中文 3 字节）
/// 跨块时会被切成两半，变成 U+FFFD 乱码。这里只解出完整的有效前缀，
/// 把尾部不完整的多字节字符留在 `pending` 里等下一块；真正无效的字节
/// 才按单个 U+FFFD 输出并跳过。
#[cfg_attr(not(desktop), allow(dead_code))]
#[derive(Debug, Default)]
pub struct Utf8ChunkDecoder {
  pending: Vec<u8>,
}

#[cfg_attr(not(desktop), allow(dead_code))]
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
/// 能一次杀掉整棵进程树；Job 建好即设 `KILL_ON_JOB_CLOSE`，父进程
/// 崩溃时句柄关闭也会杀掉整棵树，不留孤儿进程。句柄只在 watcher 线程
/// 收到主流程的 done 信号后才释放（见 execute_local_cli_turn 里的
/// job_done 通道），避免子进程退出瞬间就杀掉还占着管道排空的孙进程。
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

// SAFETY: CliJobGuard 独占 Job Object 句柄（Drop 时唯一一次 CloseHandle），
// 从不同时被两个线程持有，只会被 move 进唯一一个监视线程，因此可以 Send。
// （windows 0.62 起 HANDLE 内部是 *mut c_void，本身不是 Send，需手动声明。）
#[cfg(all(desktop, windows))]
unsafe impl Send for CliJobGuard {}

/// 超时/取消错误里附带的部分输出：最多取前 500 字，避免错误消息过长。
/// 仅桌面 LocalCli 路径使用（execute_local_cli_turn 是 #[cfg(desktop)]）。
#[cfg(desktop)]
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
#[cfg(desktop)]
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

/// Windows：解析 CLI 可执行文件。`Command::new("codex")` 只自动补 `.exe`，
/// npm 全局安装的 CLI 多为 `.cmd` 垫片，直接 spawn 会报找不到程序。
/// 这里自己遍历 PATH × PATHEXT，返回完整路径。
/// 仅 Windows 编译，Linux 上做不了单测，见 hardening-notes-cli.md 的手工步骤。
#[cfg(windows)]
fn resolve_cli_executable(command: &str) -> Result<std::path::PathBuf, String> {
  use std::path::{Path, PathBuf};

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
      return Ok(path);
    }
    // 带路径但无扩展名时也按 PATHEXT 补全。
    if path.extension().is_none() {
      for ext in pathext() {
        let candidate = path.with_extension(ext.trim_start_matches('.'));
        if candidate.is_file() {
          return Ok(candidate);
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
      return Ok(bare);
    }
    for ext in pathext() {
      let candidate = Path::new(dir).join(format!("{command}{ext}"));
      if candidate.is_file() {
        return Ok(candidate);
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
  // 白名单二次校验：历史配置（本轮之前保存的）可能含白名单外的命令。
  validate_local_cli_command(&agent.command)?;
  let (final_args, stdin_payload) = build_cli_args_and_stdin(&agent.args, full_prompt)?;
  // Windows：先解析可执行文件（.cmd 垫片需按 PATH × PATHEXT 查找）。
  #[cfg(windows)]
  let resolved_command = resolve_cli_executable(&agent.command)?;
  #[cfg(not(windows))]
  let resolved_command: std::path::PathBuf = std::path::PathBuf::from(&agent.command);
  let mut cmd = Command::new(&resolved_command);
  cmd.args(&final_args);
  if let Some(dir) = agent.workdir.as_deref().filter(|d| !d.trim().is_empty()) {
    // 执行时再 canonicalize 一次：兜住保存前就存在的旧记录，以及
    // 保存后目录被删除/替换的情况；current_dir 用解析后的真实路径。
    let canonical =
      std::fs::canonicalize(dir).map_err(|e| format!("工作目录不存在或无法访问: {e}"))?;
    if !canonical.is_dir() {
      return Err("工作目录不是一个目录".to_string());
    }
    cmd.current_dir(canonical);
  }
  // 最小环境：清空继承的环境变量，只给 CLI 运行必需的几项。
  apply_minimal_cli_env(&mut cmd);
  cmd.stdin(Stdio::piped());
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
  // 建 Job 时即设 KILL_ON_JOB_CLOSE：父进程崩溃、句柄关闭时整棵树也被
  // 杀掉，不留孤儿进程。任一步失败都留空，杀灭时回退 taskkill /T /F。
  #[cfg(windows)]
  let job_guard: Option<CliJobGuard> = (|| {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
      AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
      SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
      JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    let job = unsafe { CreateJobObjectW(None, windows::core::PCWSTR::null()) }.ok()?;
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let set_ok = unsafe {
      SetInformationJobObject(
        job,
        JobObjectExtendedLimitInformation,
        &info as *const _ as *const std::ffi::c_void,
        std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
      )
    }
    .is_ok();
    if !set_ok {
      unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(job);
      }
      return None;
    }
    // windows 0.62 起 HANDLE 的内部类型是 *mut c_void，as_raw_handle() 本身就是该类型，直接包起来即可。
    let proc = HANDLE(child.as_raw_handle());
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

  if let Some(mut stdin_handle) = child.stdin.take() {
    std::thread::spawn(move || {
      let _ = stdin_handle.write_all(stdin_payload.as_bytes());
      let _ = stdin_handle.flush();
    });
  }

  // 有界通道（128 条 × 每条至多 4 KiB）：子进程疯狂输出时，读线程在
  // 通道满后阻塞（blocking_send），管道随之顶满、子进程被迫停下，
  // 内存不再随输出无限增长；同时配合 stdout 2 MiB 累计上限。
  let (tx, mut rx) = tokio::sync::mpsc::channel::<CliStreamEvent>(128);

  if let Some(mut stdout) = child.stdout.take() {
    let tx_out = tx.clone();
    std::thread::spawn(move || {
      let mut buf = [0u8; 4096];
      while let Ok(n) = stdout.read(&mut buf) {
        if n == 0 {
          break;
        }
        // 发原始字节：多字节字符可能跨块，解码由接收端增量完成。
        let _ = tx_out.blocking_send(CliStreamEvent::Stdout(buf[..n].to_vec()));
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
        let _ = tx_err.blocking_send(CliStreamEvent::Stderr(buf[..n].to_vec()));
      }
    });
  }

  let (kill_tx, kill_rx) = std::sync::mpsc::channel::<()>();
  // done 通道：发送端随主流程结束（任何返回路径，包括超时提前返回）
  // 自动 drop，watcher 收到断开才退出并释放 Job 句柄。KILL_ON_JOB_CLOSE
  // 下句柄关闭会杀掉 Job 里残留的孙进程；若 watcher 在子进程退出时
  // 立刻返回，500ms 排空窗口会被提前截断（占着管道的孙进程输出收不到）。
  let (_job_done_tx, job_done_rx) = std::sync::mpsc::channel::<()>();
  let tx_done = tx.clone();
  drop(tx);
  std::thread::spawn(move || loop {
    match child.try_wait() {
      Ok(Some(status)) => {
        let _ = tx_done.blocking_send(CliStreamEvent::Finished(Ok(status.code())));
        // 等主流程排空完毕再释放 Job 句柄（见上）。
        let _ = job_done_rx.recv();
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
          let _ = tx_done.blocking_send(CliStreamEvent::Finished(Err(
            "本地 CLI 执行超时".to_string(),
          )));
          return;
        }
        std::thread::sleep(Duration::from_millis(80));
      }
      Err(err) => {
        let _ = tx_done.blocking_send(CliStreamEvent::Finished(Err(format!(
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
        if stdout_acc.len() > MAX_CLI_STDOUT_BYTES {
          let _ = kill_tx.send(());
          return Err(format!(
            "本地 CLI 输出超过 {} 字节上限，已终止",
            MAX_CLI_STDOUT_BYTES
          ));
        }
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
    // 非零退出一律报错：之前“有输出就当成功”会把失败当成正常回复
    // 发进群聊。错误里附上 stderr 与部分输出，方便定位。
    Some(Ok(Some(code))) if code != 0 => {
      let err_detail: String = stderr_acc.trim().chars().take(240).collect();
      let partial: String = final_out.chars().take(500).collect();
      if err_detail.is_empty() && partial.is_empty() {
        Err(format!("本地 CLI 退出码 {code}，未返回内容"))
      } else if err_detail.is_empty() {
        Err(format!("本地 CLI 退出码 {code}，输出未被采纳：{partial}"))
      } else if partial.is_empty() {
        Err(format!("CLI 错误({code}): {err_detail}"))
      } else {
        Err(format!(
          "CLI 错误({code}): {err_detail}（部分输出：{partial}）"
        ))
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

  #[test]
  fn openai_endpoint_host_filter_blocks_private_and_allows_loopback() {
    // 本机 Ollama（127.0.0.1 / ::1）是文档化的合法用法，必须放行；
    // 公网 IP 放行；其余内网/保留地址一律拒绝。
    assert!(
      super::validate_openai_endpoint_host("http://127.0.0.1:11434/v1/chat/completions").is_ok()
    );
    assert!(super::validate_openai_endpoint_host("http://[::1]:11434/v1/chat/completions").is_ok());
    assert!(super::validate_openai_endpoint_host("https://8.8.8.8/v1/chat/completions").is_ok());
    assert!(super::validate_openai_endpoint_host(
      "https://[2606:4700:4700::1111]/v1/chat/completions"
    )
    .is_ok());
    for bad in [
      "https://10.0.0.1/v1/chat/completions",
      "https://192.168.1.1/v1/chat/completions",
      "https://172.16.0.5/v1/chat/completions",
      // 云厂商元数据地址（SSRF 经典目标）。
      "http://169.254.169.254/latest/meta-data/",
      "https://100.64.0.1/v1/chat/completions",
      // IPv4 映射的 IPv6 不能绕过检查。
      "https://[::ffff:10.0.0.1]/v1/chat/completions",
      "https://[fc00::1]/v1/chat/completions",
      "https://[fe80::1]/v1/chat/completions",
      // is_global_ip 手写表覆盖的其余特殊范围（稳定 Rust 无 is_global 可用）。
      "https://0.0.0.0/v1/chat/completions",
      "https://224.0.0.1/v1/chat/completions",
      "https://255.255.255.255/v1/chat/completions",
      "https://192.0.2.1/v1/chat/completions",
      "https://198.51.100.1/v1/chat/completions",
      "https://203.0.113.1/v1/chat/completions",
      "https://198.18.0.1/v1/chat/completions",
      "https://[::]/v1/chat/completions",
      "https://[2001:db8::1]/v1/chat/completions",
      "https://[ff02::1]/v1/chat/completions",
      "https://[2002:0808:0808::1]/v1/chat/completions",
      "https://[64:ff9b:1::c000:201]/v1/chat/completions",
    ] {
      assert!(
        super::validate_openai_endpoint_host(bad).is_err(),
        "{bad} should be rejected"
      );
    }
    assert!(super::validate_openai_endpoint_host("not a url").is_err());
    assert!(super::validate_openai_endpoint_host("ftp://example.com/x").is_err());
    assert!(super::validate_openai_endpoint_host("https:///v1/chat/completions").is_err());
  }

  /// 测试用假 CLI：temp 目录下一个名为 `claude` 的可执行脚本，
  /// 内容是把参数原样转给 /bin/sh。白名单只放行 claude 等名字，
  /// 直接拿 `sh` 跑测试会被执行时校验挡住，所以用这个垫片。
  #[cfg(unix)]
  fn fake_cli_command() -> String {
    use std::sync::OnceLock;
    static FAKE: OnceLock<String> = OnceLock::new();
    FAKE
      .get_or_init(|| {
        #[cfg(unix)]
        {
          let dir = std::env::temp_dir().join("maam-test-fake-claude");
          std::fs::create_dir_all(&dir).unwrap();
          let path = dir.join("claude");
          std::fs::write(&path, "#!/bin/sh\nexec /bin/sh \"$@\"\n").unwrap();
          use std::os::unix::fs::PermissionsExt;
          let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
          path.to_string_lossy().into_owned()
        }
        #[cfg(not(unix))]
        {
          "claude".to_string()
        }
      })
      .clone()
  }

  #[cfg(unix)]
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
      api_key: zeroize::Zeroizing::new(String::new()),
      api_key_ref: None,
      model: String::new(),
      command: fake_cli_command(),
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

  #[cfg(unix)]
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

  #[cfg(unix)]
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

  #[cfg(unix)]
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

  #[cfg(unix)]
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
  fn cli_args_reject_prompt_placeholder_and_always_use_stdin() {
    // 参数里出现 {prompt} 即拒绝（prompt 改走 stdin）。
    let err = build_cli_args_and_stdin(&["-p".into(), "{prompt}".into()], "hello world")
      .expect_err("{prompt} in argv should be rejected");
    assert!(err.contains("{prompt}"), "{err}");
    assert!(err.contains("stdin"), "{err}");
    let err2 = build_cli_args_and_stdin(&["--flag={prompt}".into()], "hi")
      .expect_err("embedded {prompt} should be rejected");
    assert!(err2.contains("{prompt}"), "{err2}");

    let (args2, stdin2) =
      build_cli_args_and_stdin(&["exec".into()], "hello world").expect("plain args should succeed");
    assert_eq!(args2, vec!["exec"]);
    assert_eq!(stdin2, "hello world");
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

    // Windows 上 /tmp 不存在，workdir 校验会 canonicalize 失败：用系统临时目录。
    let tmp_workdir = std::env::temp_dir().to_string_lossy().into_owned();
    let cli_ok = normalize_external_agent_input(
      None,
      ExternalAgentKind::LocalCli,
      "Claude Code",
      "",
      "",
      "",
      "claude",
      vec!["-p".into()],
      Some(tmp_workdir.as_str()),
      None,
      None,
      None,
    )
    .expect("cli agent should succeed");
    assert_eq!(cli_ok.command, "claude");
    assert_eq!(cli_ok.account_key().provider_id.as_str(), "cli");
  }

  // 只在 unix 下编译：merge_augmented_path 本身是 #[cfg(unix)]，且用例全是 unix 风格路径。
  #[test]
  #[cfg(unix)]
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

  // 只在 unix 下编译：merge_augmented_path 本身是 #[cfg(unix)]，且用例全是 unix 风格路径。
  #[test]
  #[cfg(unix)]
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
      api_key: zeroize::Zeroizing::new(String::new()),
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
  fn local_cli_command_allowlist_rejects_shells_and_unknown_binaries() {
    for ok in [
      "claude",
      "codex",
      "gemini",
      "/usr/local/bin/claude",
      "codex.cmd",
      "Qwen",
    ] {
      assert!(
        super::validate_local_cli_command(ok).is_ok(),
        "{ok} should pass"
      );
    }
    for bad in [
      "sh",
      "bash",
      "powershell",
      "python3",
      "node",
      "rm",
      "curl",
      "claude --flag",
    ] {
      assert!(
        super::validate_local_cli_command(bad).is_err(),
        "{bad} should be rejected"
      );
    }
    // normalize 保存时同样拦截白名单外命令与 {prompt} 参数。
    let bad_cmd = normalize_external_agent_input(
      None,
      ExternalAgentKind::LocalCli,
      "Bad",
      "",
      "",
      "",
      "sh",
      vec![],
      None,
      None,
      None,
      None,
    );
    assert!(bad_cmd.is_err());
    let bad_args = normalize_external_agent_input(
      None,
      ExternalAgentKind::LocalCli,
      "Bad",
      "",
      "",
      "",
      "claude",
      vec!["exec {prompt}".into()],
      None,
      None,
      None,
      None,
    );
    assert!(bad_args.is_err());
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn cli_turn_nonzero_exit_is_error_even_with_output() {
    // 非零退出即使有输出也必须报错，不能当成正常回复发进群聊。
    let agent = test_cli_agent("echo PARTIAL-OUT; echo ERR-DETAIL >&2; exit 3", 600, 60);
    let cancel = fresh_cancel_flag();
    let result = super::execute_local_cli_turn(&agent, "hi", &cancel, |_| {}).await;
    let err = result.expect_err("non-zero exit should be an error");
    assert!(err.contains('3'), "{err}");
    assert!(
      err.contains("PARTIAL-OUT") || err.contains("ERR-DETAIL"),
      "{err}"
    );
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn cli_turn_stdout_cap_kills_runaway_output() {
    let agent = test_cli_agent("head -c 3000000 /dev/zero", 600, 60);
    let cancel = fresh_cancel_flag();
    let start = std::time::Instant::now();
    let result = super::execute_local_cli_turn(&agent, "hi", &cancel, |_| {}).await;
    let err = result.expect_err("over-cap output should error");
    assert!(err.contains("2097152"), "{err}");
    assert!(
      start.elapsed() < std::time::Duration::from_secs(30),
      "耗时 {:?}，应在触发 2MB 上限后迅速返回",
      start.elapsed()
    );
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
    let path = found.expect("应在 PATH 里找到 .cmd 垫片");
    // Windows 文件系统不区分大小写：解析器按 PATHEXT（大写 .CMD）拼出
    // 的候选路径，与实际创建的小写 .cmd 是同一文件，断言只比大小写
    // 无关的相等，不能要求扩展名大小写逐字相同。
    assert!(
      path
        .to_string_lossy()
        .eq_ignore_ascii_case(&shim.to_string_lossy()),
      "解析到的垫片应与创建的文件同一路径（不区分大小写）：{path:?} vs {shim:?}"
    );
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
