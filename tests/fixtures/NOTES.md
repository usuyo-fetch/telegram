# Fixture notes

All files here are hand-written with invented content. NOT MEASURED: nobody involved had a real Telegram Desktop export, so none of this has been compared with one.

- `result.json`: the shape of a full export (`chats.list[]` with `type: saved_messages`, a second ordinary chat that must be ignored). Fields follow `docs/TELEGRAM_FORMAT.md` of [mdemyanov/tg-parser](https://github.com/mdemyanov/tg-parser) (MIT, Copyright (c) 2025 Maxim Demyanov): `id`, `type`, `date`, `date_unixtime`, `from`, `from_id`, `text` as a string or an array of strings and `{type, text}` objects, `forwarded_from`, `media_type`, `photo` / `file` placeholders, service messages. `text_link` with `href`, `text_entities`, `file_name` follow [Retro-Zero/telegram-export-md](https://github.com/Retro-Zero/telegram-export-md) (MIT, Copyright (c) 2026 Arian Karimi) `converter.py`. The `chats.list` wrapper and the type string `saved_messages` are from the author's memory of Telegram Desktop's JSON writer and are not confirmed by either source.
- `chat-export.json`: the shape of a single-chat export (`name`, `type`, `id`, `messages` at the top level), per tg-parser's document.
- The `_fixture` key in each JSON file is not part of Telegram's format; the plugin ignores it.
- Media bytes (photo, voice, video, pdf) are made up inside the tests; the zip layout `ChatExport_<date>/result.json` plus `photos/`, `voice_messages/`, `video_files/`, `files/` follows how Telegram Desktop lays out an export folder (not confirmed).
