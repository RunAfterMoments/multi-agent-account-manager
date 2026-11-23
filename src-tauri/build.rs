const APP_COMMANDS: &[&str] = &[
  "get_manager_state",
  "select_provider",
  "add_custom_provider",
  "update_custom_provider",
  "remove_custom_provider",
  "add_account",
  "rename_account",
  "test_account_proxy",
  "copy_account_cookie",
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
  "trigger_quota_sync",
  "update_app_settings",
  "check_for_updates",
  "download_and_install_update",
  "open_external_url",
];

fn main() {
  let attributes = tauri_build::Attributes::new()
    .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS));
  tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
