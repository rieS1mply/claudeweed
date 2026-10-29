//! "Недавно удалённые": deleted files wait here for 30 days before they are gone for good.
//!
//! Each entry is a folder `<trash>/<id>/` holding the file under its original name
//! plus `meta.json` with where it came from.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::storage::{self, Kind};

pub const RETENTION_DAYS: i64 = 30;
const DAY_MS: i64 = 86_400_000;
const META: &str = "meta.json";

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub id: String,
    /// "photos", "files" or "notes".
    pub section: String,
    /// Original location, relative to the section folder.
    pub path: String,
    pub name: String,
    pub size: u64,
    pub kind: Kind,
    pub deleted_at: i64,
    /// The file's own date (its mtime), put back on restore.
    pub date: i64,
}

impl Entry {
    pub fn expires_at(&self) -> i64 {
        self.deleted_at + RETENTION_DAYS * DAY_MS
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to).or_else(|_| {
        std::fs::copy(from, to)?;
        std::fs::remove_file(from)
    })
}

/// Moves `file` (which lives at `rel` inside `section`) into the trash.
pub fn move_in(trash: &Path, section: &str, rel: &str, file: &Path) -> Result<Entry> {
    let meta = std::fs::metadata(file)?;
    let name = file.file_name().context("у файла нет имени")?.to_string_lossy().into_owned();
    let entry = Entry {
        id: crate::auth::random_hex(12),
        section: section.to_string(),
        path: rel.to_string(),
        kind: Kind::of(&name),
        size: meta.len(),
        deleted_at: now_ms(),
        date: storage::mtime_ms(&meta),
        name,
    };
    let dir = trash.join(&entry.id);
    std::fs::create_dir_all(&dir)?;
    // Metadata first: a file in the trash without it could never be restored or purged.
    std::fs::write(dir.join(META), serde_json::to_vec_pretty(&entry)?)?;
    if let Err(e) = move_file(file, &dir.join(&entry.name)) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e.into());
    }
    Ok(entry)
}

pub fn get(trash: &Path, id: &str) -> Option<(Entry, PathBuf)> {
    if !valid_id(id) {
        return None;
    }
    let dir = trash.join(id);
    let entry: Entry = serde_json::from_slice(&std::fs::read(dir.join(META)).ok()?).ok()?;
    let file = dir.join(&entry.name);
    file.is_file().then_some((entry, file))
}

pub fn list(trash: &Path) -> Vec<Entry> {
    let mut items: Vec<Entry> = std::fs::read_dir(trash)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| get(trash, &e.file_name().to_string_lossy()).map(|(entry, _)| entry))
        .collect();
    items.sort_by_key(|e| std::cmp::Reverse(e.deleted_at));
    items
}

pub fn remove(trash: &Path, id: &str) -> Result<()> {
    if !valid_id(id) {
        bail!("неверный id");
    }
    std::fs::remove_dir_all(trash.join(id))?;
    Ok(())
}

/// Puts the file back where it was (next to it, if that name is taken now).
/// Returns the restored path relative to `section_root`.
pub fn restore(trash: &Path, id: &str, section_root: &Path) -> Result<String> {
    let (entry, file) = get(trash, id).context("в корзине такого нет")?;
    let wanted = storage::safe_join(section_root, &entry.path).context("неверный путь")?;
    let dir = wanted.parent().context("неверный путь")?;
    std::fs::create_dir_all(dir)?;
    let dest = storage::unique_path(dir, &entry.name);
    move_file(&file, &dest)?;
    let ft = filetime::FileTime::from_unix_time(entry.date.div_euclid(1000), 0);
    let _ = filetime::set_file_times(&dest, ft, ft);
    let _ = std::fs::remove_dir_all(trash.join(id));
    Ok(storage::rel_path(section_root, &dest))
}

/// Deletes everything older than the retention period. Returns how many entries went.
pub fn purge_expired(trash: &Path) -> usize {
    let now = now_ms();
    list(trash)
        .into_iter()
        .filter(|e| now >= e.expires_at())
        .filter(|e| remove(trash, &e.id).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_and_restore_round_trip() {
        let base = std::env::temp_dir().join(format!("lp-trash-{}", std::process::id()));
        let (root, trash) = (base.join("Фото"), base.join("trash"));
        std::fs::create_dir_all(root.join("2024/03-Март")).unwrap();
        std::fs::create_dir_all(&trash).unwrap();
        let file = root.join("2024/03-Март/a.jpg");
        std::fs::write(&file, "x").unwrap();

        let entry = move_in(&trash, "photos", "2024/03-Март/a.jpg", &file).unwrap();
        assert!(!file.exists());
        assert_eq!(list(&trash).len(), 1);

        // Something new took the name meanwhile: restore must not overwrite it.
        std::fs::write(&file, "new").unwrap();
        let rel = restore(&trash, &entry.id, &root).unwrap();
        assert_eq!(rel, "2024/03-Март/a (2).jpg");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
        assert!(list(&trash).is_empty());
        assert!(get(&trash, "../etc").is_none());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn purges_only_expired() {
        let base = std::env::temp_dir().join(format!("lp-purge-{}", std::process::id()));
        let trash = base.join("trash");
        std::fs::create_dir_all(&trash).unwrap();
        for (name, age_days) in [("old.txt", RETENTION_DAYS + 1), ("fresh.txt", RETENTION_DAYS - 1)] {
            let f = base.join(name);
            std::fs::write(&f, "x").unwrap();
            let mut e = move_in(&trash, "files", name, &f).unwrap();
            e.deleted_at -= age_days * DAY_MS;
            std::fs::write(trash.join(&e.id).join(META), serde_json::to_vec(&e).unwrap()).unwrap();
        }
        assert_eq!(purge_expired(&trash), 1);
        let left: Vec<String> = list(&trash).into_iter().map(|e| e.name).collect();
        assert_eq!(left, vec!["fresh.txt"]);
        std::fs::remove_dir_all(&base).unwrap();
    }
}
