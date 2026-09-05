//! Tauri 命令：前端唯一可调用面。

use std::sync::atomic::Ordering;

use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_clipboard_manager::ClipboardExt as _;

use crate::state::{self, AppState, Snapshot};
use crate::{storage, tray, uploader};

#[tauri::command]
pub fn get_state(app: AppHandle) -> Snapshot {
    state::build_snapshot(&app)
}

#[tauri::command]
pub fn upload_paths(app: AppHandle, paths: Vec<String>) {
    if !paths.is_empty() {
        uploader::enqueue(&app, paths);
    }
}

#[tauri::command]
pub async fn pick_and_upload(app: AppHandle) -> Result<(), String> {
    if let Some(files) = rfd::AsyncFileDialog::new().pick_files().await {
        let paths: Vec<String> = files
            .into_iter()
            .map(|f| f.path().to_string_lossy().into_owned())
            .collect();
        if !paths.is_empty() {
            uploader::enqueue(&app, paths);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn paste_upload(app: AppHandle) -> Result<(), String> {
    let paths = crate::paste::clipboard_file_paths();
    if paths.is_empty() {
        return Err("剪贴板中没有文件".into());
    }
    uploader::enqueue(&app, paths);
    Ok(())
}

#[tauri::command]
pub fn retry_upload(app: AppHandle, key: u64) {
    let st = app.state::<AppState>();
    let path = {
        let mut uploads = st.uploads.lock().unwrap();
        match uploads
            .iter()
            .position(|i| i.key == key && i.status == "failed")
        {
            Some(idx) => uploads.remove(idx).path,
            None => return,
        }
    };
    uploader::enqueue(&app, vec![path]);
}

#[tauri::command]
pub fn clear_upload(app: AppHandle, key: u64) {
    let st = app.state::<AppState>();
    st.uploads.lock().unwrap().retain(|i| i.key != key);
    state::emit_snapshot(&app);
}

#[tauri::command]
pub async fn renew(app: AppHandle, id: String) -> Result<(), String> {
    let st = app.state::<AppState>();
    let base = {
        let b = st.base_url.read().unwrap();
        b.trim_end_matches('/').to_string()
    };
    if base.is_empty() {
        return Err("未配置服务地址".into());
    }
    let secret = storage::keychain_get(&storage::secret_account(&id))?
        .ok_or("缺少续期密钥（该记录可能来自旧设备）")?;

    let resp = st
        .client
        .post(format!("{base}/f/{id}/renew"))
        .header("X-Renewal-Secret", secret)
        .send()
        .await
        .map_err(|e| format!("网络错误：{e}"))?;

    match resp.status().as_u16() {
        200 => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct R {
                expires_at: String,
            }
            let r: R = resp.json().await.map_err(|e| format!("响应解析失败：{e}"))?;
            let expires = state::rfc3339_to_ms(&r.expires_at)?;
            {
                let mut h = st.history.lock().unwrap();
                if let Some(it) = h.iter_mut().find(|i| i.id == id) {
                    it.expires_at = expires;
                    it.revoked_at = None;
                }
            }
            storage::save_history(&app, &st.history.lock().unwrap());
            state::emit_snapshot(&app);
            Ok(())
        }
        410 => Err("链接已过期，无法续期".into()),
        403 => Err("续期密钥无效".into()),
        404 => Err("链接不存在".into()),
        s => Err(format!("服务返回 {s}")),
    }
}

#[tauri::command]
pub async fn revoke(app: AppHandle, id: String) -> Result<(), String> {
    let st = app.state::<AppState>();
    let base = {
        let b = st.base_url.read().unwrap();
        b.trim_end_matches('/').to_string()
    };
    if base.is_empty() {
        return Err("未配置服务地址".into());
    }
    let secret = storage::keychain_get(&storage::secret_account(&id))?
        .ok_or("缺少续期密钥（该记录可能来自旧设备）")?;

    let resp = st
        .client
        .delete(format!("{base}/f/{id}"))
        .header("X-Renewal-Secret", secret)
        .send()
        .await
        .map_err(|e| format!("网络错误：{e}"))?;

    match resp.status().as_u16() {
        // 已撤销/已过期（410）也标记本地状态，保证一致
        200 | 410 => {
            let now = state::now_ms();
            {
                let mut h = st.history.lock().unwrap();
                if let Some(it) = h.iter_mut().find(|i| i.id == id) {
                    it.revoked_at.get_or_insert(now);
                }
            }
            storage::save_history(&app, &st.history.lock().unwrap());
            state::emit_snapshot(&app);
            Ok(())
        }
        403 => Err("续期密钥无效".into()),
        404 => Err("服务端查无此链接".into()),
        405 => Err("服务端版本过旧，不支持撤销，请先升级 filelink 服务".into()),
        s => Err(format!("服务返回 {s}")),
    }
}

#[tauri::command]
pub fn copy_link(app: AppHandle, url: String) -> Result<(), String> {
    app.clipboard().write_text(url).map_err(|e| e.to_string())
}

fn normalize_base(input: &str) -> Result<String, String> {
    let s = input.trim().trim_end_matches('/');
    if s.is_empty() {
        return Err("请填写服务地址".into());
    }
    if !s.starts_with("http://") && !s.starts_with("https://") {
        return Err("服务地址需以 http:// 或 https:// 开头".into());
    }
    Ok(s.to_string())
}

#[tauri::command]
pub async fn save_config(
    app: AppHandle,
    base_url: String,
    token: Option<String>,
) -> Result<(), String> {
    let base = normalize_base(&base_url)?;
    let st = app.state::<AppState>();
    let was_configured = st.configured.load(Ordering::Relaxed);

    if let Some(t) = token.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        storage::keychain_set(storage::KEY_TOKEN, t)?;
    }
    storage::save_config(&app, &base)?;
    *st.base_url.write().unwrap() = base;

    let has_token = storage::keychain_get(storage::KEY_TOKEN)?.is_some();
    let configured = has_token;
    st.configured.store(configured, Ordering::Relaxed);

    // 首次配置成功后按设计默认开启开机自启
    if !was_configured && configured && app.autolaunch().enable().is_ok() {
        st.autostart.store(true, Ordering::Relaxed);
    }
    tray::refresh_menu(&app);
    state::emit_snapshot(&app);
    Ok(())
}

/// 两段探测：GET /f/__probe__ 应为 404/410（确认是 filelink 服务）；
/// 带 token 的空上传应为 400（令牌有效，仅缺文件名）。
#[tauri::command]
pub async fn test_connection(
    app: AppHandle,
    base_url: String,
    token: Option<String>,
) -> Result<(), String> {
    let base = normalize_base(&base_url)?;
    let st = app.state::<AppState>();

    let probe = st
        .client
        .get(format!("{base}/f/__probe__"))
        .send()
        .await
        .map_err(|e| format!("无法连接服务：{e}"))?;
    let s = probe.status().as_u16();
    if s != 404 && s != 410 {
        return Err(format!("服务返回 {s}，这不像一个 filelink 地址"));
    }

    let token = match token.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => t.to_string(),
        None => storage::keychain_get(storage::KEY_TOKEN)?
            .ok_or("请填写上传令牌")?,
    };
    let r = st
        .client
        .post(format!("{base}/api/upload"))
        .header("Authorization", format!("Bearer {token}"))
        .header("X-Filename", "__probe__")
        .body(String::new())
        .send()
        .await
        .map_err(|e| format!("网络错误：{e}"))?;
    match r.status().as_u16() {
        // 服务端先校验令牌再校验文件名：400 = 令牌有效
        400 => Ok(()),
        401 => Err("上传令牌无效".into()),
        413 => Ok(()), // 超限也说明令牌已通过
        s => Err(format!("服务返回 {s}")),
    }
}

#[tauri::command]
pub async fn set_autostart(app: AppHandle, enable: bool) -> Result<bool, String> {
    let m = app.autolaunch();
    if enable {
        m.enable().map_err(|e| e.to_string())?;
    } else {
        m.disable().map_err(|e| e.to_string())?;
    }
    let on = m.is_enabled().map_err(|e| e.to_string())?;
    app.state::<AppState>()
        .autostart
        .store(on, Ordering::Relaxed);
    tray::refresh_menu(&app);
    state::emit_snapshot(&app);
    Ok(on)
}

#[tauri::command]
pub fn hide_popover(app: AppHandle) {
    if let Some(win) = app.get_webview_window("popover") {
        let _ = win.hide();
    }
}
