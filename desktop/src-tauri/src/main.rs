//! filelink 桌面客户端：菜单栏常驻，拖入文件即得临时公链。

mod commands;
mod icons;
mod paste;
mod state;
mod storage;
mod tray;
mod uploader;

use std::sync::atomic::Ordering;

use tauri::Manager;
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};
use tokio::sync::mpsc;

fn main() {
    let (queue_tx, queue_rx) = mpsc::unbounded_channel::<(u64, Vec<(u64, String)>)>();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tray::show_at_icon(app);
        }))
        .plugin(tauri_plugin_menubar_dnd::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(move |app| {
            // 菜单栏常驻应用：不显示 Dock 图标
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();

            let st = state::AppState::new(queue_tx);
            let cfg = storage::load_config(&handle);
            *st.base_url.write().unwrap() = cfg.base_url.clone();
            st.configured.store(
                !cfg.base_url.is_empty()
                    && storage::keychain_get(storage::KEY_TOKEN)
                        .map(|v| v.is_some())
                        .unwrap_or(false),
                Ordering::Relaxed,
            );
            *st.history.lock().unwrap() = storage::load_history(&handle);
            st.autostart.store(
                handle.autolaunch().is_enabled().unwrap_or(false),
                Ordering::Relaxed,
            );
            app.manage(st);

            uploader::start_worker(handle.clone(), queue_rx);
            tray::setup(&handle)?;
            tray::start_heartbeat(handle);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::upload_paths,
            commands::pick_and_upload,
            commands::paste_upload,
            commands::retry_upload,
            commands::clear_upload,
            commands::renew,
            commands::revoke,
            commands::copy_link,
            commands::save_config,
            commands::test_connection,
            commands::set_autostart,
            commands::hide_popover,
        ])
        .run(tauri::generate_context!())
        .expect("error while running filelink desktop");
}
