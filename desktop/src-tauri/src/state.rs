//! 状态类型与全局共享状态。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

/// 7 天后从本地历史清除已过期/已撤销的记录。
pub const GREY_DAYS_MS: i64 = 7 * 24 * 3600 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub id: String,
    pub url: String,
    pub filename: String,
    pub mime: String,
    pub size: i64,
    /// epoch 毫秒
    pub created_at: i64,
    /// epoch 毫秒
    pub expires_at: i64,
    /// 撤销时刻；None = 未撤销
    pub revoked_at: Option<i64>,
}

impl HistoryItem {
    pub fn dead_since(&self) -> i64 {
        self.revoked_at.unwrap_or(self.expires_at)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadItem {
    pub key: u64,
    pub filename: String,
    pub size: Option<u64>,
    pub sent: u64,
    /// "uploading" | "done" | "failed"
    pub status: String,
    pub error: Option<String>,
    pub url: Option<String>,
    /// 本地文件路径（仅供重试，不下发前端）
    #[serde(skip)]
    pub path: String,
    /// 完成时刻，用于延迟从列表移除
    #[serde(skip)]
    pub done_at: Option<Instant>,
}

pub struct Batch {
    pub urls: Vec<String>,
    pub any_failed: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub configured: bool,
    pub base_url: String,
    pub autostart: bool,
    pub uploads: Vec<UploadItem>,
    pub history: Vec<HistoryItem>,
}

pub struct AppState {
    pub base_url: RwLock<String>,
    pub configured: AtomicBool,
    pub autostart: AtomicBool,
    pub history: Mutex<Vec<HistoryItem>>,
    pub uploads: Mutex<Vec<UploadItem>>,
    pub batches: Mutex<HashMap<u64, Batch>>,
    /// key -> 已发送字节数（上传任务写入，心跳任务读取同步到 uploads）
    pub progress: Mutex<HashMap<u64, std::sync::Arc<std::sync::atomic::AtomicU64>>>,
    pub next_key: AtomicU64,
    pub next_batch: AtomicU64,
    /// 批次成功后的托盘 ✓ 闪烁截止时刻
    pub success_flash_until: Mutex<Option<Instant>>,
    /// 最近一次因失焦自动隐藏的时刻（防“点图标关不掉”抖动）
    pub blur_hide_at: Mutex<Option<Instant>>,
    pub queue_tx: mpsc::UnboundedSender<(u64, Vec<(u64, String)>)>,
    pub client: reqwest::Client,
}

impl AppState {
    pub fn new(queue_tx: mpsc::UnboundedSender<(u64, Vec<(u64, String)>)>) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .user_agent("filelink-desktop/0.1")
            .build()
            .expect("build http client");
        Self {
            base_url: RwLock::new(String::new()),
            configured: AtomicBool::new(false),
            autostart: AtomicBool::new(false),
            history: Mutex::new(Vec::new()),
            uploads: Mutex::new(Vec::new()),
            batches: Mutex::new(HashMap::new()),
            progress: Mutex::new(HashMap::new()),
            next_key: AtomicU64::new(0),
            next_batch: AtomicU64::new(0),
            success_flash_until: Mutex::new(None),
            blur_hide_at: Mutex::new(None),
            queue_tx,
            client,
        }
    }
}

/// 是否保留这条历史：未过期，或过期/撤销未满 7 天。
pub fn keep(it: &HistoryItem) -> bool {
    let now = now_ms();
    it.dead_since() > now - GREY_DAYS_MS
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn build_snapshot(app: &AppHandle) -> Snapshot {
    let st = app.state::<AppState>();
    let configured = st.configured.load(Ordering::Relaxed);
    let base_url = st.base_url.read().unwrap().clone();
    let autostart = st.autostart.load(Ordering::Relaxed);
    let uploads = st.uploads.lock().unwrap().clone();
    let history = st.history.lock().unwrap().clone();
    Snapshot {
        configured,
        base_url,
        autostart,
        uploads,
        history,
    }
}

pub fn emit_snapshot(app: &AppHandle) {
    let _ = app.emit("state://changed", build_snapshot(app));
}

/// RFC 3339 → epoch 毫秒。
pub fn rfc3339_to_ms(s: &str) -> Result<i64, String> {
    use time::format_description::well_known::Rfc3339;
    let t = time::OffsetDateTime::parse(s, &Rfc3339).map_err(|e| format!("时间解析失败：{e}"))?;
    Ok(t.unix_timestamp() * 1000 + t.millisecond() as i64)
}
