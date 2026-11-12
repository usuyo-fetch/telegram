//! 从 `result.json` 里选出要读的聊天，列出消息（协议 §3.8、§3.9）。
//!
//! 两种形状：整个账号的导出（`chats.list[]`，只取 `type` 为 `saved_messages` 的），
//! 和单个聊天的导出（顶层就有 `messages`，不管它是哪种聊天）。
//! 消息先当 `RawValue` 存着，用到哪条才解析，内存不超过文件本身的两倍。

use std::cmp::Reverse;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;
use usuyo_plugin_kit::protocol::{
    Access, Author, Evidence, Identity, ListAction, ListItem, ListPage, PlatformId, Rfc3339,
};

use crate::error::{Error, NO_SAVED, NOT_AN_EXPORT, TOO_LARGE};
use crate::input::{self, Files};
use crate::limits::Limits;
use crate::message;

/// 一页最多多少条。
pub const PAGE: usize = 500;
const PLATFORM: &str = "telegram";
const SAVED: &str = "saved_messages";

pub struct Chat {
    pub id: Option<String>,
    pub saved: bool,
    messages: Vec<Box<RawValue>>,
}

/// 读进来的导出：选出的聊天，和能取附件的那一头。
pub struct Loaded {
    pub chats: Vec<Chat>,
    pub files: Files,
    pub json_name: String,
}

#[derive(Deserialize)]
struct Top {
    #[serde(rename = "type")]
    kind: Option<String>,
    id: Option<serde_json::Number>,
    messages: Option<Vec<Box<RawValue>>>,
    chats: Option<Box<RawValue>>,
}

#[derive(Deserialize)]
struct Wrapped {
    list: Vec<ChatWire>,
}

#[derive(Deserialize)]
struct ChatWire {
    #[serde(rename = "type")]
    kind: Option<String>,
    id: Option<serde_json::Number>,
    messages: Option<Vec<Box<RawValue>>>,
}

fn data_error(file: &str, field: &str, e: &serde_json::Error) -> Error {
    if e.is_data() {
        Error::changed(file, field, e.to_string())
    } else {
        Error::unsupported(file, field, format!("not valid JSON: {e}"), NOT_AN_EXPORT)
    }
}

fn first_byte(bytes: &[u8]) -> Option<u8> {
    bytes.iter().copied().find(|b| !b.is_ascii_whitespace())
}

pub fn load(path: &Path, limits: &Limits) -> Result<Loaded, Error> {
    let opened = input::open(path, limits)?;
    let file = opened.json_name.as_str();
    let bytes = opened
        .json
        .strip_prefix(b"\xef\xbb\xbf")
        .unwrap_or(&opened.json);
    if first_byte(bytes) != Some(b'{') {
        return Err(Error::unsupported(
            file,
            "(top level)",
            "the top level is not a JSON object",
            NOT_AN_EXPORT,
        ));
    }
    let top: Top =
        serde_json::from_slice(bytes).map_err(|e| data_error(file, "(top level)", &e))?;
    let wires = match (top.messages, top.chats) {
        (Some(messages), _) => vec![ChatWire {
            kind: top.kind,
            id: top.id,
            messages: Some(messages),
        }],
        (None, Some(chats)) => {
            let raw = chats.get().as_bytes();
            let parsed = if first_byte(raw) == Some(b'[') {
                serde_json::from_slice::<Vec<ChatWire>>(raw)
            } else {
                serde_json::from_slice::<Wrapped>(raw).map(|w| w.list)
            };
            parsed
                .map_err(|e| data_error(file, "chats.list", &e))?
                .into_iter()
                .filter(|c| c.kind.as_deref() == Some(SAVED))
                .collect()
        }
        (None, None) => {
            return Err(Error::unsupported(
                file,
                "messages, chats",
                "neither `messages` nor `chats` at the top level",
                NOT_AN_EXPORT,
            ));
        }
    };
    let mut chats = Vec::new();
    for wire in wires {
        let messages = wire.messages.unwrap_or_default();
        if messages.len() > limits.messages {
            return Err(Error::unsupported(
                file,
                "messages",
                format!(
                    "{} messages, over the {} limit",
                    messages.len(),
                    limits.messages
                ),
                TOO_LARGE,
            ));
        }
        chats.push(Chat {
            id: wire.id.map(|n| n.to_string()),
            saved: wire.kind.as_deref() == Some(SAVED),
            messages,
        });
    }
    if chats.is_empty() {
        return Err(Error::unsupported(
            file,
            "chats.list[].type",
            format!("no chat of type `{SAVED}`"),
            NO_SAVED,
        ));
    }
    Ok(Loaded {
        chats,
        files: opened.files,
        json_name: opened.json_name,
    })
}

impl Loaded {
    /// 找一条消息：`chat` 是 `list` 给的聊天序号。
    pub fn message(&self, chat: usize, id: i64) -> Result<Option<Value>, Error> {
        let Some(c) = self.chats.get(chat) else {
            return Ok(None);
        };
        for raw in &c.messages {
            let Ok(value) = serde_json::from_str::<Value>(raw.get()) else {
                continue;
            };
            if message::id(&value) == Some(id) {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    pub fn chat(&self, chat: usize) -> Option<&Chat> {
        self.chats.get(chat)
    }
}

/// `ref`：`m/<聊天序号>/<消息 id>`。
pub fn reference(chat: usize, id: i64) -> String {
    format!("m/{chat}/{id}")
}

pub fn parse_reference(reference: &str) -> Option<(usize, i64)> {
    let rest = reference.strip_prefix("m/")?;
    let (chat, id) = rest.split_once('/')?;
    Some((chat.parse().ok()?, id.parse().ok()?))
}

/// 这条消息在平台上的身份：聊天 id 和消息 id 合起来；导出里没有聊天 id 时没有身份。
pub fn identity(chat: &Chat, id: i64) -> Option<Identity> {
    Some(Identity::Platform {
        platform: PlatformId::try_from(PLATFORM).ok()?,
        content_id: format!("{}:{id}", chat.id.as_ref()?),
        url: None,
    })
}

/// 作者：转发来的写转发来源；单个聊天的导出里自己写的消息写发送者；Saved Messages 里自己写的不写。
pub fn author(chat: &Chat, value: &Value) -> Option<Author> {
    let name = message::forwarded_from(value).or_else(|| {
        if chat.saved {
            None
        } else {
            message::sender(value)
        }
    })?;
    Some(Author {
        name: name.to_owned(),
        platform_id: None,
        url: None,
    })
}

pub fn title(text: &str, value: &Value) -> Option<String> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .or_else(|| {
            let name = value.get("file_name")?.as_str()?.trim();
            (!name.is_empty()).then_some(name)
        })?;
    Some(line.chars().take(100).collect())
}

struct Entry {
    id: i64,
    chat: usize,
    item: ListItem,
}

fn entries(loaded: &Loaded) -> Result<Vec<Entry>, Error> {
    let mut out = Vec::new();
    let mut unreadable = 0usize;
    let mut first_problem: Option<String> = None;
    let mut seen = 0usize;
    for (index, chat) in loaded.chats.iter().enumerate() {
        for raw in &chat.messages {
            let Ok(value) = serde_json::from_str::<Value>(raw.get()) else {
                seen += 1;
                unreadable += 1;
                first_problem.get_or_insert("a message is not valid JSON".into());
                continue;
            };
            let Some(id) = message::id(&value) else {
                // 服务消息，或没有 id 的
                if value.get("type").and_then(Value::as_str) == Some("service") {
                    continue;
                }
                seen += 1;
                unreadable += 1;
                first_problem.get_or_insert("a message has no integer `id`".into());
                continue;
            };
            seen += 1;
            let acted_at = match message::time(&value) {
                Ok(t) => t,
                Err(problem) => {
                    unreadable += 1;
                    first_problem.get_or_insert(format!("message {id}: {problem}"));
                    continue;
                }
            };
            let text = message::text(&value);
            let has_files = !message::attachments(&value).is_empty();
            let author = author(chat, &value);
            let item = match message::sole_link(&text).filter(|_| !has_files) {
                // 整条就是一个链接：交给这个链接所属平台的插件去取
                Some(url) => ListItem {
                    url: Some(url),
                    reference: None,
                    identity: None,
                    title: None,
                    author,
                    published_at: None,
                    action: Some(ListAction::Favorite),
                    acted_at,
                },
                None => ListItem {
                    url: None,
                    reference: Some(reference(index, id)),
                    identity: identity(chat, id),
                    title: title(&text, &value),
                    author,
                    published_at: None,
                    action: Some(ListAction::Favorite),
                    acted_at,
                },
            };
            out.push(Entry {
                id,
                chat: index,
                item,
            });
        }
    }
    if unreadable > 0 {
        eprintln!(
            "{unreadable} messages were skipped: {}",
            first_problem.as_deref().unwrap_or("")
        );
        if unreadable == seen {
            return Err(Error::changed(
                &loaded.json_name,
                "messages[]",
                format!(
                    "none of the {seen} messages is readable: {}",
                    first_problem.unwrap_or_default()
                ),
            ));
        }
    }
    // 新的在前：同一个聊天里消息 id 随时间增大；几个聊天时 id 不能比，按时间
    if loaded.chats.len() == 1 {
        out.sort_by_key(|e| Reverse(e.id));
    } else {
        out.sort_by_key(|e| {
            (
                Reverse(e.item.acted_at.as_ref().map(|t| t.timestamp())),
                Reverse(e.id),
                e.chat,
            )
        });
    }
    Ok(out)
}

/// 列一页。`cursor` 是从第几条开始（插件自己的不透明字符串）。
pub fn list(
    source: &str,
    input: Option<&Path>,
    cursor: Option<&str>,
    limits: &Limits,
) -> Result<ListPage, Error> {
    if !source.starts_with("file:") {
        return Err(Error::InvalidArgs(format!(
            "source `{source}` is not a file import"
        )));
    }
    let input = input
        .ok_or_else(|| Error::InvalidArgs("USUYO_INPUT is not set for a file import".into()))?;
    let start = match cursor {
        None => 0,
        Some(c) => c
            .parse::<usize>()
            .map_err(|_| Error::InvalidArgs(format!("cursor `{c}` is not ours")))?,
    };
    let fetched_at = Rfc3339::from_timestamp(jiff::Timestamp::now());
    let all = entries(&load(input, limits)?)?;
    let end = start.saturating_add(PAGE).min(all.len());
    let items = all
        .get(start..end)
        .unwrap_or_default()
        .iter()
        .map(|e| e.item.clone())
        .collect();
    Ok(ListPage {
        items,
        next_cursor: (end < all.len()).then(|| end.to_string()),
        evidence: Evidence {
            fetched_at,
            access: Access::OfficialExport,
            site: None,
            upstream: None,
            status: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references() {
        assert_eq!(parse_reference(&reference(2, 4041)), Some((2, 4041)));
        assert_eq!(parse_reference("m/x/1"), None);
        assert_eq!(parse_reference("msg/1/1"), None);
        assert_eq!(parse_reference("m/1"), None);
    }
}
