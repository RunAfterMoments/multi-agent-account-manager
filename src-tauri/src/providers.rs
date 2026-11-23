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

  pub fn from_validated_custom(id: String) -> Self {
    Self(id)
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

  pub fn is_builtin(&self) -> bool {
    self.is_muse() || self.is_hark()
  }

  pub fn parse(value: &str) -> Option<Self> {
    let clean = value.trim().to_ascii_lowercase();
    if clean == "muse" || clean == "hark" {
      return Some(Self(clean));
    }
    let suffix = clean.strip_prefix("custom-")?;
    if (4..=40).contains(&suffix.len())
      && suffix
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
    {
      Some(Self(clean))
    } else {
      None
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomProviderRecord {
  pub id: ProviderId,
  pub name: String,
  pub entry_url: String,
  #[serde(default)]
  pub created_at_ms: u64,
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
  pub is_custom: bool,
}

#[derive(Debug, Clone)]
pub struct ProviderSpec {
  pub info: ProviderInfo,
  pub allowed_hosts: Vec<String>,
}

pub fn normalize_custom_provider_input(raw_name: &str, raw_url: &str) -> Result<(String, String), String> {
  let name = raw_name.trim();
  if name.is_empty() {
    return Err("请输入服务名称".to_string());
  }
  if name.chars().count() > 32 {
    return Err("服务名称不能超过 32 个字符".to_string());
  }
  let raw_url = raw_url.trim();
  if raw_url.is_empty() {
    return Err("请输入网站入口地址".to_string());
  }
  let candidate = if raw_url.contains("://") {
    raw_url.to_string()
  } else {
    format!("https://{raw_url}")
  };
  let parsed = Url::parse(&candidate).map_err(|_| "请输入有效的 HTTPS 网站入口地址".to_string())?;
  if parsed.scheme() != "https" || parsed.host_str().is_none_or(|h| h.is_empty()) {
    return Err("自定义服务仅支持 https:// 开头的网页地址".to_string());
  }
  if !parsed.username().is_empty() || parsed.password().is_some() {
    return Err("入口地址不能包含用户名或密码".to_string());
  }
  Ok((name.to_string(), parsed.to_string()))
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
        is_custom: false,
      },
      allowed_hosts: vec![
        "muse.ai".to_string(),
        "meta.com".to_string(),
        "meta.ai".to_string(),
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
        is_custom: false,
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

pub fn custom_provider_spec(rec: &CustomProviderRecord) -> ProviderSpec {
  let origin = Url::parse(&rec.entry_url)
    .ok()
    .map(|u| u.origin().ascii_serialization())
    .into_iter()
    .collect();
  ProviderSpec {
    info: ProviderInfo {
      id: rec.id.clone(),
      name: rec.name.clone(),
      short_name: rec.name.clone(),
      enabled: true,
      status_text: "自定义".to_string(),
      entry_url: rec.entry_url.clone(),
      homepage_url: rec.entry_url.clone(),
      callback_origins: origin,
      verified_methods: Vec::new(),
      is_custom: true,
    },
    allowed_hosts: vec!["*".to_string()],
  }
}

pub fn resolve_provider(custom: &[CustomProviderRecord], id: &ProviderId) -> Option<ProviderSpec> {
  if id.is_builtin() {
    return builtin_provider_specs()
      .into_iter()
      .find(|p| &p.info.id == id);
  }
  custom
    .iter()
    .find(|rec| &rec.id == id)
    .map(custom_provider_spec)
}

pub fn all_provider_infos(custom: &[CustomProviderRecord]) -> Vec<ProviderInfo> {
  let mut list: Vec<ProviderInfo> = builtin_provider_specs()
    .into_iter()
    .map(|p| p.info)
    .collect();
  for rec in custom {
    list.push(custom_provider_spec(rec).info);
  }
  list
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationDecision {
  AllowBlank,
  AllowHttps,
  ExternalGoogleOAuth,
  Deny,
}

fn host_matches(host: &str, domain: &str) -> bool {
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
  "firebaseapp.com",
  "stripe.com",
  "stripecdn.com",
];

fn is_embedded_widget_url(url: &Url, host: &str) -> bool {
  if EMBEDDED_WIDGET_HOSTS.iter().any(|allowed| host_matches(host, allowed)) {
    return true;
  }
  (host.eq_ignore_ascii_case("www.google.com") || host.eq_ignore_ascii_case("google.com"))
    && url.path().starts_with("/recaptcha")
}

pub fn classify_top_level_url(spec: &ProviderSpec, url: &Url, allow_blank: bool) -> NavigationDecision {
  let raw = url.as_str();
  if raw == "about:srcdoc" || raw.starts_with("blob:https://") {
    return NavigationDecision::AllowBlank;
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
  if spec.info.is_custom {
    return NavigationDecision::AllowHttps;
  }
  if host_matches(host, "accounts.google.com") {
    return NavigationDecision::ExternalGoogleOAuth;
  }
  if spec.allowed_hosts.iter().any(|allowed| host_matches(host, allowed))
    || is_embedded_widget_url(url, host)
  {
    NavigationDecision::AllowHttps
  } else {
    NavigationDecision::Deny
  }
}

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
