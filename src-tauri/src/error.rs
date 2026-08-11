//! IPC 错误。
//!
//! **不携带任何正文片段**（§14 ①）：错误会穿过 IPC 边界到前端，再经 `console.error`
//! 落进 WebView 的日志。只允许 id、长度、错误码。
//!
//! 存储与加密的错误变体在它们各自的阶段加进来——现在加会变成对着空 crate 写 `#[from]`。

use serde::Serialize;

/// 跨 IPC 边界的错误。
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// 阶段三接上数据层后就有构造点了。
    #[allow(dead_code)]
    #[error("找不到记录")]
    NotFound,
    #[error("窗口 {0} 不存在")]
    NoSuchWindow(&'static str),
    #[error("窗口操作失败：{0}")]
    Window(String),
}

/// 序列化成前端能分辨的形状：`{ code, message }`。
///
/// 前端按 `code` 分支，`message` 只用于开发期排查——**不要**把它直接显示给用户，
/// 它是给工程师看的。
impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

impl AppError {
    /// 稳定的错误码。前端与日志都按它分支，改动等同于改 API。
    pub fn code(&self) -> &'static str {
        match self {
            AppError::NotFound => "not_found",
            AppError::NoSuchWindow(_) => "no_such_window",
            AppError::Window(_) => "window",
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 错误码稳定() {
        assert_eq!(AppError::NotFound.code(), "not_found");
        assert_eq!(AppError::NoSuchWindow("quick").code(), "no_such_window");
        assert_eq!(AppError::Window("x".into()).code(), "window");
    }

    #[test]
    fn 序列化成_code_与_message() {
        let json = serde_json::to_value(AppError::NotFound).unwrap();
        assert_eq!(json["code"], "not_found");
        assert!(json["message"].is_string());
    }
}
