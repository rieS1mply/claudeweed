//! Minimal Telegram Bot API client: just enough to find the owner's chat and send messages.

use std::time::Duration;

use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::config::TelegramConfig;

/// One client for everything: the connection to api.telegram.org is reused between
/// long polls instead of doing DNS + TLS again every minute.
fn client() -> &'static reqwest::Client {
    static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .expect("http client")
    });
    &CLIENT
}

/// Ordinary requests; a long poll gets its own wait time on top of this.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

async fn call(token: &str, method: &str, body: Value) -> Result<Value> {
    // getUpdates holds the request open for up to `timeout` seconds by design.
    let wait = body["timeout"].as_u64().map_or(Duration::ZERO, Duration::from_secs);
    let url = format!("https://api.telegram.org/bot{token}/{method}");
    // Errors must not carry the URL: it contains the bot token.
    let res: Value = client()
        .post(url)
        .timeout(REQUEST_TIMEOUT + wait)
        .json(&body)
        .send()
        .await
        .map_err(reqwest::Error::without_url)?
        .json()
        .await
        .map_err(reqwest::Error::without_url)?;
    if res["ok"].as_bool() != Some(true) {
        bail!("{}", res["description"].as_str().unwrap_or("ошибка Telegram API"));
    }
    Ok(res["result"].clone())
}

/// Returns the bot's @username; fails when the token is wrong.
pub async fn get_me(token: &str) -> Result<String> {
    let me = call(token, "getMe", json!({})).await?;
    Ok(me["username"].as_str().unwrap_or("?").to_string())
}

pub async fn latest_update_id(token: &str) -> Result<Option<i64>> {
    let updates = call(token, "getUpdates", json!({ "offset": -1, "timeout": 0 })).await?;
    Ok(updates.as_array().and_then(|a| a.last()).and_then(|u| u["update_id"].as_i64()))
}

/// Long-polls for the next private message. Returns the next offset and,
/// if a message arrived, its chat id and a human-readable sender name.
pub async fn wait_for_chat(token: &str, offset: i64) -> Result<(i64, Option<(i64, String)>)> {
    let updates = call(token, "getUpdates", json!({ "offset": offset, "timeout": 25 })).await?;
    let mut next = offset;
    for u in updates.as_array().into_iter().flatten() {
        next = next.max(u["update_id"].as_i64().unwrap_or(0) + 1);
        let msg = &u["message"];
        if msg["chat"]["type"].as_str() == Some("private") {
            if let Some(chat_id) = msg["chat"]["id"].as_i64() {
                let first = msg["from"]["first_name"].as_str().unwrap_or("");
                let who = match msg["from"]["username"].as_str() {
                    Some(user) => format!("{first} (@{user})"),
                    None => first.to_string(),
                };
                return Ok((next, Some((chat_id, who))));
            }
        }
    }
    Ok((next, None))
}

/// Long-polls for new updates (up to `timeout` seconds).
pub async fn get_updates(token: &str, offset: i64, timeout: u64) -> Result<Vec<Value>> {
    let updates = call(token, "getUpdates", json!({
        "offset": offset,
        "timeout": timeout,
        "allowed_updates": ["message"],
    }))
    .await?;
    Ok(updates.as_array().cloned().unwrap_or_default())
}

/// Downloads a file the bot received. Telegram only lets bots fetch files up to 20 MB.
pub async fn download_file(token: &str, file_id: &str) -> Result<Vec<u8>> {
    let file = call(token, "getFile", json!({ "file_id": file_id })).await?;
    let Some(path) = file["file_path"].as_str() else { bail!("Telegram не отдал путь к файлу") };
    let url = format!("https://api.telegram.org/file/bot{token}/{path}");
    let res = client()
        .get(url)
        .timeout(Duration::from_secs(300))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(reqwest::Error::without_url)?;
    Ok(res.bytes().await.map_err(reqwest::Error::without_url)?.to_vec())
}

/// Answers a particular message in the owner's chat.
pub async fn reply(tg: &TelegramConfig, reply_to: i64, text: &str) -> Result<()> {
    call(&tg.token, "sendMessage", json!({
        "chat_id": tg.chat_id,
        "text": text,
        "reply_parameters": { "message_id": reply_to, "allow_sending_without_reply": true },
        "disable_web_page_preview": true,
    }))
    .await?;
    Ok(())
}

/// Sends a message (as an answer to `reply_to`, if given; `html`: Telegram's HTML subset)
/// and returns its id, so it can be edited later.
pub async fn send_message(tg: &TelegramConfig, text: &str, reply_to: Option<i64>, html: bool) -> Result<i64> {
    let mut body = json!({ "chat_id": tg.chat_id, "text": text, "disable_web_page_preview": true });
    if let Some(id) = reply_to {
        body["reply_parameters"] = json!({ "message_id": id, "allow_sending_without_reply": true });
    }
    if html {
        body["parse_mode"] = json!("HTML");
    }
    let msg = call(&tg.token, "sendMessage", body).await?;
    Ok(msg["message_id"].as_i64().unwrap_or(0))
}

pub async fn edit_message(tg: &TelegramConfig, message_id: i64, text: &str) -> Result<()> {
    call(&tg.token, "editMessageText", json!({
        "chat_id": tg.chat_id,
        "message_id": message_id,
        "text": text,
        "disable_web_page_preview": true,
    }))
    .await?;
    Ok(())
}

pub async fn delete_message(tg: &TelegramConfig, message_id: i64) -> Result<()> {
    call(&tg.token, "deleteMessage", json!({ "chat_id": tg.chat_id, "message_id": message_id })).await?;
    Ok(())
}

/// "typing…" under the bot's name; Telegram shows it for about 5 seconds.
pub async fn typing(tg: &TelegramConfig) {
    let _ = call(&tg.token, "sendChatAction", json!({ "chat_id": tg.chat_id, "action": "typing" })).await;
}

/// The menu of commands shown by the "/" button.
pub async fn set_commands(tg: &TelegramConfig, commands: &[(&str, &str)]) -> Result<()> {
    let list: Vec<Value> = commands.iter().map(|(c, d)| json!({ "command": c, "description": d })).collect();
    call(&tg.token, "setMyCommands", json!({ "commands": list })).await?;
    Ok(())
}

pub async fn send(tg: &TelegramConfig, text: &str) -> Result<()> {
    call(&tg.token, "sendMessage", json!({
        "chat_id": tg.chat_id,
        "text": text,
        "disable_web_page_preview": true,
    }))
    .await?;
    Ok(())
}

/// Like [`send`], but retries for a while — at boot the network may not be up yet.
pub async fn send_with_retry(tg: &TelegramConfig, text: &str) {
    let text = &crate::i18n::tr(text);
    for attempt in 1..=6u64 {
        match send(tg, text).await {
            Ok(()) => return,
            Err(e) => {
                crate::log!("Telegram: не удалось отправить сообщение ({e:#}), попытка {attempt}/6");
                tokio::time::sleep(Duration::from_secs(10 * attempt)).await;
            }
        }
    }
}
