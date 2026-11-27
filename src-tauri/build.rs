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
    .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS));
  tauri_build::try_build(attributes).expect("failed to run tauri-build");

  // Windows: 测试程序也要带 Common-Controls v6 清单，否则加载器在任何测试
  // 运行前就以 0xc0000139 拒绝它——tauri-runtime-wry 的对话框代码要调
  // TaskDialogIndirect，而它只在 comctl32 v6 里才有，系统默认的 v5.82 没有。
  // 应用本体由 tauri-winres 通过 resource.rc 嵌了同一份清单，测试程序不走
  // 那条路，所以这里只给 tests 目标再补一份（内容与 tauri-build 默认清单
  // 逐字相同，见 windows-app-manifest.xml）。只在 Windows 宿主上发 link.exe
  // 参数：MinGW 交叉链接和 lld-link 都消化不了 /MANIFESTINPUT（后者还要
  // 外挂 mt.exe）。
  if cfg!(target_os = "windows") {
    let manifest = std::path::PathBuf::from(
      std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo"),
    )
    .join("windows-app-manifest.xml");
    println!("cargo:rerun-if-changed=windows-app-manifest.xml");
    println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
    println!(
      "cargo:rustc-link-arg-tests=/MANIFESTINPUT={}",
      manifest.display()
    );
  }
}
