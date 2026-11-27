mod group_transport;
mod key_vault;
mod platform;
mod providers;
mod session_manager;
mod site_adapters;
mod storage;
mod updater;

use platform::ProxyTestResult;
use session_manager::{ManagerSnapshot, SessionManager, APP_TITLE};
use site_adapters::SiteAdaptersSnapshot;
#[cfg(desktop)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use storage::{
  AccountKey, ExternalAgentKind, GroupMemberRole, GroupSharedSkill, GroupWorkflowMode,
};
use tauri::{AppHandle, Manager, State};
use updater::{InstallUpdateOutcome, InstallUpdateRequest, UpdateCheckResult};

#[cfg(desktop)]
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

#[tauri::command]
async fn get_manager_state(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以读取管理状态".to_string());
  }
  Ok(mgr.get_snapshot().await)
}

#[tauri::command]
async fn select_provider(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  provider_id: String,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以切换服务".to_string());
  }
  mgr.select_provider(&app, &provider_id).await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn add_account(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  provider_id: String,
  alias: String,
  proxy_url: Option<String>,
  timezone: Option<String>,
  raw_cookies: Option<String>,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以添加账号".to_string());
  }
  mgr
    .add_account(
      &app,
      &provider_id,
      &alias,
      proxy_url.as_deref(),
      timezone.as_deref(),
      raw_cookies.as_deref(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn rename_account(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  provider_id: String,
  uuid: String,
  alias: String,
  proxy_url: Option<String>,
  timezone: Option<String>,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以修改账号设置".to_string());
  }
  mgr
    .update_account_settings(
      &app,
      &provider_id,
      &uuid,
      &alias,
      proxy_url.as_deref(),
      timezone.as_deref(),
    )
    .await
}

#[tauri::command]
async fn copy_account_cookie(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  provider_id: String,
  uuid: String,
) -> Result<(), String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以复制 Cookie".to_string());
  }
  mgr.copy_account_cookie(&app, &provider_id, &uuid).await
}

#[tauri::command]
async fn import_account_cookies(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  provider_id: String,
  uuid: String,
  raw_cookies: String,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以导入 Cookie".to_string());
  }
  mgr
    .import_account_cookies(&app, &provider_id, &uuid, &raw_cookies)
    .await
}

#[tauri::command]
async fn remove_account(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  provider_id: String,
  uuid: String,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以删除账号".to_string());
  }
  mgr.remove_account(&app, &provider_id, &uuid).await
}

#[tauri::command]
async fn reorder_accounts(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  provider_id: String,
  ordered_uuids: Vec<String>,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以调整账号顺序".to_string());
  }
  mgr.reorder_accounts(&provider_id, ordered_uuids).await
}

#[tauri::command]
async fn reveal_downloaded_file(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  download_id: u64,
) -> Result<(), String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以打开下载文件夹".to_string());
  }
  mgr.reveal_downloaded_file(download_id).await
}

#[tauri::command]
async fn switch_account(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  provider_id: String,
  uuid: String,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以切换账号".to_string());
  }
  mgr.switch_account(&app, &provider_id, &uuid).await
}

#[tauri::command]
async fn set_modal_active(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  active: bool,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以设置弹窗状态".to_string());
  }
  mgr.set_modal_active(&app, active).await
}

#[tauri::command]
async fn set_sidebar_collapsed(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  collapsed: bool,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以切换侧边栏折叠".to_string());
  }
  mgr.set_sidebar_collapsed(&app, collapsed).await
}

#[tauri::command]
async fn open_official_site(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  provider_id: String,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以打开官网".to_string());
  }
  mgr.open_official_site(&app, &provider_id).await
}

#[tauri::command]
async fn resolve_uuid_conflict(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  keep_provider_id: String,
  uuid: String,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以处理 UUID 冲突".to_string());
  }
  mgr.resolve_uuid_conflict(&keep_provider_id, &uuid).await
}

#[tauri::command]
async fn dismiss_notice(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以关闭提示".to_string());
  }
  Ok(mgr.dismiss_notice().await)
}

#[tauri::command]
async fn continue_with_empty_config(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以继续使用空配置".to_string());
  }
  mgr.continue_with_empty_config().await
}

#[tauri::command]
async fn restore_config_from_backup(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以从备份恢复配置".to_string());
  }
  mgr.restore_config_from_backup().await
}

#[tauri::command]
async fn trigger_quota_sync(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以触发额度同步".to_string());
  }
  mgr.trigger_silent_sync(&app, true).await;
  Ok(mgr.get_snapshot().await)
}

#[tauri::command]
async fn test_account_proxy(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  provider_id: String,
  proxy_url: String,
) -> Result<ProxyTestResult, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以测试代理连接".to_string());
  }
  mgr.test_account_proxy(&provider_id, &proxy_url).await
}

#[tauri::command]
async fn reload_account(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  provider_id: String,
  uuid: String,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以刷新账号".to_string());
  }
  mgr.reload_account(&app, &provider_id, &uuid).await
}

#[tauri::command]
async fn clear_account_data(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  provider_id: String,
  uuid: String,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以清理账号缓存".to_string());
  }
  mgr.clear_account_data(&app, &provider_id, &uuid).await
}

/// Tauri 命令的参数由前端扁平传入（invoke payload 的字段与参数一一对应），
/// 无法像普通函数那样收成结构体而不改前端调用；8 个参数是三块加固
/// （P0-0 禁用自动更新检查、P0-3 命令类审批开关）各加一个可选开关所致。
/// 该 lint 由合并引入，此处显式 allow；基线既有的同类错误另由 clippy
/// 清理 commit 统一处理。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn update_app_settings(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  minimize_to_tray: bool,
  global_shortcut: String,
  auto_approve: Option<bool>,
  disable_auto_update_check: Option<bool>,
  auto_approve_commands: Option<bool>,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以修改应用设置".to_string());
  }
  mgr
    .update_app_settings(
      &app,
      minimize_to_tray,
      &global_shortcut,
      auto_approve,
      disable_auto_update_check,
      auto_approve_commands,
    )
    .await
}

#[tauri::command]
async fn select_group_chat(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  group_id: Option<String>,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以切换群聊".to_string());
  }
  mgr.select_group_chat(&app, group_id).await
}

#[tauri::command]
async fn create_group_chat(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  name: String,
  members: Vec<AccountKey>,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以创建群聊".to_string());
  }
  mgr.create_group_chat(&app, name, members).await
}

#[tauri::command]
async fn update_group_chat(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  group_id: String,
  name: String,
  members: Vec<AccountKey>,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以编辑群聊".to_string());
  }
  mgr.update_group_chat(&app, group_id, name, members).await
}

#[tauri::command]
async fn delete_group_chat(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  group_id: String,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以删除群聊".to_string());
  }
  mgr.delete_group_chat(&app, group_id).await
}

#[tauri::command]
async fn send_group_message(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  group_id: String,
  content: String,
  target_members: Option<Vec<AccountKey>>,
  auto_channels: Option<Vec<String>>,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以发送群聊消息".to_string());
  }
  mgr
    .send_group_message(&app, group_id, content, target_members, auto_channels)
    .await
}

#[tauri::command]
async fn retry_group_message(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  group_id: String,
  msg_id: String,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以重试群聊消息".to_string());
  }
  mgr.retry_group_message(&app, group_id, msg_id).await
}

#[tauri::command]
async fn sync_group_message_reply(
  app: AppHandle,
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  group_id: String,
  msg_id: String,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以同步群聊回复".to_string());
  }
  mgr.sync_group_message_reply(&app, group_id, msg_id).await
}

#[tauri::command]
async fn clear_group_messages(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  group_id: String,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以清空群聊记录".to_string());
  }
  mgr.clear_group_messages(group_id).await
}

#[tauri::command]
async fn start_group_new_topic(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  group_id: String,
  keep_shared_context: bool,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以开启新话题".to_string());
  }
  mgr
    .start_group_new_topic(group_id, keep_shared_context)
    .await
}

#[tauri::command]
async fn update_group_knowledge(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  group_id: String,
  shared_context: String,
  shared_skills: Vec<GroupSharedSkill>,
  member_roles: Vec<GroupMemberRole>,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以保存群聊知识配置".to_string());
  }
  mgr
    .update_group_knowledge(group_id, shared_context, shared_skills, member_roles)
    .await
}

#[tauri::command]
async fn append_group_shared_context(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  group_id: String,
  snippet: String,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以沉淀群聊上下文".to_string());
  }
  mgr.append_group_shared_context(group_id, snippet).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn upsert_external_agent(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
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
  if webview.label() != "manager" {
    return Err("仅账号管理页可以管理 API / CLI 成员".to_string());
  }
  mgr
    .upsert_external_agent(
      id,
      kind,
      name,
      base_url,
      api_key,
      model,
      command,
      args,
      workdir,
      timeout_secs,
      idle_timeout_secs,
    )
    .await
}

#[tauri::command]
async fn cancel_external_member_turn(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  group_id: String,
  msg_id: String,
) -> Result<bool, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以停止外部成员回复".to_string());
  }
  #[cfg(desktop)]
  {
    Ok(mgr.cancel_external_member_turn(group_id, msg_id).await)
  }
  #[cfg(not(desktop))]
  {
    let _ = (&mgr, group_id, msg_id);
    Err("当前平台不支持停止外部成员回复".to_string())
  }
}

#[tauri::command]
async fn delete_external_agent(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  id: String,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以删除 API / CLI 成员".to_string());
  }
  mgr.delete_external_agent(id).await
}

#[tauri::command]
async fn update_group_workflow(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  group_id: String,
  workflow_mode: GroupWorkflowMode,
  max_relay_hops: u8,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以修改协作模式".to_string());
  }
  mgr
    .update_group_workflow(group_id, workflow_mode, max_relay_hops)
    .await
}

#[tauri::command]
async fn reset_group_thread_binding(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  group_id: String,
  member: AccountKey,
) -> Result<ManagerSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以重置成员绑定会话".to_string());
  }
  mgr.reset_group_thread_binding(group_id, member).await
}

#[tauri::command]
async fn get_site_adapters(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
) -> Result<SiteAdaptersSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以查看站点适配规则".to_string());
  }
  Ok(mgr.get_site_adapters().await)
}

#[tauri::command]
async fn save_site_adapter_overrides(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
  raw_json: String,
) -> Result<SiteAdaptersSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以保存站点适配规则".to_string());
  }
  mgr.save_site_adapter_overrides(raw_json).await
}

#[tauri::command]
async fn reset_site_adapter_overrides(
  mgr: State<'_, Arc<SessionManager>>,
  webview: tauri::Webview,
) -> Result<SiteAdaptersSnapshot, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以重置站点适配规则".to_string());
  }
  mgr.reset_site_adapter_overrides().await
}

#[tauri::command]
async fn check_for_updates(
  app: AppHandle,
  webview: tauri::Webview,
) -> Result<UpdateCheckResult, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以检查更新".to_string());
  }
  updater::check_for_updates(&app).await
}

#[tauri::command]
async fn download_and_install_update(
  app: AppHandle,
  webview: tauri::Webview,
  request: InstallUpdateRequest,
) -> Result<InstallUpdateOutcome, String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以下载并安装更新".to_string());
  }
  updater::download_and_install_update(&app, request).await
}

#[tauri::command]
async fn open_external_url(
  app: AppHandle,
  webview: tauri::Webview,
  url: String,
) -> Result<(), String> {
  if webview.label() != "manager" {
    return Err("仅账号管理页可以打开外部链接".to_string());
  }
  updater::open_external_url(&app, &url).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  let builder = tauri::Builder::default();

  #[cfg(desktop)]
  let builder = builder.plugin(tauri_plugin_global_shortcut::Builder::new().build());

  #[cfg(mobile)]
  let builder = builder.plugin(platform::mobile::init());

  builder
    .setup(|app| {
      let app_data_dir = app.path().app_data_dir()?;
      let session_mgr = Arc::new(tauri::async_runtime::block_on(
        SessionManager::new_with_app(app_data_dir, app.handle().clone()),
      ));
      app.manage(Arc::clone(&session_mgr));

      #[cfg(desktop)]
      {
        use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
        use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

        let toggle_item =
          MenuItem::with_id(app, "tray_toggle", "显示 / 隐藏窗口", true, None::<&str>)?;
        let sep_item = PredefinedMenuItem::separator(app)?;
        let quit_item = MenuItem::with_id(app, "tray_quit", "退出叠号", true, None::<&str>)?;
        let tray_menu = Menu::with_items(app, &[&toggle_item, &sep_item, &quit_item])?;
        let tray_icon = match app.default_window_icon().cloned() {
          Some(icon) => icon,
          None => tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))?,
        };
        let _tray = TrayIconBuilder::with_id("main-tray")
          .icon(tray_icon)
          .menu(&tray_menu)
          .tooltip("叠号 · AI 助手账号切换器")
          .show_menu_on_left_click(false)
          .on_menu_event(|app_handle, event| match event.id().as_ref() {
            "tray_toggle" => SessionManager::toggle_main_window(app_handle),
            "tray_quit" => {
              QUIT_REQUESTED.store(true, Ordering::SeqCst);
              app_handle.exit(0);
            }
            _ => {}
          })
          .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
              button: MouseButton::Left,
              button_state: MouseButtonState::Up,
              ..
            } = event
            {
              SessionManager::toggle_main_window(tray.app_handle());
            }
          })
          .build(app)?;

        // 切号黑闪修复第 3 步：主窗口与 manager 默认背景色按系统主题取
        // 浅色白 / 深色 #0f141f（--stage-bg），未绘区域不再是黑底。
        let window_bg = if platform::system_uses_dark_theme() {
          tauri::webview::Color(15, 20, 31, 255)
        } else {
          tauri::webview::Color(255, 255, 255, 255)
        };

        let main_window = tauri::window::WindowBuilder::new(app, "main")
          .title(APP_TITLE)
          .inner_size(1240.0, 800.0)
          .min_inner_size(760.0, 520.0)
          .background_color(window_bg)
          .build()?;

        let manager_builder = tauri::webview::WebviewBuilder::new(
          "manager",
          tauri::WebviewUrl::App("index.html".into()),
        )
        .background_color(window_bg)
        .disable_drag_drop_handler()
        .on_navigation(|url| {
          url.scheme() == "tauri"
            || url.domain() == Some("tauri.localhost")
            || (cfg!(dev) && url.host_str() == Some("localhost"))
        });

        let _manager_wv = main_window.add_child(
          manager_builder,
          tauri::LogicalPosition::new(0.0, 0.0),
          tauri::LogicalSize::new(1240.0, 800.0),
        )?;
        #[cfg(windows)]
        platform::lock_windows_manager_webview_zoom(&_manager_wv);

        let app_for_resize = app.handle().clone();
        let mgr_for_resize = Arc::clone(&session_mgr);
        main_window.on_window_event(move |event| match event {
          tauri::WindowEvent::Resized(_)
          | tauri::WindowEvent::Moved(_)
          | tauri::WindowEvent::ScaleFactorChanged { .. } => {
            let app_inner = app_for_resize.clone();
            let mgr_inner = Arc::clone(&mgr_for_resize);
            tauri::async_runtime::spawn(async move {
              mgr_inner.recompute_desktop_bounds(&app_inner).await;
            });
          }
          tauri::WindowEvent::Focused(focused) if *focused => {
            let app_inner = app_for_resize.clone();
            let mgr_inner = Arc::clone(&mgr_for_resize);
            tauri::async_runtime::spawn(async move {
              mgr_inner.trigger_silent_sync(&app_inner, false).await;
            });
          }
          tauri::WindowEvent::CloseRequested { api, .. }
            if !QUIT_REQUESTED.load(Ordering::SeqCst)
              && mgr_for_resize.minimize_to_tray_enabled() =>
          {
            api.prevent_close();
            if let Some(win) = app_for_resize.get_window("main") {
              let _ = win.hide();
            }
          }
          _ => {}
        });
      }

      #[cfg(mobile)]
      {
        let _ = APP_TITLE;
        tauri::WebviewWindowBuilder::new(
          app,
          "manager",
          tauri::WebviewUrl::App("index.html".into()),
        )
        .background_color(if platform::system_uses_dark_theme() {
          tauri::webview::Color(15, 20, 31, 255)
        } else {
          tauri::webview::Color(255, 255, 255, 255)
        })
        // 与桌面端同款导航锁：manager 只允许加载应用自身页面，
        // 外站链接一律不许在 manager 内打开（走 open_external_url）。
        .on_navigation(|url| {
          url.scheme() == "tauri"
            || url.domain() == Some("tauri.localhost")
            || (cfg!(dev) && url.host_str() == Some("localhost"))
        })
        .build()?;
      }

      let app_for_boot = app.handle().clone();
      let mgr_for_boot = Arc::clone(&session_mgr);
      tauri::async_runtime::spawn(async move {
        mgr_for_boot.run_startup_tasks(&app_for_boot).await;
      });

      Ok(())
    })
    .invoke_handler(tauri::generate_handler![
      get_manager_state,
      select_provider,
      add_account,
      rename_account,
      test_account_proxy,
      copy_account_cookie,
      import_account_cookies,
      remove_account,
      reorder_accounts,
      reload_account,
      clear_account_data,
      reveal_downloaded_file,
      switch_account,
      set_modal_active,
      set_sidebar_collapsed,
      open_official_site,
      resolve_uuid_conflict,
      dismiss_notice,
      continue_with_empty_config,
      restore_config_from_backup,
      trigger_quota_sync,
      update_app_settings,
      select_group_chat,
      create_group_chat,
      update_group_chat,
      delete_group_chat,
      send_group_message,
      retry_group_message,
      sync_group_message_reply,
      clear_group_messages,
      start_group_new_topic,
      update_group_knowledge,
      append_group_shared_context,
      upsert_external_agent,
      delete_external_agent,
      cancel_external_member_turn,
      update_group_workflow,
      reset_group_thread_binding,
      get_site_adapters,
      save_site_adapter_overrides,
      reset_site_adapter_overrides,
      check_for_updates,
      download_and_install_update,
      open_external_url,
    ])
    .build(tauri::generate_context!())
    .expect("error while building tauri application")
    .run(|app_handle, event| {
      #[cfg(target_os = "macos")]
      if let tauri::RunEvent::Reopen {
        has_visible_windows: false,
        ..
      } = event
      {
        if let Some(win) = app_handle.get_window("main") {
          let _ = win.unminimize();
          let _ = win.show();
          let _ = win.set_focus();
        }
      }
      #[cfg(not(target_os = "macos"))]
      let _ = (app_handle, event);
    });
}

#[cfg(test)]
mod acl_consistency_tests {
  // 四方 ACL 一致性单测（v6.5 2B.1）：新增任何 Tauri Command 时，四方必须同步——
  //   lib.rs `generate_handler![...]`（运行时注册）
  //   == build.rs `APP_COMMANDS`（tauri-build 清单，生成 autogenerated/*.toml）
  //   == permissions/autogenerated/*.toml（每命令一个 allow/deny 权限文件）
  //   == capabilities/manager.json `permissions` 里的 `allow-*` 项（实际授予 manager webview 的权限）
  // 任一方漏配，CI 在 `cargo test` 直接红灯，而不是等到运行时才报权限拒绝。
  //
  // 解析方式与局限（逐方）：
  // - manager.json：结构化解析（serde_json，正式依赖），读 `permissions` 数组，
  //   `allow-<kebab-case>` 映射回 snake_case 命令名；`core:event:*` 等非 allow-*
  //   项与命令无关，跳过。
  // - autogenerated/*.toml：toml crate 不在 dev-dependencies，且加 dev-dependency
  //   要改 Cargo.toml / Cargo.lock（本次任务只改测试文件与 ci.yml），故不用 toml 解析。
  //   这些文件由 tauri-build 自动生成，格式固定为 `commands.allow = ["<cmd>"]` 单行单条，
  //   故词法抽取该行是精确的；若上游改格式导致抽不到，测试会报“未解析出任何命令”
  //   而不是静默通过。
  // - lib.rs / build.rs：Rust 源码无现成解析器，用词法抽取字面量条目——只认
  //   `generate_handler![...]` 内的标识符与 `APP_COMMANDS` 内的字符串字面量。
  //   局限：宏内出现 #[cfg] 条件编译、被注释掉的命令、或命令抽到别的宏/变量里时
  //   抽取会失真；新增命令必须以字面量写进这两处列表。
  // - 四方只比集合不比顺序：各方列表顺序本就不同（如 lib.rs 与 build.rs 的
  //   set_sidebar_collapsed 位置不同），顺序不影响权限语义。
  use std::collections::BTreeSet;
  use std::path::PathBuf;

  fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
  }

  fn read_source(relative: &str) -> String {
    let path = crate_dir().join(relative);
    std::fs::read_to_string(&path)
      .unwrap_or_else(|e| panic!("ACL 一致性测试读不到 {relative}（{path:?}）：{e}"))
  }

  fn is_ident(token: &str) -> bool {
    !token.is_empty()
      && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
      && !token.chars().next().unwrap().is_ascii_digit()
  }

  /// 抽取一段文本里的 `"..."` 字符串字面量（不处理转义——命令名不含转义字符）。
  fn quoted_strings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('"') {
      let after = &rest[open + 1..];
      match after.find('"') {
        Some(close) => {
          out.push(after[..close].to_string());
          rest = &after[close + 1..];
        }
        None => panic!("ACL 一致性测试：字符串字面量未闭合：{text:?}"),
      }
    }
    out
  }

  /// 抽取 `generate_handler![a, b, ...]` 内的命令标识符。
  /// 返回（去重后集合，重复条目）。
  fn commands_from_generate_handler(src: &str) -> (BTreeSet<String>, Vec<String>) {
    let start = src
      .find("generate_handler![")
      .expect("lib.rs 找不到 generate_handler![...]");
    let body = &src[start + "generate_handler![".len()..];
    let end = body.find(']').expect("lib.rs generate_handler![ 未闭合");
    let mut commands = BTreeSet::new();
    let mut duplicates = Vec::new();
    for line in body[..end].lines() {
      let token = line
        .split("//")
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches(',');
      if token.is_empty() {
        continue;
      }
      assert!(
        is_ident(token),
        "lib.rs generate_handler! 出现无法解析的条目 {token:?}（只支持字面量标识符）"
      );
      if !commands.insert(token.to_string()) {
        duplicates.push(token.to_string());
      }
    }
    (commands, duplicates)
  }

  /// 抽取 `APP_COMMANDS: &[&str] = &["a", "b", ...]` 内的字符串字面量。
  /// 返回（去重后集合，重复条目）。
  fn commands_from_build_rs(src: &str) -> (BTreeSet<String>, Vec<String>) {
    let start = src
      .find("APP_COMMANDS")
      .expect("build.rs 找不到 APP_COMMANDS");
    let assign = src[start..]
      .find("= &[")
      .expect("build.rs APP_COMMANDS 后找不到 `= &[`");
    let body = &src[start + assign + "= &[".len()..];
    let end = body.find("];").expect("build.rs APP_COMMANDS 列表未闭合");
    let mut commands = BTreeSet::new();
    let mut duplicates = Vec::new();
    for literal in quoted_strings(&body[..end]) {
      assert!(
        is_ident(&literal),
        "build.rs APP_COMMANDS 出现无法解析的条目 {literal:?}（只支持字面量命令名）"
      );
      if !commands.insert(literal.clone()) {
        duplicates.push(literal);
      }
    }
    (commands, duplicates)
  }

  /// 解析 capabilities/manager.json：`permissions` 数组里 `allow-<kebab>` 的命令集合。
  /// 返回（去重后集合，重复条目）。
  fn commands_from_manager_json(src: &str) -> (BTreeSet<String>, Vec<String>) {
    let value: serde_json::Value =
      serde_json::from_str(src).expect("capabilities/manager.json 不是合法 JSON");
    let permissions = value
      .get("permissions")
      .and_then(|p| p.as_array())
      .expect("capabilities/manager.json 缺少 permissions 数组");
    let mut commands = BTreeSet::new();
    let mut duplicates = Vec::new();
    for permission in permissions {
      let permission = permission
        .as_str()
        .expect("capabilities/manager.json permissions 含非字符串项");
      let Some(kebab) = permission.strip_prefix("allow-") else {
        continue; // core:event:* 等与命令无关的权限，跳过。
      };
      let command = kebab.replace('-', "_");
      assert!(
        is_ident(&command),
        "capabilities/manager.json 的权限 {permission:?} 映射不出合法命令名"
      );
      if !commands.insert(command) {
        duplicates.push(permission.to_string());
      }
    }
    (commands, duplicates)
  }

  /// 解析 permissions/autogenerated/*.toml：每文件取 `commands.allow = ["<cmd>"]` 的唯一命令，
  /// 并要求文件名就是 `<cmd>.toml`。返回（去重后集合，重复条目）。
  fn commands_from_permission_tomls() -> (BTreeSet<String>, Vec<String>) {
    let dir = crate_dir().join("permissions/autogenerated");
    let entries = std::fs::read_dir(&dir)
      .unwrap_or_else(|e| panic!("ACL 一致性测试读不到 permissions/autogenerated（{dir:?}）：{e}"));
    let mut commands = BTreeSet::new();
    let mut duplicates = Vec::new();
    let mut file_count = 0;
    for entry in entries {
      let path = entry
        .expect("读 permissions/autogenerated 目录项失败")
        .path();
      if path.extension().is_none_or(|ext| ext != "toml") {
        continue;
      }
      file_count += 1;
      let file_name = path
        .file_name()
        .expect("toml 文件无文件名")
        .to_string_lossy()
        .into_owned();
      let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("ACL 一致性测试读不到 {file_name}：{e}"));
      let marker = text.find("commands.allow = [").unwrap_or_else(|| {
        panic!("{file_name} 里找不到 `commands.allow = [...]` 行（格式变了？）")
      });
      let after = &text[marker + "commands.allow = [".len()..];
      let end = after
        .find(']')
        .unwrap_or_else(|| panic!("{file_name} 的 commands.allow 列表未闭合"));
      let allowed = quoted_strings(&after[..end]);
      assert_eq!(
        allowed.len(),
        1,
        "{file_name} 的 commands.allow 应恰好含 1 条命令，实际 {} 条：{allowed:?}",
        allowed.len()
      );
      let command = allowed.into_iter().next().unwrap();
      assert_eq!(
        file_name,
        format!("{command}.toml"),
        "{file_name} 的 commands.allow 是 {command}，与文件名不符"
      );
      if !commands.insert(command) {
        duplicates.push(file_name);
      }
    }
    assert!(
      file_count > 0,
      "permissions/autogenerated 下一个 .toml 文件都没读到"
    );
    (commands, duplicates)
  }

  /// 把 a 有、b 没有的命令记为“b 缺少命令 X（a 有）”，反向亦然。
  fn diff(
    a: &BTreeSet<String>,
    a_label: &str,
    b: &BTreeSet<String>,
    b_label: &str,
    problems: &mut Vec<String>,
  ) {
    for cmd in a.difference(b) {
      problems.push(format!("{b_label} 缺少命令 {cmd}（{a_label} 有）"));
    }
    for cmd in b.difference(a) {
      problems.push(format!("{a_label} 缺少命令 {cmd}（{b_label} 有）"));
    }
  }

  fn check_duplicates(side: &str, duplicates: &[String]) {
    assert!(
      duplicates.is_empty(),
      "{side} 有重复命令：{}",
      duplicates.join(", ")
    );
  }

  #[test]
  fn four_way_acl_consistency() {
    let (from_lib, lib_dups) = commands_from_generate_handler(&read_source("src/lib.rs"));
    let (from_build, build_dups) = commands_from_build_rs(&read_source("build.rs"));
    let (from_toml, toml_dups) = commands_from_permission_tomls();
    let (from_manager, manager_dups) =
      commands_from_manager_json(&read_source("capabilities/manager.json"));

    check_duplicates("lib.rs generate_handler!", &lib_dups);
    check_duplicates("build.rs APP_COMMANDS", &build_dups);
    check_duplicates("permissions/autogenerated/*.toml", &toml_dups);
    check_duplicates("capabilities/manager.json", &manager_dups);

    assert!(
      !from_lib.is_empty() && !from_build.is_empty() && !from_toml.is_empty() && !from_manager.is_empty(),
      "四方 ACL 至少一方解析出 0 条命令（lib {} / build {} / toml {} / manager {}），解析逻辑可能已失效",
      from_lib.len(),
      from_build.len(),
      from_toml.len(),
      from_manager.len()
    );

    let mut problems = Vec::new();
    diff(&from_lib, "lib.rs", &from_build, "build.rs", &mut problems);
    diff(
      &from_lib,
      "lib.rs",
      &from_toml,
      "permissions/autogenerated/*.toml",
      &mut problems,
    );
    diff(
      &from_lib,
      "lib.rs",
      &from_manager,
      "capabilities/manager.json",
      &mut problems,
    );
    assert!(
      problems.is_empty(),
      "四方 ACL 不一致（{} 处）：\n{}",
      problems.len(),
      problems.join("\n")
    );
  }
}
