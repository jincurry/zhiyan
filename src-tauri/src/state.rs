//! 应用状态。
//!
//! 阶段四会把 `Store` 放进来。现在先占住位置，把「状态从哪来、谁能拿」这件事的形状定下来：
//!
//! `Store` 里有一个 `rusqlite::Connection`，它是 `Send` 但不是 `Sync`，所以要用
//! `Mutex` 包起来才能进 Tauri 的托管状态。命令里持锁的时间必须短——Tauri 的命令跑在
//! 线程池上，一个长查询持着锁会把其余命令全堵住。

/// 托管状态的占位。
#[allow(dead_code)] // 阶段四装上 Store 后就有构造点了
#[derive(Default)]
pub struct AppState {
    // 阶段四：pub store: std::sync::Mutex<Option<zhiyan_store::Store>>,
}
