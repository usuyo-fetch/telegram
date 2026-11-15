//! 全部离线：列和取都只读本机的文件，错误在读文件时就答。前半起插件进程（清空环境，代理指向关着的端口），
//! 后半（文件太大）用小的上限直接调库函数，不必真造几百 MB。

mod common;

use usuyo_plugin::{
    CallLimits, CallOptions, FetchInput, FetchRequest, HostConfig, ListRequest, PluginHost,
    discover,
};

use std::collections::BTreeSet;
use std::path::PathBuf;

use common::{JPEG, MP4, OGG, PDF, envelope, export_zip, fixture, lone_json, plugin, zip_of};
use serde_json::{Value, json};
use usuyo_fetch_telegram::fetch::{Request, fetch as fetch_message};
use usuyo_fetch_telegram::{Error, Limits, chat};
use usuyo_plugin::protocol::{
    Access, Account, FetchInputKind, FetchOutput, ListOrder, ListPage, Login, PluginKind,
    PluginManifest,
};

fn manifest() -> PluginManifest {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("usuyo-plugin.yaml"),
    )
    .unwrap();
    let parsed = PluginManifest::from_yaml(&text).unwrap();
    assert!(
        parsed.unknown_fields.is_empty(),
        "{:?}",
        parsed.unknown_fields
    );
    parsed.value
}

#[test]
fn the_manifest_declares_the_export_method() {
    let m = manifest();
    let PluginKind::Fetch(spec) = &m.kind else {
        panic!("not a fetch plugin");
    };
    assert_eq!(
        spec.platforms
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>(),
        ["telegram"]
    );
    assert_eq!(spec.methods.len(), 1);
    let method = spec.method("export").unwrap();
    assert!(method.built);
    assert_eq!(
        (method.access, method.login, method.account),
        (Access::OfficialExport, Login::None, Account::None)
    );
    let list = method.capabilities.list.as_ref().unwrap();
    assert_eq!(list.order, ListOrder::NewestFirst);
    assert!(list.files.iter().any(|f| f == "result.json") && list.reads.is_some());
    let fetch = method.capabilities.fetch.as_ref().unwrap();
    assert_eq!(
        fetch.input.iter().collect::<Vec<_>>(),
        [&FetchInputKind::Ref]
    );
    assert!(spec.match_patterns.is_empty());
}

fn list(args_source: &str, input: &std::path::Path, cursor: Option<&str>) -> Value {
    let mut args = vec!["list", args_source, "--method", "export"];
    if let Some(c) = cursor {
        args.extend(["--cursor", c]);
    }
    let out = plugin(&args, Some(input));
    assert!(out.status.success(), "{out:?}");
    envelope(&out)
}

fn assert_saved_messages(value: &Value) {
    assert_eq!(value["ok"], true);
    let mut page = value.clone();
    page.as_object_mut().unwrap().remove("ok");
    let parsed: ListPage = serde_json::from_value(page).unwrap();
    assert_eq!(parsed.next_cursor, None);
    let items = value["items"].as_array().unwrap();
    // 服务消息和另一个聊天的消息不在；新的在前
    let rows: Vec<(Option<&str>, Option<&str>)> = items
        .iter()
        .map(|i| (i["ref"].as_str(), i["url"].as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            (Some("m/0/9"), None),
            (Some("m/0/8"), None),
            (Some("m/0/7"), None),
            (Some("m/0/6"), None),
            (Some("m/0/5"), None),
            (None, Some("https://example.org/post/9")),
            (None, Some("https://example.com/brew-guide?ref=tg")),
            (Some("m/0/1"), None),
        ]
    );
    // 整条是链接的：只有 url，没有 ref 和身份
    for i in [5, 6] {
        assert!(
            items[i].get("ref").is_none() && items[i].get("identity").is_none(),
            "{}",
            items[i]
        );
    }
    // 带 ref 的：身份是聊天 id 加消息 id；标题是第一行；时间是 UTC
    assert_eq!(
        items[7]["identity"],
        json!({"platform": "telegram", "contentId": "1000001:1"})
    );
    assert_eq!(items[7]["title"], "Note to self: grind the beans coarser");
    assert_eq!(items[7]["actedAt"], "2026-09-01T09:00:00Z");
    assert_eq!(items[7]["action"], "favorite");
    assert!(items[7].get("author").is_none());
    // 转发来的写来源；链接实体压平，不带格式
    assert_eq!(items[4]["author"]["name"], "Coffee Channel");
    assert_eq!(
        items[4]["title"],
        "Dialling in V60 https://example.net/recipe #coffee"
    );
    assert_eq!(items[2]["author"]["name"], "Roaster Vlog");
    // 只有文件的：标题用文件名，没有文件名就没有标题
    assert_eq!(items[1]["title"], "menu.pdf");
    assert!(items[3].get("title").is_none());
    assert_eq!(value["evidence"]["access"], "official-export");
}

#[test]
fn lists_saved_messages_from_a_lone_result_json() {
    let dir = tempfile::tempdir().unwrap();
    let json = lone_json(dir.path(), "input", &fixture("result.json"));
    assert_saved_messages(&list("file:result.json", &json, None));
}

#[test]
fn lists_saved_messages_from_the_zipped_export_folder() {
    let dir = tempfile::tempdir().unwrap();
    let zip = export_zip(dir.path());
    assert_saved_messages(&list("file:ChatExport_2026-11-01.zip", &zip, None));
}

#[test]
fn a_single_chat_export_lists_that_chat() {
    let dir = tempfile::tempdir().unwrap();
    let json = lone_json(dir.path(), "input.json", &fixture("chat-export.json"));
    let value = list("file:result.json", &json, None);
    let items = value["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["ref"], "m/0/11");
    assert_eq!(items[1]["ref"], "m/0/10");
    // 不是 Saved Messages：别人发的写发送者；自己发的也写
    assert_eq!(items[1]["author"]["name"], "Alice");
    assert_eq!(items[0]["author"]["name"], "Test");
    assert_eq!(items[1]["identity"]["contentId"], "3000003:10");
}

#[test]
fn list_pages_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let messages: Vec<Value> = (1..=1200)
        .map(|n| json!({"id": n, "type": "message", "date_unixtime": (1_788_000_000 + n).to_string(), "text": format!("note {n}")}))
        .collect();
    let export =
        json!({"name": "Saved Messages", "type": "saved_messages", "id": 7, "messages": messages});
    let path = lone_json(dir.path(), "input", export.to_string().as_bytes());
    let mut cursor: Option<String> = None;
    let mut titles = Vec::new();
    let mut pages = 0;
    loop {
        let value = list("file:r.json", &path, cursor.as_deref());
        pages += 1;
        titles.extend(
            value["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| i["title"].as_str().unwrap().to_owned()),
        );
        match value["nextCursor"].as_str() {
            Some(c) => cursor = Some(c.to_owned()),
            None => break,
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(titles.len(), 1200);
    assert_eq!(titles[0], "note 1200");
    assert_eq!(titles[1199], "note 1");
}

fn fetch(
    reference: &str,
    input: &std::path::Path,
    out: &std::path::Path,
    want: Option<&str>,
) -> Value {
    let mut args = vec![
        "fetch",
        reference,
        "--out",
        out.to_str().unwrap(),
        "--method",
        "export",
    ];
    if let Some(w) = want {
        args.extend(["--want", w]);
    }
    envelope(&plugin(&args, Some(input)))
}

fn files_under(dir: &std::path::Path) -> Vec<String> {
    fn walk(base: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                out.push(
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn parsed(value: &Value) -> FetchOutput {
    let mut v = value.clone();
    v.as_object_mut().unwrap().remove("ok");
    serde_json::from_value(v).unwrap()
}

#[test]
fn fetch_a_message_with_a_photo_from_the_zip() {
    let dir = tempfile::tempdir().unwrap();
    let zip = export_zip(dir.path());
    let out = tempfile::tempdir().unwrap();
    let value = fetch("m/0/5", &zip, out.path(), None);
    assert_eq!(value["ok"], true, "{value}");
    let output = parsed(&value);
    assert_eq!(value["identity"]["contentId"], "1000001:5");
    assert_eq!(value["kind"], "post");
    assert_eq!(value["author"]["name"], "Coffee Channel");
    // 转发来的：date 是存进来的时间，不当发布时间
    assert!(value.get("publishedAt").is_none());
    assert_eq!(
        value["text"],
        "Dialling in V60 https://example.net/recipe #coffee"
    );
    assert_eq!(value["images"], json!(["images/001.jpg"]));
    assert_eq!(value["completeness"], "full");
    assert_eq!(value["missing"], json!([]));
    assert_eq!(output.images.len(), 1);
    assert_eq!(
        std::fs::read(out.path().join("images/001.jpg")).unwrap(),
        JPEG
    );
    assert_eq!(files_under(out.path()), ["images/001.jpg", "meta.json"]);
    // meta.json 是原样的那条消息
    let meta: Value =
        serde_json::from_slice(&std::fs::read(out.path().join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["id"], 5);
    assert_eq!(meta["forwarded_from"], "Coffee Channel");
    assert_eq!(meta["text"][3]["href"], "https://example.net/recipe");
    assert_eq!(value["evidence"]["access"], "official-export");
}

#[test]
fn fetch_voice_video_and_document() {
    let dir = tempfile::tempdir().unwrap();
    let zip = export_zip(dir.path());
    for (reference, rel, bytes, kind) in [
        ("m/0/6", "media/audio-001.ogg", OGG, "audio"),
        ("m/0/7", "media/video-001.mp4", MP4, "video"),
        ("m/0/8", "media/file-001.pdf", PDF, "other"),
    ] {
        let out = tempfile::tempdir().unwrap();
        let value = fetch(reference, &zip, out.path(), None);
        assert_eq!(value["ok"], true, "{reference}: {value}");
        assert_eq!(value["media"][0]["path"], rel);
        assert_eq!(value["media"][0]["kind"], kind);
        assert_eq!(std::fs::read(out.path().join(rel)).unwrap(), bytes);
        // 缩略图不取
        let mut expected = ["meta.json", rel];
        expected.sort();
        assert_eq!(files_under(out.path()), expected);
        assert_eq!(value["completeness"], "full");
    }
    let out = tempfile::tempdir().unwrap();
    let value = fetch("m/0/6", &zip, out.path(), None);
    assert_eq!(value["media"][0]["mime"], "audio/ogg");
    assert_eq!(value["media"][0]["durationSec"], 4.0);
    // 转发来的视频带文字
    let out = tempfile::tempdir().unwrap();
    assert_eq!(
        fetch("m/0/7", &zip, out.path(), None)["text"],
        "Pour-over clip"
    );
}

#[test]
fn fetch_a_note_published_by_the_user() {
    let dir = tempfile::tempdir().unwrap();
    let zip = export_zip(dir.path());
    let out = tempfile::tempdir().unwrap();
    let value = fetch("m/0/1", &zip, out.path(), None);
    assert_eq!(value["publishedAt"], "2026-09-01T09:00:00Z");
    assert_eq!(
        value["text"],
        "Note to self: grind the beans coarser\nsecond line"
    );
    assert_eq!(value["images"], json!([]));
    assert_eq!(value["media"], json!([]));
}

#[test]
fn what_is_not_in_the_input_is_missing_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    // 只给了 result.json：附件都不在手边
    let json = lone_json(dir.path(), "input", &fixture("result.json"));
    let out = tempfile::tempdir().unwrap();
    let value = fetch("m/0/5", &json, out.path(), None);
    assert_eq!(value["ok"], true);
    assert_eq!(value["completeness"], "partial");
    assert_eq!(value["missing"], json!(["images"]));
    assert!(value["missingReason"].as_str().unwrap().contains("photo:"));
    assert_eq!(
        value["text"],
        "Dialling in V60 https://example.net/recipe #coffee"
    );
    assert_eq!(files_under(out.path()), ["meta.json"]);
    // 导出设置里没勾附件：照写原话
    let out = tempfile::tempdir().unwrap();
    let value = fetch("m/0/9", &json, out.path(), None);
    assert_eq!(value["missing"], json!(["images"]));
    assert!(
        value["missingReason"]
            .as_str()
            .unwrap()
            .contains("File not included")
    );
    // 文档没有自己的词表：只在原因里
    let out = tempfile::tempdir().unwrap();
    let value = fetch("m/0/8", &json, out.path(), None);
    assert_eq!(value["missing"], json!([]));
    assert_eq!(value["completeness"], "partial");
    assert!(
        value["missingReason"]
            .as_str()
            .unwrap()
            .starts_with("file:")
    );
    // zip 里少了附件
    let short = dir.path().join("short.zip");
    zip_of(&short, &[("result.json", &fixture("result.json"))]);
    let out = tempfile::tempdir().unwrap();
    assert_eq!(
        fetch("m/0/6", &short, out.path(), None)["missing"],
        json!(["media:audio"])
    );
}

#[test]
fn want_limits_what_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let zip = export_zip(dir.path());
    let out = tempfile::tempdir().unwrap();
    let value = fetch("m/0/5", &zip, out.path(), Some("text"));
    assert_eq!(value["images"], json!([]));
    assert_eq!(
        value["text"],
        "Dialling in V60 https://example.net/recipe #coffee"
    );
    assert!(files_under(out.path()).is_empty());
    let out = tempfile::tempdir().unwrap();
    let value = fetch("m/0/5", &zip, out.path(), Some("images,meta"));
    assert!(value.get("text").is_none());
    assert_eq!(value["images"], json!(["images/001.jpg"]));
    assert_eq!(files_under(out.path()), ["images/001.jpg", "meta.json"]);
}

#[test]
fn fetch_refuses_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let zip = export_zip(dir.path());
    let out = tempfile::tempdir().unwrap();
    let run = |reference: &str, input: Option<&std::path::Path>, out_arg: &str| {
        envelope(&plugin(
            &["fetch", reference, "--out", out_arg, "--method", "export"],
            input,
        ))
    };
    let out_arg = out.path().to_str().unwrap();
    assert_eq!(
        run("m/0/999", Some(&zip), out_arg)["error"]["code"],
        "not-found"
    );
    assert_eq!(
        run("m/3/1", Some(&zip), out_arg)["error"]["code"],
        "not-found"
    );
    assert_eq!(
        run("nope", Some(&zip), out_arg)["error"]["code"],
        "invalid-args"
    );
    assert_eq!(run("m/0/1", None, out_arg)["error"]["code"], "invalid-args");
    assert_eq!(
        run("m/0/1", Some(&zip), "relative/dir")["error"]["code"],
        "invalid-args"
    );
    // 方法不对
    let other = envelope(&plugin(
        &["fetch", "m/0/1", "--out", out_arg, "--method", "guest"],
        Some(&zip),
    ));
    assert_eq!(other["error"]["code"], "invalid-args");
    assert!(files_under(out.path()).is_empty());
}

fn failure(input: &[u8], name: &str) -> Value {
    let dir = tempfile::tempdir().unwrap();
    let path = lone_json(dir.path(), name, input);
    let out = plugin(&["list", "file:x", "--method", "export"], Some(&path));
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    envelope(&out)
}

#[test]
fn files_that_are_not_exports_are_answered_with_a_detail() {
    // 不是 JSON
    let v = failure(b"just some text", "input");
    assert_eq!(v["error"]["code"], "unsupported-input");
    assert!(
        v["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("file: input")
    );
    // JSON 但是数组
    let v = failure(b"[1, 2]", "input");
    assert_eq!(v["error"]["code"], "unsupported-input");
    assert!(
        v["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("field: (top level)")
    );
    // 截断的 JSON
    let v = failure(b"{\"name\": \"x\", \"messages\": [{\"id\": 1", "input");
    assert_eq!(v["error"]["code"], "unsupported-input");
    // 是 JSON 对象，但不是导出
    let v = failure(b"{\"hello\": 1}", "input");
    assert_eq!(v["error"]["code"], "unsupported-input");
    assert!(
        v["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("field: messages, chats")
    );
    // HTML 的导出：提示换 JSON
    let v = failure(
        b"<!DOCTYPE html><html><body>messages.html</body></html>",
        "input",
    );
    assert_eq!(v["error"]["code"], "unsupported-input");
    assert!(v["error"]["hint"].as_str().unwrap().contains("JSON"));
    // 完整导出里没有 Saved Messages
    let v = failure(
        json!({"chats": {"list": [{"name": "A", "type": "personal_chat", "id": 1, "messages": []}]}}).to_string().as_bytes(),
        "input",
    );
    assert_eq!(v["error"]["code"], "unsupported-input");
    assert!(
        v["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("chats.list[].type")
    );
    assert!(
        v["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("Saved Messages")
    );
    // 字段的形状变了：messages 不是数组
    let v = failure(
        json!({"name": "A", "type": "saved_messages", "messages": "many"})
            .to_string()
            .as_bytes(),
        "input",
    );
    assert_eq!(v["error"]["code"], "upstream-changed");
    assert!(
        v["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("file: input")
    );
    // 每条消息都没有 id
    let v = failure(
        json!({"name": "A", "type": "saved_messages", "id": 1, "messages": [{"type": "message", "text": "a"}, {"type": "message", "text": "b"}]})
            .to_string()
            .as_bytes(),
        "input",
    );
    assert_eq!(v["error"]["code"], "upstream-changed");
    assert!(
        v["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("field: messages[]")
    );
    // 每条消息的时间都读不出
    let v = failure(
        json!({"name": "A", "type": "saved_messages", "id": 1, "messages": [{"id": 1, "type": "message", "date_unixtime": "soon", "text": "a"}]})
            .to_string()
            .as_bytes(),
        "input",
    );
    assert_eq!(v["error"]["code"], "upstream-changed");
    assert!(v["error"]["detail"].is_string());
}

#[test]
fn zips_that_are_not_exports() {
    let dir = tempfile::tempdir().unwrap();
    let none = dir.path().join("a.zip");
    zip_of(&none, &[("readme.txt", b"hi")]);
    let out = plugin(&["list", "file:a.zip", "--method", "export"], Some(&none));
    let v = envelope(&out);
    assert_eq!(v["error"]["code"], "unsupported-input");
    assert!(
        v["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("no result.json")
    );
    // 只有 HTML 的导出
    let html = dir.path().join("b.zip");
    zip_of(&html, &[("ChatExport/messages.html", b"<html></html>")]);
    let v = envelope(&plugin(
        &["list", "file:b.zip", "--method", "export"],
        Some(&html),
    ));
    assert!(v["error"]["hint"].as_str().unwrap().contains("JSON"));
    // 坏掉的 zip
    let broken = dir.path().join("c.zip");
    let mut bytes = std::fs::read(export_zip(dir.path())).unwrap();
    bytes.truncate(bytes.len() / 2);
    std::fs::write(&broken, bytes).unwrap();
    let v = envelope(&plugin(
        &["list", "file:c.zip", "--method", "export"],
        Some(&broken),
    ));
    assert_eq!(v["error"]["code"], "unsupported-input");
    // result.json 本身坏了
    let bad = dir.path().join("d.zip");
    zip_of(&bad, &[("x/result.json", b"{\"messages\": 5}")]);
    let v = envelope(&plugin(
        &["list", "file:d.zip", "--method", "export"],
        Some(&bad),
    ));
    assert_eq!(v["error"]["code"], "upstream-changed");
    assert!(
        v["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("file: x/result.json")
    );
}

#[test]
fn a_zip_with_hostile_entry_names_writes_nothing_outside_the_output() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("work");
    std::fs::create_dir(&base).unwrap();
    // 导出里一条消息的附件路径也指向 zip 外面
    let export = json!({"name": "S", "type": "saved_messages", "id": 1, "messages": [
        {"id": 1, "type": "message", "date_unixtime": "1788253200", "text": "a", "photo": "../../secret.jpg"},
        {"id": 2, "type": "message", "date_unixtime": "1788253201", "text": "b", "photo": "photos/ok.jpg"},
        {"id": 3, "type": "message", "date_unixtime": "1788253202", "text": "c", "photo": "/etc/hosts"}
    ]});
    let zip = dir.path().join("hostile.zip");
    zip_of(
        &zip,
        &[
            ("../../escape-1.txt", b"x"),
            ("/tmp/usuyo-escape-2.txt", b"x"),
            ("a/../../result.json", export.to_string().as_bytes()),
            ("..\\..\\win.txt", b"x"),
            ("secret.jpg", JPEG),
            ("ok/../photos/ok.jpg", JPEG),
            ("pack/result.json", export.to_string().as_bytes()),
            ("pack/photos/ok.jpg", JPEG),
        ],
    );
    let v = envelope(&plugin(
        &["list", "file:hostile.zip", "--method", "export"],
        Some(&zip),
    ));
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["items"].as_array().unwrap().len(), 3);
    for (reference, missing) in [("m/0/1", true), ("m/0/2", false), ("m/0/3", true)] {
        let out = tempfile::tempdir().unwrap();
        let value = fetch(reference, &zip, out.path(), None);
        assert_eq!(value["ok"], true);
        assert_eq!(
            value["missing"] == json!(["images"]),
            missing,
            "{reference}: {value}"
        );
        assert!(
            files_under(out.path())
                .iter()
                .all(|f| f == "meta.json" || f == "images/001.jpg")
        );
    }
    // 只有名字有问题的 result.json：不当它存在
    let only = dir.path().join("only.zip");
    zip_of(&only, &[("../result.json", export.to_string().as_bytes())]);
    let v = envelope(&plugin(
        &["list", "file:only.zip", "--method", "export"],
        Some(&only),
    ));
    assert_eq!(v["error"]["code"], "unsupported-input");
    // 没有东西写到工作目录或它的上一级
    assert_eq!(std::fs::read_dir(&base).unwrap().count(), 0);
    let mut siblings: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    siblings.sort();
    assert_eq!(siblings, ["hostile.zip", "only.zip", "work"]);
    assert!(!std::path::Path::new("/tmp/usuyo-escape-2.txt").exists());
}

#[test]
fn health_and_canonicalize_send_no_request() {
    let v = envelope(&plugin(&["health"], None));
    assert_eq!(v["ok"], true);
    assert_eq!(v["ready"], true);
    assert_eq!(v["protocol"], 1);
    let v = envelope(&plugin(&["health", "--probe"], None));
    assert_eq!(v["ready"], true);
    let v = envelope(&plugin(&["canonicalize", "https://t.me/someone/1"], None));
    assert_eq!(v, json!({"ok": true, "match": false}));
    let out = plugin(&["unfavorite", "x"], None);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn list_refuses_what_is_not_a_file_import() {
    let dir = tempfile::tempdir().unwrap();
    let json = lone_json(dir.path(), "input", &fixture("result.json"));
    let v = envelope(&plugin(
        &["list", "telegram:saved", "--method", "export"],
        Some(&json),
    ));
    assert_eq!(v["error"]["code"], "invalid-args");
    let v = envelope(&plugin(&["list", "file:x", "--method", "export"], None));
    assert_eq!(v["error"]["code"], "invalid-args");
    let v = envelope(&plugin(
        &["list", "file:x", "--method", "export", "--cursor", "abc"],
        Some(&json),
    ));
    assert_eq!(v["error"]["code"], "invalid-args");
    let v = envelope(&plugin(
        &["list", "file:x", "--method", "other"],
        Some(&json),
    ));
    assert_eq!(v["error"]["code"], "invalid-args");
}

fn small() -> Limits {
    Limits {
        zip_entries: 4,
        json_bytes: 2048,
        file_bytes: 8,
        files_total: 64,
        messages: 3,
    }
}

fn unsupported_detail(e: Error) -> String {
    match e {
        Error::Unsupported { detail, .. } => detail,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_json_over_the_size_limit_is_refused_without_reading_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = lone_json(dir.path(), "input", &fixture("result.json"));
    let e = chat::list("file:x", Some(&path), None, &small()).unwrap_err();
    assert!(unsupported_detail(e).contains("over the 2048 byte limit"));
}

#[test]
fn a_zip_entry_over_the_size_limit_and_too_many_entries() {
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.zip");
    zip_of(&big, &[("r/result.json", &fixture("result.json"))]);
    let e = chat::list("file:x", Some(&big), None, &small()).unwrap_err();
    assert!(unsupported_detail(e).contains("file: r/result.json"));
    let many = dir.path().join("many.zip");
    zip_of(
        &many,
        &[
            ("a", b"1"),
            ("b", b"1"),
            ("c", b"1"),
            ("d", b"1"),
            ("e", b"1"),
        ],
    );
    let e = chat::list("file:x", Some(&many), None, &small()).unwrap_err();
    assert!(unsupported_detail(e).contains("5 entries"));
}

#[test]
fn too_many_messages() {
    let dir = tempfile::tempdir().unwrap();
    let messages: Vec<_> = (1..=4)
        .map(|n| json!({"id": n, "type": "message", "text": "a"}))
        .collect();
    let export = json!({"name": "S", "type": "saved_messages", "id": 1, "messages": messages});
    let path = lone_json(dir.path(), "input", export.to_string().as_bytes());
    let e = chat::list("file:x", Some(&path), None, &small()).unwrap_err();
    assert!(unsupported_detail(e).contains("4 messages"));
}

#[test]
fn an_attachment_over_the_limit_is_missing_and_leaves_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let zip = export_zip(dir.path());
    let out = tempfile::tempdir().unwrap();
    let limits = Limits {
        file_bytes: JPEG.len() as u64 - 1,
        ..Limits::default()
    };
    let output = fetch_message(Request {
        reference: "m/0/5",
        out: out.path(),
        want: None::<BTreeSet<_>>,
        input: Some(&zip),
        limits: &limits,
    })
    .unwrap();
    assert!(output.images.is_empty());
    assert_eq!(output.missing.len(), 1);
    assert!(output.missing_reason.unwrap().contains("byte limit"));
    assert!(!out.path().join("images/001.jpg").exists());
    // 刚好够的
    let limits = Limits {
        file_bytes: JPEG.len() as u64,
        ..Limits::default()
    };
    let out = tempfile::tempdir().unwrap();
    let output = fetch_message(Request {
        reference: "m/0/5",
        out: out.path(),
        want: None,
        input: Some(&zip),
        limits: &limits,
    })
    .unwrap();
    assert_eq!(output.images.len(), 1);
}

// ---- 清单经宿主装上，列（和取）经宿主调一次：输出要过宿主的 schema 和路径检查。
// 文件按宿主的方式交来：副本固定叫 `input.<扩展名>`，路径放在 `USUYO_INPUT`，来源是 `file:<显示名>`。

const HOST_BIN: &str = env!("CARGO_BIN_EXE_usuyo-fetch-telegram");

/// 清单装成宿主认的样子：`<root>/usuyo-fetch/telegram/<version>/`。
fn host() -> (tempfile::TempDir, PluginHost) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("plugins/usuyo-fetch/telegram/0.1.0");
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("usuyo-plugin.yaml");
    std::fs::copy(manifest, dir.join("usuyo-plugin.yaml")).unwrap();
    std::fs::copy(HOST_BIN, dir.join("usuyo-fetch-telegram")).unwrap();
    let found = discover(&tmp.path().join("plugins")).unwrap();
    assert!(found.rejected.is_empty(), "{:?}", found.rejected);
    let host = PluginHost::new(
        HostConfig {
            work_dir: tmp.path().join("work"),
            cache_dir: tmp.path().join("cache"),
            locale: usuyo_plugin::Locale::new("zh-CN"),
            limits: CallLimits::default(),
        },
        found.fetch,
    );
    (tmp, host)
}

#[tokio::test]
async fn the_manifest_loads_and_a_file_is_listed_through_the_host() {
    let (tmp, host) = host();
    let plugin = &host.plugins()[0];
    assert_eq!(plugin.id().as_str(), "usuyo-fetch/telegram");
    let method = plugin.spec.method("export").unwrap();
    let list = method.capabilities.list.as_ref().unwrap();
    assert!(!list.files.is_empty() && list.reads.is_some());
    // 副本固定叫 input.zip
    let made = export_zip(tmp.path());
    let input = tmp.path().join("input.zip");
    std::fs::copy(made, &input).unwrap();
    let page = host
        .list(
            plugin,
            ListRequest {
                method: "export".into(),
                source: "file:ChatExport.zip".into(),
                cursor: None,
                since: None,
                input_file: Some(input.clone()),
            },
            &CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 8);
    assert!(
        page.items
            .iter()
            .all(|i| i.url.is_some() || i.reference.is_some())
    );
    // 经宿主取一条：输出过宿主的校验（文件名、路径、完整度），文件在输出目录里
    let snap = host
        .fetch(
            plugin,
            FetchRequest {
                method: "export".into(),
                input: FetchInput::Ref("m/0/5".into()),
                want: None,
                input_file: Some(input),
            },
            &CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(snap.output.images.len(), 1);
    assert_eq!(
        snap.output.completeness,
        usuyo_plugin::protocol::Completeness::Full
    );
    assert!(snap.out_dir().join("images/001.jpg").is_file());

    let health = host.health(plugin, &CallOptions::default()).await.unwrap();
    assert!(health.ready);
}
