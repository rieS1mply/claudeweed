//! Who is logged in, and logging in without the password.
//!
//! - The owner's list of devices (sessions) and recent login attempts; ending one session
//!   or all the others.
//! - A Telegram message after every password login, with buttons to end that session or every
//!   session; a warning when an address gets locked out for guessing.
//! - Login through Telegram: the login page asks, the bot shows the request with the same
//!   short code as the page, and the owner's "Войти" button logs that browser in. Not offered
//!   while app codes (2FA) are on: it would skip them.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::auth;
use crate::config::TelegramConfig;
use crate::telegram;
use crate::web::{csrf_ok, error, App};

/// A Telegram login request waits this long for the owner's answer.
const TG_LOGIN_TTL: Duration = Duration::from_secs(180);
/// At most this many requests waiting at once, and this many per hour: the button on the
/// public login page must not become a way to flood the owner's chat.
const TG_LOGIN_PENDING: usize = 3;
const TG_LOGIN_PER_HOUR: usize = 10;

#[derive(Clone, Copy, PartialEq)]
enum Answer {
    Waiting,
    Approved,
    Denied,
}

struct Pending {
    code: String,
    ip: String,
    agent: String,
    created: Instant,
    answer: Answer,
    message_id: i64,
}

#[derive(Default)]
pub struct TgLogins {
    pending: Mutex<HashMap<String, Pending>>,
    recent: Mutex<VecDeque<Instant>>,
}

impl TgLogins {
    fn prune(pending: &mut HashMap<String, Pending>) {
        pending.retain(|_, p| p.created.elapsed() < TG_LOGIN_TTL);
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// "iPhone · Safari" from a User-Agent; "" when nothing in it is recognised.
pub fn device_name(agent: &str) -> String {
    let has = |s: &str| agent.contains(s);
    let system = [
        ("iPhone", "iPhone"), ("iPad", "iPad"), ("Android", "Android"), ("Windows", "Windows"),
        ("Macintosh", "Mac"), ("Mac OS X", "Mac"), ("CrOS", "ChromeOS"), ("Linux", "Linux"),
    ]
    .into_iter()
    .find(|(key, _)| has(key))
    .map(|(_, name)| name);
    // Order matters: Edge and Opera also say "Chrome/", Chrome also says "Safari/".
    let browser = [
        ("Telegram", "Telegram"), ("EdgiOS", "Edge"), ("Edg/", "Edge"), ("OPR/", "Opera"), ("Opera", "Opera"),
        ("YaBrowser", "Яндекс Браузер"), ("SamsungBrowser", "Samsung Internet"), ("FxiOS", "Firefox"),
        ("Firefox/", "Firefox"), ("CriOS", "Chrome"), ("Chrome/", "Chrome"), ("Safari/", "Safari"), ("curl/", "curl"),
    ]
    .into_iter()
    .find(|(key, _)| has(key))
    .map(|(_, name)| name);
    match (system, browser) {
        (Some(s), Some(b)) => format!("{s} · {b}"),
        (Some(one), None) | (None, Some(one)) => one.to_string(),
        (None, None) => String::new(),
    }
}

/// "iPhone · Safari" for messages and the list; the raw string when nothing is recognised.
fn device(agent: &str) -> String {
    let name = device_name(agent);
    if !name.is_empty() {
        name
    } else if agent.is_empty() {
        crate::i18n::tr("неизвестное устройство")
    } else {
        agent.chars().take(60).collect()
    }
}

// ---------- list of devices and logins ----------

pub async fn sessions_list(State(app): State<App>, headers: HeaderMap) -> Response {
    let list: Vec<Value> = app
        .auth
        .list(&headers)
        .into_iter()
        .map(|v| {
            json!({
                "id": v.id, "current": v.current, "device": device(&v.session.agent),
                "ip": v.session.ip, "created": v.session.created, "seen": v.session.seen, "via": v.session.via,
            })
        })
        .collect();
    let logins: Vec<Value> = app
        .auth
        .logins()
        .into_iter()
        .take(30)
        .map(|l| json!({ "time": l.time, "ip": l.ip, "device": device(&l.agent), "via": l.via, "ok": l.ok }))
        .collect();
    Json(json!({ "sessions": list, "logins": logins })).into_response()
}

pub async fn session_end(State(app): State<App>, Path(id): Path<String>) -> Response {
    match app.auth.end_by_id(&id) {
        Some(s) => {
            crate::log!("Сеанс завершён со страницы: {} (адрес: {})", device(&s.agent), s.ip);
            Json(json!({ "ok": true })).into_response()
        }
        None => error(StatusCode::NOT_FOUND, "Сеанс не найден"),
    }
}

pub async fn sessions_end_others(State(app): State<App>, headers: HeaderMap) -> Response {
    let ended = app.auth.end_others(&headers);
    crate::log!("Завершены все другие сеансы со страницы: {ended}");
    Json(json!({ "ended": ended })).into_response()
}

// ---------- Telegram notices ----------

/// After a password login: who, from where, and buttons to undo it.
pub fn notify_login(app: &App, token: &str, ip: &str, agent: &str) {
    let Some(tg) = app.config.telegram.clone() else { return };
    let id = auth::session_id(token);
    let text = format!(
        "🔑 {}\n{} · {}\n\n{}",
        esc(&crate::i18n::tr("Вход в облако по паролю")),
        esc(&device(agent)),
        esc(ip),
        esc(&crate::i18n::tr("Если это не вы — завершите сеанс и смените пароль: local-photo --new-login")),
    );
    let buttons = [(crate::i18n::tr("Завершить этот сеанс"), format!("end:{id}")), (crate::i18n::tr("Выйти везде"), "endall".to_string())];
    tokio::spawn(async move {
        let buttons: Vec<(&str, String)> = buttons.iter().map(|(l, d)| (l.as_str(), d.clone())).collect();
        if let Err(e) = telegram::send_buttons(&tg, &text, &buttons).await {
            crate::log!("Telegram: уведомление о входе не отправлено: {e:#}");
        }
    });
}

/// An address was just locked out for wrong passwords (at most once an hour per address).
pub fn notify_lockout(app: &App, ip: &str) {
    let Some(tg) = app.config.telegram.clone() else { return };
    if !app.auth.should_alert(ip) {
        return;
    }
    let text = crate::i18n::tr(&format!("⚠️ Несколько неверных паролей подряд с адреса {ip} — вход с него временно закрыт."));
    tokio::spawn(async move { telegram::send_with_retry(&tg, &text).await });
}

// ---------- login through Telegram ----------

/// Shown on the login page only when it can work: a bot to ask, and no app codes to skip.
pub fn tg_login_available(app: &App) -> bool {
    app.config.telegram.is_some() && app.totp_secret().is_none()
}

pub async fn tg_login_start(State(app): State<App>, headers: HeaderMap) -> Response {
    if !csrf_ok(&headers) {
        return error(StatusCode::FORBIDDEN, "Запрос отклонён");
    }
    let Some(tg) = app.config.telegram.clone().filter(|_| tg_login_available(&app)) else {
        return error(StatusCode::NOT_FOUND, "Вход через Telegram не настроен");
    };
    let ip = auth::client_ip(&headers);
    if app.auth.check_allowed(&ip).is_err() {
        return error(StatusCode::TOO_MANY_REQUESTS, "Слишком много попыток. Попробуйте позже.");
    }
    {
        let mut recent = app.tg_logins.recent.lock().unwrap();
        recent.retain(|t| t.elapsed() < Duration::from_secs(3600));
        let mut pending = app.tg_logins.pending.lock().unwrap();
        TgLogins::prune(&mut pending);
        if recent.len() >= TG_LOGIN_PER_HOUR || pending.len() >= TG_LOGIN_PENDING {
            return error(StatusCode::TOO_MANY_REQUESTS, "Слишком много запросов на вход. Попробуйте позже.");
        }
        recent.push_back(Instant::now());
    }
    let id = auth::random_hex(16);
    // Four digits, shown both on the page and in Telegram: the owner approves only their own request.
    let code = format!("{:04}", u16::from_str_radix(&auth::random_hex(2), 16).unwrap_or(0) % 10000);
    let agent = auth::short_agent(&headers);
    let text = format!(
        "🔐 {}\n{} · {}\n\n{} <b>{code}</b>\n{}",
        esc(&crate::i18n::tr("Запрос на вход в облако")),
        esc(&device(&agent)),
        esc(&ip),
        esc(&crate::i18n::tr("Код на странице входа:")),
        esc(&crate::i18n::tr("Нажимайте «Войти», только если это вы и код совпадает.")),
    );
    let buttons = [(crate::i18n::tr("✅ Войти"), format!("tgok:{id}")), (crate::i18n::tr("❌ Отклонить"), format!("tgno:{id}"))];
    let buttons: Vec<(&str, String)> = buttons.iter().map(|(l, d)| (l.as_str(), d.clone())).collect();
    let message_id = match telegram::send_buttons(&tg, &text, &buttons).await {
        Ok(m) => m,
        Err(e) => {
            crate::log!("Вход через Telegram: сообщение не отправлено: {e:#}");
            return error(StatusCode::BAD_GATEWAY, "Не удалось отправить запрос в Telegram");
        }
    };
    crate::log!("Запрос на вход через Telegram (адрес: {ip})");
    app.tg_logins.pending.lock().unwrap().insert(id.clone(), Pending { code: code.clone(), ip, agent, created: Instant::now(), answer: Answer::Waiting, message_id });
    Json(json!({ "id": id, "code": code })).into_response()
}

/// The login page asks every couple of seconds; once approved, this answer logs it in.
pub async fn tg_login_poll(State(app): State<App>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if !csrf_ok(&headers) {
        return error(StatusCode::FORBIDDEN, "Запрос отклонён");
    }
    let taken = {
        let mut pending = app.tg_logins.pending.lock().unwrap();
        TgLogins::prune(&mut pending);
        match pending.get(&id).map(|p| p.answer) {
            None => return Json(json!({ "state": "expired" })).into_response(),
            Some(Answer::Waiting) => return Json(json!({ "state": "wait" })).into_response(),
            Some(_) => pending.remove(&id).unwrap(),
        }
    };
    if taken.answer == Answer::Denied {
        app.auth.record_login(&taken.ip, &taken.agent, "telegram", false);
        return Json(json!({ "state": "denied" })).into_response();
    }
    // Only the browser that asked knows the 128-bit id. The address isn't compared: a phone may
    // switch between its IPv4 and IPv6 address within these minutes.
    let ip = auth::client_ip(&headers);
    app.auth.record_success(&ip);
    app.auth.record_login(&ip, &taken.agent, "telegram", true);
    crate::log!("Вход через Telegram выполнен (адрес: {ip})");
    let token = app.auth.create_session(&ip, &taken.agent, "telegram");
    let cookie = auth::session_cookie_header(&token, auth::is_https(&headers));
    ([(header::SET_COOKIE, cookie)], Json(json!({ "state": "ok" }))).into_response()
}

// ---------- buttons in the owner's chat ----------

/// A tap on one of the buttons above. `tg` is the owner's chat; taps from elsewhere are ignored.
pub async fn on_callback(app: &App, tg: &TelegramConfig, query: &Value) {
    let callback_id = query["id"].as_str().unwrap_or_default();
    if query["message"]["chat"]["id"].as_i64() != Some(tg.chat_id) {
        return;
    }
    let message_id = query["message"]["message_id"].as_i64().unwrap_or(0);
    let data = query["data"].as_str().unwrap_or_default();
    let tr = crate::i18n::tr;
    let (note, edited) = if let Some(id) = data.strip_prefix("tgok:").or_else(|| data.strip_prefix("tgno:")) {
        let approve = data.starts_with("tgok:");
        let found = {
            let mut pending = app.tg_logins.pending.lock().unwrap();
            TgLogins::prune(&mut pending);
            pending.get_mut(id).filter(|p| p.answer == Answer::Waiting).map(|p| {
                p.answer = if approve { Answer::Approved } else { Answer::Denied };
                (device(&p.agent), p.ip.clone(), p.code.clone(), p.message_id)
            })
        };
        match found {
            Some((dev, ip, code, msg)) => {
                let head = if approve { tr("✅ Вход разрешён") } else { tr("❌ Вход отклонён") };
                crate::log!("Вход через Telegram {} (адрес: {ip})", if approve { "разрешён" } else { "отклонён" });
                (head.clone(), Some((msg, format!("{}\n{} · {} · {}", esc(&head), esc(&dev), esc(&ip), esc(&code)))))
            }
            None => (tr("Запрос устарел"), None),
        }
    } else if let Some(id) = data.strip_prefix("end:") {
        match app.auth.end_by_id(id) {
            Some(s) => {
                crate::log!("Сеанс завершён из Telegram: {} (адрес: {})", device(&s.agent), s.ip);
                let head = tr("Сеанс завершён");
                (head.clone(), Some((message_id, format!("🚪 {} — {} · {}", esc(&head), esc(&device(&s.agent)), esc(&s.ip)))))
            }
            None => (tr("Этот сеанс уже завершён"), None),
        }
    } else if data == "endall" {
        let ended = app.auth.end_others(&HeaderMap::new());
        crate::log!("Все сеансы завершены из Telegram: {ended}");
        let head = tr(&format!("Все устройства вышли: {ended}"));
        (head.clone(), Some((message_id, format!("🚪 {}", esc(&head)))))
    } else {
        return;
    };
    telegram::answer_callback(tg, callback_id, &note).await;
    if let Some((msg, html)) = edited {
        let _ = telegram::edit_html(tg, msg, &html).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_from_user_agents() {
        let cases = [
            ("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1", "iPhone · Safari"),
            ("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/130.0.6723.90 Mobile/15E148 Safari/604.1", "iPhone · Chrome"),
            ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36", "Windows · Chrome"),
            ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0", "Windows · Edge"),
            ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15", "Mac · Safari"),
            ("Mozilla/5.0 (Android 14; Mobile; rv:131.0) Gecko/131.0 Firefox/131.0", "Android · Firefox"),
            ("Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0", "Linux · Firefox"),
            ("curl/8.4.0", "curl"),
            ("", ""),
            ("something odd", ""),
        ];
        for (ua, want) in cases {
            assert_eq!(device_name(ua), want, "{ua}");
        }
    }
}
