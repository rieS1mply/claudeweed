//! "Download selected" as one ZIP. The archive is built in the background into a temp
//! file; the page polls for progress and then downloads it with a normal link. (Building
//! inside one request could take longer than Cloudflare waits for a response.)

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{Datelike, Local, TimeZone, Timelike};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Finished archives are kept this long for the download, then deleted.
const KEEP: Duration = Duration::from_secs(3600);

pub enum State {
    Building,
    Ready,
    Failed(String),
}

pub struct Job {
    pub path: PathBuf,
    pub total: u64,
    pub done: AtomicU64,
    pub state: Mutex<State>,
    pub created: Instant,
}

#[derive(Default)]
pub struct Jobs(Mutex<HashMap<String, Arc<Job>>>);

impl Jobs {
    pub fn get(&self, id: &str) -> Option<Arc<Job>> {
        self.0.lock().unwrap().get(id).cloned()
    }

    /// Starts building `files` into `<dir>/<id>.zip`; returns the job id.
    pub fn start(&self, dir: &Path, files: Vec<PathBuf>) -> String {
        let id = crate::auth::random_hex(12);
        let job = Arc::new(Job {
            path: dir.join(format!("{id}.zip")),
            total: files.iter().filter_map(|f| f.metadata().ok()).map(|m| m.len()).sum(),
            done: AtomicU64::new(0),
            state: Mutex::new(State::Building),
            created: Instant::now(),
        });
        self.0.lock().unwrap().insert(id.clone(), job.clone());
        tokio::task::spawn_blocking(move || {
            let result = build(&job, &files);
            *job.state.lock().unwrap() = match result {
                Ok(()) => State::Ready,
                Err(e) => {
                    let _ = std::fs::remove_file(&job.path);
                    State::Failed(format!("{e:#}"))
                }
            };
        });
        id
    }

    /// Deletes archives older than an hour.
    pub fn cleanup(&self) {
        self.0.lock().unwrap().retain(|_, job| {
            let keep = job.created.elapsed() < KEEP || matches!(*job.state.lock().unwrap(), State::Building);
            if !keep {
                let _ = std::fs::remove_file(&job.path);
            }
            keep
        });
    }
}

fn build(job: &Job, files: &[PathBuf]) -> Result<()> {
    let mut zip = ZipWriter::new(BufWriter::new(File::create(&job.path)?));
    let mut used = HashMap::<String, u32>::new();
    for file in files {
        let meta = file.metadata()?;
        let name = unique_name(&mut used, &file.file_name().unwrap_or_default().to_string_lossy());
        // Photos and videos are already compressed; storing them as is is fast and loses nothing.
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(meta.len() >= u32::MAX as u64)
            .last_modified_time(zip_time(crate::storage::mtime_ms(&meta)));
        zip.start_file(name, options)?;
        let mut reader = BufReader::new(File::open(file)?);
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            std::io::Write::write_all(&mut zip, &buf[..n])?;
            job.done.fetch_add(n as u64, Ordering::Relaxed);
        }
    }
    zip.finish()?;
    Ok(())
}

/// Same name from different months → "IMG_0001 (2).JPG" inside the archive.
fn unique_name(used: &mut HashMap<String, u32>, name: &str) -> String {
    let count = used.entry(name.to_lowercase()).or_insert(0);
    *count += 1;
    if *count == 1 {
        return name.to_string();
    }
    match name.rfind('.') {
        Some(i) if i > 0 => format!("{} ({}){}", &name[..i], count, &name[i..]),
        _ => format!("{name} ({count})"),
    }
}

/// The file's date inside the archive, so unpacked photos keep it.
fn zip_time(ms: i64) -> zip::DateTime {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .and_then(|t| {
            zip::DateTime::from_date_and_time(
                t.year().clamp(1980, 2107) as u16,
                t.month() as u8,
                t.day() as u8,
                t.hour() as u8,
                t.minute() as u8,
                t.second() as u8,
            )
            .ok()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_names_get_numbers() {
        let mut used = HashMap::new();
        assert_eq!(unique_name(&mut used, "IMG.jpg"), "IMG.jpg");
        assert_eq!(unique_name(&mut used, "img.JPG"), "img (2).JPG");
        assert_eq!(unique_name(&mut used, "IMG.jpg"), "IMG (3).jpg");
    }
}
