//! 窗口创建、定位、置前台（§4.5 `windows.rs`）。
//!
//! 数据相关的命令在 `db/`（阶段三），这里只管窗口。先把这几条接上是为了让
//! IPC 边界从第一天就是真的：前端的 `store.js` / `platform.js` 有真实的调用
//! 路径可走，而不是等到最后才发现序列化形状对不上。

use serde::Serialize;
use tauri::{Manager, Window};

use crate::error::{AppError, AppResult};

/// 应用自身的信息。前端用它显示版本号，也用来判断「我确实跑在 Tauri 里」。
#[derive(Debug, Serialize)]
pub struct AppInfo {
    pub version: &'static str,
    pub platform: &'static str,
    /// 本地库是否真的加密了。
    ///
    /// 前端在设置页里如实显示它。**不能假设发布构建一定开了 sqlcipher 特性**——
    /// 显示一个假的「已加密」比不显示更糟。
    pub encrypted_store: bool,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        encrypted_store: cfg!(feature = "sqlcipher"),
    }
}

#[tauri::command]
pub fn window_minimize(window: Window) -> AppResult<()> {
    window
        .minimize()
        .map_err(|e| AppError::Window(e.to_string()))
}

#[tauri::command]
pub fn window_toggle_maximize(window: Window) -> AppResult<bool> {
    let maximized = window
        .is_maximized()
        .map_err(|e| AppError::Window(e.to_string()))?;
    if maximized {
        window.unmaximize()
    } else {
        window.maximize()
    }
    .map_err(|e| AppError::Window(e.to_string()))?;
    Ok(!maximized)
}

/// 关闭主窗口 = 隐藏而非退出（§5.1，设置项可改为真退出）。
///
/// 托盘还没接（阶段五），在它到位之前只能靠任务栏或 `Ctrl+Alt+Z` 把窗口叫回来。
#[tauri::command]
pub fn window_hide(window: Window) -> AppResult<()> {
    window.hide().map_err(|e| AppError::Window(e.to_string()))
}

/// 唤起/收起速记浮窗（§5.3）。
///
/// **常驻但隐藏**（`hide()` 非 `close()`），保证唤出 < 80ms。代价约 30–40MB。
///
/// 还差一件事：**前台焦点抢占**。Windows 的前台锁定会让非前台进程的
/// `SetForegroundWindow` 静默失败，只闪任务栏。热键触发时系统通常短暂授权，
/// 但不保证——要靠 `AttachThreadInput` 那套绕过去，属于 `platform_win.rs`
/// 的活（阶段五）。
#[tauri::command]
pub fn quick_toggle(app: tauri::AppHandle) -> AppResult<bool> {
    let quick = app
        .get_webview_window("quick")
        .ok_or(AppError::NoSuchWindow("quick"))?;

    let visible = quick
        .is_visible()
        .map_err(|e| AppError::Window(e.to_string()))?;
    if visible {
        quick.hide().map_err(|e| AppError::Window(e.to_string()))?;
        Ok(false)
    } else {
        // 顺序要紧：先摆位再显示，否则会看到窗口从上一次的位置跳过来
        position_on_cursor_monitor(&quick)?;
        quick.show().map_err(|e| AppError::Window(e.to_string()))?;
        quick
            .set_focus()
            .map_err(|e| AppError::Window(e.to_string()))?;
        Ok(true)
    }
}

/// 把浮窗摆到**鼠标当前所在的显示器**上，居中偏上（工作区高度 20%）——§5.3 ②。
///
/// 记忆位置时要存「相对某显示器的偏移」而非绝对坐标，否则拔掉外接屏后窗口
/// 会跑到屏幕外。这里每次唤出都按当前光标所在屏重算，连偏移都不记。
fn position_on_cursor_monitor(window: &tauri::WebviewWindow) -> AppResult<()> {
    let monitor = window
        .cursor_position()
        .ok()
        .and_then(|p| window.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten());

    let Some(monitor) = monitor else {
        return Ok(()); // 取不到显示器信息就用系统默认位置，不至于开不出来
    };

    let scale = monitor.scale_factor();
    let area = monitor.size().to_logical::<f64>(scale);
    let origin = monitor.position().to_logical::<f64>(scale);
    let size = window
        .outer_size()
        .map_err(|e| AppError::Window(e.to_string()))?
        .to_logical::<f64>(scale);

    window
        .set_position(tauri::LogicalPosition::new(
            origin.x + (area.width - size.width) / 2.0,
            origin.y + area.height * 0.2,
        ))
        .map_err(|e| AppError::Window(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 应用信息如实报告加密状态() {
        let info = app_info();
        assert_eq!(info.encrypted_store, cfg!(feature = "sqlcipher"));
        assert!(!info.version.is_empty());
    }

    #[test]
    fn 应用信息可序列化() {
        let json = serde_json::to_value(app_info()).unwrap();
        assert!(json["version"].is_string());
        assert!(json["encryptedStore"].is_boolean() || json["encrypted_store"].is_boolean());
    }
}
