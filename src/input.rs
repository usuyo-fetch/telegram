//! 打开交来的文件：单独的 `result.json`，或装着它的 zip。
//!
//! zip 里的条目只按名字读进内存或流进输出目录里我们自己起名的文件；条目名从不拼成要写的路径。
//! 名字有 `..`、以 `/` 开头、含反斜杠或控制字符的条目当没有。

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use zip::ZipArchive;

use crate::error::{Error, HTML_EXPORT, NOT_AN_EXPORT, TOO_LARGE};
use crate::limits::Limits;

const RESULT: &str = "result.json";
/// 交来的单个文件在报错里叫什么。
const PLAIN: &str = "input";

/// 打开后的样子：`result.json` 的字节，加上能按相对路径取附件的那一头。
pub struct Opened {
    pub json: Vec<u8>,
    /// `result.json` 在交来的文件里叫什么，报错时写。
    pub json_name: String,
    pub files: Files,
}

pub enum Files {
    /// 只有一个 `result.json`，附件不在手边。
    Absent,
    Zip(ZipFiles),
}

pub struct ZipFiles {
    archive: ZipArchive<File>,
    names: HashMap<String, usize>,
    /// `result.json` 所在的文件夹，带结尾的 `/`，可以是空的。
    dir: String,
}

/// 附件读出来是什么结果。
#[derive(Debug, PartialEq, Eq)]
pub enum Copied {
    Done(u64),
    /// 交来的文件里没有这个附件。
    Absent,
    /// 比上限大，没有留下文件。
    TooLarge,
}

pub fn open(path: &Path, limits: &Limits) -> Result<Opened, Error> {
    let mut file = File::open(path)?;
    let mut magic = [0u8; 4];
    let n = file.read(&mut magic)?;
    if n == 4 && magic.starts_with(b"PK") && matches!(&magic[2..], [3, 4] | [5, 6]) {
        return open_zip(file, limits);
    }
    let len = file.metadata()?.len();
    if len > limits.json_bytes {
        return Err(Error::unsupported(
            PLAIN,
            "(whole file)",
            format!(
                "the file is {len} bytes, over the {} byte limit",
                limits.json_bytes
            ),
            TOO_LARGE,
        ));
    }
    let mut json = magic[..n].to_vec();
    file.read_to_end(&mut json)?;
    if looks_like_html(&json) {
        return Err(Error::unsupported(
            PLAIN,
            "(whole file)",
            "the file is HTML, not JSON",
            HTML_EXPORT,
        ));
    }
    Ok(Opened {
        json,
        json_name: PLAIN.into(),
        files: Files::Absent,
    })
}

fn looks_like_html(bytes: &[u8]) -> bool {
    let head = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    head.iter()
        .find(|b| !b.is_ascii_whitespace())
        .is_some_and(|b| *b == b'<')
}

/// 名字能不能当 zip 里的一条正常条目：不会指到别处。
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('/')
        && !name.contains('\\')
        && !name.chars().any(char::is_control)
        && !name.split('/').any(|seg| seg == "..")
        && name.as_bytes().get(1) != Some(&b':')
}

/// 导出里写的附件路径（相对 `result.json`）：只认干净的相对路径。
pub fn safe_relative(rel: &str) -> bool {
    safe_name(rel) && !rel.split('/').any(|seg| seg.is_empty() || seg == ".")
}

fn open_zip(file: File, limits: &Limits) -> Result<Opened, Error> {
    let mut archive = ZipArchive::new(file).map_err(|e| {
        Error::unsupported(
            PLAIN,
            "(zip)",
            format!("not a readable zip file: {e}"),
            NOT_AN_EXPORT,
        )
    })?;
    if archive.len() > limits.zip_entries {
        return Err(Error::unsupported(
            PLAIN,
            "(zip entries)",
            format!(
                "the zip has {} entries, over the {} limit",
                archive.len(),
                limits.zip_entries
            ),
            TOO_LARGE,
        ));
    }
    let mut names = HashMap::new();
    for i in 0..archive.len() {
        // 名字读不出来（不是 UTF-8）的条目当没有
        if let Some(Ok(name)) = archive.name_for_index(i)
            && !name.ends_with('/')
        {
            names.entry(name.into_owned()).or_insert(i);
        }
    }
    // 最浅的、名字最小的那个 result.json
    let found = names
        .keys()
        .filter(|n| safe_name(n) && n.rsplit('/').next() == Some(RESULT))
        .min_by_key(|n| (n.matches('/').count(), n.as_str()))
        .cloned();
    let Some(json_name) = found else {
        let html = names.keys().any(|n| safe_name(n) && n.ends_with(".html"));
        return Err(Error::unsupported(
            PLAIN,
            "(zip entries)",
            "no result.json in the zip",
            if html { HTML_EXPORT } else { NOT_AN_EXPORT },
        ));
    };
    let index = names[&json_name];
    let json = read_entry(&mut archive, index, &json_name, limits.json_bytes)?;
    let dir = json_name[..json_name.len() - RESULT.len()].to_owned();
    Ok(Opened {
        json,
        json_name,
        files: Files::Zip(ZipFiles {
            archive,
            names,
            dir,
        }),
    })
}

fn read_entry(
    archive: &mut ZipArchive<File>,
    index: usize,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>, Error> {
    let entry = archive.by_index(index).map_err(|e| {
        Error::unsupported(
            name,
            "(zip entry)",
            format!("cannot read the entry: {e}"),
            NOT_AN_EXPORT,
        )
    })?;
    if entry.size() > limit {
        return Err(Error::unsupported(
            name,
            "(zip entry)",
            format!(
                "the entry is {} bytes, over the {limit} byte limit",
                entry.size()
            ),
            TOO_LARGE,
        ));
    }
    let mut bytes = Vec::new();
    entry.take(limit + 1).read_to_end(&mut bytes).map_err(|e| {
        Error::unsupported(
            name,
            "(zip entry)",
            format!("cannot read the entry: {e}"),
            NOT_AN_EXPORT,
        )
    })?;
    if bytes.len() as u64 > limit {
        return Err(Error::unsupported(
            name,
            "(zip entry)",
            format!("the entry is over the {limit} byte limit when unpacked"),
            TOO_LARGE,
        ));
    }
    Ok(bytes)
}

impl Files {
    /// 把导出里的 `rel`（相对 `result.json`）流进 `dest`（我们自己起名的新文件）。
    pub fn copy_to(&mut self, rel: &str, dest: &Path, limit: u64) -> Result<Copied, Error> {
        let Files::Zip(zip) = self else {
            return Ok(Copied::Absent);
        };
        if !safe_relative(rel) {
            return Ok(Copied::Absent);
        }
        let Some(&index) = zip.names.get(&format!("{}{rel}", zip.dir)) else {
            return Ok(Copied::Absent);
        };
        let Ok(entry) = zip.archive.by_index(index) else {
            return Ok(Copied::Absent);
        };
        if entry.size() > limit {
            return Ok(Copied::TooLarge);
        }
        let mut out = File::create_new(dest)?;
        let copied = std::io::copy(&mut entry.take(limit + 1), &mut out)?;
        out.flush()?;
        if copied > limit {
            drop(out);
            std::fs::remove_file(dest)?;
            return Ok(Copied::TooLarge);
        }
        Ok(Copied::Done(copied))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(safe_name("ChatExport/result.json"));
        assert!(!safe_name("../result.json"));
        assert!(!safe_name("a/../../result.json"));
        assert!(!safe_name("/etc/result.json"));
        assert!(!safe_name("a\\result.json"));
        assert!(!safe_name("C:/result.json"));
        assert!(!safe_name("a\0b/result.json"));
        assert!(safe_relative("photos/a b@1.jpg"));
        assert!(!safe_relative("photos//a.jpg"));
        assert!(!safe_relative("./a.jpg"));
        assert!(!safe_relative("../a.jpg"));
    }

    #[test]
    fn html_sniff() {
        assert!(looks_like_html(b"\xef\xbb\xbf  <!DOCTYPE html>"));
        assert!(!looks_like_html(b"{\"a\":1}"));
    }
}
