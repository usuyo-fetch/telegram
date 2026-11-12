//! 这个插件会失败的几种方式，和它们对应的协议错误码（协议 §3.3）。
//! 不回网络类的码，也不回 `login-required`：插件只读本机的文件。
//! 读不了文件的错带 `detail`：哪个文件、哪个字段（给修插件的人看，宿主只记进失败日志）。

use usuyo_plugin_kit::protocol::{CallFailure, ErrorCode};

/// 界面上给用户的两句话：中文、英文。
type Hint = (&'static str, &'static str);

pub const NOT_AN_EXPORT: Hint = (
    "这个文件读不出 Telegram Desktop 的导出：要的是「导出聊天记录」里选「机器可读的 JSON」得到的 result.json，或连文件夹一起压成的 zip",
    "This file does not read as a Telegram Desktop export: choose the machine-readable JSON format when exporting chat history, then give result.json or the zipped export folder",
);
pub const HTML_EXPORT: Hint = (
    "这是 HTML 格式的导出：请在 Telegram Desktop 里重新导出，格式选「机器可读的 JSON」",
    "This is the HTML export: export again from Telegram Desktop and choose the machine-readable JSON format",
);
pub const NO_SAVED: Hint = (
    "这份导出里没有 Saved Messages：导出时在聊天列表里勾上「Saved Messages」，或只导出那一个聊天",
    "This export has no Saved Messages: tick Saved Messages in the chat list when exporting, or export that one chat alone",
);
pub const TOO_LARGE: Hint = (
    "这个文件超过了插件愿意读的大小：导出时少选一些聊天或缩短日期范围",
    "This file is larger than the plugin will read: export fewer chats or a shorter date range",
);
pub const CHANGED: Hint = (
    "Telegram Desktop 的导出格式和这个插件认得的不一样，插件要更新",
    "The Telegram Desktop export differs from what this plugin reads; the plugin needs an update",
);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    /// 不是这个插件读得了的文件。
    #[error("unsupported input: {message}")]
    Unsupported {
        message: String,
        detail: String,
        hint: Hint,
    },
    /// 认得出是导出，但某个字段的形状变了。
    #[error("export format changed: {message}")]
    UpstreamChanged { message: String, detail: String },
    /// `ref` 指的消息不在这份文件里（换了文件，或文件变了）。
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Internal(String),
    #[error("write output: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    /// `file` 是交来的文件里的哪一个（zip 里的条目名，或 `input`），`field` 是哪个字段。
    pub fn unsupported(file: &str, field: &str, message: impl Into<String>, hint: Hint) -> Self {
        let message = message.into();
        Error::Unsupported {
            detail: format!("file: {file}\nfield: {field}\n{message}"),
            message,
            hint,
        }
    }

    pub fn changed(file: &str, field: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        Error::UpstreamChanged {
            detail: format!("file: {file}\nfield: {field}\n{message}"),
            message,
        }
    }
}

/// `hint` 按 `USUYO_LOCALE` 写：`zh` 开头写中文，其余写英文。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    pub fn from_locale(locale: Option<&str>) -> Self {
        match locale {
            Some(l) if !l.to_ascii_lowercase().starts_with("zh") => Lang::En,
            _ => Lang::Zh,
        }
    }

    fn pick(self, hint: Hint) -> String {
        match self {
            Lang::Zh => hint.0.into(),
            Lang::En => hint.1.into(),
        }
    }
}

impl Error {
    pub fn to_failure(&self, lang: Lang) -> CallFailure {
        let message = self.to_string();
        match self {
            Error::InvalidArgs(_) => CallFailure::new(ErrorCode::InvalidArgs, message),
            Error::Unsupported { detail, hint, .. } => CallFailure {
                hint: Some(lang.pick(*hint)),
                ..CallFailure::new(ErrorCode::UnsupportedInput, message)
            }
            .with_detail(detail),
            Error::UpstreamChanged { detail, .. } => CallFailure {
                hint: Some(lang.pick(CHANGED)),
                ..CallFailure::new(ErrorCode::UpstreamChanged, message)
            }
            .with_detail(detail),
            Error::NotFound(_) => CallFailure::new(ErrorCode::NotFound, message),
            Error::Internal(_) | Error::Io(_) => CallFailure::new(ErrorCode::Internal, message),
        }
    }
}
