use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::AppHandle;
#[cfg(desktop)]
use tauri::Emitter;
use url::Url;

/// 更新源仓库，编译期通过 `MAAM_UPDATE_REPO`（格式 `owner/repo`）配置。
/// 未配置或格式非法时返回 `None`，表示更新功能默认关闭。
fn parse_update_repo(raw: &str) -> Option<String> {
  let raw = raw.trim();
  if raw.matches('/').count() != 1 {
    return None;
  }
  let (owner, repo) = raw.split_once('/')?;
  let valid_segment = |s: &str| {
    !s.is_empty()
      && s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
  };
  if !valid_segment(owner) || !valid_segment(repo) {
    return None;
  }
  Some(raw.to_string())
}

fn update_repo() -> Option<String> {
  parse_update_repo(option_env!("MAAM_UPDATE_REPO").unwrap_or(""))
}

fn latest_manifest_url(repo: &str) -> String {
  format!("https://github.com/{repo}/releases/latest/download/latest.json")
}

fn github_latest_api_url(repo: &str) -> String {
  format!("https://api.github.com/repos/{repo}/releases/latest")
}

fn releases_page_url(repo: &str) -> String {
  format!("https://github.com/{repo}/releases")
}

static UPDATE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

struct UpdateLockGuard;

impl UpdateLockGuard {
  fn try_acquire() -> Result<Self, String> {
    if UPDATE_IN_FLIGHT
      .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
      .is_err()
    {
      return Err("已有更新任务正在进行中".to_string());
    }
    Ok(Self)
  }
}

impl Drop for UpdateLockGuard {
  fn drop(&mut self) {
    UPDATE_IN_FLIGHT.store(false, Ordering::SeqCst);
  }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
  pub current_version: String,
  pub latest_version: String,
  pub has_update: bool,
  pub release_notes: String,
  pub pub_date: Option<String>,
  pub download_url: Option<String>,
  pub sha256: Option<String>,
  pub filename: Option<String>,
  pub size: Option<u64>,
  pub release_page_url: String,
  /// 未配置更新源（`MAAM_UPDATE_REPO`）时为 true：本次未做任何检查。
  pub updates_disabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallUpdateRequest {
  pub version: String,
  pub download_url: String,
  pub sha256: Option<String>,
  pub filename: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallUpdateOutcome {
  pub status: String,
  pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgressEvent {
  pub stage: String,
  pub downloaded: u64,
  pub total: Option<u64>,
  pub percent: Option<u8>,
  pub message: String,
}

#[derive(Debug, Deserialize)]
struct LatestManifest {
  version: String,
  #[serde(default)]
  notes: String,
  #[serde(default)]
  pub_date: Option<String>,
  #[serde(default)]
  release_page_url: Option<String>,
  #[serde(default)]
  platforms: HashMap<String, LatestPlatformAsset>,
}

#[derive(Debug, Deserialize)]
struct LatestPlatformAsset {
  url: String,
  #[serde(default)]
  sha256: Option<String>,
  #[serde(default)]
  filename: Option<String>,
  #[serde(default)]
  size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
  tag_name: String,
  #[serde(default)]
  body: Option<String>,
  #[serde(default)]
  published_at: Option<String>,
  #[serde(default)]
  html_url: Option<String>,
  #[serde(default)]
  assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
  name: String,
  browser_download_url: String,
  size: u64,
  #[serde(default)]
  digest: Option<String>,
}

fn clean_version(raw: &str) -> &str {
  raw.trim().trim_start_matches(['v', 'V'])
}

#[derive(Debug, Eq, PartialEq)]
struct ParsedVersion {
  nums: Vec<u64>,
  is_prerelease: bool,
}

fn parse_version(raw: &str) -> ParsedVersion {
  let no_prefix = clean_version(raw);
  let no_build = no_prefix.split('+').next().unwrap_or(no_prefix);
  let (core, is_prerelease) = match no_build.split_once('-') {
    Some((c, pre)) => (c, !pre.trim().is_empty()),
    None => (no_build, false),
  };
  let nums = core
    .split('.')
    .map(|seg| seg.trim().parse::<u64>().unwrap_or(0))
    .collect();
  ParsedVersion {
    nums,
    is_prerelease,
  }
}

fn is_newer_version(current: &str, latest: &str) -> bool {
  // 首选 semver 比较：正确处理预发布版本（1.0.0-rc.1 < 1.0.0-rc.2）。
  // 无法解析为 semver 的字符串（如缺段、含非法字符）回退到旧的数值比较。
  if let (Some(c), Some(l)) = (parse_semver(current), parse_semver(latest)) {
    return l > c;
  }
  let c = parse_version(current);
  let l = parse_version(latest);
  let max_len = c.nums.len().max(l.nums.len());
  for i in 0..max_len {
    let cv = *c.nums.get(i).unwrap_or(&0);
    let lv = *l.nums.get(i).unwrap_or(&0);
    if lv != cv {
      return lv > cv;
    }
  }
  c.is_prerelease && !l.is_prerelease
}

/// 把 `1.2` / `v1.2.3-rc.1+build` 这类版本号规范化为 semver 要求
/// 的三段式；不合法时返回 `None`，由调用方回退到旧比较。
fn parse_semver(raw: &str) -> Option<semver::Version> {
  let cleaned = clean_version(raw);
  // 构建元数据（+build）按 SemVer 规范在比较版本先后时必须忽略，
  // 且 semver crate 的 Ord 会比较它，故直接丢弃，只校验核心与预发布段。
  let (core_pre, _build) = match cleaned.split_once('+') {
    Some((head, tail)) => (head, Some(tail)),
    None => (cleaned, None),
  };
  let (core, pre) = match core_pre.split_once('-') {
    Some((c, p)) => (c, Some(p)),
    None => (core_pre, None),
  };
  let segments: Vec<&str> = core.split('.').collect();
  if segments.is_empty()
    || segments.len() > 3
    || segments
      .iter()
      .any(|s| s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()))
  {
    return None;
  }
  let mut full = core.to_string();
  for _ in segments.len()..3 {
    full.push_str(".0");
  }
  if let Some(p) = pre {
    full.push('-');
    full.push_str(p);
  }
  semver::Version::parse(&full).ok()
}

/// 下载地址必须严格位于配置仓库的 Releases 下载路径下：
/// `https://github.com/<owner>/<repo>/releases/download/<文件名>`。
/// 只校验主机不限定路径的旧实现会放行任意 GitHub 用户的 release 文件。
fn validate_download_url(raw: &str, repo: &str) -> Result<Url, String> {
  let trimmed = raw.trim();
  let prefix = format!("https://github.com/{repo}/releases/download/");
  if trimmed.len() < prefix.len() || !trimmed[..prefix.len()].eq_ignore_ascii_case(&prefix) {
    return Err("更新包下载地址必须位于配置仓库的 Releases 下载路径下".to_string());
  }
  let rest = &trimmed[prefix.len()..];
  if rest.is_empty() || rest.contains(['?', '#']) {
    return Err("更新包下载地址缺少文件名".to_string());
  }
  let parsed = Url::parse(trimmed).map_err(|_| "无效的更新包下载地址".to_string())?;
  if parsed.scheme() != "https" {
    return Err("仅允许通过 HTTPS 下载更新包".to_string());
  }
  if !parsed.username().is_empty() || parsed.password().is_some() {
    return Err("更新包下载地址不得包含用户信息".to_string());
  }
  let host = parsed
    .host_str()
    .ok_or_else(|| "更新地址缺少主机名".to_string())?;
  if !host.eq_ignore_ascii_case("github.com") {
    return Err(format!("不支持的更新源主机: {host}"));
  }
  Ok(parsed)
}

fn sanitize_filename(raw: Option<&str>, fallback: &str) -> String {
  let base = raw
    .unwrap_or("")
    .rsplit(['/', '\\'])
    .next()
    .unwrap_or("")
    .trim();
  let cleaned: String = base
    .chars()
    .map(|c| {
      if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
        c
      } else {
        '_'
      }
    })
    .collect();
  let trimmed = cleaned.trim_start_matches('.').to_string();
  if trimmed.is_empty() || trimmed == ".." {
    fallback.to_string()
  } else {
    trimmed
  }
}

fn default_filename_for_platform(version: &str, url: &str) -> String {
  let from_url = Url::parse(url).ok().and_then(|u| {
    u.path_segments()
      .and_then(|mut s| s.next_back().map(str::to_string))
  });
  if let Some(name) = from_url {
    if !name.trim().is_empty() {
      return sanitize_filename(Some(&name), "update.bin");
    }
  }
  let ver = clean_version(version);
  match crate::platform::current_platform_name() {
    "macos" => format!("account-manager-{ver}-macos.app.tar.gz"),
    "windows" => format!("account-manager-{ver}-windows-setup.exe"),
    "android" => format!("account-manager-{ver}-android.apk"),
    "ios" => "account-manager-ios-unsigned.ipa".to_string(),
    _ => format!("account-manager-{ver}.bin"),
  }
}

fn select_github_asset<'a>(assets: &'a [GithubAsset], platform: &str) -> Option<&'a GithubAsset> {
  match platform {
    "macos" => assets
      .iter()
      .find(|a| a.name.to_ascii_lowercase().ends_with(".app.tar.gz"))
      .or_else(|| {
        assets
          .iter()
          .find(|a| a.name.to_ascii_lowercase().ends_with(".dmg"))
      }),
    "windows" => assets
      .iter()
      .find(|a| a.name.to_ascii_lowercase().ends_with("-setup.exe"))
      .or_else(|| {
        assets
          .iter()
          .find(|a| a.name.to_ascii_lowercase().ends_with(".exe"))
      }),
    "android" => assets
      .iter()
      .find(|a| a.name.to_ascii_lowercase().ends_with("-release.apk"))
      .or_else(|| {
        assets
          .iter()
          .find(|a| a.name.to_ascii_lowercase().ends_with(".apk"))
      }),
    "ios" => assets
      .iter()
      .find(|a| a.name.to_ascii_lowercase().ends_with("-ios-unsigned.ipa"))
      .or_else(|| {
        assets
          .iter()
          .find(|a| a.name.to_ascii_lowercase().ends_with(".ipa"))
      }),
    _ => None,
  }
}

fn extract_sha256_digest(raw: Option<&str>) -> Option<String> {
  let clean = raw?.trim();
  if clean.is_empty() {
    return None;
  }
  let hex = clean
    .strip_prefix("sha256:")
    .or_else(|| clean.strip_prefix("SHA256:"))
    .unwrap_or(clean)
    .trim();
  if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
    Some(hex.to_ascii_lowercase())
  } else {
    None
  }
}

/// sha256 必填：缺失或格式非法直接拒绝，不下载、不复用、不安装。
/// 这是桌面端与 Android 分支的单点关口——`download_and_install_update`
/// 是唯一调用 Android 插件 `downloadAndInstallApk` 的入口，空 sha256
/// 到不了 Kotlin 侧（插件内 `expectedSha == null` 的放行逻辑因此不可达）。
fn require_expected_sha256(sha: Option<&str>) -> Result<String, String> {
  extract_sha256_digest(sha).ok_or_else(|| "缺少更新包 SHA-256，拒绝安装".to_string())
}

/// 每一跳重定向目标都必须满足：https 且主机在白名单内。
/// 白名单：github.com、api.github.com（清单 API 自身跳转，见笔记）、
/// objects.githubusercontent.com、release-assets.githubusercontent.com。
/// 子域名不在白名单内（如 github.com.evil.com、sub.github.com）。
fn is_allowed_redirect_target(url: &Url) -> bool {
  if url.scheme() != "https" {
    return false;
  }
  let Some(host) = url.host_str() else {
    return false;
  };
  matches!(
    host.to_ascii_lowercase().as_str(),
    "github.com"
      | "api.github.com"
      | "objects.githubusercontent.com"
      | "release-assets.githubusercontent.com"
  )
}

#[cfg(desktop)]
fn build_http_client(user_agent: &str, timeout_secs: u64) -> Result<reqwest::Client, String> {
  reqwest::Client::builder()
    .user_agent(user_agent)
    .connect_timeout(std::time::Duration::from_secs(15))
    .timeout(std::time::Duration::from_secs(timeout_secs))
    .redirect(reqwest::redirect::Policy::custom(|attempt| {
      // previous 含初始 URL；超过 10 跳即停（与旧 Policy::limited(10) 同限）。
      // stop() 会把 30x 响应原样返回，调用方按非 2xx 报错。
      if attempt.previous().len() > 10 || !is_allowed_redirect_target(attempt.url()) {
        attempt.stop()
      } else {
        attempt.follow()
      }
    }))
    .build()
    .map_err(|e| format!("创建网络客户端失败: {e}"))
}

async fn fetch_text_with_status(
  app: &AppHandle,
  url: &str,
  user_agent: &str,
  accept: Option<&str>,
) -> Result<(u16, String), String> {
  #[cfg(desktop)]
  {
    let _ = app;
    let client = build_http_client(user_agent, 25)?;
    let mut req = client.get(url);
    if let Some(acc) = accept {
      req = req.header("Accept", acc);
    }
    let resp = req
      .send()
      .await
      .map_err(|e| format!("检查更新网络请求失败: {e}"))?;
    if resp.url().scheme() != "https" {
      return Err("更新响应重定向到了非 HTTPS 地址".to_string());
    }
    let status = resp.status().as_u16();
    let body = resp
      .text()
      .await
      .map_err(|e| format!("读取更新响应内容失败: {e}"))?;
    Ok((status, body))
  }

  #[cfg(mobile)]
  {
    let res = crate::platform::mobile::fetch_text(app, url, user_agent, accept).await?;
    Ok((res.status, res.body))
  }
}

/// 未配置更新源时返回的结果：不发起任何网络请求，只提示已关闭。
fn disabled_check_result(current_version: &str) -> UpdateCheckResult {
  UpdateCheckResult {
    current_version: current_version.to_string(),
    latest_version: current_version.to_string(),
    has_update: false,
    release_notes: "未配置更新源（编译时未设置 MAAM_UPDATE_REPO），检查更新与一键安装已关闭。"
      .to_string(),
    pub_date: None,
    download_url: None,
    sha256: None,
    filename: None,
    size: None,
    release_page_url: String::new(),
    updates_disabled: true,
  }
}

pub async fn check_for_updates(app: &AppHandle) -> Result<UpdateCheckResult, String> {
  let current_version = app.package_info().version.to_string();
  let Some(repo) = update_repo() else {
    return Ok(disabled_check_result(&current_version));
  };
  let platform_key = crate::platform::current_platform_name();
  let user_agent = format!("multi-agent-account-manager/{current_version}");

  if let Ok((200..=299, body)) = fetch_text_with_status(
    app,
    &latest_manifest_url(&repo),
    &user_agent,
    Some("application/json"),
  )
  .await
  {
    if let Ok(manifest) = serde_json::from_str::<LatestManifest>(&body) {
      let latest_version = clean_version(&manifest.version).to_string();
      let asset = manifest.platforms.get(platform_key);
      let has_update = is_newer_version(&current_version, &latest_version);
      return Ok(UpdateCheckResult {
        current_version,
        latest_version: if latest_version.is_empty() {
          app.package_info().version.to_string()
        } else {
          latest_version
        },
        has_update,
        release_notes: manifest.notes,
        pub_date: manifest.pub_date,
        download_url: asset.map(|a| a.url.clone()),
        sha256: asset.and_then(|a| extract_sha256_digest(a.sha256.as_deref())),
        filename: asset.and_then(|a| a.filename.clone()),
        size: asset.and_then(|a| a.size),
        release_page_url: manifest
          .release_page_url
          .unwrap_or_else(|| releases_page_url(&repo)),
        updates_disabled: false,
      });
    }
  }

  let (status, body) = fetch_text_with_status(
    app,
    &github_latest_api_url(&repo),
    &user_agent,
    Some("application/vnd.github+json"),
  )
  .await?;

  if status == 404 {
    return Ok(UpdateCheckResult {
      current_version: current_version.clone(),
      latest_version: current_version,
      has_update: false,
      release_notes: "当前仓库尚未发布任何 GitHub Release 版本。".to_string(),
      pub_date: None,
      download_url: None,
      sha256: None,
      filename: None,
      size: None,
      release_page_url: releases_page_url(&repo),
      updates_disabled: false,
    });
  }

  if !(200..=299).contains(&status) {
    return Err(format!("检查更新失败（GitHub API HTTP {status}）"));
  }

  let release: GithubRelease =
    serde_json::from_str(&body).map_err(|e| format!("解析更新信息失败: {e}"))?;
  let latest_version = clean_version(&release.tag_name).to_string();
  let has_update = is_newer_version(&current_version, &latest_version);
  let asset = select_github_asset(&release.assets, platform_key);

  Ok(UpdateCheckResult {
    current_version: current_version.clone(),
    latest_version: if latest_version.is_empty() {
      current_version
    } else {
      latest_version
    },
    has_update,
    release_notes: release.body.unwrap_or_default(),
    pub_date: release.published_at,
    download_url: asset.map(|a| a.browser_download_url.clone()),
    sha256: asset.and_then(|a| extract_sha256_digest(a.digest.as_deref())),
    filename: asset.map(|a| a.name.clone()),
    size: asset.map(|a| a.size),
    release_page_url: release.html_url.unwrap_or_else(|| releases_page_url(&repo)),
    updates_disabled: false,
  })
}

#[cfg(desktop)]
fn emit_progress(
  app: &AppHandle,
  stage: &str,
  downloaded: u64,
  total: Option<u64>,
  percent: Option<u8>,
  message: &str,
) {
  // 只发给 manager webview：账号页不应收到更新进度（可借此探测版本/更新态）。
  let _ = app.emit_to(
    "manager",
    "update-progress",
    UpdateProgressEvent {
      stage: stage.to_string(),
      downloaded,
      total,
      percent,
      message: message.to_string(),
    },
  );
}

#[cfg(desktop)]
/// 更新包下载上限（512 MiB）：服务端谎报/不报 Content-Length 时，
/// 流式循环里按已下载量硬停，避免无限下载撑满磁盘。清单里声明的
/// size 仅作展示，不作为放行依据。
const MAX_UPDATE_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;

#[cfg(desktop)]
async fn download_to_updates_cache(
  app: &AppHandle,
  url: &str,
  expected_sha256: &str,
  filename: &str,
  user_agent: &str,
) -> Result<(std::path::PathBuf, u64), String> {
  use sha2::{Digest, Sha256};
  use std::io::Write;
  use tauri::Manager;

  let cache_root = app
    .path()
    .app_cache_dir()
    .map_err(|e| format!("获取缓存目录失败: {e}"))?;
  let updates_dir = cache_root.join("updates");
  std::fs::create_dir_all(&updates_dir).map_err(|e| format!("创建更新目录失败: {e}"))?;

  let dest_path = updates_dir.join(filename);
  // sha256 必填：非法值在调用方已拒绝，这里再校验一次。
  let expected_hex = require_expected_sha256(Some(expected_sha256))?;

  // 缓存复用只在哈希一致时成立。
  if dest_path.is_file() {
    if let Ok(actual) = compute_file_sha256_hex(&dest_path) {
      if actual == expected_hex {
        let len = std::fs::metadata(&dest_path).map(|m| m.len()).unwrap_or(0);
        return Ok((dest_path, len));
      }
    }
  }

  let part_path = updates_dir.join(format!("{filename}.part"));
  let _ = std::fs::remove_file(&part_path);

  emit_progress(
    app,
    "downloading",
    0,
    None,
    Some(0),
    "正在连接下载服务器...",
  );

  let client = build_http_client(user_agent, 600)?;
  let mut resp = client
    .get(url)
    .send()
    .await
    .map_err(|e| format!("下载更新包失败: {e}"))?;

  if resp.url().scheme() != "https" {
    return Err("下载响应重定向到了非 HTTPS 地址".to_string());
  }
  if !resp.status().is_success() {
    return Err(format!("下载更新包失败（HTTP {}）", resp.status().as_u16()));
  }

  let total = resp.content_length().filter(|&n| n > 0);
  // Content-Length 预检：声明就超过上限直接拒绝，不建临时文件。
  if total.is_some_and(|t| t > MAX_UPDATE_DOWNLOAD_BYTES) {
    return Err(format!(
      "更新包声明大小超过 {} 字节上限，已拒绝下载",
      MAX_UPDATE_DOWNLOAD_BYTES
    ));
  }
  let mut file =
    std::fs::File::create(&part_path).map_err(|e| format!("创建临时更新文件失败: {e}"))?;
  let mut hasher = Sha256::new();
  let mut downloaded: u64 = 0;
  let mut last_emit = std::time::Instant::now();
  let mut last_percent: Option<u8> = None;

  while let Some(chunk) = resp.chunk().await.map_err(|e| {
    let _ = std::fs::remove_file(&part_path);
    format!("读取下载数据失败: {e}")
  })? {
    file.write_all(&chunk).map_err(|e| {
      let _ = std::fs::remove_file(&part_path);
      format!("写入临时更新文件失败: {e}")
    })?;
    hasher.update(&chunk);
    downloaded = downloaded.saturating_add(chunk.len() as u64);
    // 流式硬上限：Content-Length 缺失或谎报时按已下载量停，
    // 删临时文件并报错（sha256 校验在下载完成后，不兜底）。
    if downloaded > MAX_UPDATE_DOWNLOAD_BYTES {
      drop(file);
      let _ = std::fs::remove_file(&part_path);
      return Err(format!(
        "更新包超过 {} 字节上限，已停止下载",
        MAX_UPDATE_DOWNLOAD_BYTES
      ));
    }

    let pct = total.map(|t| ((downloaded.saturating_mul(100)) / t).min(100) as u8);
    if last_emit.elapsed() >= std::time::Duration::from_millis(150)
      || (pct.is_some() && pct != last_percent)
    {
      last_emit = std::time::Instant::now();
      last_percent = pct;
      emit_progress(
        app,
        "downloading",
        downloaded,
        total,
        pct,
        "正在下载更新包...",
      );
    }
  }

  file.flush().map_err(|e| format!("刷新缓存失败: {e}"))?;
  drop(file);

  emit_progress(
    app,
    "verifying",
    downloaded,
    total.or(Some(downloaded)),
    Some(100),
    "正在校验更新包完整性...",
  );

  let actual_hex = format!("{:x}", hasher.finalize());
  if actual_hex != expected_hex {
    let _ = std::fs::remove_file(&part_path);
    return Err("更新包 SHA-256 校验失败，请重试".to_string());
  }

  if dest_path.exists() {
    let _ = std::fs::remove_file(&dest_path);
  }
  std::fs::rename(&part_path, &dest_path).map_err(|e| {
    let _ = std::fs::remove_file(&part_path);
    format!("保存更新包失败: {e}")
  })?;

  if let Ok(entries) = std::fs::read_dir(&updates_dir) {
    for entry in entries.flatten() {
      let p = entry.path();
      if p != dest_path {
        if p.is_dir() {
          let _ = std::fs::remove_dir_all(&p);
        } else {
          let _ = std::fs::remove_file(&p);
        }
      }
    }
  }

  Ok((dest_path, downloaded))
}

#[cfg(desktop)]
fn compute_file_sha256_hex(path: &std::path::Path) -> Result<String, std::io::Error> {
  use sha2::{Digest, Sha256};
  use std::io::Read;

  let mut file = std::fs::File::open(path)?;
  let mut hasher = Sha256::new();
  let mut buf = [0u8; 16 * 1024];
  loop {
    let n = file.read(&mut buf)?;
    if n == 0 {
      break;
    }
    hasher.update(&buf[..n]);
  }
  Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(target_os = "macos")]
fn find_current_app_bundle() -> Option<std::path::PathBuf> {
  let exe = std::env::current_exe().ok()?;
  let mut cur = exe.as_path();
  while let Some(parent) = cur.parent() {
    if parent
      .extension()
      .and_then(|s| s.to_str())
      .map(|ext| ext.eq_ignore_ascii_case("app"))
      .unwrap_or(false)
    {
      return Some(parent.to_path_buf());
    }
    cur = parent;
  }
  None
}

#[cfg(target_os = "macos")]
fn has_write_permission(dir: &std::path::Path) -> bool {
  let probe = dir.join(format!(".maam-write-probe-{}", std::process::id()));
  match std::fs::OpenOptions::new()
    .write(true)
    .create_new(true)
    .open(&probe)
  {
    Ok(_) => {
      let _ = std::fs::remove_file(&probe);
      true
    }
    Err(_) => false,
  }
}

/// macOS 覆盖更新用的 swap 脚本（仅 macOS 与测试目标可见，便于单元
/// 测试检查回滚与超时逻辑；只在 macOS 安装分支使用）。
/// 参数：$1 旧进程 pid、$2 新 .app、$3 目标 .app、$4 解压目录。
/// 行为：等旧进程退出（最多 10 秒，超时直接 exit 2 中止，不替换）；
/// 替换失败时先 rm -rf 目标（清掉跨卷失败留下的半截目录）再把备份移回。
#[cfg(any(target_os = "macos", test))]
const MACOS_SWAP_SCRIPT: &str = r#"pid="$1"
src_app="$2"
dst_app="$3"
extract_dir="$4"
i=0
while kill -0 "$pid" 2>/dev/null && [ "$i" -lt 100 ]; do
  sleep 0.1
  i=$((i + 1))
done
if kill -0 "$pid" 2>/dev/null; then
  exit 2
fi
backup="${dst_app}.maam-backup.$$"
rm -rf "$backup" 2>/dev/null || true
if [ -e "$dst_app" ]; then
  mv "$dst_app" "$backup" || exit 1
fi
if mv "$src_app" "$dst_app"; then
  rm -rf "$backup" "$extract_dir" 2>/dev/null || true
  /usr/bin/xattr -dr com.apple.quarantine "$dst_app" 2>/dev/null || true
  /usr/bin/open -n "$dst_app"
else
  if [ -e "$backup" ]; then
    rm -rf "$dst_app" 2>/dev/null || true
    mv "$backup" "$dst_app" 2>/dev/null || true
  fi
  exit 1
fi"#;

#[cfg(target_os = "macos")]
async fn install_macos_update(
  app: &AppHandle,
  dest_path: &std::path::Path,
  downloaded: u64,
) -> Result<InstallUpdateOutcome, String> {
  let lower_name = dest_path
    .file_name()
    .and_then(|n| n.to_str())
    .unwrap_or("")
    .to_ascii_lowercase();

  if lower_name.ends_with(".dmg") {
    open::that_detached(dest_path).map_err(|e| format!("打开 DMG 安装包失败: {e}"))?;
    let msg = "已打开 DMG 镜像，请将应用拖入 Applications 完成覆盖安装";
    emit_progress(app, "ready", downloaded, Some(downloaded), Some(100), msg);
    return Ok(InstallUpdateOutcome {
      status: "installer_launched".to_string(),
      message: msg.to_string(),
    });
  }

  emit_progress(
    app,
    "installing",
    downloaded,
    Some(downloaded),
    Some(100),
    "正在解压新版本应用...",
  );

  let parent_dir = dest_path
    .parent()
    .ok_or_else(|| "更新包路径异常".to_string())?
    .to_path_buf();
  let extract_dir = parent_dir.join(format!(
    "extract-{}",
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map(|d| d.as_secs())
      .unwrap_or(0)
  ));
  let archive_path = dest_path.to_path_buf();
  let extract_dir_for_task = extract_dir.clone();

  let extracted_app = tokio::task::spawn_blocking(move || -> Result<std::path::PathBuf, String> {
    if extract_dir_for_task.exists() {
      let _ = std::fs::remove_dir_all(&extract_dir_for_task);
    }
    std::fs::create_dir_all(&extract_dir_for_task).map_err(|e| format!("创建解压目录失败: {e}"))?;

    let status = std::process::Command::new("/usr/bin/tar")
      .arg("-xzf")
      .arg(&archive_path)
      .arg("-C")
      .arg(&extract_dir_for_task)
      .status()
      .map_err(|e| format!("调用 tar 解压失败: {e}"))?;

    if !status.success() {
      return Err("解压更新压缩包失败".to_string());
    }

    let mut found_app = None;
    for entry in std::fs::read_dir(&extract_dir_for_task)
      .map_err(|e| format!("扫描解压目录失败: {e}"))?
      .flatten()
    {
      let p = entry.path();
      if p.is_dir()
        && p
          .extension()
          .and_then(|s| s.to_str())
          .map(|ext| ext.eq_ignore_ascii_case("app"))
          .unwrap_or(false)
      {
        found_app = Some(p);
        break;
      }
    }

    let app_bundle = found_app.ok_or_else(|| "更新包中未找到 .app 应用目录".to_string())?;
    // 注意：不要在这里去隔离属性。去隔离只在 swap 脚本成功替换后
    // 执行一次（见 install_macos_update 的 swap 脚本）；而哈希校验
    // 已在下载步骤（download_to_updates_cache）完成，去隔离发生在
    // 校验通过之后。解压后立即去隔离会让未经替换的新 .app 在校验
    // 链外就绕过 Gatekeeper。
    Ok(app_bundle)
  })
  .await
  .map_err(|e| format!("解压任务中断: {e}"))??;

  let Some(current_app) = find_current_app_bundle() else {
    open::that_detached(&extract_dir).map_err(|e| format!("打开新版本目录失败: {e}"))?;
    let msg = "当前处于开发运行模式，已解压新版本到目录，请手动替换";
    emit_progress(app, "ready", downloaded, Some(downloaded), Some(100), msg);
    return Ok(InstallUpdateOutcome {
      status: "installer_launched".to_string(),
      message: msg.to_string(),
    });
  };

  let app_parent = current_app
    .parent()
    .ok_or_else(|| "无法定位当前应用所在目录".to_string())?;
  if !has_write_permission(app_parent) {
    open::that_detached(&extract_dir).map_err(|e| format!("打开新版本目录失败: {e}"))?;
    let msg = "当前应用目录无写入权限，已打开新版本所在目录，请手动拖入替换";
    emit_progress(app, "ready", downloaded, Some(downloaded), Some(100), msg);
    return Ok(InstallUpdateOutcome {
      status: "installer_launched".to_string(),
      message: msg.to_string(),
    });
  }

  let swap_script = MACOS_SWAP_SCRIPT;

  std::process::Command::new("/bin/sh")
    .arg("-c")
    .arg(swap_script)
    .arg("_")
    .arg(std::process::id().to_string())
    .arg(&extracted_app)
    .arg(&current_app)
    .arg(&extract_dir)
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()
    .map_err(|e| format!("启动覆盖更新进程失败: {e}"))?;

  let msg = "更新已就绪，正在重启应用...";
  emit_progress(app, "ready", downloaded, Some(downloaded), Some(100), msg);

  let app_clone = app.clone();
  tauri::async_runtime::spawn(async move {
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    app_clone.exit(0);
  });

  Ok(InstallUpdateOutcome {
    status: "restarting".to_string(),
    message: msg.to_string(),
  })
}

#[cfg(windows)]
async fn install_windows_update(
  app: &AppHandle,
  dest_path: &std::path::Path,
  downloaded: u64,
) -> Result<InstallUpdateOutcome, String> {
  let lower_name = dest_path
    .file_name()
    .and_then(|n| n.to_str())
    .unwrap_or("")
    .to_ascii_lowercase();
  if !lower_name.ends_with(".exe") {
    return Err("不支持的 Windows 安装包格式".to_string());
  }

  emit_progress(
    app,
    "installing",
    downloaded,
    Some(downloaded),
    Some(100),
    "正在启动安装程序...",
  );

  std::process::Command::new(dest_path)
    .args(["/P", "/UPDATE", "/R"])
    .spawn()
    .map_err(|e| format!("启动安装程序失败: {e}"))?;

  let msg = "已启动安装程序，正在退出并完成替换...";
  emit_progress(app, "ready", downloaded, Some(downloaded), Some(100), msg);

  let app_clone = app.clone();
  tauri::async_runtime::spawn(async move {
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    app_clone.exit(0);
  });

  Ok(InstallUpdateOutcome {
    status: "restarting".to_string(),
    message: msg.to_string(),
  })
}

/// 安装时按最新清单重新核对前端传来的请求：只接受与清单一致的
/// version / download_url / sha256 / filename，不再信任前端传值。
/// 返回本次安装应使用的文件名（已清洗）。
fn verify_install_request_against_manifest(
  req: &InstallUpdateRequest,
  expected_sha: &str,
  manifest: &UpdateCheckResult,
) -> Result<String, String> {
  if manifest.updates_disabled {
    return Err("未配置更新源，无法核对更新清单".to_string());
  }
  if clean_version(&req.version) != manifest.latest_version {
    return Err("请求安装的版本与最新更新清单不一致，请重新检查更新".to_string());
  }
  let Some(manifest_url) = manifest.download_url.as_deref() else {
    return Err("最新更新清单中没有本平台的安装包".to_string());
  };
  if manifest_url != req.download_url {
    return Err("下载地址与最新更新清单不一致，拒绝安装".to_string());
  }
  let Some(manifest_sha) = manifest.sha256.as_deref() else {
    return Err("最新更新清单中缺少 SHA-256，拒绝安装".to_string());
  };
  if manifest_sha != expected_sha {
    return Err("SHA-256 与最新更新清单不一致，拒绝安装".to_string());
  }
  let filename = match manifest.filename.as_deref() {
    Some(manifest_name) => {
      let wanted = sanitize_filename(Some(manifest_name), "");
      let given = sanitize_filename(req.filename.as_deref(), "");
      if !given.is_empty() && given != wanted {
        return Err("文件名与最新更新清单不一致，拒绝安装".to_string());
      }
      wanted
    }
    None => sanitize_filename(
      req.filename.as_deref(),
      &default_filename_for_platform(&req.version, manifest_url),
    ),
  };
  if filename.is_empty() {
    return Err("无法确定安装包文件名".to_string());
  }
  Ok(filename)
}

pub async fn download_and_install_update(
  app: &AppHandle,
  req: InstallUpdateRequest,
) -> Result<InstallUpdateOutcome, String> {
  let Some(repo) = update_repo() else {
    return Err("未配置更新源（编译时未设置 MAAM_UPDATE_REPO），无法下载更新".to_string());
  };
  let validated_url = validate_download_url(&req.download_url, &repo)?;
  // sha256 必填：所有平台分支共用的关口，缺失直接拒绝。
  let expected_sha = require_expected_sha256(req.sha256.as_deref())?;
  let _guard = UpdateLockGuard::try_acquire()?;
  // 安装时重新取最新清单核对，不再信任前端传来的 url / sha256 / 文件名。
  let manifest = check_for_updates(app).await?;
  let filename = verify_install_request_against_manifest(&req, &expected_sha, &manifest)?;
  let current_version = app.package_info().version.to_string();
  // 防降级：清单说没有更新（最新版不高于当前版）时拒绝安装。只靠
  // verify 核对 req.version == manifest.latest_version 不够——一份
  // 指向旧版本的清单（过期缓存/被篡改）会让旧版覆盖新版。
  if !manifest.has_update {
    return Err(format!(
      "最新版本 {} 不高于当前版本 {}，拒绝安装（防降级）",
      manifest.latest_version, current_version
    ));
  }
  let user_agent = format!("multi-agent-account-manager/{current_version}");

  #[cfg(target_os = "android")]
  {
    let res = crate::platform::mobile::download_and_install_apk(
      app,
      validated_url.as_str(),
      Some(expected_sha.as_str()),
      &filename,
      &user_agent,
    )
    .await?;
    Ok(InstallUpdateOutcome {
      status: res.status,
      message: res.message,
    })
  }

  #[cfg(target_os = "ios")]
  {
    // iOS 只打开下载地址（Safari 内下载、用户自签安装），无法在应用内
    // 校验 IPA；但请求仍须携带 sha256（上方统一关口），缺失即拒绝。
    let _ = (filename, user_agent, &expected_sha);
    crate::platform::mobile::open_external_url(app, validated_url.as_str()).await?;
    Ok(InstallUpdateOutcome {
      status: "installer_launched".to_string(),
      message: "已在 Safari 打开新版 IPA 下载地址，下载后请通过自签工具覆盖安装".to_string(),
    })
  }

  #[cfg(desktop)]
  {
    let (dest_path, downloaded) = match download_to_updates_cache(
      app,
      validated_url.as_str(),
      &expected_sha,
      &filename,
      &user_agent,
    )
    .await
    {
      Ok(pair) => pair,
      Err(err) => {
        emit_progress(app, "error", 0, None, None, &err);
        return Err(err);
      }
    };

    #[cfg(target_os = "macos")]
    {
      match install_macos_update(app, &dest_path, downloaded).await {
        Ok(outcome) => Ok(outcome),
        Err(err) => {
          emit_progress(app, "error", downloaded, Some(downloaded), None, &err);
          Err(err)
        }
      }
    }

    #[cfg(windows)]
    {
      match install_windows_update(app, &dest_path, downloaded).await {
        Ok(outcome) => Ok(outcome),
        Err(err) => {
          emit_progress(app, "error", downloaded, Some(downloaded), None, &err);
          Err(err)
        }
      }
    }

    #[cfg(all(not(target_os = "macos"), not(windows)))]
    {
      open::that_detached(&dest_path).map_err(|e| format!("打开更新文件失败: {e}"))?;
      let msg = "已下载更新包并打开文件";
      emit_progress(app, "ready", downloaded, Some(downloaded), Some(100), msg);
      Ok(InstallUpdateOutcome {
        status: "installer_launched".to_string(),
        message: msg.to_string(),
      })
    }
  }
}

/// 「发布页」只允许打开配置仓库的 Releases 页：
/// `https://github.com/<owner>/<repo>/releases` 或
/// `https://github.com/<owner>/<repo>/releases/tag/<tag>`。
/// 旧实现只校验 scheme 为 https：清单里的 release_page_url 未经核对，
/// 前端传任意 https 链接都会被打开（P2 第二轮审查新发现，确认成立——
/// 唯一调用方是更新弹窗的「发布页」按钮，URL 来自清单）。
fn validate_releases_page_url(raw: &str, repo: &str) -> Result<Url, String> {
  let parsed = Url::parse(raw.trim()).map_err(|_| "无效的发布页地址".to_string())?;
  if parsed.scheme() != "https" {
    return Err("仅支持打开 HTTPS 链接".to_string());
  }
  if !parsed.username().is_empty()
    || parsed.password().is_some()
    || parsed.query().is_some()
    || parsed.fragment().is_some()
  {
    return Err("发布页地址不得包含用户信息、查询参数或片段".to_string());
  }
  let host = parsed.host_str().unwrap_or("");
  if !host.eq_ignore_ascii_case("github.com") {
    return Err(format!("不支持的发布页主机: {host}"));
  }
  let (owner, name) = repo.split_once('/').unwrap_or(("", ""));
  let segments: Vec<String> = parsed
    .path_segments()
    .map(|s| s.map(str::to_string).collect())
    .unwrap_or_default();
  let is_releases_page = segments.len() >= 3
    && segments[0].eq_ignore_ascii_case(owner)
    && segments[1].eq_ignore_ascii_case(name)
    && segments[2].eq_ignore_ascii_case("releases")
    && (segments.len() == 3
      || (segments.len() >= 5
        && segments[3].eq_ignore_ascii_case("tag")
        && !segments[4].is_empty()));
  if !is_releases_page {
    return Err("发布页地址必须位于配置仓库的 Releases 下".to_string());
  }
  Ok(parsed)
}

pub async fn open_external_url(app: &AppHandle, raw_url: &str) -> Result<(), String> {
  let Some(repo) = update_repo() else {
    return Err("未配置更新源，无法打开发布页".to_string());
  };
  let parsed = validate_releases_page_url(raw_url, &repo)?;

  #[cfg(desktop)]
  {
    let _ = app;
    crate::platform::open_external_browser(parsed.as_str())
  }

  #[cfg(mobile)]
  {
    crate::platform::mobile::open_external_url(app, parsed.as_str()).await
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parse_update_repo_accepts_owner_slash_repo() {
    assert_eq!(
      parse_update_repo("ldjx7/multi-agent-account-manager"),
      Some("ldjx7/multi-agent-account-manager".to_string())
    );
    assert_eq!(
      parse_update_repo("  my-org/my_repo.v2  "),
      Some("my-org/my_repo.v2".to_string())
    );
  }

  #[test]
  fn parse_update_repo_rejects_bad_shapes() {
    for raw in [
      "",
      "onlyowner",
      "a/b/c",
      "/repo",
      "owner/",
      "owner/repo with space",
      "owner/repo?x=1",
      "https://github.com/owner/repo",
    ] {
      assert_eq!(parse_update_repo(raw), None, "应拒绝: {raw}");
    }
  }

  #[test]
  fn update_repo_urls_derive_from_configured_repo() {
    let repo = "my-org/my-fork";
    assert_eq!(
      latest_manifest_url(repo),
      "https://github.com/my-org/my-fork/releases/latest/download/latest.json"
    );
    assert_eq!(
      github_latest_api_url(repo),
      "https://api.github.com/repos/my-org/my-fork/releases/latest"
    );
    assert_eq!(
      releases_page_url(repo),
      "https://github.com/my-org/my-fork/releases"
    );
  }

  #[test]
  fn disabled_check_result_reports_updates_off() {
    let res = disabled_check_result("0.1.10");
    assert!(res.updates_disabled);
    assert!(!res.has_update);
    assert_eq!(res.current_version, "0.1.10");
    assert_eq!(res.latest_version, "0.1.10");
    assert!(res.download_url.is_none());
    assert!(res.sha256.is_none());
    assert!(res.release_page_url.is_empty());
  }

  #[test]
  fn is_newer_version_compares_prereleases() {
    // 任务书 P0-2-8：1.0.0-rc.1 → 1.0.0-rc.2 必须为 true。
    assert!(is_newer_version("1.0.0-rc.1", "1.0.0-rc.2"));
    assert!(is_newer_version("1.0.0-rc.1", "1.0.0"));
    assert!(is_newer_version("1.0.0-alpha", "1.0.0-alpha.1"));
    assert!(!is_newer_version("1.0.0", "1.0.0-rc.1"));
    assert!(!is_newer_version("1.0.0-rc.2", "1.0.0-rc.1"));
  }

  #[test]
  fn is_newer_version_handles_short_and_prefixed_versions() {
    assert!(is_newer_version("1.2", "1.10"));
    assert!(is_newer_version("v1.2.3", "1.2.4"));
    assert!(!is_newer_version("1.0.0", "1.0.0"));
    assert!(!is_newer_version("1.0.0+build1", "1.0.0+build2"));
    assert!(is_newer_version("1.0.0", "1.0.1"));
    assert!(!is_newer_version("2.0.0", "1.9.9"));
  }

  const TEST_REPO: &str = "my-org/my-fork";

  #[test]
  fn validate_download_url_accepts_configured_repo_release() {
    let url = validate_download_url(
      "https://github.com/my-org/my-fork/releases/download/v0.1.10/account-manager-0.1.10-macos.app.tar.gz",
      TEST_REPO,
    )
    .expect("合法地址应通过");
    assert_eq!(url.host_str(), Some("github.com"));
  }

  #[test]
  fn validate_download_url_rejects_other_repos_and_spoof_hosts() {
    for raw in [
      // 其他用户仓库
      "https://github.com/other-user/other-repo/releases/download/v1.0.0/app.tar.gz",
      // http://
      "http://github.com/my-org/my-fork/releases/download/v1.0.0/app.tar.gz",
      // 子域名伪装
      "https://github.com.evil.com/my-org/my-fork/releases/download/v1.0.0/app.tar.gz",
      // userinfo 伪装
      "https://github.com@evil.com/my-org/my-fork/releases/download/v1.0.0/app.tar.gz",
      // 后缀伪装
      "https://evilgithub.com/my-org/my-fork/releases/download/v1.0.0/app.tar.gz",
      // 无文件名
      "https://github.com/my-org/my-fork/releases/download/",
      // 同 owner 不同仓库
      "https://github.com/my-org/other-repo/releases/download/v1.0.0/app.tar.gz",
      // 非 download 路径
      "https://github.com/my-org/my-fork/releases/tag/v1.0.0",
      // 重定向目标主机不作为初始下载地址（只允许出现在重定向链中）
      "https://objects.githubusercontent.com/1234/app.tar.gz",
    ] {
      assert!(
        validate_download_url(raw, TEST_REPO).is_err(),
        "应拒绝: {raw}"
      );
    }
  }

  #[test]
  fn require_expected_sha256_rejects_missing_or_malformed() {
    // 缺失直接拒绝：覆盖桌面端与 Android 分支（Rust 侧单点关口）。
    assert!(require_expected_sha256(None).is_err());
    assert!(require_expected_sha256(Some("")).is_err());
    assert!(require_expected_sha256(Some("abc")).is_err());
    assert!(require_expected_sha256(Some("zz".repeat(32).as_str())).is_err());
    assert!(require_expected_sha256(Some("aa".repeat(31).as_str())).is_err());
  }

  #[test]
  fn require_expected_sha256_normalizes_valid_hex() {
    let lower = "ab".repeat(32);
    assert_eq!(
      require_expected_sha256(Some(&lower)).as_deref(),
      Ok(lower.as_str())
    );
    assert_eq!(
      require_expected_sha256(Some(&format!("SHA256:{}", lower.to_ascii_uppercase()))).as_deref(),
      Ok(lower.as_str())
    );
  }

  fn test_manifest() -> UpdateCheckResult {
    UpdateCheckResult {
      current_version: "0.1.9".to_string(),
      latest_version: "0.1.10".to_string(),
      has_update: true,
      release_notes: String::new(),
      pub_date: None,
      download_url: Some(
        "https://github.com/my-org/my-fork/releases/download/v0.1.10/app.tar.gz".to_string(),
      ),
      sha256: Some("ab".repeat(32)),
      filename: Some("app.tar.gz".to_string()),
      size: Some(123),
      release_page_url: "https://github.com/my-org/my-fork/releases".to_string(),
      updates_disabled: false,
    }
  }

  fn test_request() -> InstallUpdateRequest {
    InstallUpdateRequest {
      version: "0.1.10".to_string(),
      download_url: "https://github.com/my-org/my-fork/releases/download/v0.1.10/app.tar.gz"
        .to_string(),
      sha256: Some("ab".repeat(32)),
      filename: Some("app.tar.gz".to_string()),
    }
  }

  #[test]
  fn verify_install_request_accepts_matching_manifest() {
    let req = test_request();
    let manifest = test_manifest();
    let filename =
      verify_install_request_against_manifest(&req, &"ab".repeat(32), &manifest).expect("应通过");
    assert_eq!(filename, "app.tar.gz");
    // 前端未传文件名时用清单的文件名。
    let mut no_name = test_request();
    no_name.filename = None;
    assert_eq!(
      verify_install_request_against_manifest(&no_name, &"ab".repeat(32), &manifest)
        .expect("应通过"),
      "app.tar.gz"
    );
  }

  #[test]
  fn verify_install_request_rejects_any_mismatch() {
    let sha = "ab".repeat(32);
    let manifest = test_manifest();
    let mut req = test_request();

    req.version = "0.1.9".to_string();
    assert!(verify_install_request_against_manifest(&req, &sha, &manifest).is_err());
    req.version = "v0.1.10".to_string();
    assert!(verify_install_request_against_manifest(&req, &sha, &manifest).is_ok());

    let mut req = test_request();
    req.download_url =
      "https://github.com/my-org/my-fork/releases/download/v0.1.10/other.tar.gz".to_string();
    assert!(verify_install_request_against_manifest(&req, &sha, &manifest).is_err());

    let mut req = test_request();
    req.filename = Some("evil.tar.gz".to_string());
    assert!(verify_install_request_against_manifest(&req, &sha, &manifest).is_err());

    let mut manifest = test_manifest();
    manifest.sha256 = None;
    assert!(verify_install_request_against_manifest(&test_request(), &sha, &manifest).is_err());

    let mut manifest = test_manifest();
    manifest.download_url = None;
    assert!(verify_install_request_against_manifest(&test_request(), &sha, &manifest).is_err());

    let mut manifest = test_manifest();
    manifest.updates_disabled = true;
    assert!(verify_install_request_against_manifest(&test_request(), &sha, &manifest).is_err());

    assert!(verify_install_request_against_manifest(
      &test_request(),
      &"cd".repeat(32),
      &test_manifest()
    )
    .is_err());
  }

  #[test]
  fn redirect_target_must_be_https_and_allowlisted() {
    // 合法的重定向目标（任务书 P0-2-4 的用例）。
    for raw in [
      "https://github.com/my-org/my-fork/releases/download/v1.0.0/app.tar.gz",
      "https://api.github.com/repos/my-org/my-fork/releases/latest",
      "https://objects.githubusercontent.com/123456/app.tar.gz?X-Amz-Algorithm=AWS4-HMAC-SHA256",
      "https://release-assets.githubusercontent.com/123456/app.tar.gz",
    ] {
      let url = Url::parse(raw).expect("测试地址应合法");
      assert!(is_allowed_redirect_target(&url), "应放行: {raw}");
    }
    for raw in [
      "http://github.com/my-org/my-fork/releases/download/v1.0.0/app.tar.gz",
      "https://evil.com/app.tar.gz",
      "https://github.com.evil.com/app.tar.gz",
      "https://objects.githubusercontent.com.evil.com/app.tar.gz",
      "https://sub.github.com/app.tar.gz",
      "https://githubusercontent.com/app.tar.gz",
    ] {
      let url = Url::parse(raw).expect("测试地址应合法");
      assert!(!is_allowed_redirect_target(&url), "应拒绝: {raw}");
    }
  }

  #[test]
  fn macos_swap_script_aborts_when_old_process_wont_exit() {
    // 10 秒等待后旧进程仍在即中止，不做任何替换。
    let wait_end = MACOS_SWAP_SCRIPT
      .find("i=$((i + 1))")
      .expect("应有等待循环");
    let abort = MACOS_SWAP_SCRIPT.find("exit 2").expect("应有超时中止");
    let backup = MACOS_SWAP_SCRIPT.find("maam-backup").expect("应有备份逻辑");
    assert!(wait_end < abort && abort < backup);
  }

  #[test]
  fn macos_swap_script_rollback_removes_partial_target_first() {
    // 旧回滚条件 [ ! -e "$dst_app" ] 在跨卷失败留下半截目录时不成立。
    assert!(!MACOS_SWAP_SCRIPT.contains("[ ! -e \"$dst_app\" ]"));
    let remove = MACOS_SWAP_SCRIPT
      .find("rm -rf \"$dst_app\"")
      .expect("回滚应先删目标");
    let restore = MACOS_SWAP_SCRIPT
      .find("mv \"$backup\" \"$dst_app\"")
      .expect("回滚应把备份移回");
    assert!(remove < restore);
    // 替换成功后仍去隔离（P0-2-5 保留的那一次）。
    assert!(MACOS_SWAP_SCRIPT.contains("/usr/bin/xattr -dr com.apple.quarantine \"$dst_app\""));
  }

  #[test]
  fn releases_page_url_accepts_configured_repo_releases() {
    for raw in [
      "https://github.com/my-org/my-fork/releases",
      "https://github.com/my-org/my-fork/releases/tag/v0.1.10",
      "https://github.com/My-Org/My-Fork/releases/tag/v0.1.10",
    ] {
      validate_releases_page_url(raw, TEST_REPO).expect("应通过");
    }
  }

  #[test]
  fn releases_page_url_rejects_anything_else() {
    for raw in [
      // 其他仓库
      "https://github.com/other-user/other-repo/releases",
      // http://
      "http://github.com/my-org/my-fork/releases",
      // 子域名伪装
      "https://github.com.evil.com/my-org/my-fork/releases",
      // userinfo 伪装
      "https://github.com@evil.com/my-org/my-fork/releases",
      // 仓库根目录
      "https://github.com/my-org/my-fork",
      // 其他路径
      "https://github.com/my-org/my-fork/issues",
      // 下载地址不是页面
      "https://github.com/my-org/my-fork/releases/download/v0.1.10/app.tar.gz",
      // 任意 https 外站
      "https://evil.example.com/phishing",
    ] {
      assert!(
        validate_releases_page_url(raw, TEST_REPO).is_err(),
        "应拒绝: {raw}"
      );
    }
  }
}
