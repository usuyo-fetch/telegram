# usuyo-fetch/telegram

Telegram 的获取插件，供 [usuyo](https://github.com/usuyo/usuyo) 调用：读你用 Telegram Desktop 导出的 JSON，列出 Saved Messages 里的消息并按条取出。

| 方法 | 清单里的 id | 取得 | 账号 | 层级 | 现状 |
|---|---|---|---|---|---|
| Telegram Desktop 导出 | `export` | 存过的：Saved Messages（单个聊天的导出则是那个聊天）里每条消息的文字、时间、转发来源和导出里带的图片、视频、语音、文件；整条只是一个链接的消息作为链接条目 | 不使用（你在 Telegram Desktop 里导出） | `official-export` | 已实现；只做了离线测试，样例手写，没有对着真实导出核对 |

导出格式参考 [tg-parser](https://github.com/mdemyanov/tg-parser) 的格式说明（MIT，Copyright (c) 2025 Maxim Demyanov）和 [telegram-export-md](https://github.com/Retro-Zero/telegram-export-md)（MIT，Copyright (c) 2026 Arian Karimi），Telegram 的导出说明见 [telegram.org/blog/export-and-more](https://telegram.org/blog/export-and-more)。
