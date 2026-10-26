//! Sessions and protection against password guessing.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use sha2::{Digest, Sha256};

pub const COOKIE: &str = "lp_session";
pub const SESSION_TTL: Duration = Duration::from_secs(30 * 24 * 3600);

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
    /// SHA-256 of the session token -> expiry (ms since epoch). Only hashes are kept,
    /// so the sessions file on disk can't be used to log in.
    sessions: Mutex<HashMap<String, i64>>,
    /// Where sessions are saved, so a restart doesn't log everyone out.
    store: Option<PathBuf>,
    limiter: Mutex<Limiter>,
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

impl Auth {
    /// Sessions survive restarts by living in `path` (hashes and expiry only).
    pub fn with_store(path: &Path) -> Auth {
        let sessions: HashMap<String, i64> = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let now = now_ms();
        let sessions = sessions.into_iter().filter(|(_, exp)| *exp > now).collect();
        Auth { sessions: Mutex::new(sessions), store: Some(path.to_path_buf()), ..Default::default() }
    }

    fn save(&self, sessions: &HashMap<String, i64>) {
        let Some(path) = &self.store else { return };
        let tmp = path.with_extension("tmp");
        if serde_json::to_vec(sessions).ok().and_then(|b| std::fs::write(&tmp, b).ok()).is_some() {
            crate::config::restrict_permissions(&tmp);
            let _ = std::fs::rename(&tmp, path);
        }
    }

    pub fn create_session(&self) -> String {
        let token = random_hex(32);
        let mut sessions = self.sessions.lock().unwrap();
        let now = now_ms();
        sessions.retain(|_, exp| *exp > now);
        sessions.insert(token_hash(&token), now + SESSION_TTL.as_millis() as i64);
        self.save(&sessions);
        token
    }

    pub fn is_valid(&self, headers: &HeaderMap) -> bool {
        let Some(token) = session_cookie(headers) else { return false };
        let sessions = self.sessions.lock().unwrap();
        sessions.get(&token_hash(token)).is_some_and(|exp| *exp > now_ms())
    }

    pub fn end_session(&self, headers: &HeaderMap) {
        if let Some(token) = session_cookie(headers) {
            let mut sessions = self.sessions.lock().unwrap();
            if sessions.remove(&token_hash(token)).is_some() {
                self.save(&sessions);
            }
        }
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

/// Real visitor address. Behind the tunnel every request comes from 127.0.0.1,
/// and Cloudflare puts the original address here (clients cannot forge it through Cloudflare).
pub fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("local")
        .to_string()
}

pub fn is_https(headers: &HeaderMap) -> bool {
    let header = |name| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("");
    header("x-forwarded-proto") == "https" || header("cf-visitor").contains("\"https\"")
}

/// The request came from the internet through the Cloudflare tunnel (not from this computer).
pub fn via_cloudflare(headers: &HeaderMap) -> bool {
    headers.contains_key("cf-connecting-ip")
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
        let token = Auth::with_store(&path).create_session();
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
    fn cookie_parsing() {
        let mut h = HeaderMap::new();
        h.insert("cookie", "a=1; lp_session=abc; b=2".parse().unwrap());
        assert_eq!(session_cookie(&h), Some("abc"));
        h.insert("cookie", "lp_session_x=zzz".parse().unwrap());
        assert_eq!(session_cookie(&h), None);
    }
}
