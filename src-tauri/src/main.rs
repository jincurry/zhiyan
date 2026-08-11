//! 知言的入口 + Builder 组装（§4.5）。
//!
//! ## 分层（§4.4）
//!
//! ```text
//! Rust 主进程 ── 窗口 / 热键 / 托盘 / 加密 / 数据 / 同步 / 平台
//!      ▲
//!      │ IPC (invoke / event) + zhiyan:// 协议
//!      ▼
//! WebView2 渲染层（无框架 ESM）── main window / quick window
//! ```
//!
//! **铁律**：所有持久化只经 Rust 侧。前端不碰文件系统，加密、同步、迁移只有一处实现。
//!
//! `windows_subsystem = "windows"` 让发布构建不弹控制台；debug 下保留，
//! 否则 `tracing` 的输出没地方看。
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod error;
mod state;
mod windows;

use tauri::Manager;

fn main() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            windows::app_info,
            windows::window_minimize,
            windows::window_toggle_maximize,
            windows::window_hide,
            windows::quick_toggle,
        ])
        .setup(|app| {
            // 浮窗常驻但隐藏（§5.3）：唤出要 < 80ms，每次现建 WebView 达不到
            if let Some(quick) = app.get_webview_window("quick") {
                let _ = quick.hide();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri 应用启动失败");
}

/// 日志。
///
/// **脱敏 layer 还没接**（§13）。在它到位之前默认级别压到 `warn`，
/// 免得开发期顺手加的 `debug!` 把正文写进磁盘。
fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("ZHIYAN_LOG").unwrap_or_else(|_| EnvFilter::new("warn"));
    fmt().with_env_filter(filter).with_target(false).init();
}
