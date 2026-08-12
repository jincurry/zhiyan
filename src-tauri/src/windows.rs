//! 窗口创建、定位、置前台（§4.5 `windows.rs`）。
//!
//! 数据相关的命令在 `db/`（阶段三），这里只管窗口。先把这几条接上是为了让
//! IPC 边界从第一天就是真的：前端的 `store.js` / `platform.js` 有真实的调用
//! 路径可走，而不是等到最后才发现序列化形状对不上。

use serde::Serialize;
use tauri::{Manager, State, Window};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// 应用自身的信息。前端用它显示版本号，也用来判断「我确实跑在 Tauri 里」。
///
/// `camelCase` 是必须的：`mock.js` 返回的是 `encryptedStore`，两边对不上的话
/// 设置页会读到 `undefined`，然后**如实地**把「未加密」显示成「已加密」——
/// 因为 `undefined` 在那个位置会走进 falsy 分支，而 falsy 分支的文案是安全的那一档。
/// 这种错法不会报错，只会一直说谎。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: &'static str,
    pub platform: &'static str,
    /// 本地库是否真的加密了。
    ///
    /// 前端在设置页里如实显示它。**不能假设发布构建一定开了 sqlcipher 特性**——
    /// 显示一个假的「已加密」比不显示更糟。
    ///
    /// 这是**运行时探测**出来的（`PRAGMA cipher_version`），不是 `cfg!` 猜的：
    /// `PRAGMA key` 在普通 SQLite 上会被静默忽略，库照开、数据照写，只是全是明文。
    pub encrypted_store: bool,
    /// 库是否已打开。密钥取不到时应用照样起得来，界面该显示解锁引导。
    pub unlocked: bool,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        encrypted_store: state.is_encrypted(),
        unlocked: state.is_unlocked(),
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
#[tauri::command]
pub fn quick_toggle(app: tauri::AppHandle) -> AppResult<bool> {
    toggle_quick(&app)
}

/// 同上，但可以从热键与托盘里调。
pub fn toggle_quick(app: &tauri::AppHandle) -> AppResult<bool> {
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
        focus_hard(&quick)?;
        Ok(true)
    }
}

/// 显示并聚焦主窗口。托盘与热键都用它。
pub fn show_main(app: &tauri::AppHandle) -> AppResult<()> {
    let main = app
        .get_webview_window("main")
        .ok_or(AppError::NoSuchWindow("main"))?;
    // 最小化状态下 show() 不会还原，窗口还在任务栏里躺着
    if main.is_minimized().unwrap_or(false) {
        main.unminimize()
            .map_err(|e| AppError::Window(e.to_string()))?;
    }
    main.show().map_err(|e| AppError::Window(e.to_string()))?;
    focus_hard(&main)
}

/// 切换主窗口显隐。托盘左键单击走这条。
pub fn toggle_main(app: &tauri::AppHandle) -> AppResult<bool> {
    let main = app
        .get_webview_window("main")
        .ok_or(AppError::NoSuchWindow("main"))?;
    let visible = main.is_visible().unwrap_or(false);
    let focused = main.is_focused().unwrap_or(false);

    // 可见但没聚焦时，用户的意思几乎总是「拿到前面来」，而不是「收起去」。
    // 只看 is_visible 的话，点托盘会把一个被别的窗口压住的知言直接藏掉
    if visible && focused {
        main.hide().map_err(|e| AppError::Window(e.to_string()))?;
        Ok(false)
    } else {
        show_main(app)?;
        Ok(true)
    }
}

/// 聚焦，且在 Windows 上把前台抢过来（§5.3 ①）。
///
/// Tauri 的 `set_focus` 底下就是 `SetForegroundWindow`，而**前台锁定机制会让
/// 非前台进程的这个调用静默失败**，只闪一下任务栏。热键触发时系统通常短暂
/// 授权，但不保证。所以 Windows 上再补一道 `AttachThreadInput`。
fn focus_hard(window: &tauri::WebviewWindow) -> AppResult<()> {
    #[cfg(windows)]
    crate::platform_win::steal_foreground(window);

    window
        .set_focus()
        .map_err(|e| AppError::Window(e.to_string()))
}

/// 上报最大化按钮矩形，供 Snap Layouts 用（§5.2 ②）。
///
/// 坐标随布局与 DPI 变化，Rust 那边看不到 DOM，只能由前端报。
#[tauri::command]
pub fn set_maxbutton_rect(
    #[allow(unused_variables)] x: f64,
    #[allow(unused_variables)] y: f64,
    #[allow(unused_variables)] w: f64,
    #[allow(unused_variables)] h: f64,
) {
    #[cfg(windows)]
    crate::platform_win::set_max_button_rect(crate::platform_win::MaxButton { x, y, w, h });
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
    fn 应用信息序列化成前端认得的形状() {
        let info = AppInfo {
            version: "0.1.0",
            platform: "windows",
            encrypted_store: true,
            unlocked: true,
        };
        let json = serde_json::to_value(info).unwrap();
        assert!(json["version"].is_string());
        assert!(json["encryptedStore"].is_boolean(), "{json}");
        assert!(json["unlocked"].is_boolean(), "{json}");
    }
}

// ── 位置记忆（§5.1）────────────────────────────────────────────

/// 存进 kv 的窗口几何。
///
/// **含所在显示器**：只存绝对坐标的话，拔掉外接屏之后窗口会还原到一个
/// 不存在的位置上——用户看到的是任务栏里有它、屏幕上没有。
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Geometry {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    maximized: bool,
    /// 保存时所在显示器的左上角。还原前用它校验那块屏还在不在。
    mon_x: i32,
    mon_y: i32,
}

const GEOMETRY_KEY: &str = "window.main.geometry";

/// 记下主窗口的位置。关闭与隐藏时调。
pub fn save_geometry(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    // 最大化状态下取到的是全屏尺寸，存下来会让「还原」也变成全屏
    let maximized = win.is_maximized().unwrap_or(false);
    let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) else {
        return;
    };
    let mon = win
        .current_monitor()
        .ok()
        .flatten()
        .map(|m| *m.position())
        .unwrap_or_default();

    let geo = Geometry {
        x: pos.x,
        y: pos.y,
        w: size.width,
        h: size.height,
        maximized,
        mon_x: mon.x,
        mon_y: mon.y,
    };

    let state = app.state::<AppState>();
    let _ = state.read(|s| {
        Ok(s.set_kv(
            GEOMETRY_KEY,
            &serde_json::to_string(&geo).unwrap_or_default(),
        )?)
    });
}

/// 还原主窗口的位置。
///
/// **还原前校验该显示器仍存在**（§5.1）。校验不过就什么也不做，
/// 让 Tauri 用配置里的居中默认值——窗口开在屏幕外是没法自己救回来的，
/// 用户只能去改注册表或者删配置。
pub fn restore_geometry(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let state = app.state::<AppState>();
    let Ok(Some(raw)) = state.read(|s| Ok(s.get_kv(GEOMETRY_KEY)?)) else {
        return;
    };
    let Ok(geo) = serde_json::from_str::<Geometry>(&raw) else {
        return;
    };

    let monitors = win.available_monitors().unwrap_or_default();
    let still_there = monitors.iter().any(|m| {
        let p = m.position();
        let s = m.size();
        // 窗口左上角落在这块屏里就算数。要求完全包含的话，
        // 稍微跨屏摆放的窗口每次都会被弹回中间
        geo.x >= p.x
            && geo.y >= p.y
            && geo.x < p.x + s.width as i32
            && geo.y < p.y + s.height as i32
    });
    if !still_there {
        tracing::info!("上次的显示器不在了，主窗口用默认位置");
        return;
    }

    let _ = win.set_size(tauri::PhysicalSize::new(geo.w, geo.h));
    let _ = win.set_position(tauri::PhysicalPosition::new(geo.x, geo.y));
    if geo.maximized {
        let _ = win.maximize();
    }
}
