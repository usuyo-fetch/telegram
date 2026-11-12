//! 测试用的小工具：按宿主的方式起插件进程、拼 zip、读样例。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const DEAD_PROXY: &str = "http://127.0.0.1:9";

pub fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// 清空环境，代理指向一个关着的本机端口：插件万一发请求就会失败。
pub fn plugin(args: &[&str], input: Option<&Path>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_usuyo-fetch-telegram"));
    cmd.env_clear()
        .env("USUYO_PROTOCOL", "1")
        .env("USUYO_LOCALE", "zh-CN")
        .env("HTTP_PROXY", DEAD_PROXY)
        .env("HTTPS_PROXY", DEAD_PROXY)
        .env("ALL_PROXY", DEAD_PROXY)
        .args(args);
    if let Some(i) = input {
        cmd.env("USUYO_INPUT", i);
    }
    cmd.output().unwrap()
}

/// stdout 上只有一个 JSON 值。
pub fn envelope(out: &Output) -> serde_json::Value {
    let stdout = String::from_utf8(out.stdout.clone()).unwrap();
    assert_eq!(
        stdout.lines().count(),
        1,
        "stdout is one JSON value: {stdout}"
    );
    serde_json::from_str(&stdout).unwrap()
}

/// 按名字写一个 zip；名字原样写进去，包括带 `..` 的。
pub fn zip_of(path: &Path, entries: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, bytes) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

pub const JPEG: &[u8] = b"\xff\xd8\xff\xe0fake-jpeg";
pub const OGG: &[u8] = b"OggSfake-voice";
pub const MP4: &[u8] = b"\x00\x00\x00\x18ftypmp42fake-video";
pub const PDF: &[u8] = b"%PDF-1.4 fake";

/// 完整导出的 zip：`ChatExport_2026-11-01/` 下是 result.json 和附件。
pub fn export_zip(dir: &Path) -> PathBuf {
    let path = dir.join("ChatExport_2026-11-01.zip");
    zip_of(
        &path,
        &[
            ("ChatExport_2026-11-01/result.json", &fixture("result.json")),
            (
                "ChatExport_2026-11-01/photos/photo_1@03-09-2026_18-45-00.jpg",
                JPEG,
            ),
            (
                "ChatExport_2026-11-01/voice_messages/audio_1@05-09-2026_07-00-00.ogg",
                OGG,
            ),
            ("ChatExport_2026-11-01/video_files/clip.mp4", MP4),
            ("ChatExport_2026-11-01/video_files/clip.mp4_thumb.jpg", JPEG),
            ("ChatExport_2026-11-01/files/menu.pdf", PDF),
        ],
    );
    path
}

pub fn lone_json(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}
