//! Log lines go to the console and to `local-photo.log` next to the settings,
//! so a cloud running in the background (no window) can still be diagnosed.
//!
//! The log is rotated while running: past [`MAX_LOG`] it becomes `local-photo.1.log`, the
//! previous one `local-photo.2.log`, and the oldest is dropped — at most 3 × 5 MB on disk.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const MAX_LOG: u64 = 5 * 1024 * 1024;
/// Older copies kept next to the current log.
const KEEP_OLD: u32 = 2;

struct Log {
    path: PathBuf,
    file: File,
    size: u64,
}

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

pub fn to_file(path: &Path) {
    // Left by versions that kept one ".old.log" copy.
    let _ = std::fs::remove_file(path.with_extension("old.log"));
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG) {
        rotate(path);
    }
    if let Some(log) = open(path) {
        let _ = LOG.set(Mutex::new(log));
    }
}

fn open(path: &Path) -> Option<Log> {
    let file = OpenOptions::new().create(true).append(true).open(path).ok()?;
    let size = file.metadata().map_or(0, |m| m.len());
    Some(Log { path: path.to_path_buf(), file, size })
}

/// `name.log` → `name.1.log` → `name.2.log` → gone.
fn old_copy(path: &Path, n: u32) -> PathBuf {
    path.with_extension(format!("{n}.log"))
}

fn rotate(path: &Path) {
    let _ = std::fs::remove_file(old_copy(path, KEEP_OLD));
    for n in (1..KEEP_OLD).rev() {
        let _ = std::fs::rename(old_copy(path, n), old_copy(path, n + 1));
    }
    let _ = std::fs::rename(path, old_copy(path, 1));
}

pub fn line(msg: &str) {
    let now = chrono::Local::now();
    println!("[{}] {msg}", now.format("%H:%M:%S"));
    let Some(log) = LOG.get() else { return };
    let mut log = log.lock().unwrap();
    let text = format!("[{}] {msg}\n", now.format("%Y-%m-%d %H:%M:%S"));
    if log.size + text.len() as u64 > MAX_LOG {
        let path = log.path.clone();
        rotate(&path);
        if let Some(fresh) = open(&path) {
            *log = fresh;
        }
    }
    if log.file.write_all(text.as_bytes()).is_ok() {
        log.size += text.len() as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_keeps_three_files() {
        let dir = std::env::temp_dir().join(format!("lp-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("local-photo.log");
        for round in 0..4 {
            std::fs::write(&path, format!("round {round}")).unwrap();
            rotate(&path);
        }
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(old_copy(&path, 1)).unwrap(), "round 3");
        assert_eq!(std::fs::read_to_string(old_copy(&path, 2)).unwrap(), "round 2");
        assert!(!old_copy(&path, 3).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
