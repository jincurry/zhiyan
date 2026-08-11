//! 知言的入口。
//!
//! `windows_subsystem = "windows"` 让发布构建不弹控制台窗口；debug 下保留控制台，
//! 否则 `tracing` 的输出没地方看。
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

fn main() {
    init_tracing();
    zhiyan_app::run();
}

/// 日志。
///
/// **脱敏 layer 还没接**（§14 ③）。在它到位之前默认级别压到 `warn`，
/// 免得开发期顺手加的 `debug!` 把正文写进磁盘。
fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("ZHIYAN_LOG").unwrap_or_else(|_| EnvFilter::new("warn"));
    fmt().with_env_filter(filter).with_target(false).init();
}
