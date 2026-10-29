//! Notes: plain Markdown files in `Хранилище/Заметки/<id>.md`, so they are readable
//! on the computer too. The title is the first line, like in iOS Notes.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

pub const MAX_NOTE_BYTES: usize = 1024 * 1024;

#[derive(Serialize)]
pub struct NoteMeta {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub updated: i64,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn path(dir: &Path, id: &str) -> Option<PathBuf> {
    valid_id(id).then(|| dir.join(format!("{id}.md")))
}

pub fn new_id() -> String {
    crate::auth::random_hex(8)
}

/// First non-empty line (without Markdown '#') and the text that follows it.
pub fn title_and_preview(content: &str) -> (String, String) {
    let mut lines = content.lines().map(str::trim).filter(|l| !l.is_empty());
    let title: String = lines
        .next()
        .map(|l| l.trim_start_matches('#').trim())
        .unwrap_or("")
        .chars()
        .take(80)
        .collect();
    let preview: String = lines.collect::<Vec<_>>().join(" ").chars().take(140).collect();
    (if title.is_empty() { "Новая заметка".into() } else { title }, preview)
}

pub fn list(dir: &Path) -> Vec<NoteMeta> {
    let mut notes: Vec<NoteMeta> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".md")?;
            if !valid_id(id) {
                return None;
            }
            let (content, updated) = read(dir, id)?;
            let (title, preview) = title_and_preview(&content);
            Some(NoteMeta { id: id.to_string(), title, preview, updated })
        })
        .collect();
    notes.sort_by_key(|n| std::cmp::Reverse(n.updated));
    notes
}

/// Content and last-modified time (ms) of a note.
pub fn read(dir: &Path, id: &str) -> Option<(String, i64)> {
    let p = path(dir, id)?;
    let content = std::fs::read_to_string(&p).ok()?;
    let updated = crate::storage::mtime_ms(&std::fs::metadata(&p).ok()?);
    Some((content, updated))
}

/// Saves atomically (temp file + rename) and returns the new modified time.
pub fn write(dir: &Path, id: &str, content: &str) -> Result<i64> {
    let p = path(dir, id).ok_or_else(|| anyhow::anyhow!("неверный id"))?;
    let tmp = dir.join(format!(".{id}.tmp"));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, &p)?;
    Ok(crate::storage::mtime_ms(&std::fs::metadata(&p)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(title_and_preview("# Покупки\n\nмолоко\nхлеб"), ("Покупки".into(), "молоко хлеб".into()));
        assert_eq!(title_and_preview("  \n"), ("Новая заметка".into(), String::new()));
        assert!(path(Path::new("/x"), "../../etc/passwd").is_none());
    }
}
