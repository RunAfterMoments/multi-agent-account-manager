//! API Key 凭据库。
//!
//! 优先使用系统凭据库（macOS Keychain / Windows Credential Manager /
//! Linux Secret Service，通过 `keyring` crate）；不可用时（Linux 无
//! secret service、移动端）回退为应用数据目录下的 `api-keys.json`，
//! Unix 上以 0600 权限原子写盘。`accounts.json` 里只存引用 id，
//! 明文 Key 永不落盘到配置里，也永不下发到前端。

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const KEYRING_SERVICE: &str = "multi-agent-account-manager";
pub const FALLBACK_FILE_NAME: &str = "api-keys.json";

/// `accounts.json` 里存的引用 id：`maam-api-key:<agent uuid>`。
pub fn api_key_ref_for(agent_id: &str) -> String {
  format!("maam-api-key:{agent_id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VaultBackend {
  Keyring,
  File,
}

pub struct ApiKeyVault {
  dir: PathBuf,
  backend: Mutex<Option<VaultBackend>>,
}

impl ApiKeyVault {
  pub fn new(dir: PathBuf) -> Self {
    Self {
      dir,
      backend: Mutex::new(None),
    }
  }

  pub fn fallback_path(&self) -> PathBuf {
    self.dir.join(FALLBACK_FILE_NAME)
  }

  fn detect_backend(&self) -> VaultBackend {
    let mut slot = self.backend.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(backend) = *slot {
      return backend;
    }
    let detected = self.probe_keyring();
    *slot = Some(detected);
    detected
  }

  /// 探测系统凭据库是否可用：对一个不存在的条目做 get，
  /// `NoEntry` 说明凭据库可达，其他错误说明不可用。
  #[cfg(not(any(target_os = "android", target_os = "ios")))]
  fn probe_keyring(&self) -> VaultBackend {
    match keyring::Entry::new(KEYRING_SERVICE, "__maam_probe__") {
      Ok(entry) => match entry.get_password() {
        Ok(_) | Err(keyring::Error::NoEntry) => VaultBackend::Keyring,
        Err(_) => VaultBackend::File,
      },
      Err(_) => VaultBackend::File,
    }
  }

  /// 移动端不编译 keyring，直接走文件回退。
  #[cfg(any(target_os = "android", target_os = "ios"))]
  fn probe_keyring(&self) -> VaultBackend {
    VaultBackend::File
  }

  pub fn store_key(&self, key_ref: &str, key: &str) -> Result<(), String> {
    match self.detect_backend() {
      VaultBackend::Keyring => self
        .store_keyring(key_ref, key)
        .or_else(|_| self.store_file(key_ref, key)),
      VaultBackend::File => self.store_file(key_ref, key),
    }
  }

  pub fn load_key(&self, key_ref: &str) -> Option<String> {
    match self.detect_backend() {
      // keyring 无此条目时也查回退文件（Key 可能是凭据库不可用期间写入的）。
      VaultBackend::Keyring => self
        .load_keyring(key_ref)
        .or_else(|| self.load_file(key_ref)),
      VaultBackend::File => self.load_file(key_ref),
    }
  }

  pub fn delete_key(&self, key_ref: &str) {
    let _ = self.delete_keyring(key_ref);
    self.delete_file(key_ref);
  }

  #[cfg(not(any(target_os = "android", target_os = "ios")))]
  fn store_keyring(&self, key_ref: &str, key: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, key_ref)
      .map_err(|e| format!("打开系统凭据库失败: {e}"))?;
    entry
      .set_password(key)
      .map_err(|e| format!("写入系统凭据库失败: {e}"))
  }

  #[cfg(any(target_os = "android", target_os = "ios"))]
  fn store_keyring(&self, _key_ref: &str, _key: &str) -> Result<(), String> {
    Err("移动端不支持系统凭据库".to_string())
  }

  #[cfg(not(any(target_os = "android", target_os = "ios")))]
  fn load_keyring(&self, key_ref: &str) -> Option<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, key_ref).ok()?;
    entry.get_password().ok()
  }

  #[cfg(any(target_os = "android", target_os = "ios"))]
  fn load_keyring(&self, _key_ref: &str) -> Option<String> {
    None
  }

  #[cfg(not(any(target_os = "android", target_os = "ios")))]
  fn delete_keyring(&self, key_ref: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, key_ref)
      .map_err(|e| format!("打开系统凭据库失败: {e}"))?;
    match entry.delete_credential() {
      Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
      Err(e) => Err(format!("删除系统凭据失败: {e}")),
    }
  }

  #[cfg(any(target_os = "android", target_os = "ios"))]
  fn delete_keyring(&self, _key_ref: &str) -> Result<(), String> {
    Ok(())
  }

  fn read_file_map(&self) -> BTreeMap<String, String> {
    let Ok(raw) = fs::read_to_string(self.fallback_path()) else {
      return BTreeMap::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
  }

  fn store_file(&self, key_ref: &str, key: &str) -> Result<(), String> {
    let mut map = self.read_file_map();
    map.insert(key_ref.to_string(), key.to_string());
    let bytes = serde_json::to_vec_pretty(&map).map_err(|e| format!("序列化凭据失败: {e}"))?;
    write_bytes_atomic_0600(&self.fallback_path(), &bytes)
  }

  fn load_file(&self, key_ref: &str) -> Option<String> {
    self.read_file_map().get(key_ref).cloned()
  }

  fn delete_file(&self, key_ref: &str) {
    let mut map = self.read_file_map();
    if map.remove(key_ref).is_none() {
      return;
    }
    let Ok(bytes) = serde_json::to_vec_pretty(&map) else {
      return;
    };
    let _ = write_bytes_atomic_0600(&self.fallback_path(), &bytes);
  }
}

/// 原子写盘：临时文件 → `sync_all` → `rename`，与 `save_store_atomic`
/// 的约定一致；Unix 上临时文件权限为 0600（最终文件继承该权限）。
pub(crate) fn write_bytes_atomic_0600(path: &Path, bytes: &[u8]) -> Result<(), String> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
  }
  let tmp_name = format!(
    ".{}.tmp.{}",
    path.file_name().and_then(|n| n.to_str()).unwrap_or("tmp"),
    uuid::Uuid::new_v4().simple()
  );
  let tmp_path = path.with_file_name(tmp_name);
  {
    #[cfg(unix)]
    let mut file = {
      use std::os::unix::fs::OpenOptionsExt;
      fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp_path)
        .map_err(|e| format!("创建临时文件失败: {e}"))?
    };
    #[cfg(not(unix))]
    let mut file = fs::File::create(&tmp_path).map_err(|e| format!("创建临时文件失败: {e}"))?;
    file
      .write_all(bytes)
      .map_err(|e| format!("写入临时文件失败: {e}"))?;
    file
      .sync_all()
      .map_err(|e| format!("同步临时文件失败: {e}"))?;
  }
  fs::rename(&tmp_path, path).map_err(|e| {
    let _ = fs::remove_file(&tmp_path);
    format!("原子替换文件失败: {e}")
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn temp_vault() -> (ApiKeyVault, PathBuf) {
    let dir = std::env::temp_dir().join(format!("maam-vault-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    (ApiKeyVault::new(dir.clone()), dir)
  }

  #[test]
  fn api_key_ref_format() {
    assert_eq!(api_key_ref_for("abc"), "maam-api-key:abc");
  }

  #[test]
  fn file_fallback_roundtrip_and_0600_perms() {
    let (vault, dir) = temp_vault();
    // 本机测试环境无 secret service 时走文件回退；若有则走 keyring。
    // 两种后端都应 roundtrip；回退文件若生成，权限必须为 0600。
    let key_ref = api_key_ref_for("00000000-0000-4000-8000-000000000001");
    vault.store_key(&key_ref, "sk-test-123").unwrap();
    assert_eq!(vault.load_key(&key_ref).as_deref(), Some("sk-test-123"));
    let fallback = vault.fallback_path();
    if fallback.exists() {
      #[cfg(unix)]
      {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&fallback).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "回退凭据文件必须为 0600");
      }
      let raw = std::fs::read_to_string(&fallback).unwrap();
      assert!(raw.contains("sk-test-123"), "回退文件应含该条 Key");
    }
    vault.delete_key(&key_ref);
    assert_eq!(vault.load_key(&key_ref), None);
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn write_bytes_atomic_0600_sets_mode_and_overwrite_keeps_mode() {
    let dir = std::env::temp_dir().join(format!("maam-vault-0600-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("secret.json");
    write_bytes_atomic_0600(&path, b"{\"a\":1}").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}");
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
      assert_eq!(mode, 0o600);
    }
    // 覆盖写走 rename 替换，权限仍来自新临时文件（0600）。
    write_bytes_atomic_0600(&path, b"{\"a\":2}").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":2}");
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
      assert_eq!(mode, 0o600);
    }
    let _ = std::fs::remove_dir_all(&dir);
  }
}
