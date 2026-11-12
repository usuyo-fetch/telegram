//! 一条消息（`result.json` 里 `messages` 的一项）：文字、时间、转发来源、附件。
//!
//! 字段照 Telegram Desktop 的 JSON 导出（见 README 末行的来源）：`text` 是字符串，或字符串和
//! `{"type", "text", "href"?}` 交错的数组；`date_unixtime` 是秒数的字符串；附件在 `photo` 和 `file`，
//! 没导出附件时写的是一句以 `(File` 开头的话。每处只读用得到的字段，别的原样留在 `meta.json`。

use serde_json::Value;
use usuyo_plugin_kit::protocol::Rfc3339;

/// 一条真正的消息的 id；服务消息（入群、置顶……）、没有 id 的回 `None`。
pub fn id(message: &Value) -> Option<i64> {
    if message
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|t| t != "message")
    {
        return None;
    }
    message.get("id")?.as_i64()
}

fn string<'a>(message: &'a Value, key: &str) -> Option<&'a str> {
    message.get(key)?.as_str().filter(|s| !s.is_empty())
}

/// 消息的文字：实体压平，链接留目标（`text_link` 写它的 `href`），格式去掉。
pub fn text(message: &Value) -> String {
    let parts = match message.get("text") {
        Some(Value::String(s)) if !s.is_empty() => return s.clone(),
        Some(Value::Array(a)) if !a.is_empty() => a,
        _ => match message.get("text_entities") {
            Some(Value::Array(a)) => a,
            _ => return String::new(),
        },
    };
    let mut out = String::new();
    for part in parts {
        match part {
            Value::String(s) => out.push_str(s),
            Value::Object(o) => {
                let href = (o.get("type").and_then(Value::as_str) == Some("text_link"))
                    .then(|| o.get("href").and_then(Value::as_str))
                    .flatten()
                    .filter(|h| !h.is_empty());
                if let Some(s) = href.or_else(|| o.get("text").and_then(Value::as_str)) {
                    out.push_str(s);
                }
            }
            _ => {}
        }
    }
    out
}

/// 整条消息就是一个 http(s) 链接时回它。
pub fn sole_link(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let url = url::Url::parse(text).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some()).then(|| text.to_owned())
}

/// 发送（或存下）的时间：只认 `date_unixtime`。`date` 是本地时间、没有偏移，写不成 RFC 3339。
/// 有这个字段却读不出，回 `Err` 带字段名。
pub fn time(message: &Value) -> Result<Option<Rfc3339>, String> {
    let Some(raw) = message.get("date_unixtime") else {
        return Ok(None);
    };
    let secs = match raw {
        Value::String(s) => s.parse::<i64>().ok(),
        Value::Number(n) => n.as_i64(),
        _ => None,
    };
    secs.and_then(|s| jiff::Timestamp::from_second(s).ok())
        .map(|t| Some(Rfc3339::from_timestamp(t)))
        .ok_or_else(|| "`date_unixtime` is not a Unix time in seconds".into())
}

/// 转发来源的名字。
pub fn forwarded_from(message: &Value) -> Option<&str> {
    string(message, "forwarded_from")
}

pub fn sender(message: &Value) -> Option<&str> {
    string(message, "from")
}

/// 一个附件的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
    Audio,
    Other,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Image => "photo",
            Kind::Video => "video",
            Kind::Audio => "audio",
            Kind::Other => "file",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub kind: Kind,
    /// 导出里写的相对路径；没导出附件时是 `None`，`note` 写原话。
    pub path: Option<String>,
    pub note: Option<String>,
    pub name: Option<String>,
    pub mime: Option<String>,
    pub duration_sec: Option<f64>,
}

/// 导出设置里没勾附件，或附件太大、取不到时，Telegram Desktop 在字段里写这一类的话。
fn placeholder(value: &str) -> bool {
    value.starts_with("(File") && value.ends_with(')')
}

/// 消息带的附件：`photo` 和 `file`（缩略图不算）。
pub fn attachments(message: &Value) -> Vec<Attachment> {
    let mut out = Vec::new();
    for (key, kind) in [("photo", Some(Kind::Image)), ("file", None)] {
        let Some(value) = string(message, key) else {
            continue;
        };
        let kind = kind.unwrap_or_else(|| match string(message, "media_type") {
            Some("video_file" | "video_message" | "animation") => Kind::Video,
            Some("voice_message" | "audio_file") => Kind::Audio,
            _ => Kind::Other,
        });
        let missing = placeholder(value);
        out.push(Attachment {
            kind,
            path: (!missing).then(|| value.to_owned()),
            note: missing.then(|| value.to_owned()),
            name: string(message, "file_name").map(str::to_owned),
            mime: string(message, "mime_type").map(str::to_owned),
            duration_sec: message.get("duration_seconds").and_then(Value::as_f64),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flattens_entities() {
        let m = json!({"text": ["see ", {"type": "link", "text": "https://a.test/x"},
            " and ", {"type": "text_link", "text": "this page", "href": "https://b.test/y"},
            " ", {"type": "bold", "text": "now"}, {"type": "code", "text": "!"}]});
        assert_eq!(text(&m), "see https://a.test/x and https://b.test/y now!");
        assert_eq!(text(&json!({"text": "plain"})), "plain");
        assert_eq!(text(&json!({"text": ""})), "");
        let entities = json!({"text": "", "text_entities": [{"type": "plain", "text": "a"}, {"type": "link", "text": "https://c.test"}]});
        assert_eq!(text(&entities), "ahttps://c.test");
        assert_eq!(text(&json!({"text": 5})), "");
    }

    #[test]
    fn sole_links() {
        assert_eq!(
            sole_link(" https://a.test/x?y=1\n"),
            Some("https://a.test/x?y=1".into())
        );
        assert_eq!(sole_link("https://a.test/x and more"), None);
        assert_eq!(sole_link("ftp://a.test/x"), None);
        assert_eq!(sole_link("a.test/x"), None);
        assert_eq!(sole_link(""), None);
    }

    #[test]
    fn times() {
        let m = json!({"date": "2026-10-01T12:00:00", "date_unixtime": "1790856000"});
        assert_eq!(time(&m).unwrap().unwrap().as_str(), "2026-10-01T12:00:00Z");
        assert_eq!(time(&json!({"date": "2026-10-01T12:00:00"})), Ok(None));
        assert!(time(&json!({"date_unixtime": "soon"})).is_err());
    }

    #[test]
    fn ids_skip_service_messages() {
        assert_eq!(id(&json!({"id": 7, "type": "message"})), Some(7));
        assert_eq!(id(&json!({"id": 8, "type": "service"})), None);
        assert_eq!(id(&json!({"type": "message"})), None);
    }

    #[test]
    fn attachment_kinds() {
        let photo = attachments(&json!({"photo": "photos/a.jpg"}));
        assert_eq!(photo[0].kind, Kind::Image);
        assert_eq!(photo[0].path.as_deref(), Some("photos/a.jpg"));
        let voice = attachments(
            &json!({"file": "voice_messages/a.ogg", "media_type": "voice_message", "duration_seconds": 4}),
        );
        assert_eq!(
            (voice[0].kind, voice[0].duration_sec),
            (Kind::Audio, Some(4.0))
        );
        let doc = attachments(
            &json!({"file": "files/a.pdf", "file_name": "a.pdf", "mime_type": "application/pdf"}),
        );
        assert_eq!(doc[0].kind, Kind::Other);
        let gone = attachments(
            &json!({"photo": "(File not included. Change data exporting settings to download.)"}),
        );
        assert_eq!(gone[0].path, None);
        assert!(gone[0].note.is_some());
        assert!(attachments(&json!({"text": "x", "thumbnail": "t.jpg"})).is_empty());
    }
}
