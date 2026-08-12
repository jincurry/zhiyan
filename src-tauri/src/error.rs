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
    #[error("窗口 {0} 不存在")]
    NoSuchWindow(&'static str),
    #[error("窗口操作失败：{0}")]
    Window(String),

    /// 存储层。**`zhiyan-db` 的错误本身已经不带正文**，可以直接透传。
    #[error("存储失败：{0}")]
    Db(#[from] zhiyan_db::DbError),

    /// 加密层。同样不带正文与密钥字节（§8 的错误类型就是这么设计的）。
    #[error("加密失败：{0}")]
    Crypto(#[from] zhiyan_crypto::CryptoError),

    /// 密钥保管（§9.4）。
    #[error("密钥保管失败：{0}")]
    Vault(String),

    /// 库还没打开。启动时密钥取不到就会停在这个状态，界面该显示解锁引导
    /// 而不是一堆失败的查询。
    #[error("本地库尚未打开")]
    Locked,

    /// 前端传来的参数不合法。**只说哪个参数**，不回显它的值——
    /// 那个值可能就是用户的正文。
    #[error("参数 {0} 不合法")]
    BadArgument(&'static str),
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
            AppError::NoSuchWindow(_) => "no_such_window",
            AppError::Window(_) => "window",
            // 存储层自己的错误码更细（sql / crypto / io / blob_corrupted），透传出去
            AppError::Db(e) => e.code(),
            AppError::Crypto(_) => "crypto",
            AppError::Vault(_) => "vault",
            AppError::Locked => "locked",
            AppError::BadArgument(_) => "bad_argument",
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 错误码稳定() {
        assert_eq!(AppError::NoSuchWindow("quick").code(), "no_such_window");
        assert_eq!(AppError::Window("x".into()).code(), "window");
    }

    #[test]
    fn 序列化成_code_与_message() {
        let json = serde_json::to_value(AppError::Locked).unwrap();
        assert_eq!(json["code"], "locked");
        assert!(json["message"].is_string());
    }
}
