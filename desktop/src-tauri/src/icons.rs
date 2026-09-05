//! 托盘模板图标（36px @2x，显示为 18pt）。

use tauri_plugin_menubar_dnd as menubar;

pub enum IconKind {
    Idle,
    /// 8 帧旋转
    Spin(u8),
    Check,
    Warn,
}

impl IconKind {
    /// 用于变更检测的标签（Spin 每帧都算变更）。
    pub fn tag(&self) -> String {
        match self {
            IconKind::Idle => "idle".into(),
            IconKind::Spin(f) => format!("spin-{f}"),
            IconKind::Check => "check".into(),
            IconKind::Warn => "warn".into(),
        }
    }

    fn bytes(&self) -> &'static [u8] {
        match self {
            IconKind::Idle => include_bytes!("../icons/tray-idle.png"),
            IconKind::Spin(0) => include_bytes!("../icons/tray-spin-0.png"),
            IconKind::Spin(1) => include_bytes!("../icons/tray-spin-1.png"),
            IconKind::Spin(2) => include_bytes!("../icons/tray-spin-2.png"),
            IconKind::Spin(3) => include_bytes!("../icons/tray-spin-3.png"),
            IconKind::Spin(4) => include_bytes!("../icons/tray-spin-4.png"),
            IconKind::Spin(5) => include_bytes!("../icons/tray-spin-5.png"),
            IconKind::Spin(6) => include_bytes!("../icons/tray-spin-6.png"),
            IconKind::Spin(7) => include_bytes!("../icons/tray-spin-7.png"),
            IconKind::Spin(_) => unreachable!(),
            IconKind::Check => include_bytes!("../icons/tray-check.png"),
            IconKind::Warn => include_bytes!("../icons/tray-warn.png"),
        }
    }

    pub fn apply(&self) -> Result<(), String> {
        menubar::set_icon(self.bytes().to_vec(), true)
    }
}
