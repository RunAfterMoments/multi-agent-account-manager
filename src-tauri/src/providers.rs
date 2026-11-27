use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderId(String);

impl ProviderId {
  pub fn muse() -> Self {
    Self("muse".to_string())
  }

  pub fn hark() -> Self {
    Self("hark".to_string())
  }

  pub fn api() -> Self {
    Self("api".to_string())
  }

  pub fn cli() -> Self {
    Self("cli".to_string())
  }

  pub fn as_str(&self) -> &str {
    &self.0
  }

  pub fn is_muse(&self) -> bool {
    self.0 == "muse"
  }

  pub fn is_hark(&self) -> bool {
    self.0 == "hark"
  }

  pub fn is_api(&self) -> bool {
    self.0 == "api"
  }

  pub fn is_cli(&self) -> bool {
    self.0 == "cli"
  }

  #[cfg_attr(not(desktop), allow(dead_code))]
  pub fn is_external(&self) -> bool {
    self.is_api() || self.is_cli()
  }

  pub fn is_builtin(&self) -> bool {
    self.is_muse() || self.is_hark()
  }

  pub fn parse(value: &str) -> Option<Self> {
    let clean = value.trim().to_ascii_lowercase();
    if clean == "muse" || clean == "hark" || clean == "api" || clean == "cli" {
      Some(Self(clean))
    } else {
      None
    }
  }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
  pub id: ProviderId,
  pub name: String,
  pub short_name: String,
  pub enabled: bool,
  pub status_text: String,
  pub entry_url: String,
  pub homepage_url: String,
  pub callback_origins: Vec<String>,
  pub verified_methods: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderSpec {
  pub info: ProviderInfo,
  pub allowed_hosts: Vec<String>,
}

pub fn builtin_provider_specs() -> Vec<ProviderSpec> {
  vec![
    ProviderSpec {
      info: ProviderInfo {
        id: ProviderId::muse(),
        name: "Muse · Meta".to_string(),
        short_name: "Muse".to_string(),
        enabled: true,
        status_text: "已启用".to_string(),
        entry_url: "https://muse.ai/".to_string(),
        homepage_url: "https://muse.ai/".to_string(),
        callback_origins: vec![
          "https://muse.ai".to_string(),
          "https://www.muse.ai".to_string(),
        ],
        verified_methods: vec!["email".to_string(), "phone".to_string()],
      },
      allowed_hosts: vec![
        "muse.ai".to_string(),
        "meta.com".to_string(),
        "meta.ai".to_string(),
        "metaaiusercontent.com".to_string(),
        "ecto1usercontent.com".to_string(),
        "meta-agents-apps.workers.dev".to_string(),
        "facebook.com".to_string(),
        "instagram.com".to_string(),
        "fb.com".to_string(),
        "appleid.apple.com".to_string(),
        "idmsa.apple.com".to_string(),
      ],
    },
    ProviderSpec {
      info: ProviderInfo {
        id: ProviderId::hark(),
        name: "Hark".to_string(),
        short_name: "Hark".to_string(),
        enabled: true,
        status_text: "已启用".to_string(),
        entry_url: "https://app.hark.com/".to_string(),
        homepage_url: "https://hark.com/".to_string(),
        callback_origins: vec![
          "https://app.hark.com".to_string(),
          "https://hark.com".to_string(),
        ],
        verified_methods: vec![
          "email".to_string(),
          "phone".to_string(),
          "apple".to_string(),
        ],
      },
      allowed_hosts: vec![
        "app.hark.com".to_string(),
        "hark.com".to_string(),
        "appleid.apple.com".to_string(),
        "idmsa.apple.com".to_string(),
      ],
    },
  ]
}

pub fn resolve_provider(id: &ProviderId) -> Option<ProviderSpec> {
  if !id.is_builtin() {
    return None;
  }
  builtin_provider_specs()
    .into_iter()
    .find(|p| &p.info.id == id)
}

pub fn all_provider_infos() -> Vec<ProviderInfo> {
  builtin_provider_specs()
    .into_iter()
    .map(|p| p.info)
    .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationDecision {
  AllowBlank,
  AllowHttps,
  ExternalGoogleOAuth,
  Deny,
}

pub fn host_matches(host: &str, domain: &str) -> bool {
  if host.eq_ignore_ascii_case(domain) {
    return true;
  }
  let suffix = format!(".{}", domain.to_ascii_lowercase());
  host.to_ascii_lowercase().ends_with(&suffix)
}

const EMBEDDED_WIDGET_HOSTS: &[&str] = &[
  "challenges.cloudflare.com",
  "turnstile.cloudflare.com",
  "hcaptcha.com",
  "recaptcha.net",
  "gstatic.com",
  "arkoselabs.com",
  "funcaptcha.com",
  "friendlycaptcha.com",
  "geetest.com",
  "captcha-delivery.com",
  // 注意：firebaseapp.com 是 Firebase Hosting 公共后缀，任何人都能在上面
  // 托管任意内容，不能当作可信的内嵌 widget 域名（之前会放行整个后缀）。
  "stripe.com",
  "stripecdn.com",
];

fn is_embedded_widget_url(url: &Url, host: &str) -> bool {
  if EMBEDDED_WIDGET_HOSTS
    .iter()
    .any(|allowed| host_matches(host, allowed))
  {
    return true;
  }
  (host.eq_ignore_ascii_case("www.google.com") || host.eq_ignore_ascii_case("google.com"))
    && url.path().starts_with("/recaptcha")
}

pub fn classify_top_level_url(
  spec: &ProviderSpec,
  url: &Url,
  allow_blank: bool,
) -> NavigationDecision {
  let raw = url.as_str();
  if raw == "about:srcdoc" || raw.starts_with("blob:null/") {
    return NavigationDecision::AllowBlank;
  }
  if let Some(inner_raw) = raw.strip_prefix("blob:") {
    // blob:https:// 剥前缀验内层 host（与 iOS 一致）：内层必须是 https
    // 且 host 在白名单内；blob:http:// 或内层外站一律拒绝。
    if let Ok(inner) = Url::parse(inner_raw) {
      if inner.scheme() == "https" {
        if let Some(host) = inner.host_str() {
          if spec
            .allowed_hosts
            .iter()
            .any(|allowed| host_matches(host, allowed))
          {
            return NavigationDecision::AllowBlank;
          }
        }
      }
    }
    return NavigationDecision::Deny;
  }
  if allow_blank && raw == "about:blank" {
    return NavigationDecision::AllowBlank;
  }
  if url.scheme() != "https" {
    return NavigationDecision::Deny;
  }
  let Some(host) = url.host_str() else {
    return NavigationDecision::Deny;
  };
  if host_matches(host, "accounts.google.com") {
    return NavigationDecision::ExternalGoogleOAuth;
  }
  if spec
    .allowed_hosts
    .iter()
    .any(|allowed| host_matches(host, allowed))
    || is_embedded_widget_url(url, host)
  {
    NavigationDecision::AllowHttps
  } else {
    NavigationDecision::Deny
  }
}

#[cfg_attr(not(desktop), allow(dead_code))]
pub fn is_verified_callback_origin(spec: &ProviderSpec, url: &Url) -> bool {
  if url.scheme() != "https" {
    return false;
  }
  let origin = url.origin().ascii_serialization();
  spec
    .info
    .callback_origins
    .iter()
    .any(|allowed| allowed.eq_ignore_ascii_case(&origin))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn muse_spec() -> ProviderSpec {
    resolve_provider(&ProviderId::muse()).expect("内置 Muse 服务应存在")
  }

  fn hark_spec() -> ProviderSpec {
    resolve_provider(&ProviderId::hark()).expect("内置 Hark 服务应存在")
  }

  #[test]
  fn host_matches_exact_and_subdomain() {
    assert!(host_matches("muse.ai", "muse.ai"));
    assert!(host_matches("www.muse.ai", "muse.ai"));
    assert!(host_matches("a.b.muse.ai", "muse.ai"));
    assert!(host_matches("WWW.MUSE.AI", "muse.ai"));
    assert!(host_matches("MUSE.AI", "muse.ai"));
  }

  #[test]
  fn host_matches_rejects_lookalike_suffix() {
    // 仅共享后缀、前面没有 "." 分隔的不是子域名。
    assert!(!host_matches("evilmuse.ai", "muse.ai"));
    assert!(!host_matches("notmuse.ai", "muse.ai"));
    assert!(!host_matches("fakemuse.ai", "muse.ai"));
  }

  #[test]
  fn host_matches_rejects_parent_domain_suffix() {
    // 目标域名出现在主机名中间、后面还有别的标签也不是子域名。
    assert!(!host_matches("muse.ai.evil.com", "muse.ai"));
    assert!(host_matches("muse.ai.evil.com", "muse.ai.evil.com"));
  }

  #[test]
  fn classify_top_level_url_allows_builtin_hosts() {
    let spec = muse_spec();
    let url = Url::parse("https://muse.ai/").unwrap();
    assert_eq!(
      classify_top_level_url(&spec, &url, false),
      NavigationDecision::AllowHttps
    );
    let url = Url::parse("https://www.muse.ai/chat").unwrap();
    assert_eq!(
      classify_top_level_url(&spec, &url, false),
      NavigationDecision::AllowHttps
    );
  }

  #[test]
  fn classify_top_level_url_denies_http() {
    let spec = muse_spec();
    let url = Url::parse("http://muse.ai/").unwrap();
    assert_eq!(
      classify_top_level_url(&spec, &url, false),
      NavigationDecision::Deny
    );
  }

  #[test]
  fn classify_top_level_url_denies_javascript() {
    let spec = muse_spec();
    let url = Url::parse("javascript:alert(1)").unwrap();
    assert_eq!(
      classify_top_level_url(&spec, &url, false),
      NavigationDecision::Deny
    );
  }

  #[test]
  fn classify_top_level_url_denies_unknown_host() {
    let spec = muse_spec();
    let url = Url::parse("https://unknown.example.com/").unwrap();
    assert_eq!(
      classify_top_level_url(&spec, &url, false),
      NavigationDecision::Deny
    );
    // 用户信息段伪装成白名单主机：实际主机是 evil.com。
    let url = Url::parse("https://muse.ai@evil.com/").unwrap();
    assert_eq!(
      classify_top_level_url(&spec, &url, false),
      NavigationDecision::Deny
    );
  }

  #[test]
  fn classify_top_level_url_google_oauth_is_external() {
    let spec = muse_spec();
    let url = Url::parse("https://accounts.google.com/o/oauth2/auth").unwrap();
    assert_eq!(
      classify_top_level_url(&spec, &url, false),
      NavigationDecision::ExternalGoogleOAuth
    );
  }

  #[test]
  fn classify_top_level_url_blank_handling() {
    let spec = muse_spec();
    let url = Url::parse("about:blank").unwrap();
    assert_eq!(
      classify_top_level_url(&spec, &url, true),
      NavigationDecision::AllowBlank
    );
    assert_eq!(
      classify_top_level_url(&spec, &url, false),
      NavigationDecision::Deny
    );
    let url = Url::parse("blob:https://muse.ai/1234").unwrap();
    assert_eq!(
      classify_top_level_url(&spec, &url, false),
      NavigationDecision::AllowBlank
    );
    // 内层外站 / 内层 http 一律拒绝（与 iOS 子帧规则一致）。
    let url = Url::parse("blob:https://evil.com/1234").unwrap();
    assert_eq!(
      classify_top_level_url(&spec, &url, false),
      NavigationDecision::Deny
    );
    let url = Url::parse("blob:http://muse.ai/1234").unwrap();
    assert_eq!(
      classify_top_level_url(&spec, &url, false),
      NavigationDecision::Deny
    );
  }

  #[test]
  fn is_verified_callback_origin_accepts_listed_origins() {
    let spec = muse_spec();
    assert!(is_verified_callback_origin(
      &spec,
      &Url::parse("https://muse.ai/").unwrap()
    ));
    assert!(is_verified_callback_origin(
      &spec,
      &Url::parse("https://www.muse.ai/callback?code=x").unwrap()
    ));
    // 默认 443 端口在 origin 序列化里会被省略，仍应命中。
    assert!(is_verified_callback_origin(
      &spec,
      &Url::parse("https://muse.ai:443/").unwrap()
    ));
    let hark = hark_spec();
    assert!(is_verified_callback_origin(
      &hark,
      &Url::parse("https://app.hark.com/").unwrap()
    ));
  }

  #[test]
  fn is_verified_callback_origin_rejects_others() {
    let spec = muse_spec();
    // http 明文。
    assert!(!is_verified_callback_origin(
      &spec,
      &Url::parse("http://muse.ai/").unwrap()
    ));
    // 长得像的子域名 / 后缀伪装。
    assert!(!is_verified_callback_origin(
      &spec,
      &Url::parse("https://muse.ai.evil.com/").unwrap()
    ));
    assert!(!is_verified_callback_origin(
      &spec,
      &Url::parse("https://evilmuse.ai/").unwrap()
    ));
    // 完全无关的主机。
    assert!(!is_verified_callback_origin(
      &spec,
      &Url::parse("https://evil.com/").unwrap()
    ));
    // 在 allowed_hosts 里但不在 callback_origins 里的主机：classify 会
    // 放行（AllowHttps），但 thread_url / new_chat_url 跳转只认本站，
    // 这些一律拒绝，否则会话页会带着账号 Cookie 跳过去。
    for host in [
      "https://metaaiusercontent.com/file",
      "https://facebook.com/login",
      "https://appleid.apple.com/auth",
    ] {
      assert!(
        !is_verified_callback_origin(&spec, &Url::parse(host).unwrap()),
        "{host} must not be a verified callback origin"
      );
    }
    // 非默认端口不是同一个 origin。
    assert!(!is_verified_callback_origin(
      &spec,
      &Url::parse("https://muse.ai:8443/").unwrap()
    ));
  }
}
