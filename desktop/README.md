# filelink desktop

filelink 的 macOS 桌面客户端：常驻菜单栏，文件拖到状态栏图标即得临时公链，链接自动进剪贴板。

技术栈：[Tauri 2](https://v2.tauri.app/) + 无框架单页 UI（与仓库 `web/` 同风格），macOS 先行、架构预留跨平台。

## 功能

- **拖到菜单栏图标上传**：拖放落下时自动弹出面板展示进度，成功即自动复制链接
- **popover 面板**：拖放区（拖入 / 点击选择 / ⌘V 粘贴 Finder 文件）、上传进度条、历史列表
- **链接管理**：剩余时间倒计时（<10 分钟高亮）、一键 +TTL 续期、再次复制、**撤销**（凭续期密钥调用服务端 `DELETE /f/{id}`，需服务端 ≥ 含此接口的版本）
- **历史本地留存**：过期/撤销后灰显 7 天再自动清除；续期密钥存 Keychain，历史存 `~/Library/Application Support/filelink/`
- **多文件**：逐个独立上传，全部完成后链接按行拼进剪贴板；失败条目保留、可重试
- **反馈全在面板内**：无系统通知；托盘图标状态——上传中转圈、成功短暂 ✓、失败警示
- 开机自启（首次配置成功后默认开启，可在设置或右键菜单关闭）
- 单实例：重复启动会唤起已有实例的面板

## 构建

依赖：Rust（rustup）、Node（仅用于 Tauri CLI）。

```bash
# 调试运行（直接出菜单栏图标）
cd desktop/src-tauri && cargo run

# 生成 .app（未签名，本机自用无 Gatekeeper 问题）
npx @tauri-apps/cli build     # 在 desktop/ 目录下执行
# 产物：desktop/src-tauri/target/release/bundle/macos/filelink.app
```

图标由 `gen_icons.py` 生成（托盘模板图标 + 应用图标源图），改完重跑：

```bash
python3 gen_icons.py
npx @tauri-apps/cli icon src-tauri/icons/appicon.png -o src-tauri/icons
```

## 服务端要求

上传 / 下载 / 续期兼容任意版本 filelink 服务端；**撤销**功能需要服务端支持 `DELETE /f/{id}`（随本仓库同版本提供）。对旧服务端撤销时会提示"服务端版本过旧"。

## 代码结构

```
desktop/
├── ui/                  # popover 前端（无框架单页）
│   ├── index.html
│   ├── style.css
│   └── app.js
├── gen_icons.py         # 托盘/应用图标生成脚本
└── src-tauri/
    ├── tauri.conf.json  # 无边框置顶窗口（popover）、无 Dock 图标
    ├── capabilities/    # 权限：core + menubar-dnd
    └── src/
        ├── main.rs      # 装配：插件、状态加载、命令注册
        ├── state.rs     # 共享状态 / 快照事件（state://changed）
        ├── storage.rs   # config/history JSON + Keychain
        ├── uploader.rs  # 串行上传队列 + 进度 + 批次剪贴板聚合
        ├── tray.rs      # 状态栏交互、popover 开合、心跳
        ├── commands.rs  # 前端命令面
        ├── icons.rs     # 托盘图标状态机素材
        └── paste.rs     # macOS 剪贴板文件读取（objc2）
```

设计决策（形态、边界、服务端配套改动）经逐项确认，见仓库根 README 与 `docs/API.md` 的 DELETE 接口部分。
