//! Sessions (with the device each one belongs to), the list of login attempts and protection
//! against password guessing.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const COOKIE: &str = "lp_session";
pub const SESSION_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
/// "Last active" is written to disk at most this often per session.
const SEEN_EVERY_MS: i64 = 60_000;
/// Login attempts kept for the owner's list.
const MAX_LOGINS: usize = 100;
/// A Telegram warning about guessing, per address, at most this often.
const ALERT_EVERY: Duration = Duration::from_secs(3600);
/// Session ids shown to the page: a prefix of the token's hash (not the token).
const ID_LEN: usize = 16;

/// One logged-in device. The map key is the SHA-256 of its token; the token itself is never kept.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Session {
    /// Milliseconds since epoch, like the other times here.
    pub expires: i64,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub seen: i64,
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub agent: String,
    /// How it logged in: "password" or "telegram".
    #[serde(default)]
    pub via: String,
}

/// Versions before 2026-10-30 stored only the expiry.
#[derive(Deserialize)]
#[serde(untagged)]
enum Stored {
    Full(Session),
    Expiry(i64),
}

/// A login attempt, for the owner's list.
#[derive(Serialize, Deserialize, Clone)]
pub struct LoginEvent {
    pub time: i64,
    pub ip: String,
    pub agent: String,
    pub via: String,
    pub ok: bool,
}

/// A session as the owner's list shows it.
pub struct SessionView {
    pub id: String,
    pub session: Session,
    pub current: bool,
}

/// The id under which a session appears in the owner's list (and in Telegram buttons).
pub fn session_id(token: &str) -> String {
    token_hash(token)[..ID_LEN].to_string()
}

/// User-Agent as stored: enough to tell devices apart, not a whole header's worth.
pub fn short_agent(headers: &HeaderMap) -> String {
    let ua = headers.get(axum::http::header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("");
    ua.chars().take(300).collect()
}

/// Wrong attempts allowed from one address before it gets locked out.
const FREE_ATTEMPTS: u32 = 5;
const FIRST_LOCKOUT: Duration = Duration::from_secs(30);
const MAX_LOCKOUT: Duration = Duration::from_secs(3600);
/// Global brake in case someone rotates addresses.
const GLOBAL_WINDOW: Duration = Duration::from_secs(3600);
const GLOBAL_MAX_FAILURES: usize = 50;
const GLOBAL_LOCKOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Default)]
pub struct Auth {
    /// SHA-256 of the session token -> the session. Only hashes are kept,
    /// so the sessions file on disk can't be used to log in.
    sessions: Mutex<HashMap<String, Session>>,
    /// Where sessions are saved, so a restart doesn't log everyone out.
    store: Option<PathBuf>,
    /// Recent login attempts, oldest first (saved next to the sessions).
    logins: Mutex<VecDeque<LoginEvent>>,
    limiter: Mutex<Limiter>,
    /// When each address last caused a Telegram warning about guessing.
    alerted: Mutex<HashMap<String, Instant>>,
}

#[derive(Default)]
struct Limiter {
    per_ip: HashMap<String, IpState>,
    recent_failures: VecDeque<Instant>,
    global_locked_until: Option<Instant>,
}

struct IpState {
    failures: u32,
    locked_until: Option<Instant>,
    last_failure: Instant,
}

fn now_ms() -> i64 {
    crate::trash::now_ms()
}

fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn write_private(path: &Path, value: &impl Serialize) {
    let tmp = path.with_extension("tmp");
    if serde_json::to_vec(value).ok().and_then(|b| std::fs::write(&tmp, b).ok()).is_some() {
        crate::config::restrict_permissions(&tmp);
        let _ = std::fs::rename(&tmp, path);
    }
}

impl Auth {
    /// Sessions survive restarts by living in `path` (hashes, never tokens); login attempts
    /// live next to it in logins.json.
    pub fn with_store(path: &Path) -> Auth {
        let stored: HashMap<String, Stored> = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let now = now_ms();
        let sessions = stored
            .into_iter()
            .map(|(hash, s)| match s {
                Stored::Full(s) => (hash, s),
                Stored::Expiry(expires) => (hash, Session { expires, ..Default::default() }),
            })
            .filter(|(_, s)| s.expires > now)
            .collect();
        let logins: VecDeque<LoginEvent> = std::fs::read(Self::logins_path(path))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Auth { sessions: Mutex::new(sessions), store: Some(path.to_path_buf()), logins: Mutex::new(logins), ..Default::default() }
    }

    fn logins_path(sessions: &Path) -> PathBuf {
        sessions.with_file_name("logins.json")
    }

    fn save(&self, sessions: &HashMap<String, Session>) {
        if let Some(path) = &self.store {
            write_private(path, sessions);
        }
    }

    /// A new session for a device that just logged in (`via`: "password" or "telegram").
    pub fn create_session(&self, ip: &str, agent: &str, via: &str) -> String {
        let token = random_hex(32);
        let mut sessions = self.sessions.lock().unwrap();
        let now = now_ms();
        sessions.retain(|_, s| s.expires > now);
        let session = Session {
            expires: now + SESSION_TTL.as_millis() as i64,
            created: now,
            seen: now,
            ip: ip.to_string(),
            agent: agent.to_string(),
            via: via.to_string(),
        };
        sessions.insert(token_hash(&token), session);
        self.save(&sessions);
        token
    }

    /// Also notes when (and from where) the device was last active, for the owner's list.
    pub fn is_valid(&self, headers: &HeaderMap) -> bool {
        let Some(token) = session_cookie(headers) else { return false };
        let mut sessions = self.sessions.lock().unwrap();
        let now = now_ms();
        let Some(session) = sessions.get_mut(&token_hash(token)).filter(|s| s.expires > now) else { return false };
        if now - session.seen > SEEN_EVERY_MS {
            session.seen = now;
            session.ip = client_ip(headers);
            self.save(&sessions);
        }
        true
    }

    pub fn end_session(&self, headers: &HeaderMap) {
        if let Some(token) = session_cookie(headers) {
            let mut sessions = self.sessions.lock().unwrap();
            if sessions.remove(&token_hash(token)).is_some() {
                self.save(&sessions);
            }
        }
    }

    /// Every live session, the most recently active first.
    pub fn list(&self, headers: &HeaderMap) -> Vec<SessionView> {
        let current = session_cookie(headers).map(token_hash);
        let now = now_ms();
        let sessions = self.sessions.lock().unwrap();
        let mut list: Vec<SessionView> = sessions
            .iter()
            .filter(|(_, s)| s.expires > now)
            .map(|(hash, s)| SessionView { id: hash[..ID_LEN].to_string(), session: s.clone(), current: current.as_deref() == Some(hash) })
            .collect();
        list.sort_by_key(|v| std::cmp::Reverse(v.session.seen));
        list
    }

    /// Ends the session with this id (from [`list`](Self::list)); the ended session, if any.
    pub fn end_by_id(&self, id: &str) -> Option<Session> {
        if id.len() != ID_LEN {
            return None;
        }
        let mut sessions = self.sessions.lock().unwrap();
        let hash = sessions.keys().find(|h| h.starts_with(id))?.clone();
        let ended = sessions.remove(&hash);
        self.save(&sessions);
        ended
    }

    /// Ends every session but the one making the request (none, if `headers` has no session).
    pub fn end_others(&self, headers: &HeaderMap) -> usize {
        let keep = session_cookie(headers).map(token_hash);
        let mut sessions = self.sessions.lock().unwrap();
        let before = sessions.len();
        sessions.retain(|hash, _| keep.as_deref() == Some(hash));
        let ended = before - sessions.len();
        self.save(&sessions);
        ended
    }

    pub fn record_login(&self, ip: &str, agent: &str, via: &str, ok: bool) {
        let mut logins = self.logins.lock().unwrap();
        logins.push_back(LoginEvent { time: now_ms(), ip: ip.to_string(), agent: agent.to_string(), via: via.to_string(), ok });
        while logins.len() > MAX_LOGINS {
            logins.pop_front();
        }
        if let Some(path) = &self.store {
            write_private(&Self::logins_path(path), &*logins);
        }
    }

    /// Login attempts, newest first.
    pub fn logins(&self) -> Vec<LoginEvent> {
        self.logins.lock().unwrap().iter().rev().cloned().collect()
    }

    /// True at most once an hour per address: worth a Telegram warning.
    pub fn should_alert(&self, ip: &str) -> bool {
        let mut alerted = self.alerted.lock().unwrap();
        alerted.retain(|_, t| t.elapsed() < ALERT_EVERY);
        alerted.insert(ip.to_string(), Instant::now()).is_none()
    }

    /// `Err(wait)` when logins from this address are currently blocked.
    pub fn check_allowed(&self, ip: &str) -> Result<(), Duration> {
        let mut l = self.limiter.lock().unwrap();
        let now = Instant::now();
        if let Some(until) = l.global_locked_until {
            if until > now {
                return Err(until - now);
            }
            l.global_locked_until = None;
        }
        match l.per_ip.get(ip).and_then(|s| s.locked_until) {
            Some(until) if until > now => Err(until - now),
            _ => Ok(()),
        }
    }

    pub fn record_failure(&self, ip: &str) {
        let mut l = self.limiter.lock().unwrap();
        let now = Instant::now();

        // Forget addresses that have been quiet for a day.
        l.per_ip.retain(|_, s| now.duration_since(s.last_failure) < Duration::from_secs(86400));
        let state = l.per_ip.entry(ip.to_string()).or_insert(IpState { failures: 0, locked_until: None, last_failure: now });
        state.failures += 1;
        state.last_failure = now;
        if state.failures >= FREE_ATTEMPTS {
            let doublings = (state.failures - FREE_ATTEMPTS).min(10);
            let lock = (FIRST_LOCKOUT * 2u32.pow(doublings)).min(MAX_LOCKOUT);
            state.locked_until = Some(now + lock);
        }

        while l.recent_failures.front().is_some_and(|t| now.duration_since(*t) > GLOBAL_WINDOW) {
            l.recent_failures.pop_front();
        }
        l.recent_failures.push_back(now);
        if l.recent_failures.len() >= GLOBAL_MAX_FAILURES {
            l.global_locked_until = Some(now + GLOBAL_LOCKOUT);
            l.recent_failures.clear();
        }
    }

    pub fn record_success(&self, ip: &str) {
        self.limiter.lock().unwrap().per_ip.remove(ip);
    }
}

fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|pair| pair.trim().strip_prefix(COOKIE)?.strip_prefix('='))
}

/// Real visitor address. Behind a tunnel every request comes from 127.0.0.1: Cloudflare puts
/// the original address into CF-Connecting-IP, Tailscale Funnel into X-Forwarded-For (both
/// overwrite what the visitor sent, so it can't be forged from outside).
pub fn client_ip(headers: &HeaderMap) -> String {
    let header = |name| headers.get(name).and_then(|v| v.to_str().ok());
    header("cf-connecting-ip")
        .or_else(|| header("x-forwarded-for").and_then(|v| v.split(',').next()).map(str::trim).filter(|v| !v.is_empty()))
        .unwrap_or("local")
        .to_string()
}

pub fn is_https(headers: &HeaderMap) -> bool {
    let header = |name| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("");
    header("x-forwarded-proto") == "https" || header("cf-visitor").contains("\"https\"")
        // Funnel only exists as https://….ts.net.
        || headers.contains_key("tailscale-funnel-request")
}

/// The request came from the internet through a tunnel (Cloudflare or Tailscale Funnel),
/// not from this computer. Both proxies add these headers to every request they pass on.
pub fn via_tunnel(headers: &HeaderMap) -> bool {
    ["cf-connecting-ip", "x-forwarded-for", "tailscale-funnel-request"].iter().any(|h| headers.contains_key(*h))
}

pub fn session_cookie_header(token: &str, secure: bool) -> String {
    format!(
        "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        SESSION_TTL.as_secs(),
        if secure { "; Secure" } else { "" }
    )
}

pub fn clear_cookie_header() -> String {
    format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0")
}

pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::getrandom(&mut buf).expect("OS random generator");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lockout_after_free_attempts() {
        let auth = Auth::default();
        for _ in 0..FREE_ATTEMPTS - 1 {
            auth.record_failure("1.2.3.4");
            assert!(auth.check_allowed("1.2.3.4").is_ok());
        }
        auth.record_failure("1.2.3.4");
        assert!(auth.check_allowed("1.2.3.4").is_err());
        assert!(auth.check_allowed("5.6.7.8").is_ok());
    }

    #[test]
    fn sessions_survive_restart_as_hashes() {
        let path = std::env::temp_dir().join(format!("lp-sessions-{}.json", std::process::id()));
        let token = Auth::with_store(&path).create_session("1.2.3.4", "test", "password");
        let mut h = HeaderMap::new();
        h.insert("cookie", format!("lp_session={token}").parse().unwrap());
        let restarted = Auth::with_store(&path);
        assert!(restarted.is_valid(&h));
        assert!(!std::fs::read_to_string(&path).unwrap().contains(&token), "raw token must not be stored");
        restarted.end_session(&h);
        assert!(!Auth::with_store(&path).is_valid(&h));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn old_sessions_file_still_loads() {
        let path = std::env::temp_dir().join(format!("lp-sessions-old-{}.json", std::process::id()));
        let future = now_ms() + 1_000_000;
        std::fs::write(&path, format!(r#"{{"{}":{future}}}"#, token_hash("tok"))).unwrap();
        let mut h = HeaderMap::new();
        h.insert("cookie", "lp_session=tok".parse().unwrap());
        assert!(Auth::with_store(&path).is_valid(&h));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn list_and_end_sessions() {
        let auth = Auth::default();
        let mine = auth.create_session("1.1.1.1", "phone", "password");
        auth.create_session("2.2.2.2", "laptop", "telegram");
        auth.create_session("3.3.3.3", "other", "password");
        let mut h = HeaderMap::new();
        h.insert("cookie", format!("lp_session={mine}").parse().unwrap());
        let list = auth.list(&h);
        assert_eq!(list.len(), 3);
        assert_eq!(list.iter().filter(|v| v.current).count(), 1);
        let laptop = list.iter().find(|v| v.session.agent == "laptop").unwrap().id.clone();
        assert!(auth.end_by_id(&laptop).is_some());
        assert!(auth.end_by_id(&laptop).is_none());
        assert!(auth.end_by_id("short").is_none());
        assert_eq!(auth.end_others(&h), 1);
        assert!(auth.is_valid(&h));
        assert_eq!(auth.list(&h).len(), 1);
    }

    #[test]
    fn alerts_once_per_address() {
        let auth = Auth::default();
        assert!(auth.should_alert("9.9.9.9"));
        assert!(!auth.should_alert("9.9.9.9"));
        assert!(auth.should_alert("8.8.8.8"));
    }

    #[test]
    fn cookie_parsing() {
        let mut h = HeaderMap::new();
        h.insert("cookie", "a=1; lp_session=abc; b=2".parse().unwrap());
        assert_eq!(session_cookie(&h), Some("abc"));
        h.insert("cookie", "lp_session_x=zzz".parse().unwrap());
        assert_eq!(session_cookie(&h), None);
    }
}
