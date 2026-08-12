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

mod commands;
mod error;
mod protocol;
mod state;
mod vault;
mod windows;

use tauri::Manager;

use state::AppState;

fn main() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // 附件走自定义协议在内存里解密（§12.2）。
        //
        // 用异步版：解密几 MB 的图要读盘 + 走一遍 AEAD，压在主线程上会让界面
        // 在插图多的时间线上一顿一顿的。
        .register_asynchronous_uri_scheme_protocol("zhiyan", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                responder.respond(protocol::handle(&app, &request));
            });
        })
        .invoke_handler(tauri::generate_handler![
            windows::app_info,
            windows::window_minimize,
            windows::window_toggle_maximize,
            windows::window_hide,
            windows::quick_toggle,
            commands::list_memos,
            commands::get_memo,
            commands::upsert_memo,
            commands::soft_delete,
            commands::restore,
            commands::purge,
            commands::purge_all,
            commands::set_pinned,
            commands::search,
            commands::backlinks,
            commands::stats,
            commands::put_blob,
            commands::export,
            commands::get_kv,
            commands::set_kv,
            commands::run_gc,
            commands::import_legacy,
            commands::lock_store,
            commands::unlock_store,
            commands::forget_device,
        ])
        .setup(|app| {
            // §9.1：目录名用 ASCII，避免第三方工具处理中文路径出错
            let dir = app
                .path()
                .app_data_dir()
                .map(|d| d.with_file_name("Zhiyan"))
                .unwrap_or_else(|_| std::path::PathBuf::from("Zhiyan"));

            let state = AppState::new(dir);

            // 开库失败**不阻止应用起来**：密钥可能取不到（凭据管理器不可用、
            // 还没登录）。那时该起一个能显示引导界面的空壳，而不是让用户
            // 面对一个闪一下就没了的进程——那种失败最难报告。
            if let Err(e) = state.unlock() {
                tracing::error!("本地库未能打开（{}），以未解锁状态启动", e.code());
            }
            app.manage(state);

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
