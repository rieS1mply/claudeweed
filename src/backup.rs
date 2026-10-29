//! Daily copy of the storage to another folder, ideally another disk (`--backup <folder>`).
//!
//! New and changed files are copied; nothing is ever deleted from the copy, so a file
//! removed by mistake (or by someone with the password) is still there. Service files
//! (thumbnails, unfinished uploads, the trash) are skipped: they can be rebuilt or are
//! already deleted.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{Config, SYSTEM_DIR};
use crate::telegram;

/// The copy goes into this subfolder of the chosen one, so it can't mix with other files there.
pub const COPY_DIR: &str = "Облако (резервная копия)";
const STATE_FILE: &str = "backup.json";
pub const EVERY: Duration = Duration::from_secs(24 * 3600);
/// How often a missing backup disk is reported, not every hourly retry.
const NOTIFY_FAILURE_EVERY: i64 = 24 * 3600 * 1000;
/// Warn when the last good copy is older than this.
const STALE_AFTER: i64 = 3 * 24 * 3600 * 1000;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct State {
    /// Unix ms of the last copy that finished without errors.
    #[serde(default)]
    pub last_ok: i64,
    #[serde(default)]
    pub last_attempt: i64,
    #[serde(default)]
    pub last_failure_notified: i64,
    /// Files in the copy after the last run, and how many of them that run wrote.
    #[serde(default)]
    pub files: u64,
    #[serde(default)]
    pub copied: u64,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub error: Option<String>,
}

impl State {
    pub fn load(config_dir: &Path) -> State {
        std::fs::read_to_string(config_dir.join(STATE_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    fn save(&self, config_dir: &Path) {
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(config_dir.join(STATE_FILE), text);
        }
    }
}

#[derive(Default, Debug)]
pub struct Report {
    pub files: u64,
    pub copied: u64,
    pub bytes: u64,
    /// First few problems (file -> reason); the copy goes on past them.
    pub errors: Vec<String>,
    pub failed: u64,
}

/// Refuses a backup folder inside the storage (it would copy itself) or containing it.
pub fn check_target(storage: &Path, target: &Path) -> Result<()> {
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    if real(target).starts_with(real(storage)) {
        bail!("папка копии не может быть внутри хранилища ({})", storage.display());
    }
    Ok(())
}

/// True when both folders are on the same drive (Windows letter / Unix device).
pub fn same_disk(storage: &Path, target: &Path) -> bool {
    #[cfg(windows)]
    {
        let letter = |p: &Path| p.canonicalize().ok().and_then(|p| p.to_string_lossy().trim_start_matches(r"\\?\").chars().next().map(|c| c.to_ascii_uppercase()));
        letter(storage).is_some() && letter(storage) == letter(target)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        matches!((std::fs::metadata(storage), std::fs::metadata(target)), (Ok(a), Ok(b)) if a.dev() == b.dev())
    }
}

/// Copies `storage` into `target/COPY_DIR`. Blocking: run it in `spawn_blocking`.
pub fn run(storage: &Path, target: &Path) -> Result<Report> {
    if !target.is_dir() {
        bail!("папка копии недоступна: {} (если это внешний диск — подключите его)", target.display());
    }
    check_target(storage, target)?;
    let dest = target.join(COPY_DIR);
    std::fs::create_dir_all(&dest).with_context(|| format!("не удалось создать {}", dest.display()))?;
    let mut report = Report::default();
    copy_tree(storage, &dest, true, &mut report);
    Ok(report)
}

fn copy_tree(src: &Path, dest: &Path, top: bool, report: &mut Report) {
    let entries = match std::fs::read_dir(src) {
        Ok(e) => e,
        Err(e) => return note_error(report, src, &e.to_string()),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if top && name == SYSTEM_DIR {
            continue;
        }
        let (from, to) = (entry.path(), dest.join(&name));
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            if let Err(e) = std::fs::create_dir_all(&to) {
                note_error(report, &to, &e.to_string());
                continue;
            }
            copy_tree(&from, &to, false, report);
        } else if meta.is_file() {
            report.files += 1;
            if is_current(&meta, &to) {
                continue;
            }
            match copy_file(&from, &to, &meta) {
                Ok(()) => {
                    report.copied += 1;
                    report.bytes += meta.len();
                }
                Err(e) => note_error(report, &from, &format!("{e:#}")),
            }
        }
    }
}

/// Same size and not older: already copied.
fn is_current(src: &std::fs::Metadata, dest: &Path) -> bool {
    let Ok(d) = std::fs::metadata(dest) else { return false };
    let mtime = |m: &std::fs::Metadata| m.modified().ok();
    d.len() == src.len() && matches!((mtime(src), mtime(&d)), (Some(s), Some(d)) if d >= s)
}

/// Through a temporary name, so a copy cut short (unplugged disk) never looks complete.
fn copy_file(from: &Path, to: &Path, meta: &std::fs::Metadata) -> Result<()> {
    let mut tmp: PathBuf = to.to_path_buf();
    tmp.as_mut_os_string().push(".part");
    std::fs::copy(from, &tmp)?;
    if let Ok(m) = meta.modified() {
        let _ = filetime::set_file_mtime(&tmp, filetime::FileTime::from_system_time(m));
    }
    std::fs::rename(&tmp, to).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

fn note_error(report: &mut Report, path: &Path, why: &str) {
    report.failed += 1;
    if report.errors.len() < 5 {
        report.errors.push(format!("{}: {why}", path.display()));
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn fmt_time(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map_or_else(|| "—".into(), |t| t.format("%d.%m.%Y %H:%M").to_string())
}

fn fmt_bytes(n: u64) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1} ГБ", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1} МБ", n as f64 / (1u64 << 20) as f64),
        n => format!("{} КБ", n.div_ceil(1024)),
    }
}

/// One line for people (console, Telegram).
pub fn describe(report: &Report) -> String {
    let mut s = format!("файлов в копии: {}, скопировано сейчас: {} ({})", report.files, report.copied, fmt_bytes(report.bytes));
    if report.failed > 0 {
        s.push_str(&format!(", не скопировано {}: {}", report.failed, report.errors.join("; ")));
    }
    s
}

/// Runs one copy now and records the outcome in backup.json.
pub async fn run_and_record(config_dir: &Path, storage: &Path, target: &Path) -> (Result<Report, String>, State) {
    let (s, t) = (storage.to_path_buf(), target.to_path_buf());
    let result = tokio::task::spawn_blocking(move || run(&s, &t)).await.map_err(|e| e.to_string()).and_then(|r| r.map_err(|e| format!("{e:#}")));
    let mut state = State::load(config_dir);
    state.last_attempt = now_ms();
    match &result {
        Ok(r) => {
            (state.files, state.copied, state.bytes) = (r.files, r.copied, r.bytes);
            state.error = (r.failed > 0).then(|| r.errors.join("; "));
            if r.failed == 0 {
                state.last_ok = state.last_attempt;
            }
        }
        Err(e) => state.error = Some(e.clone()),
    }
    state.save(config_dir);
    (result, state)
}

/// Background loop: once a day (checked hourly, and a few minutes after start).
pub async fn schedule(config_file: PathBuf, config_dir: PathBuf, storage: PathBuf, telegram: Option<crate::config::TelegramConfig>) {
    tokio::time::sleep(Duration::from_secs(180)).await;
    loop {
        // Read live: `--backup` works without restarting the cloud.
        let target = Config::load(&config_file).ok().flatten().and_then(|c| c.backup_dir);
        if let Some(target) = target {
            let due = now_ms() - State::load(&config_dir).last_ok >= EVERY.as_millis() as i64;
            if due {
                let (result, mut state) = run_and_record(&config_dir, &storage, &target).await;
                let message = match &result {
                    Ok(r) if r.failed == 0 => {
                        crate::log!("Резервная копия: {}", describe(r));
                        // Quiet days stay quiet; new photos get a short note.
                        (r.copied > 0).then(|| format!("Резервная копия готова: {}", describe(r)))
                    }
                    Ok(r) => Some(format!("Резервная копия с ошибками: {}", describe(r))),
                    Err(e) => Some(format!("Резервная копия не сделана: {e}")),
                };
                let failed = !matches!(&result, Ok(r) if r.failed == 0);
                if failed {
                    crate::log!("{}", message.as_deref().unwrap_or("Резервная копия: ошибка"));
                }
                let stale = state.last_ok > 0 && now_ms() - state.last_ok > STALE_AFTER;
                let notify = if failed { now_ms() - state.last_failure_notified >= NOTIFY_FAILURE_EVERY } else { message.is_some() };
                if let (true, Some(tg), Some(mut text)) = (notify, telegram.as_ref(), message) {
                    if stale {
                        text.push_str(&format!("\nПоследняя удачная копия: {}", fmt_time(state.last_ok)));
                    }
                    telegram::send_with_retry(tg, &text).await;
                    if failed {
                        state.last_failure_notified = now_ms();
                        state.save(&config_dir);
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lp-backup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn copies_new_and_changed_never_deletes() {
        let (storage, target) = (tmp("src"), tmp("dst"));
        std::fs::create_dir_all(storage.join("Фото/2026")).unwrap();
        std::fs::create_dir_all(storage.join(SYSTEM_DIR).join("thumbs")).unwrap();
        std::fs::write(storage.join("Фото/2026/a.jpg"), b"aaa").unwrap();
        std::fs::write(storage.join(SYSTEM_DIR).join("thumbs/t.jpg"), b"t").unwrap();

        let r = run(&storage, &target).unwrap();
        assert_eq!((r.files, r.copied, r.failed), (1, 1, 0));
        let copy = target.join(COPY_DIR);
        assert_eq!(std::fs::read(copy.join("Фото/2026/a.jpg")).unwrap(), b"aaa");
        assert!(!copy.join(SYSTEM_DIR).exists(), "service files are skipped");

        let r = run(&storage, &target).unwrap();
        assert_eq!(r.copied, 0, "unchanged files are not copied again");

        std::fs::write(storage.join("Фото/2026/a.jpg"), b"changed").unwrap();
        std::fs::remove_file(storage.join("Фото/2026/a.jpg")).ok();
        std::fs::write(storage.join("b.txt"), b"b").unwrap();
        let r = run(&storage, &target).unwrap();
        assert_eq!(r.copied, 1);
        assert!(copy.join("Фото/2026/a.jpg").exists(), "a file deleted from the cloud stays in the copy");
        assert!(!copy.join("b.txt.part").exists());

        std::fs::remove_dir_all(&storage).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    #[test]
    fn refuses_target_inside_storage_and_missing_disk() {
        let storage = tmp("inner");
        let inner = storage.join("copy");
        std::fs::create_dir_all(&inner).unwrap();
        assert!(run(&storage, &inner).is_err());
        assert!(run(&storage, &storage.join("нет-такой")).is_err());
        std::fs::remove_dir_all(&storage).ok();
    }
}
