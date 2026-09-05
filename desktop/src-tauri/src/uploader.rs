//! 上传管线：串行队列、进度回传、历史落盘、批次完成后的剪贴板聚合。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use reqwest::header::AUTHORIZATION;
use serde::Deserialize;
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::io::ReaderStream;

use crate::state::{self, AppState, Batch, HistoryItem, UploadItem};
use crate::storage;

pub fn start_worker(app: AppHandle, mut rx: UnboundedReceiver<(u64, Vec<(u64, String)>)>) {
    tauri::async_runtime::spawn(async move {
        while let Some((batch_id, items)) = rx.recv().await {
            for (key, path) in items {
                process_one(&app, batch_id, key, path).await;
            }
            finalize_batch(&app, batch_id).await;
        }
    });
}

/// 建批次条目并入队。paths 允许 file:// 前缀。
pub fn enqueue(app: &AppHandle, paths: Vec<String>) {
    let st = app.state::<AppState>();
    let batch_id = st.next_batch.fetch_add(1, Ordering::Relaxed) + 1;
    let mut pairs: Vec<(u64, String)> = Vec::with_capacity(paths.len());
    {
        let mut uploads = st.uploads.lock().unwrap();
        let mut batches = st.batches.lock().unwrap();
        batches.insert(
            batch_id,
            Batch {
                urls: Vec::new(),
                any_failed: false,
            },
        );
        for raw in &paths {
            let path = normalize_path(raw);
            let key = st.next_key.fetch_add(1, Ordering::Relaxed) + 1;
            uploads.push(UploadItem {
                key,
                filename: basename(&path),
                size: None,
                sent: 0,
                status: "uploading".into(),
                error: None,
                url: None,
                path: path.clone(),
                done_at: None,
            });
            pairs.push((key, path));
        }
    }
    let _ = st.queue_tx.send((batch_id, pairs));
    state::emit_snapshot(app);
}

async fn process_one(app: &AppHandle, batch_id: u64, key: u64, path: String) {
    let st = app.state::<AppState>();
    let filename = basename(&path);
    let base = st.base_url.read().unwrap().trim_end_matches('/').to_string();

    let size = tokio::fs::metadata(&path).await.ok().map(|m| m.len());
    if let Some(it) = st.uploads.lock().unwrap().iter_mut().find(|i| i.key == key) {
        it.size = size;
    }

    let result = run_upload(&st, &base, &filename, &path, key).await;
    match result {
        Ok(resp) => {
            let _ = storage::keychain_set(&storage::secret_account(&resp.id), &resp.secret);
            let item = resp.to_history_item();
            st.history.lock().unwrap().insert(0, item);
            storage::save_history(app, &st.history.lock().unwrap());
            if let Some(it) = st.uploads.lock().unwrap().iter_mut().find(|i| i.key == key) {
                it.status = "done".into();
                it.url = Some(resp.url.clone());
                it.error = None;
                it.done_at = Some(Instant::now());
            }
            if let Some(b) = st.batches.lock().unwrap().get_mut(&batch_id) {
                b.urls.push(resp.url.clone());
            }
            st.progress.lock().unwrap().remove(&key);
            log::info!("uploaded {} as {}", filename, resp.id);
        }
        Err(msg) => {
            if let Some(it) = st.uploads.lock().unwrap().iter_mut().find(|i| i.key == key) {
                it.status = "failed".into();
                it.error = Some(msg.clone());
            }
            if let Some(b) = st.batches.lock().unwrap().get_mut(&batch_id) {
                b.any_failed = true;
            }
            st.progress.lock().unwrap().remove(&key);
            log::warn!("upload {} failed: {}", filename, msg);
        }
    }
    state::emit_snapshot(app);
}

async fn run_upload(
    st: &AppState,
    base: &str,
    filename: &str,
    path: &str,
    key: u64,
) -> Result<UploadResponse, String> {
    if base.is_empty() {
        return Err("未配置服务地址".into());
    }
    let token = storage::keychain_get(storage::KEY_TOKEN)?.ok_or("未配置上传令牌")?;

    let file = tokio::fs::File::open(path).await.map_err(|e| format!("无法读取文件：{e}"))?;
    let sent = Arc::new(AtomicU64::new(0));
    st.progress.lock().unwrap().insert(key, sent.clone());

    let counter = sent.clone();
    let stream = ReaderStream::new(file).map(move |chunk| {
        if let Ok(bytes) = &chunk {
            counter.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        }
        chunk
    });

    let encoded = utf8_percent_encode(filename, NON_ALPHANUMERIC).to_string();
    let resp = st
        .client
        .post(format!("{base}/api/upload"))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header("X-Filename", encoded)
        .body(reqwest::Body::wrap_stream(stream))
        .send()
        .await
        .map_err(|e| format!("网络错误：{e}"))?;

    let status = resp.status().as_u16();
    let body = resp.bytes().await.unwrap_or_default();
    match status {
        200 => serde_json::from_slice::<UploadResponse>(&body)
            .map_err(|e| format!("响应解析失败：{e}")),
        401 => Err("上传令牌无效（401），请到设置里检查".into()),
        413 => Err("文件超过服务端大小上限（413）".into()),
        s => Err(format!(
            "服务端错误（{s}）：{}",
            String::from_utf8_lossy(&body)
                .chars()
                .take(120)
                .collect::<String>()
        )),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadResponse {
    id: String,
    url: String,
    filename: String,
    mime: String,
    size: i64,
    expires_at: String,
    secret: String,
}

impl UploadResponse {
    fn to_history_item(&self) -> HistoryItem {
        HistoryItem {
            id: self.id.clone(),
            url: self.url.clone(),
            filename: self.filename.clone(),
            mime: self.mime.clone(),
            size: self.size,
            created_at: state::now_ms(),
            expires_at: state::rfc3339_to_ms(&self.expires_at).unwrap_or(state::now_ms()),
            revoked_at: None,
        }
    }
}

async fn finalize_batch(app: &AppHandle, batch_id: u64) {
    let st = app.state::<AppState>();
    let Some(batch) = st.batches.lock().unwrap().remove(&batch_id) else {
        return;
    };
    if !batch.urls.is_empty() {
        // 设计共识：批次结束时把成功的链接按行拼进剪贴板（单文件即单链接）
        use tauri_plugin_clipboard_manager::ClipboardExt;
        let _ = app.clipboard().write_text(batch.urls.join("\n"));
        *st.success_flash_until.lock().unwrap() =
            Some(Instant::now() + Duration::from_millis(1500));
    }
    state::emit_snapshot(app);

    // 完成条目停留 4 秒后从列表移除；失败条目保留等人处理
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(4000)).await;
        let st = handle.state::<AppState>();
        st.uploads.lock().unwrap().retain(|it| it.status != "done");
        state::emit_snapshot(&handle);
    });
}

/// file:// URL → 本地路径；其余原样。
pub fn normalize_path(raw: &str) -> String {
    let s = raw.trim();
    if let Some(rest) = s.strip_prefix("file://") {
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        percent_encoding::percent_decode_str(rest)
            .decode_utf8_lossy()
            .into_owned()
    } else {
        s.to_string()
    }
}

pub fn basename(path: &str) -> String {
    let p = path.trim_end_matches('/');
    p.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| p.to_string())
}
