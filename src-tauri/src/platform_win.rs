//! Windows 原生集成（§5.2 ②、§5.3 ①③、§5.5、§4.2）。
//!
//! ## 这个模块里的东西都没法在 Linux 上验证
//!
//! CI 会在 windows-latest 上编译它，所以**类型和签名是可靠的**；但
//! 「悬停最大化按钮真的弹出了贴靠面板吗」这类事只能在真机上看。
//! 每个函数因此都写成「失败就静默降级」——原生感是加分项，
//! 不能因为它没成功就让窗口开不出来。
//!
//! ## 为什么不直接用 Tauri 的 `HWND`
//!
//! `window.hwnd()` 返回的 `HWND` 绑在 Tauri 依赖的那个 `windows` 版本上。
//! 我们自己也依赖 `windows`，两个版本不一定相同，类型就对不上。
//! 所以统一走裸指针转一手。

#![cfg(windows)]

use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Emitter, WebviewWindow};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMSBT_MAINWINDOW, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
    DWM_SYSTEMBACKDROP_TYPE,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_READ, REG_VALUE_TYPE,
};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SetFocus, TrackMouseEvent, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, SetCurrentProcessExplicitAppUserModelID, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, IsZoomed, SetForegroundWindow, ShowWindow,
    HTCLIENT, HTMAXBUTTON, SW_MAXIMIZE, SW_RESTORE, WM_DPICHANGED, WM_NCHITTEST, WM_NCLBUTTONDOWN,
    WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_SETTINGCHANGE,
};

/// 应用的 AppUserModelID。
///
/// **必须与安装器写进快捷方式的 `System.AppUserModel.ID` 一致**，否则 Toast
/// 通知根本不显示——这是这一块最常见的踩坑点（§5.5）。
const AUMID: PCWSTR = w!("Zhiyan.Desktop");

// ── 最大化按钮矩形（§5.2 ②）──────────────────────────────────────

/// 前端上报的最大化按钮矩形，**逻辑像素**，相对窗口客户区左上角。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MaxButton {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl MaxButton {
    fn contains(&self, x: f64, y: f64) -> bool {
        self.w > 0.0
            && self.h > 0.0
            && x >= self.x
            && x < self.x + self.w
            && y >= self.y
            && y < self.y + self.h
    }
}

static MAX_BUTTON: Mutex<MaxButton> = Mutex::new(MaxButton {
    x: 0.0,
    y: 0.0,
    w: 0.0,
    h: 0.0,
});
static APP: OnceLock<AppHandle> = OnceLock::new();

/// 记下前端上报的矩形。布局与 DPI 变化时都会调。
pub fn set_max_button_rect(rect: MaxButton) {
    *MAX_BUTTON.lock().unwrap() = rect;
}

fn hwnd_of(window: &WebviewWindow) -> Option<HWND> {
    // `as _` 让它同时吃得下 isize 与 *mut c_void 两种表示
    window.hwnd().ok().map(|h| HWND(h.0 as _))
}

/// 进程级初始化：AUMID。越早越好，通知与任务栏分组都认它。
pub fn init_process(app: &AppHandle) {
    let _ = APP.set(app.clone());
    // SAFETY: 只是给当前进程设一个字符串标识，无内存所有权转移
    if let Err(e) = unsafe { SetCurrentProcessExplicitAppUserModelID(AUMID) } {
        tracing::warn!("设置 AppUserModelID 失败：{}", e.code().0);
    }
}

// ── Snap Layouts（§5.2 ②）───────────────────────────────────────

/// 给主窗口挂上子类化过程。
///
/// # 为什么非做不可
///
/// Win11 用户悬停最大化按钮 1 秒会期待弹出贴靠布局面板。自绘标题栏默认没有，
/// 用户会立刻察觉「这软件不对劲」——而且说不出哪里不对，只觉得不像原生应用。
///
/// # 代价
///
/// 返回 `HTMAXBUTTON` 之后，**系统接管该区域的鼠标消息，DOM 的 click 不再触发**。
/// 所以最大化这个动作本身也得在这里自己做（`WM_NCLBUTTONUP`），
/// 而 hover 态要回传给前端画高亮。漏掉任一条，按钮看起来就是坏的。
pub fn attach_titlebar_subclass(window: &WebviewWindow) {
    let Some(hwnd) = hwnd_of(window) else {
        tracing::warn!("拿不到窗口句柄，Snap Layouts 未启用");
        return;
    };
    // SAFETY: hwnd 来自 Tauri 且窗口仍存活；subclass_proc 是 extern "system"，
    // 签名与 SUBCLASSPROC 一致
    let ok = unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0) };
    if !ok.as_bool() {
        tracing::warn!("子类化失败，Snap Layouts 未启用");
    }
}

const SUBCLASS_ID: usize = 0x5A48; // 'ZH'

/// 按下与松开必须成对：只在同一个按钮上按下又松开才算一次点击。
static PRESSED: Mutex<bool> = Mutex::new(false);
static HOVERING: Mutex<bool> = Mutex::new(false);

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => {
            let hit = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            // **只在默认判定为客户区时才改写**。默认返回 HTTOP / HTCAPTION 之类时
            // 说明鼠标在缩放边框或标题栏拖拽区上，抢过来会让窗口没法从上边缘缩放
            if hit.0 as u32 != HTCLIENT {
                return hit;
            }
            if in_max_button(hwnd, lparam) {
                return LRESULT(HTMAXBUTTON as isize);
            }
            hit
        }

        WM_NCMOUSEMOVE if wparam.0 as u32 == HTMAXBUTTON => {
            if !std::mem::replace(&mut *HOVERING.lock().unwrap(), true) {
                notify_hover(true);
                // 不订阅的话 WM_NCMOUSELEAVE 永远不会来，高亮会一直挂着
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_NONCLIENT,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = unsafe { TrackMouseEvent(&mut tme) };
            }
            LRESULT(0)
        }

        WM_NCMOUSELEAVE => {
            *PRESSED.lock().unwrap() = false;
            if std::mem::replace(&mut *HOVERING.lock().unwrap(), false) {
                notify_hover(false);
            }
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }

        WM_NCLBUTTONDOWN if wparam.0 as u32 == HTMAXBUTTON => {
            *PRESSED.lock().unwrap() = true;
            LRESULT(0) // 吞掉：交给 DefWindowProc 会进入系统菜单的移动/缩放模式
        }

        WM_NCLBUTTONUP if wparam.0 as u32 == HTMAXBUTTON => {
            if std::mem::replace(&mut *PRESSED.lock().unwrap(), false) {
                // SAFETY: hwnd 有效；ShowWindow 不接管所有权
                let zoomed = unsafe { IsZoomed(hwnd) }.as_bool();
                let _ = unsafe { ShowWindow(hwnd, if zoomed { SW_RESTORE } else { SW_MAXIMIZE }) };
                notify_hover(false);
            }
            LRESULT(0)
        }

        // 系统主题变了。通知区的底色跟着任务栏走，一套颜色总有一边糊在背景里
        WM_SETTINGCHANGE => {
            if let Some(app) = APP.get() {
                crate::tray::refresh_icon(app);
            }
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }

        WM_DPICHANGED => {
            // 缩放变了，前端上报的逻辑矩形要重算一次
            if let Some(app) = APP.get() {
                let _ = app.emit("dpi-changed", ());
            }
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }

        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}

/// `WM_NCHITTEST` 的 `lparam` 是**屏幕坐标**，要换算成窗口的逻辑坐标才能
/// 跟前端上报的矩形比。
fn in_max_button(hwnd: HWND, lparam: LPARAM) -> bool {
    let rect = *MAX_BUTTON.lock().unwrap();
    if rect.w <= 0.0 {
        return false; // 前端还没上报过
    }

    let mut pt = POINT {
        x: (lparam.0 & 0xFFFF) as i16 as i32,
        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
    };
    // SAFETY: hwnd 有效，pt 是本地变量
    let ok = unsafe { ScreenToClient(hwnd, &mut pt) };
    if !ok.as_bool() {
        return false;
    }

    // 混合 DPI（§5.3 ③）：4K@150% 与 1080p@100% 可以并存，
    // 用固定的 96 去换算会在副屏上整体偏移
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let scale = if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 };

    rect.contains(pt.x as f64 / scale, pt.y as f64 / scale)
}

fn notify_hover(on: bool) {
    if let Some(app) = APP.get() {
        let _ = app.emit("maxbutton-hover", on);
    }
}

// ── 前台焦点抢占（§5.3 ①）───────────────────────────────────────

/// 把窗口抢到前台。
///
/// # 为什么 `SetForegroundWindow` 一个不够
///
/// Windows 的前台锁定机制会让非前台进程的 `SetForegroundWindow` **静默失败**，
/// 只闪一下任务栏。热键触发时系统通常会短暂授权，但**不保证**——尤其是
/// 用户刚在别的应用里点过东西的时候。
///
/// 绕过去的办法是把自己的输入队列临时挂到当前前台窗口的线程上，
/// 在系统眼里我们就成了「前台线程的一部分」。
///
/// 调用完前端还要再 `element.focus()` 一次，两层都要做。
pub fn steal_foreground(window: &WebviewWindow) {
    let Some(hwnd) = hwnd_of(window) else { return };

    // SAFETY: 全是取当前状态的只读调用；AttachThreadInput 成对调用，
    // 中间不 panic（都是 FFI，没有可能 unwind 的代码）
    unsafe {
        let fg = GetForegroundWindow();
        if fg == hwnd {
            let _ = SetFocus(hwnd);
            return;
        }

        let fg_tid = GetWindowThreadProcessId(fg, None);
        let cur_tid = GetCurrentThreadId();

        // 同一个线程时 AttachThreadInput 会失败并且没有必要
        let attached =
            fg_tid != 0 && fg_tid != cur_tid && AttachThreadInput(cur_tid, fg_tid, true).as_bool();

        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(hwnd);

        if attached {
            // 必须解开。挂着不放会让两个线程的输入状态一直纠缠在一起，
            // 表现是别的应用偶发地响应不了键盘
            let _ = AttachThreadInput(cur_tid, fg_tid, false);
        }
    }
}

// ── Mica / Acrylic（§5.5）───────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backdrop {
    /// 主窗口：Mica。
    Mica,
    /// 浮窗：Acrylic。
    Acrylic,
}

/// 给窗口设置系统背景材质。
///
/// **启用后前端应移除 CSS 的 `backdrop-filter`**，否则是双重模糊——
/// 看起来会比不加还糊。Win10 上这个属性不被支持，静默降级为不透明。
pub fn apply_backdrop(window: &WebviewWindow, kind: Backdrop) -> bool {
    let Some(hwnd) = hwnd_of(window) else {
        return false;
    };
    let value: DWM_SYSTEMBACKDROP_TYPE = match kind {
        Backdrop::Mica => DWMSBT_MAINWINDOW,
        Backdrop::Acrylic => DWMSBT_TRANSIENTWINDOW,
    };
    // SAFETY: 传的是本地变量的地址与它的大小，DWM 只读取不保留
    let r = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &value as *const _ as *const _,
            std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
    };
    r.is_ok()
}

// ── WebView2 运行时检测（§4.2）──────────────────────────────────

/// Evergreen 运行时的注册表 GUID。
const WEBVIEW2_CLIENT: PCWSTR =
    w!("Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}");

/// 已安装的 WebView2 运行时版本。
///
/// 查三个位置：机器级的 32 位视图（`WOW6432Node`）、机器级原生视图、以及
/// 用户级安装。只查文档里那一个 `WOW6432Node` 路径的话，
/// **纯 64 位安装与用户级安装都会被误判成「没装」**。
pub fn webview2_version() -> Option<String> {
    let candidates: [(HKEY, PCWSTR); 3] = [
        (
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"),
        ),
        (
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"),
        ),
        (
            HKEY_CURRENT_USER,
            w!("SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"),
        ),
    ];
    let _ = WEBVIEW2_CLIENT; // 保留常量做文档用
    candidates
        .iter()
        .find_map(|(root, path)| read_string_value(*root, *path, w!("pv")))
        .filter(|v| !v.is_empty() && v != "0.0.0.0")
}

fn read_string_value(root: HKEY, path: PCWSTR, name: PCWSTR) -> Option<String> {
    let mut key = HKEY::default();
    // SAFETY: 出参都是本地变量；成功时用 RegCloseKey 配对释放
    unsafe {
        if RegOpenKeyExW(root, path, 0, KEY_READ, &mut key).is_err() {
            return None;
        }
    }

    let mut ty = REG_VALUE_TYPE::default();
    let mut len: u32 = 0;
    let read = unsafe { RegQueryValueExW(key, name, None, Some(&mut ty), None, Some(&mut len)) };
    if read.is_err() || len == 0 {
        unsafe {
            let _ = RegCloseKey(key);
        }
        return None;
    }

    let mut buf = vec![0u8; len as usize];
    let read = unsafe {
        RegQueryValueExW(
            key,
            name,
            None,
            Some(&mut ty),
            Some(buf.as_mut_ptr()),
            Some(&mut len),
        )
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if read.is_err() {
        return None;
    }

    // REG_SZ 是 UTF-16，末尾带 NUL
    let wide: Vec<u16> = buf
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    Some(String::from_utf16_lossy(&wide))
}

/// 「设置 → 应用 → 启动」里那个开关的真实状态（§5.5）。
///
/// `HKCU\...\StartupApproved\Run` 下每个值是 12 字节，**第一个字节偶数表示
/// 启用、奇数表示被用户禁用**。没有这条记录说明用户没动过。
///
/// 不查它的话，设置页会显示「已开启」而实际不自启——用户重装、反复开关
/// 都解决不了，因为开关本来就是开的。
pub fn startup_approved_state(name: &str) -> Option<bool> {
    let path = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = read_binary_value(HKEY_CURRENT_USER, path, PCWSTR(wide.as_ptr()))?;
    bytes.first().map(|b| b % 2 == 0)
}

fn read_binary_value(root: HKEY, path: PCWSTR, name: PCWSTR) -> Option<Vec<u8>> {
    let mut key = HKEY::default();
    // SAFETY: 出参都是本地变量；成功时与 RegCloseKey 配对
    unsafe {
        if RegOpenKeyExW(root, path, 0, KEY_READ, &mut key).is_err() {
            return None;
        }
    }
    let mut ty = REG_VALUE_TYPE::default();
    let mut len: u32 = 0;
    let probe = unsafe { RegQueryValueExW(key, name, None, Some(&mut ty), None, Some(&mut len)) };
    if probe.is_err() || len == 0 {
        unsafe {
            let _ = RegCloseKey(key);
        }
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    let read = unsafe {
        RegQueryValueExW(
            key,
            name,
            None,
            Some(&mut ty),
            Some(buf.as_mut_ptr()),
            Some(&mut len),
        )
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    read.is_ok().then_some(buf)
}

/// 系统是否处于浅色主题。托盘图标要跟着换（§5.5）。
///
/// 通知区的底色跟着任务栏走，一套颜色总有一边糊在背景里。
pub fn apps_use_light_theme() -> bool {
    let path = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    read_binary_value(HKEY_CURRENT_USER, path, w!("SystemUsesLightTheme"))
        .and_then(|b| b.first().copied())
        .map(|v| v != 0)
        .unwrap_or(true) // 读不到就按浅色，那是 Windows 的出厂设置
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 矩形命中判定含左上不含右下() {
        let r = MaxButton {
            x: 10.0,
            y: 4.0,
            w: 46.0,
            h: 32.0,
        };
        assert!(r.contains(10.0, 4.0), "左上角要算命中");
        assert!(r.contains(55.9, 35.9));
        assert!(
            !r.contains(56.0, 20.0),
            "右边界要算出界，否则与相邻按钮重叠"
        );
        assert!(!r.contains(20.0, 36.0));
        assert!(!r.contains(9.9, 20.0));
    }

    #[test]
    fn 没上报过矩形时一律不命中() {
        // 默认全零。判成命中的话，整个客户区都会被当成最大化按钮
        assert!(!MaxButton::default().contains(0.0, 0.0));
        assert!(!MaxButton::default().contains(100.0, 100.0));
    }
}
