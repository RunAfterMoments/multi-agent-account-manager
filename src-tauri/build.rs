const APP_COMMANDS: &[&str] = &[
  "get_manager_state",
  "select_provider",
  "add_account",
  "rename_account",
  "test_account_proxy",
  "copy_account_cookie",
  "import_account_cookies",
  "remove_account",
  "reorder_accounts",
  "reload_account",
  "clear_account_data",
  "reveal_downloaded_file",
  "switch_account",
  "set_modal_active",
  "open_official_site",
  "resolve_uuid_conflict",
  "dismiss_notice",
  "continue_with_empty_config",
  "restore_config_from_backup",
  "trigger_quota_sync",
  "update_app_settings",
  "set_sidebar_collapsed",
  "select_group_chat",
  "create_group_chat",
  "update_group_chat",
  "delete_group_chat",
  "send_group_message",
  "retry_group_message",
  "sync_group_message_reply",
  "clear_group_messages",
  "start_group_new_topic",
  "update_group_knowledge",
  "append_group_shared_context",
  "upsert_external_agent",
  "delete_external_agent",
  "cancel_external_member_turn",
  "update_group_workflow",
  "reset_group_thread_binding",
  "get_site_adapters",
  "save_site_adapter_overrides",
  "reset_site_adapter_overrides",
  "check_for_updates",
  "download_and_install_update",
  "open_external_url",
];

fn main() {
  let attributes = tauri_build::Attributes::new()
    .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS))
    // Windows: 别让 tauri-build 再把清单嵌进 resource.rc——下面的 linker 清单
    // 会覆盖全部产物（应用本体 + 测试程序），两处各嵌一份会在链接时报
    // CVT1100 重复资源。关掉它之后，resource.rc 只剩图标和版本信息。
    .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
  tauri_build::try_build(attributes).expect("failed to run tauri-build");

  // Windows: 全部产物都要带 Common-Controls v6 清单，否则测试程序在任何
  // 测试运行前就被加载器以 0xc0000139 拒绝——tauri-runtime-wry 的对话框
  // 代码要调 TaskDialogIndirect，而它只在 comctl32 v6 里才有，系统默认的
  // v5.82 没有。应用本体以前靠 tauri-winres 经 resource.rc 嵌这份清单，
  // 测试程序不走那条路所以一直裸奔；现在改由 linker 经 /MANIFESTINPUT
  // 统一嵌（内容与 tauri-build 默认清单逐字相同，见 windows-app-manifest.xml），
  // 应用本体有效清单不变。
  // 注：cargo:rustc-link-arg-tests 在此走不通——cargo 只认 tests/ 目录下的
  // 集成测试目标，本 crate 只有 lib 内单元测试，v6.1.3 因此在 clippy 步
  // 就被拒（invalid instruction）。只在 Windows 宿主上发 link.exe 参数：
  // MinGW 交叉链接和 lld-link 都消化不了 /MANIFESTINPUT（后者还要外挂 mt.exe）。
  // 宿主门 + 目标门缺一不可：宿主不是 Windows（cargo-xwin）时没有 link.exe；
  // 目标不是 MSVC（Windows 上交叉编 MinGW/Android）时链接器也不是 link.exe。
  let is_windows_msvc_target = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
    && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
  if cfg!(target_os = "windows") && is_windows_msvc_target {
    let manifest = std::path::PathBuf::from(
      std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo"),
    )
    .join("windows-app-manifest.xml");
    println!("cargo:rerun-if-changed=windows-app-manifest.xml");
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
  }
}
