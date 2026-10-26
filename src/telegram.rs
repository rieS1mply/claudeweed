//! Minimal Telegram Bot API client: just enough to find the owner's chat and send messages.

use std::time::Duration;

use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::config::TelegramConfig;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("http client")
}

async fn call(token: &str, method: &str, body: Value) -> Result<Value> {
    let url = format!("https://api.telegram.org/bot{token}/{method}");
    // Errors must not carry the URL: it contains the bot token.
    let res: Value = client()
        .post(url)
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
    let res = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()?
        .get(url)
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
