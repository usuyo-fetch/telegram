# usuyo-fetch/telegram

Telegram 的获取插件，供 [usuyo](https://github.com/usuyo/usuyo) 调用：读你用 Telegram Desktop 导出的 JSON，列出 Saved Messages 里的消息并按条取出。

| 功能 | 实现方式（方法 id） | 账号 | 风险层级 | 状态 |
|---|---|---|---|---|
| 读官方导出：Saved Messages | Telegram Desktop 导出（`export`） | 不使用（你在 Telegram Desktop 里导出） | `official-export`，默认开 | 已实现（离线测试，样例手写，没有对着真实导出核对） |
| 取单条消息的文字和附件 | Telegram Desktop 导出（`export`，`fetch`） | 不使用 | `official-export`，默认开 | 已实现（同上） |

风险层级用清单里 `access` 的原词；`private-api`、`client-hook` 默认关，其余层级默认开。

## 方法

### `export`

读 Telegram Desktop「导出聊天记录」选「机器可读的 JSON」得到的 `result.json`，或连导出文件夹压成的 zip。列出 Saved Messages（单个聊天的导出则是那个聊天）里每条消息的文字、时间、转发来源和导出里带的图片、视频、语音、文件；整条只是一个链接的消息作为链接条目。插件不联网。

导出格式参考 [tg-parser](https://github.com/mdemyanov/tg-parser) 的格式说明（MIT，Copyright (c) 2025 Maxim Demyanov）和 [telegram-export-md](https://github.com/Retro-Zero/telegram-export-md)（MIT，Copyright (c) 2026 Arian Karimi），Telegram 的导出说明见 [telegram.org/blog/export-and-more](https://telegram.org/blog/export-and-more)。

## 开发

`Cargo.toml` 里的 `usuyo-plugin`、`usuyo-plugin-kit` 是对 `https://github.com/usuyo/usuyo` 的 git 依赖，钉在固定的 `rev`。

本地开发时用旁边 `usuyo/` 的工作区代替它们：`usuyo-fetch/.cargo/config.toml`（不进任何插件仓库）写了下面的覆盖，cargo 从当前目录往上找这个文件，在插件文件夹里跑 cargo 就生效；从别处跑要加 `--config <这个文件>`。

```toml
[patch."https://github.com/usuyo/usuyo"]
usuyo-plugin = { path = "../usuyo/crates/usuyo-plugin" }
usuyo-plugin-kit = { path = "../usuyo/crates/usuyo-plugin-kit" }
```

`Cargo.lock` 记的是覆盖后的路径形式，所以 CI 不加 `--locked` 构建，cargo 只重新锁这两个 usuyo crate。
