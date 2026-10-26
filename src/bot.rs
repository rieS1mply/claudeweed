//! Telegram bot: photos, videos and files sent to the bot's chat land in the cloud,
//! sorted like any upload. Only the owner's chat (from the setup) is listened to.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Result};
use chrono::{Local, TimeZone};
use serde_json::Value;

use crate::config::TelegramConfig;
use crate::telegram;
use crate::web::{self, App};

/// Telegram's Bot API refuses to hand bots files bigger than this.
const BOT_DOWNLOAD_LIMIT: u64 = 20 * 1024 * 1024;

/// The current public address, for the /url command.
pub type PublicUrl = Arc<Mutex<Option<String>>>;

const HELP: &str = "Отправьте сюда фото, видео или файлы — они сохранятся в облако и разложатся по датам.\n\n\
Чтобы фото сохранилось в оригинальном качестве, отправляйте его «как файл» (без сжатия).\n\
Telegram позволяет ботам скачивать файлы до 20 МБ — большие загружайте через сайт.\n\n\
/url — адрес облака";

pub async fn run(app: App, tg: TelegramConfig, url: PublicUrl) {
    // Start from 0: anything sent while the computer was off (Telegram keeps it for a day) is picked up too.
    let mut offset = 0;
    loop {
        match telegram::get_updates(&tg.token, offset, 50).await {
            Ok(updates) => {
                for update in updates {
                    offset = offset.max(update["update_id"].as_i64().unwrap_or(0) + 1);
                    handle(&app, &tg, &url, &update["message"]).await;
                }
            }
            Err(e) => {
                crate::log!("Telegram-бот: {e:#}");
                tokio::time::sleep(Duration::from_secs(15)).await;
            }
        }
    }
}

/// What to download from a message: Telegram file id, file name, size and a hint for the reply.
struct Attachment {
    file_id: String,
    name: String,
    size: u64,
    compressed: bool,
}

fn attachment(msg: &Value) -> Option<Attachment> {
    let date = Local.timestamp_opt(msg["date"].as_i64().unwrap_or(0), 0).single().unwrap_or_else(Local::now);
    let stamp = date.format("%Y%m%d_%H%M%S");
    let named = |v: &Value, fallback: String| Attachment {
        file_id: v["file_id"].as_str().unwrap_or_default().to_string(),
        name: v["file_name"].as_str().map(str::to_string).unwrap_or(fallback),
        size: v["file_size"].as_u64().unwrap_or(0),
        compressed: false,
    };
    if let Some(sizes) = msg["photo"].as_array() {
        // Several resolutions of the same photo; the last one is the largest.
        let best = sizes.last()?;
        let mut a = named(best, format!("Telegram_{stamp}.jpg"));
        a.compressed = true;
        return Some(a);
    }
    for (field, fallback) in [
        ("document", format!("Telegram_{stamp}")),
        ("video", format!("Telegram_{stamp}.mp4")),
        ("animation", format!("Telegram_{stamp}.mp4")),
        ("video_note", format!("Telegram_{stamp}.mp4")),
        ("audio", format!("Telegram_{stamp}.mp3")),
        ("voice", format!("Telegram_{stamp}.ogg")),
    ] {
        if msg[field].is_object() {
            return Some(named(&msg[field], fallback));
        }
    }
    None
}

async fn handle(app: &App, tg: &TelegramConfig, url: &PublicUrl, msg: &Value) {
    // Strangers who find the bot are ignored.
    if msg["chat"]["id"].as_i64() != Some(tg.chat_id) {
        return;
    }
    let msg_id = msg["message_id"].as_i64().unwrap_or(0);

    if let Some(text) = msg["text"].as_str() {
        crate::log!("Telegram: команда/текст «{}»", text.chars().take(40).collect::<String>());
        let answer = match text.split_whitespace().next().unwrap_or("") {
            "/url" | "/link" => match url.lock().unwrap().clone() {
                Some(u) => format!("Облако: {u}"),
                None => "Туннель ещё не открыт — адрес придёт, как только будет готов.".to_string(),
            },
            _ => HELP.to_string(),
        };
        let _ = telegram::reply(tg, msg_id, &answer).await;
        return;
    }

    let Some(att) = attachment(msg) else {
        let kinds: Vec<&str> = msg.as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
        crate::log!("Telegram: сообщение без файла пропущено ({})", kinds.join(", "));
        return;
    };
    crate::log!("Telegram: получен «{}» ({} КБ), скачиваю…", att.name, att.size / 1024);
    let text = match save(app, tg, msg, &att).await {
        Ok((section, rel)) => {
            crate::log!("Telegram → {}/{rel}", section.dir_name());
            let mut t = format!("✅ Сохранено: {}/{rel}", section.dir_name());
            if att.compressed {
                t.push_str("\n\nTelegram сжал это фото. Для оригинала отправляйте «как файл».");
            }
            t
        }
        Err(e) => {
            crate::log!("Telegram: «{}» не сохранён: {e:#}", att.name);
            format!("❌ Не сохранено: {e:#}")
        }
    };
    let _ = telegram::reply(tg, msg_id, &text).await;
}

async fn save(app: &App, tg: &TelegramConfig, msg: &Value, att: &Attachment) -> Result<(crate::storage::Section, String)> {
    if att.size > BOT_DOWNLOAD_LIMIT {
        bail!("файл больше 20 МБ — Telegram не даёт ботам скачивать такие. Загрузите его через сайт");
    }
    let bytes = telegram::download_file(&tg.token, &att.file_id).await?;
    let tmp = app.paths.uploads.join(format!("tg-{}.part", crate::auth::random_hex(8)));
    tokio::fs::write(&tmp, &bytes).await?;
    // No EXIF (compressed photos, most videos): fall back to when the message was sent.
    let sent_ms = msg["forward_origin"]["date"].as_i64().or(msg["date"].as_i64()).unwrap_or(0) * 1000;
    web::store_file(app, tmp, &att.name, sent_ms).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn picks_largest_photo_and_names_it() {
        let msg = json!({ "date": 1_700_000_000, "photo": [
            { "file_id": "small", "file_size": 10 },
            { "file_id": "big", "file_size": 900 },
        ]});
        let a = attachment(&msg).unwrap();
        assert_eq!(a.file_id, "big");
        assert!(a.compressed && a.name.starts_with("Telegram_") && a.name.ends_with(".jpg"));
    }

    #[test]
    fn documents_keep_their_name() {
        let msg = json!({ "date": 1, "document": { "file_id": "d", "file_name": "IMG_0001.HEIC", "file_size": 5 } });
        let a = attachment(&msg).unwrap();
        assert_eq!((a.name.as_str(), a.compressed), ("IMG_0001.HEIC", false));
        assert!(attachment(&json!({ "date": 1, "text": "hi" })).is_none());
    }
}
