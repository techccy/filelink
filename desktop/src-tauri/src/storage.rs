//! 本地持久化：config.json、history.json 与 Keychain 凭据。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::state::HistoryItem;

pub const SERVICE: &str = "filelink";
pub const KEY_TOKEN: &str = "upload-token";

pub fn secret_account(id: &str) -> String {
    format!("secret-{id}")
}

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub base_url: String,
}

pub fn app_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("filelink"))
}

pub fn load_config(app: &AppHandle) -> Config {
    let path = app_dir(app).join("config.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(app: &AppHandle, base_url: &str) -> Result<(), String> {
    let dir = app_dir(app);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cfg = serde_json::to_string_pretty(&Config {
        base_url: base_url.to_string(),
    })
    .map_err(|e| e.to_string())?;
    std::fs::write(dir.join("config.json"), cfg).map_err(|e| e.to_string())
}

pub fn load_history(app: &AppHandle) -> Vec<HistoryItem> {
    let path = app_dir(app).join("history.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_history(app: &AppHandle, items: &[HistoryItem]) {
    let dir = app_dir(app);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(items) {
        let _ = std::fs::write(dir.join("history.json"), json);
    }
}

pub fn keychain_set(account: &str, value: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE, account).map_err(|e| e.to_string())?;
    entry.set_password(value).map_err(|e| e.to_string())
}

/// 返回 None 表示条目不存在。
pub fn keychain_get(account: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(SERVICE, account).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[allow(dead_code)]
pub fn keychain_delete(account: &str) {
    if let Ok(entry) = keyring::Entry::new(SERVICE, account) {
        let _ = entry.delete_credential();
    }
}
