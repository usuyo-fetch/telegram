//! usuyo 的 Telegram 获取插件：读你用 Telegram Desktop 导出的 JSON（`result.json`，单独一个文件，
//! 或连同导出文件夹一起压成的 zip），列出 Saved Messages 里的消息，`fetch` 按 `ref` 取出一条。不联网。
//!
//! - [`cli`]：子命令和信封，入口只调它。
//! - `input`：打开交来的文件（json 或 zip），按名字读 zip 里的条目，不解压到磁盘。
//! - [`chat`]：从导出里选出聊天、列消息（`list`）。
//! - [`message`]：一条消息的文字、时间和附件。
//! - [`fetch`]：把一条消息写进输出目录。

pub mod chat;
pub mod cli;
mod error;
pub mod fetch;
mod input;
mod limits;
pub mod message;

pub use error::{Error, Lang};
pub use limits::Limits;
