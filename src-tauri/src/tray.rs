//! 托盘与开机自启（§5.5）。
//!
//! ## 托盘为什么是必需的而不是装饰
//!
//! §5.1 定的关闭行为是「隐藏而非退出」。没有托盘的话，用户点了 × 之后应用
//! 就从他眼前彻底消失了——任务栏没有、Alt+Tab 没有，只剩一个全局热键，
//! 而那个热键还可能被别的程序占着。那不叫「后台运行」，那叫「失踪」。

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter};

use crate::error::AppResult;

pub const TRAY_ID: &str = "zhiyan-tray";

/// 建托盘图标与菜单。
///
/// 菜单项按 §5.5 的清单：速记 / 打开主窗口 / 本周回顾 / 立即同步 / 设置 / 退出。
/// 「立即同步」现在是禁用态占位——同步在 §10，还没做；**先摆出来是错的**，
/// 一个点了没反应的菜单项比没有更糟，所以它显示为灰的并带上「未启用」。
pub fn build(app: &AppHandle) -> AppResult<TrayIcon> {
    let quick = MenuItem::with_id(app, "quick", "速记  Ctrl+Alt+Space", true, None::<&str>)?;
    let main = MenuItem::with_id(app, "main", "打开主窗口", true, None::<&str>)?;
    let week = MenuItem::with_id(app, "week", "本周回顾", true, None::<&str>)?;
    let sync = MenuItem::with_id(app, "sync", "立即同步（未启用）", false, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出知言", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &quick,
            &main,
            &week,
            &PredefinedMenuItem::separator(app)?,
            &sync,
            &settings,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(tray_icon()?)
        .tooltip("知言")
        .menu(&menu)
        // 左键点击不弹菜单——Windows 上的习惯是左键切换窗口、右键才是菜单
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu)
        .on_tray_icon_event(on_icon)
        .build(app)?;

    Ok(tray)
}

/// 按系统主题挑一套托盘图标（§5.5）。
///
/// 通知区的底色跟着任务栏走，一套颜色总有一边糊在背景里——浅色任务栏上
/// 用浅色图标，看起来就像图标没加载出来。
fn tray_icon() -> AppResult<tauri::image::Image<'static>> {
    #[cfg(windows)]
    let light = crate::platform_win::apps_use_light_theme();
    #[cfg(not(windows))]
    let light = true;

    let bytes: &[u8] = if light {
        include_bytes!("../icons/tray-light.ico")
    } else {
        include_bytes!("../icons/tray-dark.ico")
    };
    // 从 ICO 里挑一张。多分辨率的容器交给 Tauri 自己选合适的那一张
    Ok(tauri::image::Image::from_bytes(bytes)?.to_owned())
}

/// 系统主题变了时换图标。`WM_SETTINGCHANGE` 里调。
///
/// 只有 Windows 会调它——别的平台上 `platform_win` 整个是空的。
#[cfg_attr(not(windows), allow(dead_code))]
pub fn refresh_icon(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    if let Ok(icon) = tray_icon() {
        let _ = tray.set_icon(Some(icon));
    }
}

fn on_menu(app: &AppHandle, event: MenuEvent) {
    match event.id.as_ref() {
        "quick" => {
            let _ = crate::windows::toggle_quick(app);
        }
        "main" => {
            let _ = crate::windows::show_main(app);
        }
        "week" => {
            // 先把窗口叫出来再让前端开面板，否则面板开在一个看不见的窗口里
            let _ = crate::windows::show_main(app);
            let _ = app.emit("open-week", ());
        }
        "settings" => {
            let _ = crate::windows::show_main(app);
            let _ = app.emit("open-settings", ());
        }
        "quit" => {
            // 真退出。托盘菜单里这一条是用户唯一能彻底关掉它的地方，
            // 所以它必须是真的退出，不能又变成隐藏
            app.exit(0);
        }
        _ => {}
    }
}

/// 左键单击切换主窗口显隐，双击显示并聚焦（§5.5）。
fn on_icon(tray: &TrayIcon, event: TrayIconEvent) {
    let app = tray.app_handle();
    match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } => {
            let _ = crate::windows::toggle_main(app);
        }
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => {
            let _ = crate::windows::show_main(app);
        }
        _ => {}
    }
}

// ── 开机自启（§5.5）─────────────────────────────────────────────

/// 自启是否**真的生效**。
///
/// # 为什么不能只看注册表项在不在
///
/// 用户可能在「设置 → 应用 → 启动」里把它关掉。那时
/// `HKCU\...\CurrentVersion\Run` 下的项**仍然在**，但被
/// `StartupApproved\Run` 里的一条记录禁用了。
///
/// 只读前者的话，设置页会显示「已开启」而实际不自启——用户重装、
/// 反复开关都解决不了，因为开关本来就是开的（§5.5 点名了这个坑）。
#[cfg(windows)]
pub fn autostart_effective(app: &AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;

    if !app.autolaunch().is_enabled().unwrap_or(false) {
        return false;
    }
    // StartupApproved 里的值第一个字节：偶数 = 启用，奇数 = 被用户禁用
    match crate::platform_win::startup_approved_state("zhiyan") {
        Some(enabled) => enabled,
        None => true, // 没有这条记录说明用户没动过，那就以注册表项为准
    }
}

#[cfg(not(windows))]
pub fn autostart_effective(app: &AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutostartState {
    /// 注册表项在不在。
    pub configured: bool,
    /// **真实生效状态**。与 `configured` 不一致时说明被系统设置禁用了。
    pub effective: bool,
}

#[tauri::command]
pub fn autostart_get(app: AppHandle) -> AutostartState {
    use tauri_plugin_autostart::ManagerExt;
    AutostartState {
        configured: app.autolaunch().is_enabled().unwrap_or(false),
        effective: autostart_effective(&app),
    }
}

#[tauri::command]
pub fn autostart_set(app: AppHandle, on: bool) -> AppResult<AutostartState> {
    use tauri_plugin_autostart::ManagerExt;
    let r = if on {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    };
    r.map_err(|e| crate::error::AppError::Window(format!("设置自启失败：{e}")))?;
    Ok(autostart_get(app))
}
