//! API Key 凭据库（PR-F / MAAM-H-06）。
//!
//! 桌面优先系统凭据库（macOS Keychain / Windows Credential Manager /
//! Linux Secret Service，通过 `keyring` crate）；移动端走插件桥接
//! （Android Keystore / iOS Keychain）。系统凭据库不可用时不再静默
//! 写明文：Windows 回退文件改存 DPAPI 密文（`dpapi:<base64>`，
//! CurrentUser 作用域 + UI 禁止）；Linux/macOS 与移动端桥接失败时
//! store 直接报错（中文提示出路）。
//!
//! 旧明文 `api-keys.json` 只作只读迁移源：load 读到旧条目时，若安全
//! 后端可用立即迁移（写安全后端、删旧条目）；安全后端不可用则照常
//! 返回旧值（不断现有功能）。迁移完文件为空时删除该文件。load 返回
//! `Zeroizing<String>`，内存 Key 用完即清零。

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use zeroize::Zeroizing;

pub const KEYRING_SERVICE: &str = "multi-agent-account-manager";
pub const FALLBACK_FILE_NAME: &str = "api-keys.json";
/// 回退文件里 DPAPI 密文值的前缀（仅 Windows 写入）。
const DPAPI_VALUE_PREFIX: &str = "dpapi:";

/// `accounts.json` 里存的引用 id：`maam-api-key:<agent uuid>`。
pub fn api_key_ref_for(agent_id: &str) -> String {
  format!("maam-api-key:{agent_id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VaultBackend {
  /// 仅桌面端构造（keyring 探测成功时）；移动端保留该变体以便 match 穷尽。
  #[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
  Keyring,
  File,
  /// 仅移动端构造（桥接探测成功时）；桌面端保留该变体以便 match 穷尽。
  #[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
  Bridge,
  #[cfg(test)]
  Memory,
}

pub struct ApiKeyVault {
  dir: PathBuf,
  backend: Mutex<Option<VaultBackend>>,
  /// 移动端桥接用的 AppHandle（setup 时由 SessionManager 注入；
  /// 桌面端不用）。注入后失效已缓存的后端探测结果。
  app_handle: Mutex<Option<tauri::AppHandle>>,
  /// 仅测试：内存后端的数据；用 Arc 以便跨 SessionManager 实例
  /// 共享同一份数据，模拟重启后从同一后端恢复。
  #[cfg(test)]
  memory: std::sync::Arc<Mutex<BTreeMap<String, String>>>,
}

impl ApiKeyVault {
  pub fn new(dir: PathBuf) -> Self {
    Self {
      dir,
      backend: Mutex::new(None),
      app_handle: Mutex::new(None),
      #[cfg(test)]
      memory: std::sync::Arc::new(Mutex::new(BTreeMap::new())),
    }
  }

  /// 仅测试：预置后端的 vault（不做探测）。
  #[cfg(test)]
  pub(crate) fn new_with_backend(dir: PathBuf, backend: VaultBackend) -> Self {
    Self {
      dir,
      backend: Mutex::new(Some(backend)),
      app_handle: Mutex::new(None),
      memory: std::sync::Arc::new(Mutex::new(BTreeMap::new())),
    }
  }

  /// 仅测试：内存后端 vault（数据不出进程，供无 secret service 的
  /// 机器上跑通 upsert/删除等逻辑测试）。
  #[cfg(test)]
  pub(crate) fn new_memory(dir: PathBuf) -> Self {
    Self::new_with_backend(dir, VaultBackend::Memory)
  }

  /// 仅测试：与另一 vault 共享同一份内存数据的 vault（模拟重启后
  /// 从同一后端恢复）。
  #[cfg(test)]
  pub(crate) fn new_memory_shared(
    dir: PathBuf,
    shared: std::sync::Arc<Mutex<BTreeMap<String, String>>>,
  ) -> Self {
    Self {
      dir,
      backend: Mutex::new(Some(VaultBackend::Memory)),
      app_handle: Mutex::new(None),
      memory: shared,
    }
  }

  /// 注入移动端桥接用的 AppHandle；注入后失效已缓存的后端探测。
  pub fn set_app_handle(&self, app: tauri::AppHandle) {
    *self.app_handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(app);
    *self.backend.lock().unwrap_or_else(|e| e.into_inner()) = None;
  }

  pub fn fallback_path(&self) -> PathBuf {
    self.dir.join(FALLBACK_FILE_NAME)
  }

  async fn detect_backend(&self) -> VaultBackend {
    {
      let slot = self.backend.lock().unwrap_or_else(|e| e.into_inner());
      if let Some(backend) = *slot {
        return backend;
      }
    }
    // 探测时不持锁（桥接探测要 await）。
    let detected = self.probe_backend().await;
    let mut slot = self.backend.lock().unwrap_or_else(|e| e.into_inner());
    *slot = Some(detected);
    detected
  }

  async fn probe_backend(&self) -> VaultBackend {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
      if self.probe_bridge().await {
        return VaultBackend::Bridge;
      }
      return VaultBackend::File;
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
      self.probe_keyring()
    }
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

  /// 移动端探测桥接是否可用：桥接 state 未注册时直接判不可用
  /// （不触发 app.state() 的 panic）；再对不存在的条目做一次 load，
  /// 返回 Ok 即为可用。
  #[cfg(any(target_os = "android", target_os = "ios"))]
  async fn probe_bridge(&self) -> bool {
    use tauri::Manager;
    let Some(app) = self
      .app_handle
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .clone()
    else {
      return false;
    };
    if app
      .try_state::<crate::platform::mobile::MobileSessionBridge<tauri::Wry>>()
      .is_none()
    {
      return false;
    }
    crate::platform::mobile::load_api_key(&app, "__maam_probe__")
      .await
      .is_ok()
  }

  pub async fn store_key(&self, key_ref: &str, key: &str) -> Result<(), String> {
    match self.detect_backend().await {
      VaultBackend::Keyring => self.store_keyring(key_ref, key),
      VaultBackend::File => self.store_file_secure(key_ref, key),
      VaultBackend::Bridge => self.store_bridge(key_ref, key).await,
      #[cfg(test)]
      VaultBackend::Memory => {
        self
          .memory
          .lock()
          .unwrap_or_else(|e| e.into_inner())
          .insert(key_ref.to_string(), key.to_string());
        Ok(())
      }
    }
  }

  pub async fn load_key(&self, key_ref: &str) -> Option<Zeroizing<String>> {
    match self.detect_backend().await {
      VaultBackend::Keyring => {
        if let Some(v) = self.load_keyring(key_ref) {
          return Some(v);
        }
        // 回退文件里可能有旧条目（凭据库不可用期间写入的，或旧明文）：
        // 读到即迁移到 keyring，迁不成也照常返回旧值。
        self
          .load_file_migrating(key_ref, VaultBackend::Keyring)
          .await
      }
      VaultBackend::File => self.load_file_direct(key_ref).await,
      VaultBackend::Bridge => {
        if let Some(v) = self.load_bridge(key_ref).await {
          return Some(v);
        }
        self
          .load_file_migrating(key_ref, VaultBackend::Bridge)
          .await
      }
      #[cfg(test)]
      VaultBackend::Memory => self
        .memory
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key_ref)
        .map(|v| Zeroizing::new(v.clone())),
    }
  }

  pub async fn delete_key(&self, key_ref: &str) {
    // 尽力删干净：keyring、桥接、回退文件、内存后端各删各的，失败忽略；
    // Key 可能在任一后端（后端切换期间写入的）。
    let _ = self.delete_keyring(key_ref);
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let _ = self.delete_bridge(key_ref).await;
    self.delete_file_entry(key_ref);
    #[cfg(test)]
    self
      .memory
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .remove(key_ref);
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
  fn load_keyring(&self, key_ref: &str) -> Option<Zeroizing<String>> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, key_ref).ok()?;
    entry.get_password().ok().map(Zeroizing::new)
  }

  #[cfg(any(target_os = "android", target_os = "ios"))]
  fn load_keyring(&self, _key_ref: &str) -> Option<Zeroizing<String>> {
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

  /// keyring/桥接不可用时的文件回退：Windows 用 DPAPI 加密后写盘；
  /// 其他平台没有可用的加密手段，直接报错（不再静默写明文）。
  fn store_file_secure(&self, key_ref: &str, key: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
      let cipher = dpapi_encrypt(key)?;
      self.write_file_entry(key_ref, &format!("{DPAPI_VALUE_PREFIX}{cipher}"))
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
      let _ = (key_ref, key);
      Err(
        "移动端安全存储（Keystore/Keychain）不可用，无法安全保存 API Key，请重试（本版本不再把明文 Key 写到回退文件）"
          .to_string(),
      )
    }
    #[cfg(target_os = "linux")]
    {
      let _ = (key_ref, key);
      Err(
        "系统凭据库（Secret Service）不可用，无法安全保存 API Key；请安装并启动 GNOME Keyring 或 KWallet 等 Secret Service 后重试（本版本不再把明文 Key 写到回退文件）"
          .to_string(),
      )
    }
    #[cfg(target_os = "macos")]
    {
      let _ = (key_ref, key);
      Err(
        "系统钥匙串不可用，无法安全保存 API Key；请确认钥匙串访问正常后重试（本版本不再把明文 Key 写到回退文件）"
          .to_string(),
      )
    }
    #[cfg(not(any(
      windows,
      target_os = "android",
      target_os = "ios",
      target_os = "linux",
      target_os = "macos"
    )))]
    {
      let _ = (key_ref, key);
      Err("系统凭据库不可用，无法安全保存 API Key（本版本不再把明文 Key 写到回退文件）".to_string())
    }
  }

  /// 从回退文件读一条：无则 None；有则解密/原样返回，并尝试迁移到
  /// 安全后端（写成功即删文件条目；写失败也照常返回旧值，不丢功能）。
  async fn load_file_migrating(
    &self,
    key_ref: &str,
    target: VaultBackend,
  ) -> Option<Zeroizing<String>> {
    let raw = self.read_file_map().get(key_ref).cloned()?;
    let value = self.decrypt_file_value(&raw)?;
    let migrated = match target {
      VaultBackend::Keyring => self.store_keyring(key_ref, value.as_str()).is_ok(),
      VaultBackend::Bridge => self.store_bridge(key_ref, value.as_str()).await.is_ok(),
      _ => false,
    };
    if migrated {
      self.delete_file_entry(key_ref);
    }
    Some(value)
  }

  /// 安全后端不可用时直接读回退文件：Windows 上旧明文条目读到即
  /// 改写为 DPAPI 密文；其他平台原样返回旧值（不断现有功能）。
  async fn load_file_direct(&self, key_ref: &str) -> Option<Zeroizing<String>> {
    let raw = self.read_file_map().get(key_ref).cloned()?;
    let value = self.decrypt_file_value(&raw)?;
    #[cfg(windows)]
    {
      if !raw.starts_with(DPAPI_VALUE_PREFIX) {
        if let Ok(cipher) = dpapi_encrypt(value.as_str()) {
          let _ = self.write_file_entry(key_ref, &format!("{DPAPI_VALUE_PREFIX}{cipher}"));
        }
      }
    }
    Some(value)
  }

  /// 解析回退文件里的值：`dpapi:<base64>`（仅 Windows）解密；
  /// 否则视为旧明文原样返回。解密失败返回 None。
  fn decrypt_file_value(&self, raw: &str) -> Option<Zeroizing<String>> {
    #[cfg(windows)]
    {
      if let Some(cipher_b64) = raw.strip_prefix(DPAPI_VALUE_PREFIX) {
        let cipher = base64_decode(cipher_b64)?;
        let plain = dpapi_decrypt(&cipher)?;
        return Some(Zeroizing::new(String::from_utf8(plain).ok()?));
      }
    }
    #[cfg(not(windows))]
    {
      if raw.starts_with(DPAPI_VALUE_PREFIX) {
        return None;
      }
    }
    Some(Zeroizing::new(raw.to_string()))
  }

  #[cfg(any(target_os = "android", target_os = "ios"))]
  async fn store_bridge(&self, key_ref: &str, key: &str) -> Result<(), String> {
    let app = self.require_app_handle()?;
    crate::platform::mobile::store_api_key(&app, key_ref, key).await
  }

  #[cfg(not(any(target_os = "android", target_os = "ios")))]
  async fn store_bridge(&self, _key_ref: &str, _key: &str) -> Result<(), String> {
    Err("当前平台不支持移动端安全存储桥接".to_string())
  }

  #[cfg(any(target_os = "android", target_os = "ios"))]
  async fn load_bridge(&self, key_ref: &str) -> Option<Zeroizing<String>> {
    let app = self.require_app_handle().ok()?;
    crate::platform::mobile::load_api_key(&app, key_ref)
      .await
      .ok()
      .flatten()
      .map(Zeroizing::new)
  }

  #[cfg(not(any(target_os = "android", target_os = "ios")))]
  async fn load_bridge(&self, _key_ref: &str) -> Option<Zeroizing<String>> {
    None
  }

  #[cfg(any(target_os = "android", target_os = "ios"))]
  async fn delete_bridge(&self, key_ref: &str) -> Result<(), String> {
    let app = self.require_app_handle()?;
    crate::platform::mobile::delete_api_key(&app, key_ref).await
  }

  #[cfg(any(target_os = "android", target_os = "ios"))]
  fn require_app_handle(&self) -> Result<tauri::AppHandle, String> {
    self
      .app_handle
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .clone()
      .ok_or_else(|| "移动端安全存储不可用：应用句柄未注入".to_string())
  }

  fn read_file_map(&self) -> BTreeMap<String, String> {
    let Ok(raw) = fs::read_to_string(self.fallback_path()) else {
      return BTreeMap::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
  }

  /// 写一条回退文件条目；写后 map 为空则删文件（不留空 JSON）。
  /// 仅 Windows 使用（DPAPI 密文写盘 / 旧明文改写）。
  #[cfg(windows)]
  fn write_file_entry(&self, key_ref: &str, value: &str) -> Result<(), String> {
    let mut map = self.read_file_map();
    map.insert(key_ref.to_string(), value.to_string());
    self.write_file_map(&map)
  }

  fn delete_file_entry(&self, key_ref: &str) {
    let mut map = self.read_file_map();
    if map.remove(key_ref).is_none() {
      return;
    }
    let _ = self.write_file_map(&map);
  }

  fn write_file_map(&self, map: &BTreeMap<String, String>) -> Result<(), String> {
    if map.is_empty() {
      let path = self.fallback_path();
      if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("删除空凭据回退文件失败: {e}"))?;
      }
      return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(map).map_err(|e| format!("序列化凭据失败: {e}"))?;
    write_bytes_atomic_0600(&self.fallback_path(), &bytes)
  }
}

/// Windows DPAPI：CurrentUser 作用域 + UI 禁止，把明文包成
/// `dpapi:<base64>` 写入回退文件。空输入直接拒绝。
/// 签名对照 vendored windows 0.62.2：
/// `src/Windows/Win32/Security/Cryptography/mod.rs`
/// （CryptProtectData/CryptUnprotectData，CRYPT_INTEGER_BLOB）。
#[cfg(windows)]
fn dpapi_encrypt(plaintext: &str) -> Result<String, String> {
  use base64::{engine::general_purpose::STANDARD, Engine as _};
  use windows::core::PCWSTR;
  use windows::Win32::Foundation::{LocalFree, HLOCAL};
  use windows::Win32::Security::Cryptography::{
    CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
  };

  if plaintext.is_empty() {
    return Err("DPAPI 加密拒绝空输入".to_string());
  }
  let input = CRYPT_INTEGER_BLOB {
    cbData: plaintext.len() as u32,
    pbData: plaintext.as_ptr() as *mut u8,
  };
  let mut output = CRYPT_INTEGER_BLOB::default();
  unsafe {
    CryptProtectData(
      &input,
      PCWSTR::null(),
      None,
      None,
      None,
      CRYPTPROTECT_UI_FORBIDDEN,
      &mut output,
    )
  }
  .map_err(|e| format!("DPAPI 加密失败: {e}"))?;
  // 输出由 LocalFree 释放：先拷出字节再释放；pbData 为空即算失败。
  let bytes = unsafe {
    if output.pbData.is_null() {
      return Err("DPAPI 加密返回空结果".to_string());
    }
    let b = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
    LocalFree(Some(HLOCAL(output.pbData as *mut core::ffi::c_void)));
    b
  };
  if bytes.is_empty() {
    return Err("DPAPI 加密返回空结果".to_string());
  }
  Ok(STANDARD.encode(bytes))
}

/// Windows DPAPI 解密：空密文返回 None；失败返回 None（密文损坏、
/// 换了用户或机器——调用方按“无此条目”处理，不丢其他功能）。
#[cfg(windows)]
fn dpapi_decrypt(cipher: &[u8]) -> Option<Vec<u8>> {
  use windows::Win32::Foundation::{LocalFree, HLOCAL};
  use windows::Win32::Security::Cryptography::{
    CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
  };

  if cipher.is_empty() {
    return None;
  }
  let input = CRYPT_INTEGER_BLOB {
    cbData: cipher.len() as u32,
    pbData: cipher.as_ptr() as *mut u8,
  };
  let mut output = CRYPT_INTEGER_BLOB::default();
  unsafe {
    CryptUnprotectData(
      &input,
      None,
      None,
      None,
      None,
      CRYPTPROTECT_UI_FORBIDDEN,
      &mut output,
    )
  }
  .ok()?;
  let bytes = unsafe {
    if output.pbData.is_null() {
      return None;
    }
    let b = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
    LocalFree(Some(HLOCAL(output.pbData as *mut core::ffi::c_void)));
    b
  };
  if bytes.is_empty() {
    None
  } else {
    Some(bytes)
  }
}

#[cfg(windows)]
fn base64_decode(s: &str) -> Option<Vec<u8>> {
  use base64::{engine::general_purpose::STANDARD, Engine as _};
  STANDARD.decode(s).ok()
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

  /// PR-F 新行为：无安全后端时 store 必须明确报错、不得写明文回退
  /// 文件；有安全后端时照常往返。按探测到的后端分情况断言，不写死
  /// 环境假设（本机无 secret service 走 File；有 Keychain 的机器走
  /// Keyring；Windows CI 走 Credential Manager）。
  #[tokio::test]
  async fn store_key_fails_closed_without_secure_backend() {
    let (vault, dir) = temp_vault();
    let backend = vault.detect_backend().await;
    let key_ref = api_key_ref_for("00000000-0000-4000-8000-000000000001");
    match backend {
      VaultBackend::Keyring => {
        vault.store_key(&key_ref, "sk-test-123").await.unwrap();
        assert_eq!(
          vault
            .load_key(&key_ref)
            .await
            .as_deref()
            .map(String::as_str),
          Some("sk-test-123")
        );
        vault.delete_key(&key_ref).await;
        assert_eq!(vault.load_key(&key_ref).await, None);
      }
      VaultBackend::File => {
        let err = vault.store_key(&key_ref, "sk-test-123").await.unwrap_err();
        assert!(
          err.contains("凭据库") || err.contains("安全"),
          "报错应说明原因: {err}"
        );
        assert!(
          !vault.fallback_path().exists(),
          "fail-closed：不得写明文回退文件"
        );
        assert_eq!(vault.load_key(&key_ref).await, None);
      }
      VaultBackend::Bridge => unreachable!("桌面测试不应探测到 Bridge"),
      VaultBackend::Memory => unreachable!("new() 不应预置 Memory"),
    }
    let _ = std::fs::remove_dir_all(&dir);
  }

  /// 强制 File 后端：非 Windows 上 store 必须报错（Linux 提示 Secret
  /// Service）；Windows 上 File 后端即 DPAPI，store 应成功且文件存
  /// `dpapi:` 密文。
  #[tokio::test]
  async fn store_key_errors_when_no_secure_backend() {
    let dir = std::env::temp_dir().join(format!("maam-vault-err-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let vault = ApiKeyVault::new_with_backend(dir.clone(), VaultBackend::File);
    let key_ref = api_key_ref_for("00000000-0000-4000-8000-000000000004");
    #[cfg(not(target_os = "windows"))]
    {
      let err = vault.store_key(&key_ref, "sk-x").await.unwrap_err();
      #[cfg(target_os = "linux")]
      assert!(
        err.contains("Secret Service"),
        "Linux 报错应提示 Secret Service: {err}"
      );
      #[cfg(target_os = "macos")]
      assert!(err.contains("钥匙串"), "macOS 报错应提示钥匙串: {err}");
      assert!(!vault.fallback_path().exists(), "报错时不得写回退文件");
    }
    #[cfg(target_os = "windows")]
    {
      vault.store_key(&key_ref, "sk-x").await.unwrap();
      let stored = vault
        .read_file_map()
        .get(&key_ref)
        .expect("DPAPI 值应已写盘")
        .to_string();
      assert!(
        stored.starts_with(DPAPI_VALUE_PREFIX),
        "Windows File 后端应存 dpapi: 密文"
      );
    }
    let _ = std::fs::remove_dir_all(&dir);
  }

  /// 旧明文回退文件只读迁移：load 读到旧条目必须返回旧值；有安全
  /// 后端时迁移走并清理文件条目，无安全后端时照常返回、条目保留。
  #[tokio::test]
  async fn legacy_plaintext_file_migrates_on_load() {
    let (vault, dir) = temp_vault();
    let key_ref = api_key_ref_for("00000000-0000-4000-8000-000000000002");
    let mut map = BTreeMap::new();
    map.insert(key_ref.clone(), "sk-legacy-456".to_string());
    let bytes = serde_json::to_vec_pretty(&map).unwrap();
    write_bytes_atomic_0600(&vault.fallback_path(), &bytes).unwrap();

    let backend = vault.detect_backend().await;
    let loaded = vault.load_key(&key_ref).await;
    assert_eq!(
      loaded.as_deref().map(String::as_str),
      Some("sk-legacy-456"),
      "旧明文 Key 必须能读回"
    );
    match backend {
      VaultBackend::Keyring => {
        let after = vault.read_file_map();
        assert!(!after.contains_key(&key_ref), "迁移后旧条目应从文件删除");
        assert_eq!(
          vault.load_keyring(&key_ref).as_deref().map(String::as_str),
          Some("sk-legacy-456"),
          "迁移后 Key 应在 keyring 里"
        );
        vault.delete_key(&key_ref).await;
      }
      VaultBackend::File => {
        assert!(
          vault.read_file_map().contains_key(&key_ref),
          "无安全后端时旧条目保留，下次再试迁移"
        );
      }
      VaultBackend::Bridge => unreachable!("桌面测试不应探测到 Bridge"),
      VaultBackend::Memory => unreachable!("new() 不应预置 Memory"),
    }
    let _ = std::fs::remove_dir_all(&dir);
  }

  /// 仅测试的内存后端往返（供 SessionManager 逻辑测试复用）。
  #[tokio::test]
  async fn memory_backend_roundtrip_for_tests() {
    let dir = std::env::temp_dir().join(format!("maam-vault-mem-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let vault = ApiKeyVault::new_memory(dir.clone());
    let key_ref = api_key_ref_for("00000000-0000-4000-8000-000000000005");
    vault.store_key(&key_ref, "sk-mem-1").await.unwrap();
    assert_eq!(
      vault
        .load_key(&key_ref)
        .await
        .as_deref()
        .map(String::as_str),
      Some("sk-mem-1")
    );
    vault.delete_key(&key_ref).await;
    assert_eq!(vault.load_key(&key_ref).await, None);
    assert!(!vault.fallback_path().exists(), "内存后端不得碰回退文件");
    let _ = std::fs::remove_dir_all(&dir);
  }

  /// Windows DPAPI 往返（本机编不到，CI Windows 会跑）。
  #[cfg(windows)]
  #[test]
  fn dpapi_encrypt_decrypt_roundtrip() {
    let plain = "sk-dpapi-test-789";
    let cipher = dpapi_encrypt(plain).expect("DPAPI 加密应成功");
    assert!(!cipher.contains(plain), "密文不得含明文");
    let back =
      dpapi_decrypt(&base64_decode(&cipher).expect("base64 应合法")).expect("DPAPI 解密应成功");
    assert_eq!(String::from_utf8(back).unwrap(), plain);
    assert!(dpapi_encrypt("").is_err(), "空输入应拒绝");
    assert!(dpapi_decrypt(&[]).is_none(), "空密文应返回 None");
  }

  /// Windows 回退文件 DPAPI 值往返 + 旧明文读时改写（CI Windows 会跑）。
  #[cfg(windows)]
  #[tokio::test]
  async fn dpapi_file_value_roundtrip() {
    let (vault, dir) = temp_vault();
    let key_ref = api_key_ref_for("00000000-0000-4000-8000-000000000003");
    let cipher = dpapi_encrypt("sk-file-dpapi-1").unwrap();
    vault
      .write_file_entry(&key_ref, &format!("{DPAPI_VALUE_PREFIX}{cipher}"))
      .unwrap();
    let loaded = vault.load_file_direct(&key_ref).await;
    assert_eq!(
      loaded.as_deref().map(String::as_str),
      Some("sk-file-dpapi-1")
    );
    // 明文旧值在 Windows File 后端读时改写为 DPAPI。
    vault.write_file_entry(&key_ref, "sk-plain-old").unwrap();
    let loaded2 = vault.load_file_direct(&key_ref).await;
    assert_eq!(loaded2.as_deref().map(String::as_str), Some("sk-plain-old"));
    let raw = vault.read_file_map().get(&key_ref).cloned().unwrap();
    assert!(
      raw.starts_with(DPAPI_VALUE_PREFIX),
      "旧明文应被改写为 DPAPI 密文"
    );
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
