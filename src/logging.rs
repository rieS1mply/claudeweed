//! Log lines go to the console and to `local-photo.log` next to the settings,
//! so a cloud running in the background (no window) can still be diagnosed.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// The log is started afresh when it grows past this.
const MAX_LOG: u64 = 5 * 1024 * 1024;

static FILE: OnceLock<Mutex<File>> = OnceLock::new();

pub fn to_file(path: &Path) {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG) {
        let _ = std::fs::rename(path, path.with_extension("old.log"));
    }
    if let Ok(f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = FILE.set(Mutex::new(f));
    }
}

pub fn line(msg: &str) {
    let now = chrono::Local::now();
    println!("[{}] {msg}", now.format("%H:%M:%S"));
    if let Some(f) = FILE.get() {
        let _ = writeln!(f.lock().unwrap(), "[{}] {msg}", now.format("%Y-%m-%d %H:%M:%S"));
    }
}
