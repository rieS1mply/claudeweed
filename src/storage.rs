//! Where files go and how they are found again: sections, dates, safe names and paths.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

pub const MONTHS: [&str; 12] = [
    "Январь", "Февраль", "Март", "Апрель", "Май", "Июнь",
    "Июль", "Август", "Сентябрь", "Октябрь", "Ноябрь", "Декабрь",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Photos,
    Files,
}

impl Section {
    pub const ALL: [Section; 2] = [Section::Photos, Section::Files];

    pub fn parse(s: &str) -> Option<Section> {
        match s {
            "photos" => Some(Section::Photos),
            "files" => Some(Section::Files),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Section::Photos => "photos",
            Section::Files => "files",
        }
    }

    pub fn dir_name(self) -> &'static str {
        match self {
            Section::Photos => "Фото",
            Section::Files => "Файлы",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A photo we can make a thumbnail of.
    Image,
    /// A photo format without thumbnails (HEIC, RAW, AVIF): stored and downloadable as is.
    Raw,
    Video,
    File,
}

impl Kind {
    pub fn of(name: &str) -> Kind {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" => Kind::Image,
            "heic" | "heif" | "avif" | "dng" | "cr2" | "cr3" | "nef" | "arw" | "raf" | "orf" | "rw2" => Kind::Raw,
            "mp4" | "mov" | "m4v" | "3gp" | "avi" | "mkv" | "webm" => Kind::Video,
            _ => Kind::File,
        }
    }

    pub fn section(self) -> Section {
        match self {
            Kind::File => Section::Files,
            _ => Section::Photos,
        }
    }
}

// ---------- dates ----------

/// When the photo/video was actually taken, from the file's own metadata.
/// `None` when the file carries no usable date (screenshots from messengers, documents…).
pub fn capture_date(path: &Path, kind: Kind) -> Option<DateTime<Local>> {
    match kind {
        Kind::Image | Kind::Raw => exif_date(path),
        Kind::Video => mp4_creation_time(path).map(|t| t.with_timezone(&Local)),
        Kind::File => None,
    }
}

fn exif_date(path: &Path) -> Option<DateTime<Local>> {
    let file = File::open(path).ok()?;
    let exif = exif::Reader::new().read_from_container(&mut BufReader::new(file)).ok()?;
    for tag in [exif::Tag::DateTimeOriginal, exif::Tag::DateTimeDigitized, exif::Tag::DateTime] {
        let Some(field) = exif.get_field(tag, exif::In::PRIMARY) else { continue };
        let exif::Value::Ascii(ref parts) = field.value else { continue };
        let Some(raw) = parts.first() else { continue };
        let Ok(dt) = exif::DateTime::from_ascii(raw) else { continue };
        // Cameras with an unset clock write 0000:00:00 or 1970.
        if dt.year < 1980 {
            continue;
        }
        let naive = chrono::NaiveDate::from_ymd_opt(dt.year.into(), dt.month.into(), dt.day.into())?
            .and_hms_opt(dt.hour.into(), dt.minute.into(), dt.second.into())?;
        // EXIF time is the camera's wall clock, so treat it as local time.
        return local_from_naive(naive);
    }
    None
}

fn local_from_naive(naive: NaiveDateTime) -> Option<DateTime<Local>> {
    Local.from_local_datetime(&naive).earliest()
}

/// Reads `moov/mvhd` creation time from an MP4/MOV (what phones write when recording).
fn mp4_creation_time(path: &Path) -> Option<DateTime<Utc>> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let (moov_start, moov_end) = find_box(&mut f, 0, len, b"moov")?;
    let (mvhd_start, _) = find_box(&mut f, moov_start, moov_end, b"mvhd")?;
    f.seek(SeekFrom::Start(mvhd_start)).ok()?;
    let mut version = [0u8; 4];
    f.read_exact(&mut version).ok()?;
    let secs_since_1904 = if version[0] == 1 {
        let mut b = [0u8; 8];
        f.read_exact(&mut b).ok()?;
        u64::from_be_bytes(b)
    } else {
        let mut b = [0u8; 4];
        f.read_exact(&mut b).ok()?;
        u32::from_be_bytes(b) as u64
    };
    const MAC_EPOCH_OFFSET: u64 = 2_082_844_800; // 1904-01-01 → 1970-01-01
    let unix = secs_since_1904.checked_sub(MAC_EPOCH_OFFSET)?;
    let t = Utc.timestamp_opt(unix as i64, 0).single()?;
    // Many encoders write 0 or garbage here.
    (t.year() >= 1980 && t <= Utc::now() + chrono::Duration::days(1)).then_some(t)
}

/// Finds a child box `name` in `[start, end)`; returns its payload range.
fn find_box(f: &mut File, start: u64, end: u64, name: &[u8; 4]) -> Option<(u64, u64)> {
    let mut pos = start;
    while pos + 8 <= end {
        f.seek(SeekFrom::Start(pos)).ok()?;
        let mut header = [0u8; 8];
        f.read_exact(&mut header).ok()?;
        let size32 = u32::from_be_bytes(header[0..4].try_into().ok()?) as u64;
        let (header_len, size) = match size32 {
            0 => (8, end - pos),
            1 => {
                let mut b = [0u8; 8];
                f.read_exact(&mut b).ok()?;
                (16, u64::from_be_bytes(b))
            }
            n => (8, n),
        };
        if size < header_len {
            return None;
        }
        let box_end = pos.checked_add(size)?.min(end);
        if &header[4..8] == name {
            return Some((pos + header_len, box_end));
        }
        pos = box_end;
    }
    None
}

/// "2024/03-Март"
pub fn month_folder(date: &DateTime<Local>) -> String {
    format!("{}/{:02}-{}", date.year(), date.month(), MONTHS[date.month0() as usize])
}

// ---------- names & paths ----------

const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Turns a client-supplied name into a safe single file name on every OS.
pub fn sanitize_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        .collect();
    let mut name = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    if name.is_empty() || name.chars().all(|c| c == '.') {
        name = "file".to_string();
    }
    let (stem, _) = split_ext(&name);
    if WINDOWS_RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        name = format!("_{name}");
    }
    // Keep names well under the 255-byte limit of most file systems.
    if name.len() > 180 {
        let (stem, ext) = split_ext(&name);
        let mut short: String = stem.chars().take(100).collect();
        if !ext.is_empty() {
            short.push('.');
            short.extend(ext.chars().take(16));
        }
        name = short;
    }
    name
}

/// "photo.tar.gz" → ("photo.tar", "gz"); ".bashrc" → (".bashrc", "")
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}

/// `dir/name`, or `dir/name (2).ext`, `(3)`… if that is taken. Never overwrites.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = split_ext(name);
    (2..)
        .map(|n| match ext {
            "" => dir.join(format!("{stem} ({n})")),
            _ => dir.join(format!("{stem} ({n}).{ext}")),
        })
        .find(|p| !p.exists())
        .expect("infinite iterator")
}

/// Maps a client path like "2024/03-Март/IMG.jpg" to an existing file inside `root`.
/// Rejects anything that could step outside it.
pub fn resolve(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(['\\', ':', '\0']) {
            return None;
        }
        path.push(part);
    }
    let canonical = path.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    (canonical.starts_with(&root) && canonical.is_file()).then_some(canonical)
}

/// Like [`resolve`], but for a path that may not exist yet (restoring from the trash).
pub fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(['\\', ':', '\0']) {
            return None;
        }
        path.push(part);
    }
    Some(path)
}

/// "2024/03-Март/IMG.jpg" for a file inside `root`, always with '/'.
pub fn rel_path(root: &Path, file: &Path) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let file = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    file.strip_prefix(&root)
        .unwrap_or(&file)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// After deleting a file, remove month/year folders left empty (never the section itself).
pub fn prune_empty_dirs(root: &Path, mut dir: &Path) {
    while dir != root && dir.starts_with(root) {
        if std::fs::remove_dir(dir).is_err() {
            break; // not empty
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }
}

// ---------- listing ----------

#[derive(Serialize, Clone)]
pub struct Item {
    /// Relative to the section folder, always with '/'.
    pub path: String,
    pub name: String,
    pub size: u64,
    /// Milliseconds since epoch. Stored files get their capture date as mtime,
    /// so this is "when it was taken/created", not "when it was uploaded".
    pub date: i64,
    /// When the file appeared in the cloud (its birth time on disk).
    pub added: i64,
    pub kind: Kind,
}

pub fn list(root: &Path) -> Vec<Item> {
    let mut items = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.eq_ignore_ascii_case("Thumbs.db") || name.eq_ignore_ascii_case("desktop.ini") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                let Ok(rel) = entry.path().strip_prefix(root).map(Path::to_path_buf) else { continue };
                let rel: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                items.push(Item {
                    path: rel.join("/"),
                    kind: Kind::of(&name),
                    name,
                    size: meta.len(),
                    date: mtime_ms(&meta),
                    added: added_ms(&meta),
                });
            }
        }
    }
    items.sort_by(|a, b| b.date.cmp(&a.date));
    items
}

/// When the file was put into the cloud.
///
/// Not the birth time: macOS moves it back whenever the modification date is set earlier,
/// and we set that to the capture date. On Unix the inode change time (ctime) works instead:
/// it is stamped when the file is written/renamed into place and when its dates are set,
/// and cannot be set back. (Renaming in the cloud therefore counts as "added again".)
/// Windows keeps the creation time untouched, so it is used there.
#[cfg(unix)]
pub fn added_ms(meta: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    meta.ctime() * 1000 + meta.ctime_nsec() / 1_000_000
}

#[cfg(not(unix))]
pub fn added_ms(meta: &std::fs::Metadata) -> i64 {
    meta.created()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or_else(|| mtime_ms(meta), |d| d.as_millis() as i64)
}

pub fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_paths_and_bad_chars() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("C:\\Users\\me\\a<b>.txt"), "ab.txt");
        assert_eq!(sanitize_filename(".."), "file");
        assert_eq!(sanitize_filename("con.txt"), "_con.txt");
        assert_eq!(sanitize_filename("name. . "), "name");
        assert_eq!(sanitize_filename("Фото с моря.jpg"), "Фото с моря.jpg");
    }

    #[test]
    fn resolve_rejects_traversal() {
        let dir = std::env::temp_dir().join(format!("lp-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("2024")).unwrap();
        std::fs::write(dir.join("2024/a.txt"), "x").unwrap();
        assert!(resolve(&dir, "2024/a.txt").is_some());
        assert!(resolve(&dir, "../etc/passwd").is_none());
        assert!(resolve(&dir, "2024/../2024/a.txt").is_none());
        assert!(resolve(&dir, "/etc/passwd").is_none());
        assert!(resolve(&dir, "2024").is_none()); // directories are not files
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unique_adds_counter() {
        let dir = std::env::temp_dir().join(format!("lp-uniq-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jpg"), "x").unwrap();
        assert_eq!(unique_path(&dir, "a.jpg"), dir.join("a (2).jpg"));
        assert_eq!(unique_path(&dir, "b.jpg"), dir.join("b.jpg"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn kinds() {
        assert_eq!(Kind::of("IMG_1.JPG"), Kind::Image);
        assert_eq!(Kind::of("IMG_1.HEIC"), Kind::Raw);
        assert_eq!(Kind::of("clip.MOV"), Kind::Video);
        assert_eq!(Kind::of("doc.pdf"), Kind::File);
        assert_eq!(Kind::of("noext"), Kind::File);
    }
}
