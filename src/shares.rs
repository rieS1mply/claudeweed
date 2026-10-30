//! Links that open a file or an album without the password, for a limited time.
//! Kept in `<settings>/shares.json`; the link itself is the only key, so it is long and random.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_DAYS: i64 = 30;
pub const MAX_FILES: usize = 500;
const DAY_MS: i64 = 86_400_000;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Share {
    /// The secret part of the link (`/s/<token>`). Kept so the owner can copy the link again.
    pub token: String,
    pub section: String,
    pub paths: Vec<String>,
    /// What the recipient sees as the title: the file name, or "N файлов".
    pub name: String,
    pub created: i64,
    pub expires: i64,
}

pub struct Shares {
    file: PathBuf,
    items: Mutex<Vec<Share>>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Compares hashes, not the tokens themselves: the time taken says nothing about the token.
fn same_token(a: &str, b: &str) -> bool {
    Sha256::digest(a.as_bytes()) == Sha256::digest(b.as_bytes())
}

impl Shares {
    pub fn load(file: &Path) -> Shares {
        let items = std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Shares { file: file.to_path_buf(), items: Mutex::new(items) }
    }

    fn save(&self, items: &[Share]) {
        let tmp = self.file.with_extension("tmp");
        if let Ok(json) = serde_json::to_vec_pretty(items) {
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.file);
            }
        }
    }

    /// Drops expired links; returns the ones still valid.
    fn live(&self) -> std::sync::MutexGuard<'_, Vec<Share>> {
        let mut items = self.items.lock().unwrap();
        let now = now_ms();
        let before = items.len();
        items.retain(|s| s.expires > now);
        if items.len() != before {
            self.save(&items);
        }
        items
    }

    pub fn create(&self, section: &str, paths: Vec<String>, name: String, days: i64) -> Share {
        let created = now_ms();
        let share = Share {
            token: crate::auth::random_hex(16),
            section: section.to_string(),
            paths,
            name,
            created,
            expires: created + days.clamp(1, MAX_DAYS) * DAY_MS,
        };
        let mut items = self.live();
        items.push(share.clone());
        self.save(&items);
        share
    }

    pub fn list(&self) -> Vec<Share> {
        let mut items = self.live().clone();
        items.sort_by_key(|s| std::cmp::Reverse(s.created));
        items
    }

    pub fn get(&self, token: &str) -> Option<Share> {
        if token.len() != 32 {
            return None;
        }
        self.live().iter().find(|s| same_token(&s.token, token)).cloned()
    }

    pub fn revoke(&self, token: &str) -> bool {
        let mut items = self.live();
        let before = items.len();
        items.retain(|s| !same_token(&s.token, token));
        let removed = items.len() != before;
        if removed {
            self.save(&items);
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_find_revoke_expire() {
        let file = std::env::temp_dir().join(format!("lp-shares-{}.json", std::process::id()));
        let shares = Shares::load(&file);
        let s = shares.create("photos", vec!["2026/10-Октябрь/a.jpg".into()], "a.jpg".into(), 99);
        assert_eq!(s.token.len(), 32);
        assert_eq!(s.expires - s.created, MAX_DAYS * DAY_MS);
        assert!(shares.get(&s.token).is_some());
        assert!(shares.get(&s.token[..31]).is_none());
        // Survives a restart.
        assert_eq!(Shares::load(&file).list().len(), 1);
        assert!(shares.revoke(&s.token));
        assert!(shares.get(&s.token).is_none());
        // Expired links disappear by themselves.
        let old = shares.create("files", vec!["x.txt".into()], "x.txt".into(), 1);
        shares.items.lock().unwrap()[0].expires = now_ms() - 1;
        assert!(shares.get(&old.token).is_none());
        assert!(shares.list().is_empty());
        let _ = std::fs::remove_file(&file);
    }
}
