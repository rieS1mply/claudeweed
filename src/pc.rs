//! «Компьютер»: every disk and folder of this computer on the site, like Finder.
//!
//! Off unless the owner runs `local-photo --files on` on the computer itself: whoever knows
//! the password can then read, change and delete any file this user can. Two places stay
//! out of reach even then: the cloud's own settings (password hash, bot token) and its
//! system folder in the storage. Windows' own folders (Windows, Program Files) are read-only.
//!
//! Paths come from the page as absolute strings (`C:\Users\me`); [`parse`] accepts plain
//! components only, [`Pc::check`] resolves links and applies the rules above. Deleting goes
//! to the Recycle Bin unless the owner explicitly asks for "delete now". Copies, moves and
//! deletes run as background [`Job`]s the page polls, so a big copy can't hit the tunnel's
//! request timeout.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::storage::{self, WINDOWS_RESERVED};

/// Larger files aren't opened as text (the page would choke on them anyway).
pub const MAX_TEXT: u64 = 2 * 1024 * 1024;
/// Images above this get no thumbnail: decoding them would take too much memory.
pub const MAX_THUMB_SOURCE: u64 = 64 * 1024 * 1024;
/// One Telegram message per address per this long, like the terminal.
const NOTIFY_EVERY: Duration = Duration::from_secs(3600);
/// Finished jobs are forgotten after this.
const JOB_KEEP: Duration = Duration::from_secs(600);
const SEARCH_LIMIT: usize = 500;
const SEARCH_TIME: Duration = Duration::from_secs(15);
const SEARCH_VISITS: usize = 400_000;
const SIZE_TIME: Duration = Duration::from_secs(20);

pub const CLOSED_MSG: &str = "Эта папка закрыта: в ней настройки и служебные файлы облака";
pub const READ_ONLY_MSG: &str = "Системная папка Windows — только просмотр";

pub struct Pc {
    /// Canonical folders the page can't see at all.
    closed: Vec<PathBuf>,
    /// Canonical folders the page can only look into.
    read_only: Vec<PathBuf>,
    notified: Mutex<HashMap<String, Instant>>,
    jobs: Mutex<HashMap<String, Arc<Job>>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    Read,
    /// Create something new at this path (it may not exist yet).
    Create,
    /// Change or remove what is at this path.
    Change,
}

impl Pc {
    pub fn new(closed: &[PathBuf]) -> Pc {
        let canon = |dirs: Vec<PathBuf>| dirs.into_iter().filter_map(|d| d.canonicalize().ok()).collect::<Vec<_>>();
        let mut read_only = Vec::new();
        if cfg!(windows) {
            for var in ["SystemRoot", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
                if let Some(dir) = std::env::var_os(var) {
                    read_only.push(PathBuf::from(dir));
                }
            }
        }
        Pc { closed: canon(closed.to_vec()), read_only: canon(read_only), notified: Default::default(), jobs: Default::default() }
    }

    /// The real location of `path` (links followed) if the page may use it that way.
    pub fn check(&self, path: &Path, access: Access) -> Result<PathBuf, &'static str> {
        let real = match path.canonicalize() {
            Ok(p) => p,
            Err(_) if access == Access::Create => {
                let parent = path.parent().ok_or("Неверный путь")?;
                let name = path.file_name().ok_or("Неверный путь")?;
                parent.canonicalize().map_err(|_| "Папка не найдена")?.join(name)
            }
            Err(_) => return Err("Файл не найден"),
        };
        if self.closed.iter().any(|c| real.starts_with(c)) {
            return Err(CLOSED_MSG);
        }
        if access != Access::Read {
            if self.read_only.iter().any(|r| real.starts_with(r)) {
                return Err(READ_ONLY_MSG);
            }
            // Renaming or deleting a parent would take the protected folder along.
            if access == Access::Change && self.closed.iter().chain(&self.read_only).any(|c| c.starts_with(&real)) {
                return Err(CLOSED_MSG);
            }
        }
        Ok(real)
    }

    /// True once per address per hour: worth a Telegram message.
    pub fn should_notify(&self, ip: &str) -> bool {
        let mut map = self.notified.lock().unwrap();
        map.retain(|_, t| t.elapsed() < NOTIFY_EVERY);
        if map.contains_key(ip) {
            return false;
        }
        map.insert(ip.to_string(), Instant::now());
        true
    }

    pub fn job(&self, id: &str) -> Option<Arc<Job>> {
        self.jobs.lock().unwrap().get(id).cloned()
    }

    /// Runs `work` in the background as a job the page can follow.
    pub fn start(&self, kind: &'static str, work: impl FnOnce(&Job) -> Result<()> + Send + 'static) -> String {
        let id = crate::auth::random_hex(12);
        let job = Arc::new(Job {
            kind,
            total: AtomicU64::new(0),
            done: AtomicU64::new(0),
            current: Mutex::new(String::new()),
            state: Mutex::new(JobState::Running),
            cancel: AtomicBool::new(false),
            finished: Mutex::new(None),
        });
        {
            let mut jobs = self.jobs.lock().unwrap();
            jobs.retain(|_, j| j.finished.lock().unwrap().is_none_or(|t| t.elapsed() < JOB_KEEP));
            jobs.insert(id.clone(), job.clone());
        }
        tokio::task::spawn_blocking(move || {
            let result = work(&job);
            *job.state.lock().unwrap() = match result {
                Ok(()) => JobState::Done,
                Err(_) if job.cancel.load(Ordering::Relaxed) => JobState::Failed("Отменено".into()),
                Err(e) => JobState::Failed(format!("{e:#}")),
            };
            *job.finished.lock().unwrap() = Some(Instant::now());
        });
        id
    }
}

pub enum JobState {
    Running,
    Done,
    Failed(String),
}

/// A copy, move or delete running in the background. `total`/`done` count bytes for
/// copies and items for deletes.
pub struct Job {
    pub kind: &'static str,
    pub total: AtomicU64,
    pub done: AtomicU64,
    pub current: Mutex<String>,
    pub state: Mutex<JobState>,
    pub cancel: AtomicBool,
    finished: Mutex<Option<Instant>>,
}

impl Job {
    pub fn json(&self) -> Value {
        let (state, error) = match &*self.state.lock().unwrap() {
            JobState::Running => ("running", None),
            JobState::Done => ("done", None),
            JobState::Failed(e) => ("failed", Some(e.clone())),
        };
        json!({
            "kind": self.kind,
            "total": self.total.load(Ordering::Relaxed),
            "done": self.done.load(Ordering::Relaxed),
            "current": *self.current.lock().unwrap(),
            "state": state,
            "error": error,
        })
    }

    fn check_cancel(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            bail!("Отменено");
        }
        Ok(())
    }

    fn working_on(&self, path: &Path) {
        *self.current.lock().unwrap() = name_of(path);
    }
}

// ---------- paths ----------

/// A path from the page: absolute, plain components only. On Windows `C:\…` (either
/// slash); never `..`, device or network paths (`\\?\`, `\\server`), or `name:stream`.
pub fn parse(raw: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let s = raw.replace('/', "\\");
        let b = s.as_bytes();
        // "C:foo" would be relative to the current folder of drive C.
        if b.len() < 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' || (b.len() > 2 && b[2] != b'\\') {
            return None;
        }
        let mut path = PathBuf::from(format!("{}:\\", (b[0] as char).to_ascii_uppercase()));
        for part in s[2..].split('\\').filter(|p| !p.is_empty()) {
            if !plain_name(part) {
                return None;
            }
            path.push(part);
        }
        Some(path)
    }
    #[cfg(not(windows))]
    {
        if !raw.starts_with('/') {
            return None;
        }
        let mut path = PathBuf::from("/");
        for part in raw.split('/').filter(|p| !p.is_empty()) {
            if !plain_name(part) {
                return None;
            }
            path.push(part);
        }
        Some(path)
    }
}

/// One path component that names exactly itself (no traversal, no device, no stream).
pub fn plain_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.len() > 255 {
        return false;
    }
    if name.chars().any(|c| c.is_control() || c == '/' || c == '\\') {
        return false;
    }
    if cfg!(windows) {
        if name.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*')) || name.ends_with(['.', ' ']) {
            return false;
        }
        let stem = name.split('.').next().unwrap_or("").trim_end();
        if WINDOWS_RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
            return false;
        }
    }
    true
}

/// `\\?\C:\x` → `C:\x`: canonical paths carry the prefix, people and old APIs don't want it.
pub fn plain(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path.to_path_buf(),
    }
}

/// The path as the page shows and sends it back.
pub fn display(path: &Path) -> String {
    plain(path).to_string_lossy().into_owned()
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(|| display(path), |n| n.to_string_lossy().into_owned())
}

// ---------- browsing ----------

#[derive(Serialize)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    pub mtime: i64,
    pub hidden: bool,
    /// A shortcut to elsewhere (symlink, junction).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub link: bool,
}

pub fn list(dir: &Path) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).context("Не удалось открыть папку")? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(own) = std::fs::symlink_metadata(entry.path()) else { continue };
        let link = own.file_type().is_symlink();
        // A link shows what it points to; a broken one stays a (zero-size) file.
        let meta = if link { std::fs::metadata(entry.path()).unwrap_or(own) } else { own };
        out.push(Entry {
            hidden: is_hidden(&name, &meta),
            dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            mtime: storage::mtime_ms(&meta),
            name,
            link,
        });
    }
    Ok(out)
}

#[cfg(windows)]
fn is_hidden(name: &str, meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    meta.file_attributes() & (HIDDEN | SYSTEM) != 0 || name.starts_with('.')
}

#[cfg(not(windows))]
fn is_hidden(name: &str, _meta: &std::fs::Metadata) -> bool {
    name.starts_with('.')
}

/// Sidebar: the usual user folders, the disks and the cloud's own sections.
pub fn places(cloud: &[(&str, PathBuf)]) -> Value {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let mut favorites = Vec::new();
    if let Some(home) = &home {
        favorites.push(json!({ "name": "Домой", "icon": "home", "path": display(home) }));
        for (dir, name, icon) in [
            ("Desktop", "Рабочий стол", "desktop"),
            ("Documents", "Документы", "doc"),
            ("Downloads", "Загрузки", "download"),
            ("Pictures", "Изображения", "image"),
            ("Videos", if cfg!(target_os = "macos") { "Фильмы" } else { "Видео" }, "video"),
            ("Movies", "Фильмы", "video"),
            ("Music", "Музыка", "music"),
        ] {
            // OneDrive often takes over Desktop/Documents/Pictures.
            let found = [home.join(dir), home.join("OneDrive").join(dir)].into_iter().find(|p| p.is_dir());
            if let Some(path) = found {
                favorites.push(json!({ "name": name, "icon": icon, "path": display(&path) }));
            }
        }
    }
    let cloud: Vec<Value> = cloud
        .iter()
        .filter(|(_, p)| p.is_dir())
        .map(|(name, p)| json!({ "name": name, "icon": "cloud", "path": display(p) }))
        .collect();
    json!({
        "sep": std::path::MAIN_SEPARATOR.to_string(),
        "home": home.as_deref().map(display),
        "favorites": favorites,
        "drives": drives(),
        "cloud": cloud,
    })
}

fn drives() -> Vec<Value> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        // A: and B: are floppy letters: probing them can stall for seconds.
        roots.extend((b'C'..=b'Z').map(|c| PathBuf::from(format!("{}:\\", c as char))).filter(|p| p.is_dir()));
    } else {
        roots.push(PathBuf::from("/"));
        let mounts = if cfg!(target_os = "macos") { "/Volumes" } else { "/media" };
        if let Ok(rd) = std::fs::read_dir(mounts) {
            roots.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
        }
    }
    roots
        .into_iter()
        .map(|root| {
            json!({
                "name": display(&root),
                "icon": "drive",
                "path": display(&root),
                "total": fs4::total_space(&root).ok(),
                "free": fs4::available_space(&root).ok(),
            })
        })
        .collect()
}

/// Folders and files whose name contains every word of `query` (case-insensitive, ё = е),
/// below `root`. Stops at [`SEARCH_LIMIT`] hits or [`SEARCH_TIME`]; `true` = stopped early.
pub fn search(pc: &Pc, root: &Path, query: &str, hidden: bool) -> (Vec<Value>, bool) {
    let fold = |s: &str| s.to_lowercase().replace('ё', "е");
    let words: Vec<String> = fold(query).split_whitespace().map(str::to_string).collect();
    let mut hits = Vec::new();
    if words.is_empty() {
        return (hits, false);
    }
    let started = Instant::now();
    let mut visits = 0usize;
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            visits += 1;
            if hits.len() >= SEARCH_LIMIT || visits >= SEARCH_VISITS || started.elapsed() > SEARCH_TIME {
                return (hits, true);
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            if !hidden && is_hidden(&name, &meta) {
                continue;
            }
            let path = entry.path();
            // Links aren't followed: a junction back up the tree would loop forever.
            let is_dir = meta.is_dir();
            if is_dir && !pc.closed.iter().any(|c| path.starts_with(c)) {
                queue.push_back(path.clone());
            }
            let folded = fold(&name);
            if words.iter().all(|w| folded.contains(w.as_str())) {
                hits.push(json!({
                    "path": display(&path),
                    "name": name,
                    "dir": is_dir,
                    "size": if is_dir { 0 } else { meta.len() },
                    "mtime": storage::mtime_ms(&meta),
                }));
            }
        }
    }
    (hits, false)
}

/// Total size, files and folders below `dir` (links not followed); `false` = gave up early.
pub fn folder_size(dir: &Path) -> (u64, u64, u64, bool) {
    let started = Instant::now();
    let (mut bytes, mut files, mut dirs) = (0u64, 0u64, 0u64);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for entry in rd.flatten() {
            if started.elapsed() > SIZE_TIME {
                return (bytes, files, dirs, false);
            }
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            if meta.is_dir() {
                dirs += 1;
                stack.push(entry.path());
            } else {
                files += 1;
                bytes += meta.len();
            }
        }
    }
    (bytes, files, dirs, true)
}

// ---------- text ----------

/// The file as text: `(text, exact)`; `exact` is false when it isn't valid UTF-8 (shown
/// with replacement characters, and the page won't offer to save it).
pub fn read_text(path: &Path) -> Result<(String, bool)> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_TEXT {
        bail!("Файл слишком большой, чтобы открыть его как текст");
    }
    let bytes = std::fs::read(path)?;
    if bytes[..bytes.len().min(8192)].contains(&0) {
        bail!("Это не текстовый файл");
    }
    match String::from_utf8(bytes) {
        Ok(s) => Ok((s, true)),
        Err(e) => Ok((String::from_utf8_lossy(e.as_bytes()).into_owned(), false)),
    }
}

/// Replaces the file's content in one step: a crash mid-write leaves the old version.
pub fn write_text(path: &Path, text: &[u8]) -> Result<()> {
    let dir = path.parent().context("Неверный путь")?;
    let tmp = dir.join(format!(".{}.lp-save", name_of(path)));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

// ---------- changes ----------

pub fn create_dir(path: &Path) -> Result<()> {
    if path.exists() {
        bail!("Уже есть объект с таким именем");
    }
    std::fs::create_dir(path)?;
    Ok(())
}

pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf> {
    let target = path.parent().context("Неверный путь")?.join(new_name);
    // "photo.jpg" → "Photo.jpg" is the same file on Windows: allowed.
    let same = name_of(path).eq_ignore_ascii_case(new_name);
    if target.exists() && !same {
        bail!("Уже есть объект с таким именем");
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}

/// Copies (or moves) `sources` into `dest`. A name already taken gets " (2)", like Finder's
/// duplicates; nothing is ever overwritten.
pub fn transfer(job: &Job, sources: &[PathBuf], dest: &Path, moving: bool) -> Result<()> {
    for src in sources {
        if dest.starts_with(src) {
            bail!("Нельзя положить папку внутрь неё самой");
        }
    }
    let total: u64 = sources.iter().map(|s| tree_size(s)).sum();
    job.total.store(total, Ordering::Relaxed);
    for src in sources {
        job.check_cancel()?;
        job.working_on(src);
        let name = name_of(src);
        if moving && src.parent() == Some(dest) {
            job.done.fetch_add(tree_size(src), Ordering::Relaxed);
            continue;
        }
        let target = storage::unique_path(dest, &name);
        if moving && std::fs::rename(src, &target).is_ok() {
            job.done.fetch_add(tree_size(src).max(tree_size(&target)), Ordering::Relaxed);
            continue;
        }
        // Another disk (or the rename was refused): copy, then remove the original.
        copy_tree(job, src, &target).with_context(|| format!("«{name}»"))?;
        if moving {
            remove_now(src).with_context(|| format!("«{name}» скопирован, но не удалён на старом месте"))?;
        }
    }
    Ok(())
}

fn tree_size(path: &Path) -> u64 {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => folder_size(path).0,
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

fn copy_tree(job: &Job, src: &Path, dst: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(src)?;
    if meta.is_dir() {
        std::fs::create_dir(dst)?;
        for entry in std::fs::read_dir(src)?.flatten() {
            job.check_cancel()?;
            copy_tree(job, &entry.path(), &dst.join(entry.file_name()))?;
        }
        return Ok(());
    }
    if meta.file_type().is_symlink() && std::fs::metadata(src).is_ok_and(|m| m.is_dir()) {
        // A link to a folder: copying what it points to could loop or pull in half the disk.
        return Ok(());
    }
    job.working_on(src);
    let mut from = File::open(src)?;
    let mut to = File::create(dst)?;
    let mut buf = vec![0u8; 1024 * 1024];
    let copied = (|| -> Result<()> {
        loop {
            job.check_cancel()?;
            let n = from.read(&mut buf)?;
            if n == 0 {
                break;
            }
            to.write_all(&buf[..n])?;
            job.done.fetch_add(n as u64, Ordering::Relaxed);
        }
        to.flush()?;
        Ok(())
    })();
    drop(to);
    if let Err(e) = copied {
        let _ = std::fs::remove_file(dst);
        return Err(e);
    }
    let _ = filetime::set_file_mtime(dst, filetime::FileTime::from_last_modification_time(&meta));
    Ok(())
}

pub fn remove_now(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Deletes into the Recycle Bin (Windows) / Trash (macOS, Linux), so it can be restored there.
pub fn delete(job: &Job, paths: &[PathBuf], now: bool) -> Result<()> {
    job.total.store(paths.len() as u64, Ordering::Relaxed);
    for path in paths {
        job.check_cancel()?;
        job.working_on(path);
        let name = name_of(path);
        if now { remove_now(path) } else { to_trash(path) }.with_context(|| format!("«{name}»"))?;
        job.done.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

#[cfg(windows)]
fn to_trash(path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[repr(C)]
    struct ShFileOpStructW {
        hwnd: *mut std::ffi::c_void,
        w_func: u32,
        p_from: *const u16,
        p_to: *const u16,
        f_flags: u16,
        f_any_operations_aborted: i32,
        h_name_mappings: *mut std::ffi::c_void,
        lpsz_progress_title: *const u16,
    }
    #[link(name = "shell32")]
    extern "system" {
        fn SHFileOperationW(op: *mut ShFileOpStructW) -> i32;
    }
    const FO_DELETE: u32 = 3;
    const FOF_SILENT: u16 = 0x4;
    const FOF_NOCONFIRMATION: u16 = 0x10;
    const FOF_ALLOWUNDO: u16 = 0x40;
    const FOF_NOERRORUI: u16 = 0x400;

    // The shell wants a plain path (no `\\?\`), ending with two NULs.
    let mut from: Vec<u16> = plain(path).as_os_str().encode_wide().collect();
    from.extend([0, 0]);
    let mut op = ShFileOpStructW {
        hwnd: std::ptr::null_mut(),
        w_func: FO_DELETE,
        p_from: from.as_ptr(),
        p_to: std::ptr::null(),
        f_flags: FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI,
        f_any_operations_aborted: 0,
        h_name_mappings: std::ptr::null_mut(),
        lpsz_progress_title: std::ptr::null(),
    };
    // SAFETY: `op` and the buffer it points to live until the call returns; the call has no UI.
    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 || op.f_any_operations_aborted != 0 || path.exists() {
        bail!("Не удалось переместить в Корзину (код {code:#x}). Файл открыт в программе или нет прав");
    }
    Ok(())
}

#[cfg(not(windows))]
fn to_trash(path: &Path) -> Result<()> {
    let home = PathBuf::from(std::env::var_os("HOME").context("нет HOME")?);
    let trash = if cfg!(target_os = "macos") { home.join(".Trash") } else { home.join(".local/share/Trash/files") };
    std::fs::create_dir_all(&trash)?;
    let target = storage::unique_path(&trash, &name_of(path));
    std::fs::rename(path, &target).context("Корзина на другом диске: удалите сразу")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn parse_accepts_only_plain_absolute_paths() {
        assert_eq!(parse(r"c:\Users\me"), Some(PathBuf::from(r"C:\Users\me")));
        assert_eq!(parse("D:/Фото/2024/"), Some(PathBuf::from(r"D:\Фото\2024")));
        assert_eq!(parse("C:"), Some(PathBuf::from(r"C:\")));
        for bad in [
            r"C:\Users\..\Windows",
            r"\\?\C:\x",
            r"\\server\share",
            r"C:relative",
            "relative",
            r"C:\a\file.txt:stream",
            r"C:\a\NUL",
            r"C:\a\con.txt",
            r"C:\a\trailing.",
            "",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn names_are_checked() {
        assert!(plain_name("Отчёт 2024.docx"));
        assert!(plain_name(".gitignore"));
        for bad in ["", ".", "..", "a/b", "a\\b", "a\nb"] {
            assert!(!plain_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn plain_strips_the_verbatim_prefix() {
        assert_eq!(plain(Path::new(r"\\?\C:\x")), PathBuf::from(r"C:\x"));
        assert_eq!(plain(Path::new(r"\\?\UNC\srv\s")), PathBuf::from(r"\\srv\s"));
        assert_eq!(plain(Path::new(r"C:\x")), PathBuf::from(r"C:\x"));
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lp-pc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn closed_folders_and_their_parents_are_protected() {
        let root = scratch("check");
        let closed = root.join("settings");
        std::fs::create_dir_all(closed.join("inner")).unwrap();
        std::fs::write(closed.join("config.json"), "{}").unwrap();
        std::fs::create_dir_all(root.join("free")).unwrap();
        let pc = Pc::new(std::slice::from_ref(&closed));
        assert_eq!(pc.check(&closed.join("config.json"), Access::Read), Err(CLOSED_MSG));
        assert_eq!(pc.check(&closed.join("inner"), Access::Read), Err(CLOSED_MSG));
        assert_eq!(pc.check(&closed.join("new.txt"), Access::Create), Err(CLOSED_MSG));
        // Deleting or renaming the parent would take the settings along.
        assert_eq!(pc.check(&root, Access::Change), Err(CLOSED_MSG));
        assert!(pc.check(&root, Access::Read).is_ok());
        assert!(pc.check(&root.join("free"), Access::Change).is_ok());
        assert!(pc.check(&root.join("free").join("new.txt"), Access::Create).is_ok());
        assert_eq!(pc.check(&root.join("missing"), Access::Read), Err("Файл не найден"));
        let _ = std::fs::remove_dir_all(&root);
    }

    fn job() -> Job {
        Job {
            kind: "copy",
            total: AtomicU64::new(0),
            done: AtomicU64::new(0),
            current: Mutex::new(String::new()),
            state: Mutex::new(JobState::Running),
            cancel: AtomicBool::new(false),
            finished: Mutex::new(None),
        }
    }

    #[test]
    fn copy_and_move_never_overwrite() {
        let root = scratch("copy");
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(a.join("sub")).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("sub").join("x.txt"), "hello").unwrap();
        std::fs::write(b.join("x.txt"), "old").unwrap();

        let j = job();
        transfer(&j, &[a.join("sub").join("x.txt")], &b, false).unwrap();
        assert_eq!(std::fs::read_to_string(b.join("x.txt")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(b.join("x (2).txt")).unwrap(), "hello");
        assert_eq!(j.done.load(Ordering::Relaxed), 5);

        transfer(&job(), &[a.join("sub")], &b, true).unwrap();
        assert!(!a.join("sub").exists());
        assert_eq!(std::fs::read_to_string(b.join("sub").join("x.txt")).unwrap(), "hello");

        assert!(transfer(&job(), std::slice::from_ref(&b), &b.join("sub"), false).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn text_is_read_and_replaced() {
        let root = scratch("text");
        let f = root.join("note.txt");
        std::fs::write(&f, "привет").unwrap();
        assert_eq!(read_text(&f).unwrap(), ("привет".to_string(), true));
        write_text(&f, "пока".as_bytes()).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "пока");
        std::fs::write(&f, [0xff, 0xfe, b'a']).unwrap();
        assert!(!read_text(&f).unwrap().1);
        std::fs::write(&f, [b'a', 0, b'b']).unwrap();
        assert!(read_text(&f).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn search_matches_all_words() {
        let root = scratch("search");
        std::fs::create_dir_all(root.join("Отчёты")).unwrap();
        std::fs::write(root.join("Отчёты").join("отчет за май.txt"), "").unwrap();
        std::fs::write(root.join("май.txt"), "").unwrap();
        let pc = Pc::new(&[]);
        let (hits, more) = search(&pc, &root, "ОТЧЁТ май", false);
        assert!(!more);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["name"], "отчет за май.txt");
        let _ = std::fs::remove_dir_all(&root);
    }
}
