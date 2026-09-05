//! 状态栏图标、右键菜单、popover 开合与全局心跳（进度同步/图标动画/历史修剪）。

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Listener, Manager, WindowEvent};
use tauri_plugin_menubar_dnd::{self as menubar, MenuItemDef};

use crate::state::{self, AppState};
use crate::{icons, storage, uploader};

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let win = app.get_webview_window("popover").expect("popover window");
    let _ = win.set_visible_on_all_workspaces(true);

    let _ = icons::IconKind::Idle.apply();
    let _ = menubar::set_tooltip("filelink — 拖入文件生成临时链接");
    refresh_menu(app);

    // popover 失焦自动收起
    {
        let handle = app.clone();
        win.on_window_event(move |event| {
            if let WindowEvent::Focused(false) = event {
                if let Some(w) = handle.get_webview_window("popover") {
                    if w.is_visible().unwrap_or(false) {
                        let _ = w.hide();
                        *handle
                            .state::<AppState>()
                            .blur_hide_at
                            .lock()
                            .unwrap() = Some(Instant::now());
                    }
                }
            }
        });
    }

    let handle = app.clone();
    app.listen("menubar-dnd://click", move |event| {
        if event.payload().contains("left") {
            toggle_popover(&handle);
        }
    });

    let handle = app.clone();
    app.listen("menubar-dnd://menu-item", move |event| {
        let Ok(id) = serde_json::from_str::<String>(event.payload()) else {
            return;
        };
        match id.as_str() {
            "settings" => {
                show_settings(&handle);
            }
            "autostart" => {
                toggle_autostart(&handle);
            }
            "quit" => {
                handle.exit(0);
            }
            _ => {}
        }
    });

    let handle = app.clone();
    app.listen("menubar-dnd://drop-files", move |event| {
        let Ok(paths) = serde_json::from_str::<Vec<String>>(event.payload()) else {
            return;
        };
        let paths: Vec<String> = paths.iter().map(|p| uploader::normalize_path(p)).collect();
        if !paths.is_empty() {
            uploader::enqueue(&handle, paths);
            show_at_icon(&handle);
        }
    });

    Ok(())
}

fn menu_items(app: &AppHandle) -> Vec<MenuItemDef> {
    let st = app.state::<AppState>();
    vec![
        MenuItemDef::leaf("settings", "设置…"),
        MenuItemDef::checked_leaf("autostart", "开机自启", st.autostart.load(Ordering::Relaxed)),
        MenuItemDef::separator(),
        MenuItemDef::leaf("quit", "退出 filelink"),
    ]
}

pub fn refresh_menu(app: &AppHandle) {
    let _ = menubar::set_menu(menu_items(app));
}

fn toggle_autostart(app: &AppHandle) {
    use tauri_plugin_autostart::ManagerExt;
    let st = app.state::<AppState>();
    let enable = !st.autostart.load(Ordering::Relaxed);
    let m = app.autolaunch();
    let res = if enable { m.enable() } else { m.disable() };
    if res.is_ok() {
        let on = m.is_enabled().unwrap_or(enable);
        st.autostart.store(on, Ordering::Relaxed);
    }
    refresh_menu(app);
}

pub fn toggle_popover(app: &AppHandle) {
    let Some(win) = app.get_webview_window("popover") else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
        return;
    }
    // 刚因失焦被这次点击收起时，不要立刻再弹出
    let recently_hidden = {
        let st = app.state::<AppState>();
        let v = st
            .blur_hide_at
            .lock()
            .unwrap()
            .map(|t| t.elapsed() < Duration::from_millis(300))
            .unwrap_or(false);
        v
    };
    if recently_hidden {
        return;
    }
    show_at_icon(app);
}

pub fn show_at_icon(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("popover") {
        menubar::position_window_under_status_item(&win);
        let _ = win.show();
        let _ = win.set_focus();
    }
}

fn show_settings(app: &AppHandle) {
    show_at_icon(app);
    let _ = app.emit("ui://view", "settings");
}

/// 全局心跳：进度同步 + 状态推送、托盘图标动画、历史修剪。
pub fn start_heartbeat(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut tick: u64 = 0;
        let mut last_icon = String::new();
        loop {
            tokio::time::sleep(Duration::from_millis(140)).await;
            tick += 1;
            let st = app.state::<AppState>();

            // 1) 上传进度回写 + 节流推送
            let uploading = {
                let progress = st.progress.lock().unwrap();
                if progress.is_empty() {
                    false
                } else {
                    let mut uploads = st.uploads.lock().unwrap();
                    for (key, atom) in progress.iter() {
                        if let Some(it) = uploads.iter_mut().find(|i| i.key == *key) {
                            it.sent = atom.load(Ordering::Relaxed);
                        }
                    }
                    true
                }
            };
            if uploading {
                state::emit_snapshot(&app);
            }

            // 2) 每 ~30 秒修剪一次过期历史
            if tick % 224 == 0 {
                let changed = {
                    let mut h = st.history.lock().unwrap();
                    let before = h.len();
                    h.retain(state::keep);
                    h.len() != before
                };
                if changed {
                    storage::save_history(&app, &st.history.lock().unwrap());
                    state::emit_snapshot(&app);
                }
            }

            // 3) 托盘图标状态机
            let kind = if !st.progress.lock().unwrap().is_empty() {
                icons::IconKind::Spin((tick % 8) as u8)
            } else if matches!(
                *st.success_flash_until.lock().unwrap(),
                Some(t) if Instant::now() < t
            ) {
                icons::IconKind::Check
            } else if st
                .uploads
                .lock()
                .unwrap()
                .iter()
                .any(|i| i.status == "failed")
            {
                icons::IconKind::Warn
            } else {
                icons::IconKind::Idle
            };
            let tag = kind.tag();
            if last_icon != tag {
                if kind.apply().is_ok() {
                    last_icon = tag;
                }
            }
        }
    });
}
