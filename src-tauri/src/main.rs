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
mod hotkeys;
mod platform_win;
mod protocol;
mod state;
mod tray;
mod vault;
mod windows;

use tauri::{Emitter, Manager, WindowEvent};

use state::AppState;

fn main() {
    init_tracing();

    tauri::Builder::default()
        // 单实例必须是**第一个**插件（Tauri 文档明确要求）。
        //
        // 托盘应用不做单实例的话，用户双击图标会又起一个进程：两个进程抢同一个
        // SQLite 文件、抢同一个全局热键，第二个还会因为热键被「占用」而报冲突——
        // 而占用它的正是自己。
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            let _ = windows::show_main(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // 自启走 HKCU\...\Run（§5.5）。`--autostart` 这个参数让前端知道
        // 「这次是开机拉起来的」，可以直接起在托盘里不抢焦点
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
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
            windows::set_maxbutton_rect,
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
            commands::runtime_report,
            commands::check_update,
            hotkeys::hotkey_list,
            hotkeys::hotkey_rebind,
            tray::autostart_get,
            tray::autostart_set,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // AUMID 要尽早设：通知与任务栏分组都认它，设晚了这一轮不生效
            #[cfg(windows)]
            platform_win::init_process(&handle);

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

            if let Some(main) = app.get_webview_window("main") {
                windows::restore_geometry(&handle);
                #[cfg(windows)]
                {
                    platform_win::attach_titlebar_subclass(&main);
                    platform_win::apply_backdrop(&main, platform_win::Backdrop::Mica);
                }
                // 关掉 = 隐藏而非退出（§5.1）。位置要在这一刻存，
                // 进程退出时再存就来不及了
                let h = handle.clone();
                main.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        windows::save_geometry(&h);
                        if let Some(w) = h.get_webview_window("main") {
                            let _ = w.hide();
                        }
                    }
                });
            }

            // 浮窗常驻但隐藏（§5.3）：唤出要 < 80ms，每次现建 WebView 达不到
            if let Some(quick) = app.get_webview_window("quick") {
                let _ = quick.hide();
                #[cfg(windows)]
                platform_win::apply_backdrop(&quick, platform_win::Backdrop::Acrylic);
            }

            if let Err(e) = tray::build(&handle) {
                // 托盘建不起来不该让应用退出，但要说清楚：没有托盘的话，
                // 点了关闭之后用户就再也找不到它了
                tracing::error!("托盘未能建立（{}），关闭窗口前请先改成真退出", e.code());
            }

            // 注册失败不返回 Err：一个被占用的键位不该让应用起不来。
            // 事实经 hotkey-conflict 事件广播出去，界面自己决定怎么说（§5.4）
            for st in hotkeys::register_all(&handle) {
                if st.enabled && !st.registered {
                    tracing::warn!("热键 {} 未能注册", st.combo);
                }
            }

            // 开机自启拉起来的这一次不抢焦点：用户刚登录进桌面，
            // 这时候弹一个窗口出来是打扰
            if std::env::args().any(|a| a == "--autostart") {
                if let Some(main) = app.get_webview_window("main") {
                    let _ = main.hide();
                }
            }

            let _ = handle.emit("ready", ());
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
