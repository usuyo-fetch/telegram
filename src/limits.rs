//! 交来的文件不可信：各处读多少、数多少都有上限。测试用小的上限，运行用 [`Limits::default`]。

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// zip 里最多多少个条目。
    pub zip_entries: usize,
    /// `result.json` 最多多少字节（单独的文件看文件大小，zip 里看解压后读到的字节）。
    pub json_bytes: u64,
    /// 一个附件最多多少字节。
    pub file_bytes: u64,
    /// 一条消息的附件加起来最多多少字节。
    pub files_total: u64,
    /// 一个聊天最多多少条消息。
    pub messages: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            zip_entries: 500_000,
            json_bytes: 512 << 20,
            file_bytes: 2 << 30,
            files_total: 4 << 30,
            messages: 2_000_000,
        }
    }
}
