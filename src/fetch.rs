//! `fetch <ref>`：把一条消息写进输出目录（协议 §3.6、§3.7、§3.9）。
//!
//! 输出里的文件名都是这里起的（`images/001.jpg`、`media/video-001.mp4`），导出里的路径只用来
//! 在 zip 里找条目。附件不在手边（只给了 `result.json`、导出设置里没勾附件、条目太大）时，
//! 记进 `missing` 和 `missingReason`，其余照样交出去。

use std::collections::BTreeSet;
use std::path::Path;

use usuyo_plugin_kit::protocol::{
    Access, Completeness, Evidence, FetchOutput, ItemKind, Media, MediaKind, OutputKind, RelPath,
    Rfc3339, TextFormat,
};

use crate::chat::{self, Chat};
use crate::error::Error;
use crate::input::Copied;
use crate::limits::Limits;
use crate::message::{self, Attachment, Kind};

pub struct Request<'a> {
    pub reference: &'a str,
    pub out: &'a Path,
    pub want: Option<BTreeSet<OutputKind>>,
    pub input: Option<&'a Path>,
    pub limits: &'a Limits,
}

/// 扩展名只留短的字母数字，别的一律换成这一类的常见扩展名。
fn extension(attachment: &Attachment) -> String {
    let from = |name: &str| {
        let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
        (!ext.is_empty() && ext.len() <= 8 && ext.bytes().all(|b| b.is_ascii_alphanumeric()))
            .then_some(ext)
    };
    attachment
        .path
        .as_deref()
        .and_then(from)
        .or_else(|| attachment.name.as_deref().and_then(from))
        .unwrap_or_else(|| {
            match attachment.kind {
                Kind::Image => "jpg",
                Kind::Video => "mp4",
                Kind::Audio => "ogg",
                Kind::Other => "bin",
            }
            .into()
        })
}

pub fn fetch(req: Request) -> Result<FetchOutput, Error> {
    if !req.out.is_absolute() || !req.out.is_dir() {
        return Err(Error::InvalidArgs(format!(
            "--out `{}` is not an existing absolute directory",
            req.out.display()
        )));
    }
    let (chat_index, id) = chat::parse_reference(req.reference)
        .ok_or_else(|| Error::InvalidArgs(format!("ref `{}` is not ours", req.reference)))?;
    let input = req
        .input
        .ok_or_else(|| Error::InvalidArgs("USUYO_INPUT is not set for a file import".into()))?;
    let mut loaded = chat::load(input, req.limits)?;
    let value = loaded
        .message(chat_index, id)?
        .ok_or_else(|| Error::NotFound(format!("message {id} is not in this file")))?;
    let wants = |k: OutputKind| req.want.as_ref().is_none_or(|w| w.contains(&k));
    // 文档没有自己的词表，跟着音视频走
    let wants_other = wants(OutputKind::Video) || wants(OutputKind::Audio);

    let mut images = Vec::new();
    let mut media = Vec::new();
    let mut missing = BTreeSet::new();
    let mut reasons = Vec::new();
    let mut counts = [0usize; 4];
    let mut used = 0u64;
    for attachment in message::attachments(&value) {
        let (out_kind, wanted) = match attachment.kind {
            Kind::Image => (Some(OutputKind::Images), wants(OutputKind::Images)),
            Kind::Video => (Some(OutputKind::Video), wants(OutputKind::Video)),
            Kind::Audio => (Some(OutputKind::Audio), wants(OutputKind::Audio)),
            Kind::Other => (None, wants_other),
        };
        if !wanted {
            continue;
        }
        let mut lose = |why: String| {
            missing.extend(out_kind);
            reasons.push(format!("{}: {why}", attachment.kind.label()));
        };
        let Some(path) = attachment.path.as_deref() else {
            lose(
                attachment
                    .note
                    .clone()
                    .unwrap_or_else(|| "not included in the export".into()),
            );
            continue;
        };
        let slot = attachment.kind as usize;
        counts[slot] += 1;
        let rel = match attachment.kind {
            Kind::Image => format!("images/{:03}.{}", counts[slot], extension(&attachment)),
            kind => format!(
                "media/{}-{:03}.{}",
                kind.label(),
                counts[slot],
                extension(&attachment)
            ),
        };
        std::fs::create_dir_all(req.out.join(rel.split_once('/').map_or("", |(d, _)| d)))?;
        let budget = req
            .limits
            .file_bytes
            .min(req.limits.files_total.saturating_sub(used));
        match loaded.files.copy_to(path, &req.out.join(&rel), budget)? {
            Copied::Done(bytes) => used += bytes,
            Copied::Absent => {
                lose("the file is not in what was given (the export folder is needed, not only result.json)".into());
                continue;
            }
            Copied::TooLarge => {
                lose(format!("the file is over the {budget} byte limit"));
                continue;
            }
        }
        let path = RelPath::try_from(rel).map_err(Error::Internal)?;
        match attachment.kind {
            Kind::Image => images.push(path),
            kind => media.push(Media {
                path,
                kind: match kind {
                    Kind::Video => MediaKind::Video,
                    Kind::Audio => MediaKind::Audio,
                    _ => MediaKind::Other,
                },
                mime: attachment.mime.clone(),
                lang: None,
                duration_sec: attachment.duration_sec,
                group: None,
            }),
        }
    }

    if wants(OutputKind::Meta) {
        let pretty =
            serde_json::to_string_pretty(&value).map_err(|e| Error::Internal(e.to_string()))?;
        std::fs::write(req.out.join("meta.json"), pretty)?;
    }
    let chat: &Chat = loaded
        .chat(chat_index)
        .ok_or_else(|| Error::NotFound("chat".into()))?;
    let text = message::text(&value);
    let forwarded = message::forwarded_from(&value).is_some();
    let meta = wants(OutputKind::Meta);
    let acted =
        message::time(&value).map_err(|p| Error::changed(&loaded.json_name, "date_unixtime", p))?;
    let output = FetchOutput {
        identity: chat::identity(chat, id),
        kind: ItemKind::Post,
        title: meta.then(|| chat::title(&text, &value)).flatten(),
        author: if meta {
            chat::author(chat, &value)
        } else {
            None
        },
        // 转发来的，`date` 是存进来的时间，不是原文发出的时间
        published_at: if meta && !forwarded { acted } else { None },
        text: (wants(OutputKind::Text) && !text.trim().is_empty()).then_some(text),
        text_format: Some(TextFormat::Plain),
        html: None,
        images,
        media,
        platform_tags: Vec::new(),
        comments: None,
        completeness: if reasons.is_empty() {
            Completeness::Full
        } else {
            Completeness::Partial
        },
        missing: missing.into_iter().collect(),
        missing_reason: (!reasons.is_empty()).then(|| reasons.join("; ")),
        evidence: Evidence {
            fetched_at: Rfc3339::from_timestamp(jiff::Timestamp::now()),
            access: Access::OfficialExport,
            site: None,
            upstream: None,
            status: None,
        },
        credentials_update: None,
    };
    Ok(output)
}
