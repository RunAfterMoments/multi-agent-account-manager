use std::path::Path;
use uuid::Uuid;

pub fn current_platform_name() -> &'static str {
  #[cfg(target_os = "macos")]
  {
    "macos"
  }
  #[cfg(windows)]
  {
    "windows"
  }
  #[cfg(target_os = "android")]
  {
    "android"
  }
  #[cfg(all(not(target_os = "macos"), not(windows), not(target_os = "android")))]
  {
    "linux"
  }
}

#[cfg(desktop)]
pub fn open_external_browser(url: &str) -> Result<(), String> {
  open::that_detached(url).map_err(|e| format!("打开系统浏览器失败: {e}"))
}

pub const SUPPORTS_ACCOUNT_PROXY: bool = cfg!(any(target_os = "macos", windows));

pub fn parse_account_proxy(raw: &str) -> Result<Option<url::Url>, String> {
  let raw = raw.trim();
  if raw.is_empty() {
    return Ok(None);
  }
  if !SUPPORTS_ACCOUNT_PROXY {
    return Err("当前平台不支持账号代理".to_string());
  }
  let invalid = || "请输入无认证的 HTTP CONNECT 代理地址（http://主机:端口）".to_string();
  if !raw.get(..7).is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
    || raw.bytes().any(|byte| byte.is_ascii_whitespace())
  {
    return Err(invalid());
  }
  let rest = &raw[7..];
  let authority = rest.strip_suffix('/').unwrap_or(rest);
  let explicit_port = if let Some(after_bracket) = authority.strip_prefix('[') {
    after_bracket.split_once("]:").map(|(_, port)| port)
  } else {
    authority.split_once(':').map(|(_, port)| port)
  };
  let has_explicit_port = explicit_port
    .is_some_and(|port| !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()));
  if !has_explicit_port {
    return Err(invalid());
  }
  let parsed = url::Url::parse(raw).map_err(|_| invalid())?;
  if parsed.scheme() != "http"
    || parsed.host().is_none()
    || !parsed.port_or_known_default().is_some_and(|port| port != 0)
    || !parsed.username().is_empty()
    || parsed.password().is_some()
    || parsed.path() != "/"
    || parsed.query().is_some()
    || parsed.fragment().is_some()
  {
    return Err(invalid());
  }
  if let Some(url::Host::Domain(host)) = parsed.host() {
    if !host
      .bytes()
      .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'))
    {
      return Err(invalid());
    }
  }
  Ok(Some(parsed))
}

#[allow(dead_code)]
pub fn format_account_proxy(proxy: &url::Url) -> String {
  let host = proxy.host_str().expect("validated HTTP proxy host");
  let port = proxy.port_or_known_default().expect("validated HTTP proxy port");
  format!("http://{host}:{port}")
}

#[cfg(windows)]
pub fn windows_proxy_browser_args(proxy: &url::Url) -> String {
  format!(
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
     --autoplay-policy=no-user-gesture-required --proxy-server={}",
    format_account_proxy(proxy)
  )
}

#[cfg(windows)]
pub fn with_windows_account_proxy(
  builder: tauri::webview::WebviewBuilder<tauri::Wry>,
  proxy: Option<&url::Url>,
  app_id: &str,
) -> Result<tauri::webview::WebviewBuilder<tauri::Wry>, String> {
  let Some(proxy) = proxy else {
    return Ok(builder);
  };
  check_windows_proxy_overrides(app_id)?;
  Ok(if proxy.port().is_some() {
    builder.proxy_url(proxy.clone())
  } else {
    builder.additional_browser_args(&windows_proxy_browser_args(proxy))
  })
}

#[cfg(windows)]
fn windows_browser_switch(arg: &str) -> Option<(&str, &str)> {
  let arg = arg.trim();
  let switch = arg.strip_prefix("--").or_else(|| arg.strip_prefix(['-', '/']))?;
  Some(switch.split_once('=').unwrap_or((switch, "")))
}

#[cfg(windows)]
fn check_windows_proxy_arguments(args: &str, source: &str) -> Result<(), String> {
  use windows::core::HSTRING;
  use windows::Win32::Foundation::{LocalFree, HLOCAL};
  use windows::Win32::UI::Shell::CommandLineToArgvW;

  if args.trim().is_empty() {
    return Ok(());
  }
  let command = HSTRING::from(format!("webview2 {args}"));
  let mut count = 0;
  let argv = unsafe { CommandLineToArgvW(&command, &mut count) };
  if argv.is_null() {
    return Err(format!("无法核对 Windows WebView2 启动参数（{source}），已阻止打开账号"));
  }
  let conflicts = unsafe { std::slice::from_raw_parts(argv, count as usize) }
    .iter()
    .skip(1)
    .any(|arg| {
      let arg = String::from_utf16_lossy(unsafe { arg.as_wide() });
      windows_browser_switch(&arg).is_some_and(|(name, _)| {
        let name = name.to_ascii_lowercase();
        name.starts_with("proxy-")
          || matches!(
            name.as_str(),
            "no-proxy-server" | "winhttp-proxy-resolver" | "edge-webview-switches"
          )
      })
    });
  unsafe { LocalFree(Some(HLOCAL(argv.cast()))) };
  if conflicts {
    return Err(format!(
      "Windows WebView2 启动参数（{source}）可能覆盖账号代理，已阻止打开账号"
    ));
  }
  Ok(())
}

#[cfg(windows)]
fn check_windows_proxy_overrides(app_id: &str) -> Result<(), String> {
  use windows::core::HSTRING;
  use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND};
  use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
  };
  use windows::Win32::UI::Shell::GetCurrentProcessExplicitAppUserModelID;

  const ENV_ARGS: &str = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";
  const POLICY_KEY: &str = r"SOFTWARE\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments";

  if let Some(args) = std::env::var_os(ENV_ARGS) {
    check_windows_proxy_arguments(&args.to_string_lossy(), ENV_ARGS)?;
  }
  for arg in std::env::args_os().skip(1) {
    if let Some((name, value)) = windows_browser_switch(&arg.to_string_lossy()) {
      if name.eq_ignore_ascii_case("edge-webview-switches") {
        check_windows_proxy_arguments(value, "--edge-webview-switches")?;
      }
    }
  }

  let exe = std::env::current_exe().map_err(|err| format!("无法核对 Windows 应用名称: {err}"))?;
  let exe_name = exe.file_name().ok_or_else(|| "无法核对 Windows 应用名称".to_string())?;
  let explicit_id = unsafe { GetCurrentProcessExplicitAppUserModelID() }
    .ok()
    .map(|id| HSTRING::from(webview2_com::take_pwstr(id)));
  let names = [HSTRING::from(app_id), HSTRING::from(exe_name), HSTRING::from("*")];
  let policy_key = HSTRING::from(POLICY_KEY);
  let mut buffer = vec![0u16; 32_768];
  for (root, hive) in [(HKEY_LOCAL_MACHINE, "HKLM"), (HKEY_CURRENT_USER, "HKCU")] {
    for name in explicit_id.iter().chain(names.iter()).filter(|name| !name.is_empty()) {
      let mut size = (buffer.len() * size_of::<u16>()) as u32;
      let result = unsafe {
        RegGetValueW(
          root,
          &policy_key,
          name,
          RRF_RT_REG_SZ,
          None,
          Some(buffer.as_mut_ptr().cast()),
          Some(&mut size),
        )
      };
      if matches!(result, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
        continue;
      }
      let source = format!("{hive}\\{POLICY_KEY}\\{name}");
      result
        .ok()
        .map_err(|err| format!("无法核对 Windows WebView2 启动参数（{source}）: {err}"))?;
      let args = String::from_utf16_lossy(&buffer[..size as usize / size_of::<u16>()]);
      check_windows_proxy_arguments(args.trim_end_matches('\0'), &source)?;
    }
  }
  Ok(())
}

#[cfg(windows)]
pub fn windows_webview_options_in_use(error: &tauri::Error) -> bool {
  use windows::Win32::Foundation::ERROR_INVALID_STATE;

  let tauri::Error::Runtime(tauri_runtime::Error::CreateWebview(inner)) = error else {
    return false;
  };
  matches!(
    inner.downcast_ref::<wry::Error>(),
    Some(wry::Error::WebView2Error(webview2_com::Error::WindowsError(inner)))
      if inner.code() == ERROR_INVALID_STATE.to_hresult()
  )
}

#[cfg(windows)]
pub async fn on_windows_navigation_completed(
  webview: &tauri::Webview,
  on_completed: impl Fn(Result<url::Url, ()>) + Send + 'static,
) -> Result<(), String> {
  use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2NavigationCompletedEventArgs2, COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED,
    COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN,
    COREWEBVIEW2_WEB_ERROR_STATUS_VALID_PROXY_AUTHENTICATION_REQUIRED,
  };
  use webview2_com::{take_pwstr, NavigationCompletedEventHandler};
  use windows::core::{Interface, BOOL, PWSTR};

  let (sender, receiver) = tokio::sync::oneshot::channel();
  webview
    .with_webview(move |platform_view| unsafe {
      let result = platform_view.controller().CoreWebView2().and_then(|core| {
        let handler = NavigationCompletedEventHandler::create(Box::new(move |view, args| {
          let (Some(view), Some(args)) = (view, args) else {
            return Ok(());
          };
          let mut success = BOOL::default();
          args.IsSuccess(&mut success)?;
          let mut status = Default::default();
          args.WebErrorStatus(&mut status)?;
          let mut http_status = 0;
          if !success.as_bool() {
            let _ = args
              .cast::<ICoreWebView2NavigationCompletedEventArgs2>()
              .and_then(|args| args.HttpStatusCode(&mut http_status));
          }
          if http_status == 407
            || status == COREWEBVIEW2_WEB_ERROR_STATUS_VALID_PROXY_AUTHENTICATION_REQUIRED
          {
            on_completed(Err(()));
          } else if success.as_bool() || http_status != 0 {
            let mut source = PWSTR::null();
            view.Source(&mut source)?;
            if let Ok(url) = url::Url::parse(&take_pwstr(source)) {
              if url.as_str() != "about:blank" {
                on_completed(Ok(url));
              }
            }
          } else if !matches!(
            status,
            COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN | COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED
          ) {
            on_completed(Err(()));
          }
          Ok(())
        }));
        core.add_NavigationCompleted(&handler, &mut 0)
      });
      let _ = sender.send(result);
    })
    .map_err(|err| format!("访问 Windows WebView2 失败: {err}"))?;
  receiver
    .await
    .map_err(|_| "注册 Windows WebView2 导航事件中断".to_string())?
    .map_err(|err| format!("注册 Windows WebView2 导航事件失败: {err}"))
}

#[cfg(target_os = "macos")]
unsafe fn apply_macos_proxy_to_wkwebview(
  view: &objc2_web_kit::WKWebView,
  proxy_url: Option<&url::Url>,
) -> Result<(), String> {
  use objc2_foundation::NSArray;

  let store = view.configuration().websiteDataStore();
  let configurations = proxy_url
    .map(create_http_connect_proxy)
    .transpose()?
    .map(|proxy| NSArray::arrayWithObject(&*proxy));
  let _: () = objc2::msg_send![&*store, setProxyConfigurations: configurations.as_deref()];
  Ok(())
}

#[cfg(target_os = "macos")]
pub async fn configure_desktop_proxy(
  webview: &tauri::Webview,
  proxy_url: Option<url::Url>,
) -> Result<(), String> {
  use objc2_web_kit::WKWebView;

  let (sender, receiver) = tokio::sync::oneshot::channel();
  webview
    .with_webview(move |platform_view| unsafe {
      let view: &WKWebView = &*platform_view.inner().cast();
      let result = apply_macos_proxy_to_wkwebview(view, proxy_url.as_ref());
      let _ = sender.send(result);
    })
    .map_err(|err| format!("访问 macOS WebView 失败: {err}"))?;
  receiver
    .await
    .map_err(|_| "macOS 账号代理配置中断".to_string())?
}

#[cfg(target_os = "macos")]
fn should_fail_macos_navigation(domain: &str, code: isize) -> bool {
  !((domain == "NSURLErrorDomain" && code == -999)
    || (domain == "WebKitErrorDomain" && code == 102))
}

#[cfg(target_os = "macos")]
mod macos_nav_delegate {
  use super::should_fail_macos_navigation;
  use objc2::rc::{Retained, Weak};
  use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, Sel};
  use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
  use objc2_foundation::NSError;
  use objc2_web_kit::WKWebView;

  struct MaamNavigationDelegateIvars {
    inner: Option<Weak<AnyObject>>,
    on_fail: Box<dyn Fn()>,
  }

  define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = MaamNavigationDelegateIvars]
    struct MaamNavigationDelegateProxy;

    unsafe impl NSObjectProtocol for MaamNavigationDelegateProxy {}

    impl MaamNavigationDelegateProxy {
      #[unsafe(method(respondsToSelector:))]
      fn responds_to_selector(&self, sel: Sel) -> Bool {
        let base: Bool = unsafe { msg_send![super(self), respondsToSelector: sel] };
        if base.as_bool() {
          return Bool::YES;
        }
        if let Some(inner) = self.ivars().inner.as_ref().and_then(Weak::load) {
          let inner_responds: Bool = unsafe { msg_send![&*inner, respondsToSelector: sel] };
          return inner_responds;
        }
        Bool::NO
      }

      #[unsafe(method(forwardingTargetForSelector:))]
      fn forwarding_target_for_selector(&self, sel: Sel) -> *mut AnyObject {
        if let Some(inner) = self.ivars().inner.as_ref().and_then(Weak::load) {
          let inner_responds: Bool = unsafe { msg_send![&*inner, respondsToSelector: sel] };
          if inner_responds.as_bool() {
            return Retained::autorelease_return(inner);
          }
        }
        std::ptr::null_mut()
      }

      #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
      fn did_fail_provisional_navigation(
        &self,
        webview: *mut AnyObject,
        navigation: *mut AnyObject,
        error: *mut NSError,
      ) {
        if !error.is_null() {
          let err = unsafe { &*error };
          if should_fail_macos_navigation(&err.domain().to_string(), err.code()) {
            (self.ivars().on_fail)();
          }
        }
        if let Some(inner) = self.ivars().inner.as_ref().and_then(Weak::load) {
          let sel = objc2::sel!(webView:didFailProvisionalNavigation:withError:);
          let inner_responds: Bool = unsafe { msg_send![&*inner, respondsToSelector: sel] };
          if inner_responds.as_bool() {
            let _: () = unsafe {
              msg_send![&*inner, webView: webview, didFailProvisionalNavigation: navigation, withError: error]
            };
          }
        }
      }

      #[unsafe(method(webView:didFailNavigation:withError:))]
      fn did_fail_navigation(
        &self,
        webview: *mut AnyObject,
        navigation: *mut AnyObject,
        error: *mut NSError,
      ) {
        if !error.is_null() {
          let err = unsafe { &*error };
          if should_fail_macos_navigation(&err.domain().to_string(), err.code()) {
            (self.ivars().on_fail)();
          }
        }
        if let Some(inner) = self.ivars().inner.as_ref().and_then(Weak::load) {
          let sel = objc2::sel!(webView:didFailNavigation:withError:);
          let inner_responds: Bool = unsafe { msg_send![&*inner, respondsToSelector: sel] };
          if inner_responds.as_bool() {
            let _: () = unsafe {
              msg_send![&*inner, webView: webview, didFailNavigation: navigation, withError: error]
            };
          }
        }
      }

      #[unsafe(method(webView:didReceiveAuthenticationChallenge:completionHandler:))]
      fn did_receive_authentication_challenge(
        &self,
        _webview: *mut AnyObject,
        challenge: *mut AnyObject,
        completion_handler: &block2::Block<dyn Fn(isize, *mut AnyObject)>,
      ) {
        let is_proxy = if !challenge.is_null() {
          let space: Option<Retained<AnyObject>> =
            unsafe { msg_send![challenge, protectionSpace] };
          if let Some(space) = space {
            let proxy_flag: Bool = unsafe { msg_send![&*space, isProxy] };
            proxy_flag.as_bool()
          } else {
            false
          }
        } else {
          false
        };
        if is_proxy {
          completion_handler.call((2, std::ptr::null_mut()));
          (self.ivars().on_fail)();
        } else {
          completion_handler.call((1, std::ptr::null_mut()));
        }
      }
    }
  );

  static PROXY_DELEGATE_ASSOC_KEY: u8 = 0;

  pub(super) unsafe fn attach_macos_nav_failed_delegate(
    view: &WKWebView,
    on_failed: impl Fn() + Send + 'static,
  ) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or_else(|| "当前不在 macOS 主线程".to_string())?;
    let prev: Option<Retained<AnyObject>> = msg_send![view, navigationDelegate];
    let inner = prev.as_deref().map(Weak::new);
    let allocated = mtm.alloc::<MaamNavigationDelegateProxy>();
    let set = allocated.set_ivars(MaamNavigationDelegateIvars {
      inner,
      on_fail: Box::new(on_failed),
    });
    let proxy: Retained<MaamNavigationDelegateProxy> = msg_send![super(set), init];
    objc2::ffi::objc_setAssociatedObject(
      (view as *const WKWebView) as *mut AnyObject,
      (&raw const PROXY_DELEGATE_ASSOC_KEY).cast::<std::ffi::c_void>(),
      Retained::as_ptr(&proxy) as *mut AnyObject,
      objc2::ffi::OBJC_ASSOCIATION_RETAIN_NONATOMIC,
    );
    let _: () = msg_send![view, setNavigationDelegate: &*proxy];
    Ok(())
  }
}

#[cfg(target_os = "macos")]
pub async fn on_macos_navigation_failed(
  webview: &tauri::Webview,
  on_failed: impl Fn() + Send + 'static,
) -> Result<(), String> {
  use objc2_web_kit::WKWebView;

  let (sender, receiver) = tokio::sync::oneshot::channel();
  webview
    .with_webview(move |platform_view| unsafe {
      let view: &WKWebView = &*platform_view.inner().cast();
      let result = macos_nav_delegate::attach_macos_nav_failed_delegate(view, on_failed);
      let _ = sender.send(result);
    })
    .map_err(|err| format!("访问 macOS WebView 失败: {err}"))?;
  receiver
    .await
    .map_err(|_| "注册 macOS WebView 导航失败回调中断".to_string())?
}

#[cfg(target_os = "macos")]
fn create_http_connect_proxy(
  url: &url::Url,
) -> Result<objc2::rc::Retained<objc2_foundation::NSObject>, String> {
  use objc2::rc::Retained;
  use objc2_foundation::NSObject;
  use std::ffi::{c_char, CString};

  #[link(name = "Network", kind = "framework")]
  unsafe extern "C" {
    fn nw_endpoint_create_host(host: *const c_char, port: *const c_char) -> *mut NSObject;
    fn nw_proxy_config_create_http_connect(
      endpoint: *const NSObject,
      tls_options: *const NSObject,
    ) -> *mut NSObject;
    fn nw_proxy_config_set_failover_allowed(proxy: *const NSObject, allowed: bool);
  }

  let host = url.host_str().ok_or_else(|| "缺少代理主机".to_string())?;
  let port = url
    .port_or_known_default()
    .ok_or_else(|| "缺少代理端口".to_string())?;
  let host = CString::new(host.trim_matches(['[', ']'])).map_err(|_| "代理主机无效".to_string())?;
  let port = CString::new(port.to_string()).map_err(|_| "代理端口无效".to_string())?;
  unsafe {
    let endpoint = Retained::from_raw(nw_endpoint_create_host(host.as_ptr(), port.as_ptr()))
      .ok_or_else(|| "无法创建代理端点".to_string())?;
    let proxy = Retained::from_raw(nw_proxy_config_create_http_connect(&*endpoint, std::ptr::null()))
      .ok_or_else(|| "无法创建 HTTP CONNECT 代理配置".to_string())?;
    nw_proxy_config_set_failover_allowed(&*proxy, false);
    Ok(proxy)
  }
}

#[cfg(desktop)]
pub fn verify_desktop_profile_binding(
  webview: &tauri::Webview,
  expected_uuid: Uuid,
  expected_dir: &Path,
) -> impl std::future::Future<Output = Result<(), String>> + Send {
  #[cfg(target_os = "macos")]
  {
    let _ = expected_dir;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let res = webview
      .with_webview(move |platform_wv| unsafe {
        let wk: &objc2_web_kit::WKWebView = &*platform_wv.inner().cast();
        let store = wk.configuration().websiteDataStore();
        let check = match store.identifier() {
          Some(ns_uuid) => {
            let actual = ns_uuid.UUIDString().to_string();
            let expected = expected_uuid.hyphenated().to_string();
            if actual.eq_ignore_ascii_case(&expected) {
              Ok(())
            } else {
              Err(format!(
                "WKWebsiteDataStore 标识不匹配（期望 {expected}，实际 {actual}）"
              ))
            }
          }
          None => Err("WKWebsiteDataStore 未绑定独立 UUID（落入默认资料），已阻止加载".to_string()),
        };
        let _ = tx.send(check);
      });

    async move {
      res.map_err(|e| format!("访问 macOS WebView 失败: {e}"))?;
      rx
        .await
        .map_err(|_| "核对 macOS WKWebsiteDataStore 超时或中断".to_string())?
    }
  }

  #[cfg(windows)]
  {
    let _ = expected_uuid;
    let expected_owned = expected_dir.to_path_buf();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let res = webview
      .with_webview(move |platform_wv| unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
          ICoreWebView2Environment7, ICoreWebView2_2,
        };
        use windows::core::Interface;

        let check = (|| -> Result<(), String> {
          let core = platform_wv
            .controller()
            .CoreWebView2()
            .map_err(|e| format!("获取 CoreWebView2 失败: {e}"))?;
          let core2: ICoreWebView2_2 = core
            .cast()
            .map_err(|e| format!("转换 ICoreWebView2_2 失败: {e}"))?;
          let env = core2
            .Environment()
            .map_err(|e| format!("获取 WebView2 Environment 失败: {e}"))?;
          let env7: ICoreWebView2Environment7 = env
            .cast()
            .map_err(|e| format!("转换 ICoreWebView2Environment7 失败: {e}"))?;
          let mut raw_folder = windows::core::PWSTR::null();
          env7
            .UserDataFolder(&mut raw_folder)
            .map_err(|e| format!("读取 WebView2 UserDataFolder 失败: {e}"))?;
          let actual_str = raw_folder
            .to_string()
            .map_err(|e| format!("解析 UserDataFolder 失败: {e}"))?;
          windows::Win32::System::Com::CoTaskMemFree(Some(raw_folder.0 as *const _));

          let actual_norm = normalize_win_path(Path::new(&actual_str));
          let expected_norm = normalize_win_path(&expected_owned);
          if actual_norm == expected_norm {
            Ok(())
          } else {
            Err(format!(
              "Windows WebView2 实际数据目录被系统配置覆盖（实际: {actual_str}），已阻止打开以防串号"
            ))
          }
        })();
        let _ = tx.send(check);
      });

    async move {
      res.map_err(|e| format!("访问 Windows WebView2 失败: {e}"))?;
      rx
        .await
        .map_err(|_| "核对 Windows WebView2 数据目录中断".to_string())?
    }
  }

  #[cfg(all(not(target_os = "macos"), not(windows)))]
  {
    let _ = (webview, expected_uuid, expected_dir);
    async move { Ok(()) }
  }
}

#[cfg(windows)]
fn normalize_win_path(path: &Path) -> String {
  let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
  resolved
    .to_string_lossy()
    .trim_start_matches(r"\\?\")
    .trim_end_matches(['\\', '/'])
    .to_ascii_lowercase()
}

#[cfg(target_os = "macos")]
pub async fn read_macos_cookie_header(
  app: &tauri::AppHandle,
  uuid: Uuid,
  url: url::Url,
) -> Result<String, String> {
  use objc2::{AnyThread, MainThreadMarker};
  use objc2_foundation::{NSArray, NSHTTPCookie, NSString, NSUUID};
  use objc2_web_kit::WKWebsiteDataStore;
  use std::cell::RefCell;
  use std::ptr::NonNull;

  let host = url.host_str().ok_or_else(|| "服务域名无效".to_string())?.to_owned();
  let path = url.path().to_owned();
  let (sender, receiver) = tokio::sync::oneshot::channel();
  app
    .run_on_main_thread(move || {
      let identifier = NSString::from_str(&uuid.to_string());
      let Some(identifier) = NSUUID::initWithUUIDString(NSUUID::alloc(), &identifier) else {
        let _ = sender.send(Err("账号资料标识无效".to_string()));
        return;
      };
      let Some(main_thread) = MainThreadMarker::new() else {
        let _ = sender.send(Err("读取账号 Cookie 失败".to_string()));
        return;
      };
      let store = unsafe { WKWebsiteDataStore::dataStoreForIdentifier(&identifier, main_thread) };
      let sender = RefCell::new(Some(sender));
      let callback = block2::RcBlock::new(move |cookies: NonNull<NSArray<NSHTTPCookie>>| {
        let mut pairs = Vec::new();
        for cookie in unsafe { cookies.as_ref() }.to_vec() {
          let domain = cookie.domain().to_string().to_ascii_lowercase();
          let domain_matches = match domain.strip_prefix('.') {
            Some(domain) => host == domain
              || host.strip_suffix(domain).is_some_and(|prefix| prefix.ends_with('.')),
            None => host == domain,
          };
          let cookie_path = cookie.path().to_string();
          let path_matches = path == cookie_path
            || path.strip_prefix(&cookie_path).is_some_and(|suffix| {
              cookie_path.ends_with('/') || suffix.starts_with('/')
            });
          let expired = cookie.expiresDate().is_some_and(|date| date.timeIntervalSinceNow() <= 0.0);
          if domain_matches && path_matches && !expired {
            pairs.push((cookie_path.len(), format!("{}={}", cookie.name(), cookie.value())));
          }
        }
        pairs.sort_by_key(|(path_len, _)| std::cmp::Reverse(*path_len));
        let header = pairs.into_iter().map(|(_, pair)| pair).collect::<Vec<_>>().join("; ");
        if let Some(sender) = sender.borrow_mut().take() {
          let _ = sender.send(Ok(header));
        }
      });
      unsafe { store.httpCookieStore().getAllCookies(&callback) };
    })
    .map_err(|_| "读取账号 Cookie 失败".to_string())?;
  receiver.await.map_err(|_| "读取账号 Cookie 中断".to_string())?
}

#[cfg(any(target_os = "macos", windows))]
pub async fn write_cookie_clipboard(app: &tauri::AppHandle, header: String) -> Result<(), String> {
  #[cfg(target_os = "macos")]
  use arboard::SetExtApple;
  #[cfg(windows)]
  use arboard::SetExtWindows;

  if header.is_empty() {
    return Err("该账号暂无可复制的 Cookie，请先登录".to_string());
  }
  let (sender, receiver) = tokio::sync::oneshot::channel();
  app
    .run_on_main_thread(move || {
      let result = arboard::Clipboard::new().and_then(|mut clipboard| {
        let text = clipboard.set().exclude_from_history();
        #[cfg(windows)]
        let text = text.exclude_from_cloud().exclude_from_monitoring();
        text.text(header)
      });
      let _ = sender.send(result.map_err(|_| "无法写入系统剪贴板，请重试".to_string()));
    })
    .map_err(|_| "无法写入系统剪贴板，请重试".to_string())?;
  receiver.await.map_err(|_| "写入系统剪贴板中断".to_string())?
}

#[cfg(desktop)]
pub async fn delete_desktop_profile(
  app: &tauri::AppHandle,
  app_data_dir: &Path,
  uuid_str: &str,
) -> Result<(), String> {
  let parsed = Uuid::parse_str(uuid_str).map_err(|e| format!("非法 UUID: {e}"))?;

  #[cfg(target_os = "macos")]
  {
    let _ = app_data_dir;
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let uuid_formatted = parsed.hyphenated().to_string();
    let tx_cell = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));

    app
      .run_on_main_thread(move || {
        use objc2::AnyThread;
        use objc2_foundation::{NSError, NSString, NSUUID};
        use objc2_web_kit::WKWebsiteDataStore;

        let ns_str = NSString::from_str(&uuid_formatted);
        let Some(ns_uuid) = NSUUID::initWithUUIDString(NSUUID::alloc(), &ns_str) else {
          if let Some(sender) = tx_cell.lock().unwrap().take() {
            let _ = sender.send(Err("构造 NSUUID 失败".to_string()));
          }
          return;
        };

        let Some(mtm) = objc2::MainThreadMarker::new() else {
          if let Some(sender) = tx_cell.lock().unwrap().take() {
            let _ = sender.send(Err("当前不在 macOS 主线程".to_string()));
          }
          return;
        };

        let tx_inner = tx_cell.clone();
        let block = block2::RcBlock::new(move |err: *mut NSError| {
          let outcome = if err.is_null() {
            Ok(())
          } else {
            let msg = unsafe { (&*err).localizedDescription().to_string() };
            if msg.to_ascii_lowercase().contains("not exist")
              || msg.to_ascii_lowercase().contains("not found")
            {
              Ok(())
            } else {
              Err(format!("移除 WKWebsiteDataStore 失败: {msg}"))
            }
          };
          if let Some(sender) = tx_inner.lock().unwrap().take() {
            let _ = sender.send(outcome);
          }
        });

        unsafe {
          WKWebsiteDataStore::removeDataStoreForIdentifier_completionHandler(&ns_uuid, &block, mtm);
        }
      })
      .map_err(|e| format!("调度主线程清理 WKWebsiteDataStore 失败: {e}"))?;

    rx
      .await
      .map_err(|_| "等待 WKWebsiteDataStore 清理完成中断".to_string())?
  }

  #[cfg(not(target_os = "macos"))]
  {
    let _ = (app, parsed);
    let profile_dir = app_data_dir.join("profiles").join(uuid_str);
    if profile_dir.exists() {
      std::fs::remove_dir_all(&profile_dir)
        .map_err(|e| format!("清理资料目录被占用或失败: {e}"))?;
    }
    Ok(())
  }
}

#[cfg(windows)]
pub fn lock_windows_manager_webview_zoom(webview: &tauri::Webview) {
  let _ = webview.with_webview(|pwv| unsafe {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings5;
    use windows::core::Interface;

    let controller = pwv.controller();
    let _ = controller.SetZoomFactor(1.0);
    if let Ok(core) = controller.CoreWebView2() {
      if let Ok(settings) = core.Settings() {
        let _ = settings.SetIsZoomControlEnabled(false);
        if let Ok(settings5) = settings.cast::<ICoreWebView2Settings5>() {
          let _ = settings5.SetIsPinchZoomEnabled(false);
        }
      }
    }
  });
}

#[cfg(desktop)]
pub fn reveal_path_in_file_manager(path: &Path) -> Result<(), String> {
  #[cfg(target_os = "macos")]
  {
    if path.exists() {
      std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开访达失败: {e}"))
    } else if let Some(parent) = path.parent().filter(|p| p.exists()) {
      open::that_detached(parent).map_err(|e| format!("打开文件夹失败: {e}"))
    } else {
      Err("文件已被移动或删除".to_string())
    }
  }
  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;

    if path.exists() {
      let raw = path.to_string_lossy();
      let clean = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
      std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{clean}\""))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开资源管理器失败: {e}"))
    } else if let Some(parent) = path.parent().filter(|p| p.exists()) {
      open::that_detached(parent).map_err(|e| format!("打开文件夹失败: {e}"))
    } else {
      Err("文件已被移动或删除".to_string())
    }
  }
  #[cfg(all(not(target_os = "macos"), not(windows)))]
  {
    if let Some(target) = path.parent().filter(|p| p.exists()) {
      open::that_detached(target).map_err(|e| format!("打开文件夹失败: {e}"))
    } else if path.exists() {
      open::that_detached(path).map_err(|e| format!("打开文件失败: {e}"))
    } else {
      Err("文件已被移动或删除".to_string())
    }
  }
}

#[cfg(target_os = "android")]
pub use tauri_plugin_account_session as android;

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parse_and_format_account_proxy_rules() {
    assert_eq!(parse_account_proxy("   "), Ok(None));

    if !SUPPORTS_ACCOUNT_PROXY {
      assert!(parse_account_proxy("http://127.0.0.1:7890").is_err());
      return;
    }

    let ipv4 = parse_account_proxy("  HTTP://127.0.0.1:7890/ ").unwrap().unwrap();
    assert_eq!(format_account_proxy(&ipv4), "http://127.0.0.1:7890");

    let ipv6 = parse_account_proxy("http://[::1]:10808").unwrap().unwrap();
    assert_eq!(format_account_proxy(&ipv6), "http://[::1]:10808");

    let domain = parse_account_proxy("http://proxy_1.example-host.local:80").unwrap().unwrap();
    assert_eq!(format_account_proxy(&domain), "http://proxy_1.example-host.local:80");

    for invalid in [
      "http://127.0.0.1",
      "http://127.0.0.1/",
      "http://127.0.0.1:",
      "http://[::1]",
      "http://[::1]/",
      "http://127.0.0.1:0",
      "socks5://127.0.0.1:7890",
      "https://127.0.0.1:7890",
      "http://user:pass@127.0.0.1:7890",
      "http://127.0.0.1:7890/path",
      "http://127.0.0.1:7890?a=1",
      "http://127.0.0.1:7890#frag",
      "http://127.0.0.1: 7890",
      "http://proxy!bad.local:7890",
    ] {
      assert!(parse_account_proxy(invalid).is_err(), "expected error for {invalid}");
    }
  }

  #[cfg(target_os = "macos")]
  #[test]
  fn macos_http_connect_proxy_and_nav_error_filter() {
    for raw in [
      "http://127.0.0.1:7890",
      "http://[::1]:10808",
      "http://proxy.internal:80",
    ] {
      let url = parse_account_proxy(raw).unwrap().unwrap();
      assert!(create_http_connect_proxy(&url).is_ok(), "failed for {raw}");
    }

    assert!(!should_fail_macos_navigation("NSURLErrorDomain", -999));
    assert!(!should_fail_macos_navigation("WebKitErrorDomain", 102));
    assert!(should_fail_macos_navigation("NSURLErrorDomain", -1004));
    assert!(should_fail_macos_navigation("NSURLErrorDomain", -1012));
    assert!(should_fail_macos_navigation("NSPOSIXErrorDomain", 61));
    assert!(should_fail_macos_navigation("WebKitErrorDomain", 101));
  }

  #[cfg(windows)]
  #[test]
  fn windows_proxy_args_and_error_matching() {
    let url80 = parse_account_proxy("http://proxy.local:80").unwrap().unwrap();
    let args80 = windows_proxy_browser_args(&url80);
    assert!(args80.contains("--proxy-server=http://proxy.local:80"));
    assert!(args80.contains("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"));
    assert!(args80.contains("--autoplay-policy=no-user-gesture-required"));

    let ipv6 = parse_account_proxy("http://[::1]:7890").unwrap().unwrap();
    assert!(windows_proxy_browser_args(&ipv6).contains("--proxy-server=http://[::1]:7890"));

    use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_INVALID_STATE};
    let in_use_err = tauri::Error::Runtime(tauri_runtime::Error::CreateWebview(Box::new(
      wry::Error::WebView2Error(webview2_com::Error::from(ERROR_INVALID_STATE.to_hresult())),
    )));
    assert!(windows_webview_options_in_use(&in_use_err));

    let other_err = tauri::Error::Runtime(tauri_runtime::Error::CreateWebview(Box::new(
      wry::Error::WebView2Error(webview2_com::Error::from(ERROR_ACCESS_DENIED.to_hresult())),
    )));
    assert!(!windows_webview_options_in_use(&other_err));
  }
}
