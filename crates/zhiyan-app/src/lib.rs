//! 知言的 Tauri 应用壳。
//!
//! ## 这一层的职责边界
//!
//! Tauri 方案里 UI 跑在 WebView 里，前后端之间隔着一道 **IPC 边界**。这道边界决定了
//! 什么该放哪边：
//!
//! | 归 Rust | 归前端 |
//! |---|---|
//! | SQLCipher 存储、加密、同步 | DOM、样式、动效 |
//! | 统计聚合、FTS 检索 | Markdown 渲染 |
//! | 附件的加解密与落盘 | 文本输入（`<textarea>` 白送输入法） |
//!
//! **聚合必须在 Rust 侧做**，不能把全量数据搬到前端再 `Array.filter`——JSON 序列化
//! 一万条片语的开销足以让界面卡住。这也是原型里「前端 JS 全量过滤搜索」必须换成
//! FTS5 的原因。
//!
//! 本 crate 只做装配与命令分发，领域逻辑一律在 `zhiyan-core` / `zhiyan-store` 里。

mod commands;
mod error;
mod state;

pub use error::{AppError, AppResult};

use tauri::Manager;

/// 组装并运行应用。
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::window_minimize,
            commands::window_toggle_maximize,
            commands::window_hide,
            commands::quick_toggle,
        ])
        .setup(|app| {
            // 速记浮窗常驻但隐藏（§7.4）：唤出要 < 80ms，每次现建 WebView 达不到。
            if let Some(quick) = app.get_webview_window("quick") {
                let _ = quick.hide();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri 应用启动失败");
}
