//! 全局热键（§5.4）。
//!
//! ## 原型的 `Ctrl+Shift+N` 必须换掉
//!
//! 它在 Chrome / Edge 是「新建无痕窗口」，在资源管理器是「新建文件夹」。
//! `RegisterHotKey` 是全局抢占的——注册上去就等于砸掉用户这两个习惯，
//! 而且他们多半不会想到是我们干的。
//!
//! ## 注册失败必须可见
//!
//! `ERROR_HOTKEY_ALREADY_REGISTERED` 说明键位被占了，常见占用方是搜狗输入法、
//! QQ、PowerToys、Ditto。**静默失败会让用户以为软件坏了**——按下去没反应，
//! 又没有任何提示，只能得出「这功能是假的」这个结论。
//!
//! 所以失败要走三条路：托盘角标、设置页红字、首次失败弹一次通知。
//! 这一层负责把事实广播出去（`hotkey-conflict` 事件），怎么显示归前端。

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::error::{AppError, AppResult};

/// 一个可改键的功能位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Slot {
    /// 唤起 / 收起速记浮窗。
    Quick,
    /// 显示 / 隐藏主窗口。**默认关**（§5.4）——它抢的键位更宽，
    /// 而且多数人并不需要第二个全局键。
    Main,
}

impl Slot {
    pub fn key(self) -> &'static str {
        match self {
            Slot::Quick => "hotkey.quick",
            Slot::Main => "hotkey.main",
        }
    }

    /// §5.4 给的默认键位。
    pub fn default_combo(self) -> &'static str {
        match self {
            Slot::Quick => "Ctrl+Alt+Space",
            Slot::Main => "Ctrl+Alt+Z",
        }
    }

    pub fn enabled_by_default(self) -> bool {
        matches!(self, Slot::Quick)
    }

    fn all() -> [Slot; 2] {
        [Slot::Quick, Slot::Main]
    }
}

/// 一个功能位的当前状态，给设置页看。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyState {
    pub slot: Slot,
    pub combo: String,
    pub enabled: bool,
    /// 注册成功了吗。`false` 且 `enabled` 为真 = 被别的程序占了。
    pub registered: bool,
    /// 失败原因。给工程师看的，界面上只显示「已被其他程序占用」。
    pub reason: Option<String>,
}

/// 广播给前端的冲突事件（§5.4 的示例就是这个形状）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyConflict {
    pub slot: Slot,
    pub combo: String,
    pub reason: String,
}

/// 解析组合键。前端传来的字符串是不可信输入。
fn parse(combo: &str) -> AppResult<Shortcut> {
    combo
        .parse::<Shortcut>()
        .map_err(|_| AppError::BadArgument("combo"))
}

/// 从 kv 里读一个功能位的设置。读不到就用默认值。
fn load(app: &AppHandle, slot: Slot) -> (String, bool) {
    let state = app.state::<crate::state::AppState>();
    let stored = state.read(|s| Ok(s.get_kv(slot.key())?)).ok().flatten();

    match stored.as_deref() {
        // 空串表示「用户关掉了它」，与「没设过」要分得开
        Some("") => (slot.default_combo().to_string(), false),
        Some(v) => (v.to_string(), true),
        None => (slot.default_combo().to_string(), slot.enabled_by_default()),
    }
}

fn save(app: &AppHandle, slot: Slot, combo: &str, enabled: bool) -> AppResult<()> {
    let state = app.state::<crate::state::AppState>();
    let v = if enabled { combo } else { "" };
    state.read(|s| Ok(s.set_kv(slot.key(), v)?))
}

/// 启动时注册全部热键。
///
/// 注册失败**不返回 Err**：一个被占用的键位不该让应用起不来。
/// 事实经 `hotkey-conflict` 事件广播出去，界面自己决定怎么说。
pub fn register_all(app: &AppHandle) -> Vec<HotkeyState> {
    Slot::all()
        .into_iter()
        .map(|slot| {
            let (combo, enabled) = load(app, slot);
            register_one(app, slot, &combo, enabled)
        })
        .collect()
}

fn register_one(app: &AppHandle, slot: Slot, combo: &str, enabled: bool) -> HotkeyState {
    if !enabled {
        return HotkeyState {
            slot,
            combo: combo.to_string(),
            enabled: false,
            registered: false,
            reason: None,
        };
    }

    let result = parse(combo).and_then(|sc| {
        let handle = app.clone();
        app.global_shortcut()
            .on_shortcut(sc, move |_, _, event| {
                // 只在按下时触发。不判的话按一次会跑两遍——浮窗刚弹出来又被收回去
                if event.state == ShortcutState::Pressed {
                    fire(&handle, slot);
                }
            })
            .map_err(|e| AppError::Hotkey(e.to_string()))
    });

    match result {
        Ok(()) => HotkeyState {
            slot,
            combo: combo.to_string(),
            enabled: true,
            registered: true,
            reason: None,
        },
        Err(e) => {
            // 常见占用方：搜狗输入法、QQ、PowerToys、Ditto
            tracing::warn!("热键 {combo} 注册失败：{}", e.code());
            let conflict = HotkeyConflict {
                slot,
                combo: combo.to_string(),
                reason: e.to_string(),
            };
            let _ = app.emit("hotkey-conflict", conflict);
            notify_once(app, combo);
            HotkeyState {
                slot,
                combo: combo.to_string(),
                enabled: true,
                registered: false,
                reason: Some(e.to_string()),
            }
        }
    }
}

/// 首次失败弹一次系统通知（§5.4）。
///
/// **只弹一次**。每次启动都弹的话，一个装了搜狗输入法、永远抢不到键位的用户
/// 会天天被同一条通知骚扰——他早就知道了，弹十次也改变不了什么。
///
/// 标记存在 kv 里而不是内存里：内存的话每次重启都算「首次」。
fn notify_once(app: &AppHandle, combo: &str) {
    use tauri_plugin_notification::NotificationExt;

    const KEY: &str = "hotkey.conflict.notified";
    let state = app.state::<crate::state::AppState>();
    if state.read(|s| Ok(s.get_kv(KEY)?)).ok().flatten().is_some() {
        return;
    }

    let _ = app
        .notification()
        .builder()
        .title("知言的全局热键没能注册")
        .body(format!(
            "{combo} 已被其他程序占用（常见的是输入法、QQ、PowerToys）。在设置里换一个组合键。"
        ))
        .show();

    let _ = state.read(|s| Ok(s.set_kv(KEY, "1")?));
}

/// 热键按下时做什么。
fn fire(app: &AppHandle, slot: Slot) {
    match slot {
        Slot::Quick => {
            if let Err(e) = crate::windows::toggle_quick(app) {
                tracing::warn!("唤起浮窗失败：{}", e.code());
            }
        }
        Slot::Main => {
            if let Err(e) = crate::windows::toggle_main(app) {
                tracing::warn!("切换主窗口失败：{}", e.code());
            }
        }
    }
}

fn unregister(app: &AppHandle, combo: &str) {
    if let Ok(sc) = parse(combo) {
        let _ = app.global_shortcut().unregister(sc);
    }
}

// ── 命令 ────────────────────────────────────────────────────────

#[tauri::command]
pub fn hotkey_list(app: AppHandle) -> Vec<HotkeyState> {
    Slot::all()
        .into_iter()
        .map(|slot| {
            let (combo, enabled) = load(app.app_handle(), slot);
            let registered = enabled
                && parse(&combo)
                    .map(|sc| app.global_shortcut().is_registered(sc))
                    .unwrap_or(false);
            HotkeyState {
                slot,
                combo,
                enabled,
                registered,
                reason: None,
            }
        })
        .collect()
}

/// 改键（§5.4 要求的改键 UI 背后这一条）。
///
/// **先试注册，成功才保存**。反过来的话，用户填了一个被占用的键位、
/// 设置存下了、下次启动仍然不工作——而设置页显示的是「已设置」。
///
/// 试注册之前要先把旧的取消：同一个功能位换键时，不取消旧的会留下一个
/// 幽灵热键，按下去还会触发。
pub fn rebind_impl(
    app: &AppHandle,
    slot: Slot,
    combo: &str,
    enabled: bool,
) -> AppResult<HotkeyState> {
    let (old, _) = load(app, slot);
    unregister(app, &old);

    if !enabled {
        save(app, slot, combo, false)?;
        return Ok(HotkeyState {
            slot,
            combo: combo.to_string(),
            enabled: false,
            registered: false,
            reason: None,
        });
    }

    // 先验格式，再验能不能抢到
    let sc = parse(combo)?;
    if app.global_shortcut().is_registered(sc) {
        return Err(AppError::Hotkey("这个组合键已被占用".into()));
    }

    let state = register_one(app, slot, combo, true);
    if !state.registered {
        // 没抢到就把旧的接回去，别让用户既丢了新键也丢了旧键
        let (_, old_enabled) = load(app, slot);
        register_one(app, slot, &old, old_enabled);
        return Err(AppError::Hotkey(
            state.reason.unwrap_or_else(|| "注册失败".into()),
        ));
    }

    save(app, slot, combo, true)?;
    Ok(state)
}

#[tauri::command]
pub fn hotkey_rebind(
    app: AppHandle,
    slot: Slot,
    combo: String,
    enabled: bool,
) -> AppResult<HotkeyState> {
    rebind_impl(&app, slot, &combo, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 默认键位不是原型那个() {
        // Ctrl+Shift+N 在 Chrome 是无痕窗口、在资源管理器是新建文件夹
        for slot in Slot::all() {
            assert_ne!(slot.default_combo(), "Ctrl+Shift+N");
        }
        assert_eq!(Slot::Quick.default_combo(), "Ctrl+Alt+Space");
        assert_eq!(Slot::Main.default_combo(), "Ctrl+Alt+Z");
    }

    #[test]
    fn 主窗口热键默认关闭() {
        // 它抢的键位更宽，而多数人并不需要第二个全局键
        assert!(Slot::Quick.enabled_by_default());
        assert!(!Slot::Main.enabled_by_default());
    }

    #[test]
    fn 两个功能位的存储键不同() {
        assert_ne!(Slot::Quick.key(), Slot::Main.key());
    }

    #[test]
    fn 组合键解析认得默认值也挡得住乱填() {
        for slot in Slot::all() {
            assert!(
                parse(slot.default_combo()).is_ok(),
                "{}",
                slot.default_combo()
            );
        }
        for bad in ["", "随便写的", "Ctrl+", "++", "Ctrl+Alt+不存在的键"] {
            assert!(
                matches!(parse(bad), Err(AppError::BadArgument("combo"))),
                "{bad:?} 该被拒绝"
            );
        }
    }

    #[test]
    fn 功能位按_kebab_case_序列化() {
        // 前端设置页按这个字符串分支
        assert_eq!(serde_json::to_value(Slot::Quick).unwrap(), "quick");
        assert_eq!(serde_json::to_value(Slot::Main).unwrap(), "main");
    }
}
