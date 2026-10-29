//! HTTP API and the embedded web interface.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, Semaphore};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::auth::{self, Auth};
use crate::config::{Config, Paths};
use crate::storage::{self, Kind, Section};
use crate::{notes, runner, telegram, terminal, thumbs, totp, trash, zips};

/// Matches CHUNK_SIZE in app.js, with headroom.
const MAX_CHUNK: usize = 20 * 1024 * 1024;
/// Uploads nobody touched for this long are abandoned and deleted.
const UPLOAD_EXPIRY: Duration = Duration::from_secs(6 * 3600);

#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

pub struct AppState {
    pub paths: Paths,
    pub config: Config,
    pub auth: Auth,
    /// Lets `local-photo --stop` (same computer, knows the secret) shut this copy down.
    local_secret: String,
    stop: Arc<tokio::sync::Notify>,
    uploads: std::sync::Mutex<HashMap<String, Arc<Mutex<Upload>>>>,
    /// Thumbnail decoding is memory-hungry; don't run too many at once.
    thumb_slots: Semaphore,
    /// Serializes "pick a free name + move file there".
    place_lock: Mutex<()>,
    zips: zips::Jobs,
    pub terminal: Arc<terminal::Terminal>,
    pub runner: Arc<runner::Runner>,
    /// Last authenticator step accepted at login: the same code can't log in twice.
    totp_last_step: std::sync::Mutex<u64>,
}

struct Upload {
    name: String,
    size: u64,
    last_modified: i64,
    received: u64,
    tmp: PathBuf,
    file: Option<tokio::fs::File>,
    touched: Instant,
}

pub type App = Arc<AppState>;

impl AppState {
    pub fn new(paths: Paths, config: Config, auth: Auth, local_secret: String, stop: Arc<tokio::sync::Notify>) -> App {
        Arc::new(AppState {
            paths,
            config,
            auth,
            local_secret,
            stop,
            uploads: Default::default(),
            thumb_slots: Semaphore::new(3),
            place_lock: Mutex::new(()),
            zips: Default::default(),
            terminal: Default::default(),
            runner: Default::default(),
            totp_last_step: Default::default(),
        })
    }

    fn section_root(&self, section: Section) -> PathBuf {
        self.paths.storage.join(section.dir_name())
    }

    /// Read from config.json on every use, so `--terminal off` works without a restart.
    fn terminal_enabled(&self) -> bool {
        Config::load(&self.paths.config).ok().flatten().map_or(self.config.terminal, |c| c.terminal)
    }

    fn runner_enabled(&self) -> bool {
        Config::load(&self.paths.config).ok().flatten().map_or(self.config.runner, |c| c.runner)
    }

    /// Also read live: `--2fa on` takes effect for the next login without a restart.
    fn totp_secret(&self) -> Option<String> {
        Config::load(&self.paths.config).ok().flatten().map_or_else(|| self.config.totp_secret.clone(), |c| c.totp_secret)
    }

    /// Folder of a section by its API name, including "notes".
    fn root_for(&self, section: &str) -> Option<PathBuf> {
        match section {
            "notes" => Some(self.paths.notes.clone()),
            other => Section::parse(other).map(|s| self.section_root(s)),
        }
    }
}

pub fn router(app: App) -> Router {
    let protected = Router::new()
        .route("/api/logout", post(logout))
        .route("/api/list", get(list))
        .route("/api/file", get(get_file).delete(delete_file))
        .route("/api/file/rename", post(rename_file))
        .route("/api/file/content", put(replace_file).layer(DefaultBodyLimit::max(MAX_EDIT_BODY)))
        .route("/api/versions", get(versions))
        .route("/api/file/copy", post(copy_file).layer(DefaultBodyLimit::max(MAX_EDIT_BODY)))
        .route("/api/thumb", get(thumb))
        .route("/api/zip", post(zip_start))
        .route("/api/zip/{id}", get(zip_status))
        .route("/api/zip/{id}/file", get(zip_file))
        .route("/api/notes", get(notes_list).post(note_create))
        .route("/api/notes/{id}", get(note_get).put(note_put).delete(note_delete))
        .route("/api/trash", get(trash_list).delete(trash_empty))
        .route("/api/trash/{id}", axum::routing::delete(trash_delete))
        .route("/api/trash/{id}/restore", post(trash_restore))
        .route("/api/trash/{id}/file", get(trash_file))
        .route("/api/trash/{id}/thumb", get(trash_thumb))
        .route("/api/upload/init", post(upload_init))
        .route(
            "/api/upload/{id}",
            get(upload_status).put(upload_chunk).layer(DefaultBodyLimit::max(MAX_CHUNK)),
        )
        .route("/api/upload/{id}/finish", post(upload_finish))
        .route("/api/terminal", get(terminal_ws))
        .route("/api/terminal/sessions", get(terminal_list).post(terminal_create))
        .route("/api/terminal/sessions/{id}", put(terminal_rename).delete(terminal_close))
        .route("/api/run/langs", get(run_langs))
        .route("/api/run", post(run_code).layer(DefaultBodyLimit::max(runner::MAX_SOURCE + runner::MAX_STDIN + 4096)))
        .route_layer(middleware::from_fn_with_state(app.clone(), require_session));

    Router::new()
        .route("/api/session", get(session))
        .route("/api/login", post(login))
        .route("/api/local/stop", post(local_stop))
        .merge(protected)
        .route("/", get(index))
        .route("/terminal.html", get(terminal_page))
        .route("/{file}", get(asset))
        .layer(middleware::from_fn(security_headers))
        .layer(middleware::from_fn(force_https))
        .with_state(app)
}

/// Below this the owner gets a Telegram warning: uploads stop working at [`DISK_RESERVE`].
const DISK_WARN: u64 = 10 * 1024 * 1024 * 1024;
/// Repeat the warning at most this often while the disk stays full.
const DISK_WARN_EVERY: Duration = Duration::from_secs(24 * 3600);

fn gb(bytes: u64) -> f64 {
    bytes as f64 / 1024f64.powi(3)
}

/// Checks free space on the storage disk; `last_warned` limits the warnings to one a day.
async fn check_disk(app: &AppState, last_warned: &mut Option<Instant>) {
    let Ok(free) = fs4::available_space(&app.paths.storage) else { return };
    if free >= DISK_WARN {
        *last_warned = None;
        return;
    }
    if last_warned.is_some_and(|t| t.elapsed() < DISK_WARN_EVERY) {
        return;
    }
    *last_warned = Some(Instant::now());
    let text = format!(
        "На диске с облаком осталось {:.1} ГБ. Когда останется меньше 1 ГБ, загрузка перестанет работать. Освободите место или очистите корзину.",
        gb(free)
    );
    crate::log!("{text}");
    if let Some(tg) = app.config.telegram.clone() {
        telegram::send_with_retry(&tg, &text).await;
    }
}

pub fn spawn_upload_janitor(app: App) {
    tokio::spawn(async move {
        let mut disk_warned = None;
        // A full disk is worth knowing about right after the start.
        tokio::time::sleep(Duration::from_secs(60)).await;
        check_disk(&app, &mut disk_warned).await;
        loop {
            tokio::time::sleep(Duration::from_secs(600)).await;
            check_disk(&app, &mut disk_warned).await;
            let stale: Vec<Arc<Mutex<Upload>>> = {
                let mut map = app.uploads.lock().unwrap();
                let mut stale = Vec::new();
                map.retain(|_, u| match u.try_lock() {
                    Ok(g) if g.touched.elapsed() > UPLOAD_EXPIRY => {
                        stale.push(u.clone());
                        false
                    }
                    _ => true,
                });
                stale
            };
            for u in stale {
                let mut u = u.lock().await;
                u.file = None;
                let _ = tokio::fs::remove_file(&u.tmp).await;
            }
            app.zips.cleanup();
            let trash_dir = app.paths.trash.clone();
            if let Ok(n) = tokio::task::spawn_blocking(move || trash::purge_expired(&trash_dir)).await {
                if n > 0 {
                    crate::log!("Корзина: удалено навсегда {n} (старше {} дней)", trash::RETENTION_DAYS);
                }
            }
        }
    });
}

// ---------- helpers ----------

fn error(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "Файл не найден")
}

/// Mutating requests must come from our own page: a custom header (forces a CORS
/// preflight that we never answer) plus a matching Origin when the browser sends one.
fn csrf_ok(headers: &HeaderMap) -> bool {
    if headers.get("x-cloud").and_then(|v| v.to_str().ok()) != Some("1") {
        return false;
    }
    headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()).is_none_or(|origin| same_origin(headers, origin))
}

fn same_origin(headers: &HeaderMap, origin: &str) -> bool {
    let origin_host = origin.split_once("://").map_or(origin, |(_, h)| h);
    [headers.get("x-forwarded-host"), headers.get(header::HOST)]
        .into_iter()
        .flatten()
        .filter_map(|v| v.to_str().ok())
        .any(|host| host.eq_ignore_ascii_case(origin_host))
}

async fn require_session(State(app): State<App>, req: Request, next: Next) -> Response {
    if !app.auth.is_valid(req.headers()) {
        return error(StatusCode::UNAUTHORIZED, "Требуется вход");
    }
    if !matches!(*req.method(), Method::GET | Method::HEAD) && !csrf_ok(req.headers()) {
        return error(StatusCode::FORBIDDEN, "Запрос отклонён");
    }
    next.run(req).await
}

/// Through the tunnel, plain http:// would send the password unencrypted: redirect to https.
async fn force_https(req: Request, next: Next) -> Response {
    let headers = req.headers();
    if auth::via_cloudflare(headers) && !auth::is_https(headers) {
        if let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
            let path = req.uri().path_and_query().map_or("/", |p| p.as_str());
            if let Ok(location) = HeaderValue::from_str(&format!("https://{host}{path}")) {
                return (StatusCode::PERMANENT_REDIRECT, [(header::LOCATION, location)]).into_response();
            }
        }
    }
    next.run(req).await
}

async fn security_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    // File downloads set their own stricter CSP; keep it.
    h.entry(header::CONTENT_SECURITY_POLICY).or_insert(HeaderValue::from_static(
        "default-src 'self'; img-src 'self' data: blob:; media-src 'self' blob:; style-src 'self'; \
         script-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
    ));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert("x-robots-tag", HeaderValue::from_static("noindex, nofollow"));
    h.insert(header::STRICT_TRANSPORT_SECURITY, HeaderValue::from_static("max-age=31536000"));
    res
}

#[derive(Deserialize)]
struct FileQuery {
    section: String,
    path: String,
    #[serde(default)]
    download: Option<String>,
}

impl FileQuery {
    fn locate(&self, app: &AppState) -> Option<(Section, PathBuf)> {
        let section = Section::parse(&self.section)?;
        let path = storage::resolve(&app.section_root(section), &self.path)?;
        Some((section, path))
    }
}

/// RFC 6266 attachment header that survives non-ASCII names ("Фото.jpg").
fn content_disposition(kind: &str, name: &str) -> HeaderValue {
    let ascii: String = name.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' }).collect();
    let encoded: String = name
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    HeaderValue::from_str(&format!("{kind}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"))
        .unwrap_or(HeaderValue::from_static("attachment"))
}

// ---------- static UI ----------

async fn index(headers: HeaderMap) -> Response {
    asset_response("index.html", &headers)
}

async fn asset(UrlPath(file): UrlPath<String>, headers: HeaderMap) -> Response {
    asset_response(&file, &headers)
}

/// xterm.js adds <style> elements at runtime, so this page alone allows inline styles.
/// The WebSocket host is spelled out: older Safari doesn't count wss: as 'self'.
async fn terminal_page(headers: HeaderMap) -> Response {
    let mut res = asset_response("terminal.html", &headers);
    let hosts: String = [headers.get("x-forwarded-host"), headers.get(header::HOST)]
        .into_iter()
        .flatten()
        .filter_map(|v| v.to_str().ok())
        .filter(|h| h.bytes().all(|b| b.is_ascii_alphanumeric() || b"-.:[]".contains(&b)))
        .map(|h| format!(" wss://{h} ws://{h}"))
        .collect();
    let csp = format!(
        "default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'{hosts}; \
         img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"
    );
    if let Ok(v) = HeaderValue::from_str(&csp) {
        res.headers_mut().insert(header::CONTENT_SECURITY_POLICY, v);
    }
    res
}

/// Gzipped copies of the embedded files, keyed by name and content hash.
static GZIPPED: std::sync::LazyLock<std::sync::Mutex<HashMap<String, (String, Bytes)>>> = std::sync::LazyLock::new(Default::default);

/// Served with an ETag (a repeat visit costs a few hundred bytes, not the whole UI)
/// and gzipped when the browser accepts it.
fn asset_response(name: &str, headers: &HeaderMap) -> Response {
    let Some(file) = Assets::get(name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let etag = format!("\"{}\"", file.metadata.sha256_hash()[..12].iter().map(|b| format!("{b:02x}")).collect::<String>());
    let mime = mime_guess::from_path(name).first_or_octet_stream();
    let mut res = if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains(&etag)) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let accepts_gzip = headers.get(header::ACCEPT_ENCODING).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("gzip"));
        let compressible = matches!(mime.type_(), mime_guess::mime::TEXT | mime_guess::mime::APPLICATION) && file.data.len() > 1024;
        if accepts_gzip && compressible {
            let body = {
                let mut cache = GZIPPED.lock().unwrap();
                match cache.get(name) {
                    Some((tag, gz)) if *tag == etag => gz.clone(),
                    _ => {
                        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
                        let _ = std::io::Write::write_all(&mut enc, &file.data);
                        let gz = Bytes::from(enc.finish().unwrap_or_default());
                        cache.insert(name.to_string(), (etag.clone(), gz.clone()));
                        gz
                    }
                }
            };
            ([(header::CONTENT_ENCODING, "gzip")], body).into_response()
        } else {
            Body::from(file.data.into_owned()).into_response()
        }
    };
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(mime.as_ref()) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, v);
    }
    res
}

// ---------- auth ----------

async fn session(State(app): State<App>, headers: HeaderMap) -> Response {
    let authenticated = app.auth.is_valid(&headers);
    Json(json!({ "authenticated": authenticated, "terminal": authenticated && app.terminal_enabled(), "runner": authenticated && app.runner_enabled() })).into_response()
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
    /// Authenticator code; sent on the second try once the server asked for it.
    #[serde(default)]
    code: Option<String>,
}

async fn login(State(app): State<App>, headers: HeaderMap, Json(body): Json<LoginBody>) -> Response {
    if !csrf_ok(&headers) {
        return error(StatusCode::FORBIDDEN, "Запрос отклонён");
    }
    let ip = auth::client_ip(&headers);
    if let Err(wait) = app.auth.check_allowed(&ip) {
        let mins = wait.as_secs().div_ceil(60);
        return error(StatusCode::TOO_MANY_REQUESTS, &format!("Слишком много попыток. Попробуйте через {mins} мин."));
    }

    // Always run the (slow) hash check, so a wrong login takes as long as a wrong password.
    let hash = app.config.password_hash.clone();
    let password = body.password;
    let password_ok = tokio::task::spawn_blocking(move || {
        use argon2::password_hash::{PasswordHash, PasswordVerifier};
        PasswordHash::new(&hash).is_ok_and(|h| argon2::Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
    })
    .await
    .unwrap_or(false);
    let user_ok = body.username == app.config.username;

    if !(user_ok && password_ok) {
        app.auth.record_failure(&ip);
        crate::log!("Неудачная попытка входа (адрес: {ip})");
        tokio::time::sleep(Duration::from_millis(500)).await;
        return error(StatusCode::UNAUTHORIZED, "Неверный логин или пароль");
    }

    if let Some(secret) = app.totp_secret() {
        let code = body.code.as_deref().map(str::trim).unwrap_or("");
        if code.is_empty() {
            // Not a failure: the page now shows the code field and sends everything again.
            return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "Введите код из приложения", "need_code": true }))).into_response();
        }
        let accepted = totp::verify(&secret, code, totp::unix_now()).filter(|&step| {
            let mut last = app.totp_last_step.lock().unwrap();
            let fresh = step > *last;
            if fresh {
                *last = step;
            }
            fresh
        });
        if accepted.is_none() {
            // Counts like a wrong password, so codes can't be guessed either.
            app.auth.record_failure(&ip);
            crate::log!("Неверный код второго фактора (адрес: {ip})");
            tokio::time::sleep(Duration::from_millis(500)).await;
            return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "Неверный или устаревший код", "need_code": true }))).into_response();
        }
    }

    app.auth.record_success(&ip);
    crate::log!("Вход выполнен (адрес: {ip})");
    let token = app.auth.create_session();
    let cookie = auth::session_cookie_header(&token, auth::is_https(&headers));
    ([(header::SET_COOKIE, cookie)], Json(json!({ "ok": true }))).into_response()
}

/// `local-photo --stop`. Needs the secret from status.json (readable only by this user on
/// this computer) and must not come through the tunnel.
async fn local_stop(State(app): State<App>, headers: HeaderMap) -> Response {
    let secret = headers.get("x-local-secret").and_then(|v| v.to_str().ok()).unwrap_or("");
    if auth::via_cloudflare(&headers) || secret.is_empty() || secret != app.local_secret {
        return error(StatusCode::FORBIDDEN, "Запрос отклонён");
    }
    crate::log!("Получена команда остановки");
    app.stop.notify_one();
    Json(json!({ "ok": true })).into_response()
}

async fn logout(State(app): State<App>, headers: HeaderMap) -> Response {
    app.auth.end_session(&headers);
    ([(header::SET_COOKIE, auth::clear_cookie_header())], Json(json!({ "ok": true }))).into_response()
}

// ---------- terminal ----------

#[derive(Deserialize)]
struct TerminalQuery {
    #[serde(default)]
    z: Option<String>,
    /// Session to attach to; absent from pages of older versions.
    #[serde(default)]
    id: Option<u32>,
}

/// Turned off with `--terminal off`: shells nobody is attached to must not keep running either.
fn terminal_off(app: &App) -> bool {
    if app.terminal_enabled() {
        return false;
    }
    app.terminal.disable();
    true
}

/// Brings back the terminal sessions saved before the last stop, then saves them every
/// few seconds (a reboot or power cut doesn't wait for a clean stop).
pub fn spawn_terminal_saver(app: App, dir: PathBuf) {
    tokio::spawn(async move {
        let (term, enabled) = (app.terminal.clone(), app.terminal_enabled());
        let restore_dir = dir.clone();
        let _ = tokio::task::spawn_blocking(move || term.restore(&restore_dir, enabled)).await;
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            if !app.terminal_enabled() {
                continue;
            }
            let term = app.terminal.clone();
            let _ = tokio::task::spawn_blocking(move || term.save()).await;
        }
    });
}

async fn terminal_list(State(app): State<App>) -> Response {
    if terminal_off(&app) {
        return not_found();
    }
    Json(json!({ "sessions": app.terminal.list(), "max": terminal::MAX_SESSIONS })).into_response()
}

#[derive(Deserialize, Default)]
struct SessionName {
    #[serde(default)]
    name: Option<String>,
}

async fn terminal_create(State(app): State<App>, headers: HeaderMap, body: Option<Json<SessionName>>) -> Response {
    if terminal_off(&app) {
        return not_found();
    }
    let name = body.map(|b| b.0).unwrap_or_default().name;
    let term = app.terminal.clone();
    match tokio::task::spawn_blocking(move || term.create(name.as_deref())).await {
        Ok(Ok(info)) => {
            crate::log!("Терминал: открыт сеанс «{}» (адрес: {})", info.name, auth::client_ip(&headers));
            Json(info).into_response()
        }
        Ok(Err(e)) => error(StatusCode::CONFLICT, &format!("{e:#}")),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось открыть сеанс"),
    }
}

async fn terminal_rename(State(app): State<App>, UrlPath(id): UrlPath<u32>, Json(body): Json<SessionName>) -> Response {
    if terminal_off(&app) {
        return not_found();
    }
    match app.terminal.rename(id, body.name.as_deref().unwrap_or("")) {
        Some(info) => Json(info).into_response(),
        None => error(StatusCode::BAD_REQUEST, "Сеанс не найден или пустое имя"),
    }
}

async fn terminal_close(State(app): State<App>, headers: HeaderMap, UrlPath(id): UrlPath<u32>) -> Response {
    if terminal_off(&app) {
        return not_found();
    }
    if !app.terminal.close(id) {
        return error(StatusCode::NOT_FOUND, "Сеанс не найден");
    }
    crate::log!("Терминал: сеанс {id} закрыт (адрес: {})", auth::client_ip(&headers));
    Json(json!({ "ok": true })).into_response()
}

// ---------- code runner ----------

#[derive(Deserialize)]
struct LangsQuery {
    #[serde(default)]
    refresh: Option<String>,
}

async fn run_langs(State(app): State<App>, Query(q): Query<LangsQuery>) -> Response {
    if !app.runner_enabled() {
        return not_found();
    }
    Json(json!({ "langs": app.runner.languages(q.refresh.is_some()).await })).into_response()
}

#[derive(Deserialize)]
struct RunBody {
    lang: String,
    code: String,
    #[serde(default)]
    stdin: String,
}

/// Streams the run as NDJSON (one `runner::Event` per line). Closing the request stops it.
async fn run_code(State(app): State<App>, headers: HeaderMap, Json(body): Json<RunBody>) -> Response {
    if !app.runner_enabled() {
        return not_found();
    }
    if body.code.len() > runner::MAX_SOURCE || body.stdin.len() > runner::MAX_STDIN {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "Слишком большой файл для запуска");
    }
    let ip = auth::client_ip(&headers);
    crate::log!("Запуск кода: {} (адрес: {ip})", body.lang);
    // Same warning as for the terminal: this runs anything on this computer.
    if let Some(tg) = app.config.telegram.clone().filter(|_| app.terminal.should_notify(&ip)) {
        let text = format!("Через облако запускают код на компьютере (адрес: {ip}). Если это не вы — смените пароль: local-photo --setup");
        tokio::spawn(async move { telegram::send_with_retry(&tg, &text).await });
    }
    let (tx, rx) = tokio::sync::mpsc::channel::<runner::Event>(64);
    let worker = app.clone();
    tokio::spawn(async move {
        let Some(_slot) = worker.runner.try_slot() else {
            let _ = tx.send(runner::Event::Err { v: "Уже идут два запуска — дождитесь их окончания или остановите\n".into() }).await;
            let _ = tx.send(runner::Event::Exit { code: None, ms: 0, reason: Some("error") }).await;
            return;
        };
        worker.runner.run(&body.lang, &body.code, &body.stdin, tx).await;
    });
    let lines = futures_util::stream::unfold(rx, |mut rx| async move {
        let event = rx.recv().await?;
        let mut line = serde_json::to_vec(&event).unwrap_or_default();
        line.push(b'\n');
        Some((Ok::<_, std::convert::Infallible>(Bytes::from(line)), rx))
    });
    Response::builder()
        .header(header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8")
        // Proxies (the tunnel) must pass lines through as they come, not collect the whole answer.
        .header(header::CACHE_CONTROL, "no-cache, no-transform")
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(lines))
        .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Ошибка"))
}

async fn terminal_ws(State(app): State<App>, headers: HeaderMap, Query(q): Query<TerminalQuery>, ws: WebSocketUpgrade) -> Response {
    if terminal_off(&app) {
        return not_found();
    }
    // Browsers send Origin on every WebSocket; without this check any site the user
    // visits could open the shell with their cookie.
    if !headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()).is_some_and(|o| same_origin(&headers, o)) {
        return error(StatusCode::FORBIDDEN, "Запрос отклонён");
    }
    let ip = auth::client_ip(&headers);
    crate::log!("Терминал: подключение (адрес: {ip})");
    if let Some(tg) = app.config.telegram.clone().filter(|_| app.terminal.should_notify(&ip)) {
        let text = format!("Открыт терминал облака (адрес: {ip}). Если это не вы — смените пароль: local-photo --setup");
        tokio::spawn(async move { telegram::send_with_retry(&tg, &text).await });
    }
    let check = app.clone();
    let client = terminal::Client {
        compress: q.z.as_deref() == Some("1"),
        ip: ip.clone(),
        still_allowed: Box::new(move || {
            if !check.auth.is_valid(&headers) {
                Some(terminal::CLOSE_LOGGED_OUT)
            } else if !check.terminal_enabled() {
                Some(terminal::CLOSE_DISABLED)
            } else {
                None
            }
        }),
    };
    let term = app.terminal.clone();
    let id = q.id;
    ws.on_upgrade(move |socket| async move {
        terminal::serve(term, socket, client, id).await;
        crate::log!("Терминал: отключение (адрес: {ip})");
    })
}

// ---------- browsing ----------

#[derive(Deserialize)]
struct ListQuery {
    section: String,
}

async fn list(State(app): State<App>, Query(q): Query<ListQuery>) -> Response {
    let Some(section) = Section::parse(&q.section) else {
        return error(StatusCode::BAD_REQUEST, "Неизвестный раздел");
    };
    let root = app.section_root(section);
    let items = tokio::task::spawn_blocking(move || storage::list(&root)).await.unwrap_or_default();
    let total: u64 = items.iter().map(|i| i.size).sum();
    let free = fs4::available_space(&app.paths.storage).ok();
    Json(json!({ "items": items, "total_size": total, "disk_free": free })).into_response()
}

/// Streams a stored file. Range requests work (video seeking, Safari playback needs them).
/// Only photos/videos may be shown inline; everything else is always a download, so an
/// uploaded .html/.svg can never run as a page on this site.
async fn serve_stored(path: &std::path::Path, inline: bool, req: Request) -> Response {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut res = match ServeFile::new(path).oneshot(req).await {
        Ok(r) => r.map(Body::new),
        Err(_) => return not_found(),
    };
    let h = res.headers_mut();
    h.insert(header::CONTENT_DISPOSITION, content_disposition(if inline { "inline" } else { "attachment" }, &name));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("sandbox; default-src 'none'; img-src 'self'; media-src 'self'"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    res
}

async fn get_file(State(app): State<App>, Query(q): Query<FileQuery>, req: Request) -> Response {
    let Some((section, path)) = q.locate(&app) else { return not_found() };
    let inline = section == Section::Photos && Kind::of(&q.path) != Kind::File && q.download.is_none();
    serve_stored(&path, inline, req).await
}

/// JPEG preview of `path`, cached under `cache_key` (changes with size and mtime).
async fn thumb_response(app: &AppState, cache_ns: &str, cache_key: &str, path: PathBuf) -> Response {
    let Ok(meta) = tokio::fs::metadata(&path).await else { return not_found() };
    let cached = thumbs::cache_path(&app.paths.thumbs, cache_ns, cache_key, &meta);
    if !cached.exists() {
        let _slot = app.thumb_slots.acquire().await;
        if !cached.exists() {
            let (src, dst) = (path.clone(), cached.clone());
            match tokio::task::spawn_blocking(move || thumbs::generate(&src, &dst)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    crate::log!("Превью не создано для {}: {e:#}", path.display());
                    return not_found();
                }
                Err(_) => return not_found(),
            }
        }
    }
    match tokio::fs::read(&cached).await {
        Ok(bytes) => (
            [(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, "private, max-age=604800")],
            bytes,
        )
            .into_response(),
        Err(_) => not_found(),
    }
}

async fn thumb(State(app): State<App>, Query(q): Query<FileQuery>) -> Response {
    let Some((section, path)) = q.locate(&app) else { return not_found() };
    if Kind::of(&q.path) != Kind::Image {
        return not_found();
    }
    thumb_response(&app, section.id(), &q.path, path).await
}

/// Deleting never destroys anything right away: the file goes to the trash for 30 days.
async fn delete_file(State(app): State<App>, Query(q): Query<FileQuery>) -> Response {
    let Some((section, path)) = q.locate(&app) else { return not_found() };
    if let Ok(meta) = std::fs::metadata(&path) {
        let _ = std::fs::remove_file(thumbs::cache_path(&app.paths.thumbs, section.id(), &q.path, &meta));
    }
    let (trash_dir, rel, original) = (app.paths.trash.clone(), q.path.clone(), path.clone());
    let moved = tokio::task::spawn_blocking(move || trash::move_in(&trash_dir, section.id(), &rel, &original)).await;
    let entry = match moved {
        Ok(Ok(e)) => e,
        Ok(Err(e)) => return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Не удалось удалить: {e:#}")),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось удалить"),
    };
    crate::log!("В корзину: {}/{}", section.dir_name(), q.path);
    if let (Some(parent), Ok(root)) = (path.parent(), app.section_root(section).canonicalize()) {
        storage::prune_empty_dirs(&root, parent);
    }
    Json(json!({ "ok": true, "trash_id": entry.id })).into_response()
}

// ---------- editing: rename, replace content, save a copy ----------

/// Upper bound for an edited photo / converted file sent in one request
/// (Cloudflare refuses bodies over ~100 MB).
const MAX_EDIT_BODY: usize = 90 * 1024 * 1024;

#[derive(Deserialize)]
struct RenameBody {
    name: String,
}

async fn rename_file(State(app): State<App>, Query(q): Query<FileQuery>, Json(body): Json<RenameBody>) -> Response {
    let Some((section, path)) = q.locate(&app) else { return not_found() };
    let name = storage::sanitize_filename(&body.name);
    let root = app.section_root(section);
    let _guard = app.place_lock.lock().await;
    if path.file_name().is_some_and(|n| n.to_string_lossy() == name) {
        return Json(json!({ "path": q.path })).into_response();
    }
    let Some(dir) = path.parent() else { return not_found() };
    let dest = storage::unique_path(dir, &name);
    if let Err(e) = std::fs::rename(&path, &dest) {
        return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Не удалось переименовать: {e}"));
    }
    let rel = storage::rel_path(&root, &dest);
    crate::log!("Переименован {}/{} → {}", section.dir_name(), q.path, rel);
    Json(json!({ "path": rel })).into_response()
}

#[derive(Deserialize)]
struct ReplaceQuery {
    section: String,
    path: String,
    /// Put the previous version into the trash (photo edits) instead of overwriting it.
    #[serde(default)]
    keep_original: Option<String>,
    /// Same, but as an editor version: listed in the file's history (`/api/versions`).
    #[serde(default)]
    version: Option<String>,
}

/// Overwrites a file's content in place, keeping its date so it stays where it is in the timeline.
async fn replace_file(State(app): State<App>, Query(q): Query<ReplaceQuery>, body: Bytes) -> Response {
    let Some(section) = Section::parse(&q.section) else { return not_found() };
    let Some(path) = storage::resolve(&app.section_root(section), &q.path) else { return not_found() };
    let Ok(meta) = std::fs::metadata(&path) else { return not_found() };
    let date = storage::mtime_ms(&meta);
    let _guard = app.place_lock.lock().await;

    let (trash_dir, rel, target) = (app.paths.trash.clone(), q.path.clone(), path.clone());
    let keep = q.keep_original.is_some();
    let version = q.version.is_some();
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let dir = target.parent().ok_or_else(|| anyhow::anyhow!("нет папки"))?;
        let tmp = dir.join(format!(".edit-{}.tmp", auth::random_hex(6)));
        std::fs::write(&tmp, &body)?;
        if version {
            trash::keep_version(&trash_dir, section.id(), &rel, &target)?;
        } else if keep {
            trash::move_in(&trash_dir, section.id(), &rel, &target)?;
        }
        std::fs::rename(&tmp, &target)?;
        set_date(&target, date);
        Ok(())
    })
    .await;
    match result {
        Ok(Ok(())) => {
            crate::log!("Изменён {}/{}", section.dir_name(), q.path);
            Json(json!({ "path": q.path })).into_response()
        }
        Ok(Err(e)) => error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Не удалось сохранить: {e:#}")),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось сохранить"),
    }
}

/// Earlier versions of a file saved in the editor (they live in the trash for 30 days).
async fn versions(State(app): State<App>, Query(q): Query<FileQuery>) -> Response {
    if Section::parse(&q.section).is_none() {
        return error(StatusCode::BAD_REQUEST, "Неизвестный раздел");
    }
    let (dir, section, path) = (app.paths.trash.clone(), q.section.clone(), q.path.clone());
    let items = tokio::task::spawn_blocking(move || trash::versions(&dir, &section, &path)).await.unwrap_or_default();
    let items: Vec<_> = items.into_iter().map(|e| json!({ "id": e.id, "size": e.size, "saved_at": e.deleted_at, "date": e.date })).collect();
    Json(json!({ "items": items })).into_response()
}

#[derive(Deserialize)]
struct CopyQuery {
    section: String,
    path: String,
    name: String,
}

/// Saves new content (an edited photo, a converted file) next to the original,
/// with the original's date.
async fn copy_file(State(app): State<App>, Query(q): Query<CopyQuery>, body: Bytes) -> Response {
    let Some(section) = Section::parse(&q.section) else { return not_found() };
    let root = app.section_root(section);
    let Some(path) = storage::resolve(&root, &q.path) else { return not_found() };
    let Ok(meta) = std::fs::metadata(&path) else { return not_found() };
    let Some(dir) = path.parent() else { return not_found() };
    let name = storage::sanitize_filename(&q.name);
    let _guard = app.place_lock.lock().await;
    let dest = storage::unique_path(dir, &name);
    if let Err(e) = std::fs::write(&dest, &body) {
        return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Не удалось сохранить: {e}"));
    }
    set_date(&dest, storage::mtime_ms(&meta));
    let rel = storage::rel_path(&root, &dest);
    crate::log!("Создан {}/{}", section.dir_name(), rel);
    Json(json!({ "section": section.id(), "path": rel })).into_response()
}

fn set_date(path: &std::path::Path, date_ms: i64) {
    let ft = filetime::FileTime::from_unix_time(date_ms.div_euclid(1000), 0);
    let _ = filetime::set_file_times(path, ft, ft);
}

// ---------- download several files as one ZIP ----------

const MAX_ZIP_FILES: usize = 5000;

#[derive(Deserialize)]
struct ZipBody {
    section: String,
    paths: Vec<String>,
}

async fn zip_start(State(app): State<App>, Json(body): Json<ZipBody>) -> Response {
    let Some(section) = Section::parse(&body.section) else { return error(StatusCode::BAD_REQUEST, "Неизвестный раздел") };
    if body.paths.is_empty() || body.paths.len() > MAX_ZIP_FILES {
        return error(StatusCode::BAD_REQUEST, "Выберите от 1 до 5000 файлов");
    }
    let root = app.section_root(section);
    let files: Vec<PathBuf> = body.paths.iter().filter_map(|p| storage::resolve(&root, p)).collect();
    if files.len() != body.paths.len() {
        return error(StatusCode::NOT_FOUND, "Некоторые файлы не найдены — обновите страницу");
    }
    let id = app.zips.start(&app.paths.uploads, files);
    crate::log!("Архив {id}: {} файлов", body.paths.len());
    Json(json!({ "id": id })).into_response()
}

async fn zip_status(State(app): State<App>, UrlPath(id): UrlPath<String>) -> Response {
    let Some(job) = app.zips.get(&id) else { return error(StatusCode::NOT_FOUND, "Архив не найден") };
    let (state, err) = match &*job.state.lock().unwrap() {
        zips::State::Building => ("building", None),
        zips::State::Ready => ("ready", None),
        zips::State::Failed(e) => ("failed", Some(e.clone())),
    };
    let done = job.done.load(std::sync::atomic::Ordering::Relaxed);
    Json(json!({ "state": state, "done": done, "total": job.total, "error": err })).into_response()
}

async fn zip_file(State(app): State<App>, UrlPath(id): UrlPath<String>, req: Request) -> Response {
    let Some(job) = app.zips.get(&id) else { return not_found() };
    if !matches!(*job.state.lock().unwrap(), zips::State::Ready) {
        return error(StatusCode::CONFLICT, "Архив ещё не готов");
    }
    let mut res = match ServeFile::new(&job.path).oneshot(req).await {
        Ok(r) => r.map(Body::new),
        Err(_) => return not_found(),
    };
    let name = format!("Облако {}.zip", chrono::Local::now().format("%Y-%m-%d %H-%M"));
    res.headers_mut().insert(header::CONTENT_DISPOSITION, content_disposition("attachment", &name));
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    res
}

// ---------- notes ----------

async fn notes_list(State(app): State<App>) -> Response {
    let dir = app.paths.notes.clone();
    let items = tokio::task::spawn_blocking(move || notes::list(&dir)).await.unwrap_or_default();
    Json(json!({ "items": items })).into_response()
}

#[derive(Deserialize)]
struct NoteBody {
    #[serde(default)]
    content: String,
    /// `updated` of the version the client edited; a newer one on disk means a conflict.
    #[serde(default)]
    base: Option<i64>,
}

async fn note_create(State(app): State<App>, Json(body): Json<NoteBody>) -> Response {
    let id = notes::new_id();
    match notes::write(&app.paths.notes, &id, &body.content) {
        Ok(updated) => Json(json!({ "id": id, "updated": updated })).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Не удалось создать заметку: {e:#}")),
    }
}

async fn note_get(State(app): State<App>, UrlPath(id): UrlPath<String>) -> Response {
    match notes::read(&app.paths.notes, &id) {
        Some((content, updated)) => Json(json!({ "id": id, "content": content, "updated": updated })).into_response(),
        None => error(StatusCode::NOT_FOUND, "Заметка не найдена"),
    }
}

async fn note_put(State(app): State<App>, UrlPath(id): UrlPath<String>, Json(body): Json<NoteBody>) -> Response {
    if body.content.len() > notes::MAX_NOTE_BYTES {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "Заметка слишком большая");
    }
    let _guard = app.place_lock.lock().await;
    match notes::read(&app.paths.notes, &id) {
        Some((content, updated)) if body.base.is_some_and(|b| b != updated) => {
            return (StatusCode::CONFLICT, Json(json!({
                "error": "Заметка изменена на другом устройстве",
                "content": content,
                "updated": updated,
            })))
                .into_response();
        }
        None if notes::path(&app.paths.notes, &id).is_none() => return error(StatusCode::NOT_FOUND, "Заметка не найдена"),
        _ => {}
    }
    match notes::write(&app.paths.notes, &id, &body.content) {
        Ok(updated) => Json(json!({ "updated": updated })).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Не удалось сохранить: {e:#}")),
    }
}

#[derive(Deserialize)]
struct NoteDeleteQuery {
    /// Empty notes are simply dropped, not put into the trash.
    #[serde(default)]
    permanent: Option<String>,
}

async fn note_delete(State(app): State<App>, UrlPath(id): UrlPath<String>, Query(q): Query<NoteDeleteQuery>) -> Response {
    let Some(path) = notes::path(&app.paths.notes, &id).filter(|p| p.is_file()) else {
        return error(StatusCode::NOT_FOUND, "Заметка не найдена");
    };
    if q.permanent.is_some() {
        let _ = std::fs::remove_file(&path);
        return Json(json!({ "ok": true })).into_response();
    }
    match trash::move_in(&app.paths.trash, "notes", &format!("{id}.md"), &path) {
        Ok(entry) => Json(json!({ "ok": true, "trash_id": entry.id })).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Не удалось удалить: {e:#}")),
    }
}

// ---------- trash ----------

async fn trash_list(State(app): State<App>) -> Response {
    let dir = app.paths.trash.clone();
    let items = tokio::task::spawn_blocking(move || trash::list(&dir)).await.unwrap_or_default();
    let now = trash::now_ms();
    let items: Vec<_> = items
        .into_iter()
        .map(|e| {
            let days_left = ((e.expires_at() - now).max(0) + 86_399_999) / 86_400_000;
            json!({ "id": e.id, "section": e.section, "path": e.path, "name": e.name, "size": e.size,
                    "kind": e.kind, "deleted_at": e.deleted_at, "date": e.date, "days_left": days_left, "version": e.version })
        })
        .collect();
    Json(json!({ "items": items, "retention_days": trash::RETENTION_DAYS })).into_response()
}

async fn trash_restore(State(app): State<App>, UrlPath(id): UrlPath<String>) -> Response {
    let Some((entry, _)) = trash::get(&app.paths.trash, &id) else { return error(StatusCode::NOT_FOUND, "В корзине такого нет") };
    let Some(root) = app.root_for(&entry.section) else { return error(StatusCode::BAD_REQUEST, "Неизвестный раздел") };
    let _guard = app.place_lock.lock().await;
    match trash::restore(&app.paths.trash, &id, &root) {
        Ok(rel) => {
            crate::log!("Восстановлен {}", entry.name);
            Json(json!({ "section": entry.section, "path": rel })).into_response()
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Не удалось восстановить: {e:#}")),
    }
}

async fn trash_delete(State(app): State<App>, UrlPath(id): UrlPath<String>) -> Response {
    match trash::remove(&app.paths.trash, &id) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(_) => error(StatusCode::NOT_FOUND, "В корзине такого нет"),
    }
}

async fn trash_empty(State(app): State<App>) -> Response {
    let dir = app.paths.trash.clone();
    let removed = tokio::task::spawn_blocking(move || {
        trash::list(&dir).iter().filter(|e| trash::remove(&dir, &e.id).is_ok()).count()
    })
    .await
    .unwrap_or(0);
    crate::log!("Корзина очищена ({removed})");
    Json(json!({ "removed": removed })).into_response()
}

async fn trash_file(State(app): State<App>, UrlPath(id): UrlPath<String>, req: Request) -> Response {
    let Some((entry, path)) = trash::get(&app.paths.trash, &id) else { return not_found() };
    serve_stored(&path, entry.section == "photos" && entry.kind != Kind::File, req).await
}

async fn trash_thumb(State(app): State<App>, UrlPath(id): UrlPath<String>) -> Response {
    let Some((entry, path)) = trash::get(&app.paths.trash, &id) else { return not_found() };
    if entry.kind != Kind::Image {
        return not_found();
    }
    thumb_response(&app, "trash", &id, path).await
}

// ---------- uploads ----------

#[derive(Deserialize)]
struct InitBody {
    name: String,
    size: u64,
    #[serde(default)]
    last_modified: i64,
}

/// Kept free on the storage disk, so an upload never fills it up completely.
const DISK_RESERVE: u64 = 1024 * 1024 * 1024;

async fn upload_init(State(app): State<App>, Json(body): Json<InitBody>) -> Response {
    // The size is known up front: refuse now rather than fail after hours of uploading.
    if let Ok(free) = fs4::available_space(&app.paths.uploads) {
        if body.size.saturating_add(DISK_RESERVE) > free {
            return error(
                StatusCode::INSUFFICIENT_STORAGE,
                &format!("Не хватает места на диске: свободно {:.1} ГБ, файл {:.1} ГБ", gb(free.saturating_sub(DISK_RESERVE)), gb(body.size)),
            );
        }
    }
    let id = auth::random_hex(16);
    let tmp = app.paths.uploads.join(format!("{id}.part"));
    let file = match tokio::fs::File::create(&tmp).await {
        Ok(f) => f,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Не удалось начать загрузку: {e}")),
    };
    let upload = Upload {
        name: body.name,
        size: body.size,
        last_modified: body.last_modified,
        received: 0,
        tmp,
        file: Some(file),
        touched: Instant::now(),
    };
    app.uploads.lock().unwrap().insert(id.clone(), Arc::new(Mutex::new(upload)));
    Json(json!({ "id": id })).into_response()
}

fn find_upload(app: &AppState, id: &str) -> Option<Arc<Mutex<Upload>>> {
    app.uploads.lock().unwrap().get(id).cloned()
}

async fn upload_status(State(app): State<App>, UrlPath(id): UrlPath<String>) -> Response {
    let Some(u) = find_upload(&app, &id) else { return error(StatusCode::NOT_FOUND, "Загрузка не найдена") };
    let received = u.lock().await.received;
    Json(json!({ "received": received })).into_response()
}

#[derive(Deserialize)]
struct ChunkQuery {
    offset: u64,
}

async fn upload_chunk(
    State(app): State<App>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<ChunkQuery>,
    body: Bytes,
) -> Response {
    let Some(u) = find_upload(&app, &id) else { return error(StatusCode::NOT_FOUND, "Загрузка не найдена") };
    let mut u = u.lock().await;
    u.touched = Instant::now();
    // The client resends a chunk after a network error; tell it where we actually are.
    if q.offset != u.received {
        let received = u.received;
        return (StatusCode::CONFLICT, Json(json!({ "error": "Смещение не совпадает", "received": received }))).into_response();
    }
    if u.received + body.len() as u64 > u.size {
        return error(StatusCode::BAD_REQUEST, "Данных больше, чем заявлено");
    }
    let Some(file) = u.file.as_mut() else { return error(StatusCode::GONE, "Загрузка закрыта") };
    if let Err(e) = file.write_all(&body).await {
        return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Ошибка записи: {e}"));
    }
    u.received += body.len() as u64;
    Json(json!({ "received": u.received })).into_response()
}

async fn upload_finish(State(app): State<App>, UrlPath(id): UrlPath<String>) -> Response {
    let Some(u) = find_upload(&app, &id) else { return error(StatusCode::NOT_FOUND, "Загрузка не найдена") };
    let mut u = u.lock().await;
    if u.received != u.size {
        return error(StatusCode::BAD_REQUEST, "Файл загружен не полностью");
    }
    if let Some(mut file) = u.file.take() {
        if let Err(e) = file.flush().await.and(file.sync_all().await) {
            return error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Ошибка записи: {e}"));
        }
    }
    app.uploads.lock().unwrap().remove(&id);

    let name = storage::sanitize_filename(&u.name);
    let (tmp, last_modified) = (u.tmp.clone(), u.last_modified);
    let _guard = app.place_lock.lock().await;
    let storage_root = app.paths.storage.clone();
    let placed = tokio::task::spawn_blocking(move || place_file(&storage_root, &tmp, &name, last_modified)).await;
    match placed {
        Ok(Ok((section, rel))) => {
            crate::log!("Загружен {}/{rel}", section.dir_name());
            Json(json!({ "section": section.id(), "path": rel })).into_response()
        }
        Ok(Err(e)) => {
            let _ = std::fs::remove_file(&u.tmp);
            error(StatusCode::INTERNAL_SERVER_ERROR, &format!("Не удалось сохранить: {e:#}"))
        }
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Не удалось сохранить"),
    }
}

/// Files a finished temp file into the storage (used by uploads and the Telegram bot).
pub async fn store_file(app: &AppState, tmp: PathBuf, name: &str, client_mtime_ms: i64) -> anyhow::Result<(Section, String)> {
    let name = storage::sanitize_filename(name);
    let _guard = app.place_lock.lock().await;
    let storage_root = app.paths.storage.clone();
    let tmp2 = tmp.clone();
    let placed = tokio::task::spawn_blocking(move || place_file(&storage_root, &tmp2, &name, client_mtime_ms)).await?;
    if placed.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    placed
}

/// Picks the section and YYYY/MM folder from the file's own date, moves it there,
/// and stamps that date as the file's modification time.
fn place_file(storage_root: &std::path::Path, tmp: &std::path::Path, name: &str, client_mtime_ms: i64) -> anyhow::Result<(Section, String)> {
    use chrono::{Local, TimeZone};

    let kind = Kind::of(name);
    let section = kind.section();
    let date = storage::capture_date(tmp, kind)
        .or_else(|| (client_mtime_ms > 0).then(|| Local.timestamp_millis_opt(client_mtime_ms).single()).flatten())
        .unwrap_or_else(Local::now);

    let folder = storage::month_folder(&date);
    let section_root = storage_root.join(section.dir_name());
    let dir = folder.split('/').fold(section_root, |p, part| p.join(part));
    std::fs::create_dir_all(&dir)?;
    let dest = storage::unique_path(&dir, name);
    if std::fs::rename(tmp, &dest).is_err() {
        // Different disk: copy, then remove the temp file.
        std::fs::copy(tmp, &dest)?;
        std::fs::remove_file(tmp)?;
    }
    let ft = filetime::FileTime::from_unix_time(date.timestamp(), 0);
    let _ = filetime::set_file_times(&dest, ft, ft);

    let file_name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok((section, format!("{folder}/{file_name}")))
}
