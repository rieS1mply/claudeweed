//! Downloads `cloudflared` on first run and keeps a free "quick tunnel"
//! (https://<random>.trycloudflare.com → 127.0.0.1:<port>) running.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

const RELEASES: &str = "https://github.com/cloudflare/cloudflared/releases/latest/download";
/// Pause between "connection registered" and announcing the URL, for DNS to catch up.
const DNS_GRACE: Duration = Duration::from_secs(8);

fn binary_name() -> &'static str {
    if cfg!(windows) { "cloudflared.exe" } else { "cloudflared" }
}

fn release_asset() -> Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "cloudflared-darwin-arm64.tgz",
        ("macos", "x86_64") => "cloudflared-darwin-amd64.tgz",
        ("linux", "x86_64") => "cloudflared-linux-amd64",
        ("linux", "aarch64") => "cloudflared-linux-arm64",
        ("linux", "arm") => "cloudflared-linux-arm",
        ("windows", "x86_64" | "aarch64") => "cloudflared-windows-amd64.exe",
        ("windows", "x86") => "cloudflared-windows-386.exe",
        (os, arch) => bail!("cloudflared не публикуется для {os}/{arch}"),
    })
}

/// Path to a working cloudflared: ours in `bin_dir`, downloaded if needed,
/// or one already installed in PATH as a fallback.
pub async fn ensure_cloudflared(bin_dir: &Path) -> Result<PathBuf> {
    let path = bin_dir.join(binary_name());
    if path.exists() {
        return Ok(path);
    }
    crate::log!("Скачиваю cloudflared (один раз, ~40 МБ)…");
    match download(&path).await {
        Ok(()) => {
            crate::log!("cloudflared сохранён в {}", path.display());
            Ok(path)
        }
        Err(e) => {
            let system = PathBuf::from(binary_name());
            let mut check = Command::new(&system);
            check.arg("--version").stdout(Stdio::null()).stderr(Stdio::null());
            hide_window(&mut check);
            let works = check.status().await;
            if works.is_ok_and(|s| s.success()) {
                crate::log!("Не удалось скачать cloudflared ({e:#}), использую установленный в системе");
                Ok(system)
            } else {
                Err(e)
            }
        }
    }
}

async fn download(dest: &Path) -> Result<()> {
    let asset = release_asset()?;
    let bytes = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()?
        .get(format!("{RELEASES}/{asset}"))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await
        .context("загрузка cloudflared")?;

    let tmp = dest.with_extension("download");
    let tmp2 = tmp.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        if asset.ends_with(".tgz") {
            let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..]));
            let mut found = false;
            for entry in archive.entries()? {
                let mut entry = entry?;
                if entry.path()?.file_name().is_some_and(|n| n == "cloudflared") {
                    entry.unpack(&tmp2)?;
                    found = true;
                    break;
                }
            }
            if !found {
                bail!("в архиве нет cloudflared");
            }
        } else {
            std::fs::write(&tmp2, &bytes)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp2, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    })
    .await??;
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

/// Runs cloudflared forever, restarting it when it exits. `on_url` is called with
/// the public address every time a (new) tunnel comes up.
pub async fn run<F>(bin: PathBuf, port: u16, on_url: F)
where
    F: Fn(String) + Send + Sync + 'static,
{
    let mut backoff = Duration::from_secs(5);
    loop {
        let started = Instant::now();
        if let Err(e) = run_once(&bin, port, &on_url).await {
            crate::log!("Туннель: {e:#}");
        }
        // A tunnel that lived for a while was fine; restart quickly. Otherwise back off
        // (Cloudflare rate-limits quick tunnels that are created too often).
        if started.elapsed() > Duration::from_secs(300) {
            backoff = Duration::from_secs(5);
        }
        crate::log!("Туннель остановился, перезапуск через {} с…", backoff.as_secs());
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(300));
    }
}

async fn run_once<F: Fn(String)>(bin: &Path, port: u16, on_url: &F) -> Result<()> {
    let mut cmd = Command::new(bin);
    cmd.args(["tunnel", "--no-autoupdate", "--url", &format!("http://127.0.0.1:{port}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        // If we are stopped (Ctrl+C, task aborted), the tunnel must not outlive us.
        .kill_on_drop(true);
    hide_window(&mut cmd);
    let mut child = cmd.spawn().with_context(|| format!("не удалось запустить {}", bin.display()))?;

    // cloudflared logs to stderr. Keep draining it, or it blocks once the pipe is full.
    let stderr = child.stderr.take().expect("piped stderr");
    let mut lines = BufReader::new(stderr).lines();
    // The URL is printed before the tunnel is actually connected. Announce it only once
    // cloudflared has registered a connection (and DNS had a moment): a link opened too
    // early fails, and phones then cache "domain not found" for minutes.
    let mut pending: Option<String> = None;
    let mut announced = false;
    let mut repeats = RepeatFilter::default();
    while let Some(line) = lines.next_line().await? {
        if !announced {
            if pending.is_none() {
                if let Some(url) = extract_url(&line) {
                    crate::log!("Туннель создан, жду подключения…");
                    pending = Some(url);
                    continue;
                }
            } else if line.contains("Registered tunnel connection") {
                tokio::time::sleep(DNS_GRACE).await;
                announced = true;
                on_url(pending.take().expect("checked above"));
                continue;
            }
        }
        // "canceled" = the browser dropped a request (closed a photo, scrolled on): not a problem.
        if line.contains(" ERR ") && !line.contains("canceled") {
            if let Some(note) = repeats.check(&line, Instant::now()) {
                crate::log!("cloudflared: {}{note}", line.trim());
            }
        }
    }
    let status = child.wait().await?;
    bail!("cloudflared завершился ({status})")
}

/// Some cloudflared errors repeat every few minutes for hours (e.g. "Failed to refresh DNS
/// local resolver" on networks with a slow DNS) while the tunnel works fine. Each distinct
/// message is logged at most once per [`REPEAT_WINDOW`], with how many copies were skipped.
const REPEAT_WINDOW: Duration = Duration::from_secs(6 * 3600);

#[derive(Default)]
struct RepeatFilter {
    /// Message without its timestamp -> when it was last logged, copies skipped since.
    seen: std::collections::HashMap<String, (Instant, u32)>,
}

impl RepeatFilter {
    /// `Some(suffix)` when the line should be logged now.
    fn check(&mut self, line: &str, now: Instant) -> Option<String> {
        let key = line.split_once(" ERR ").map_or(line, |(_, msg)| msg).trim().to_string();
        match self.seen.get_mut(&key) {
            Some((last, skipped)) if now.duration_since(*last) < REPEAT_WINDOW => {
                *skipped += 1;
                None
            }
            Some((last, skipped)) => {
                let note = if *skipped == 0 { String::new() } else { format!(" (повторялось ещё {skipped} раз)") };
                *last = now;
                *skipped = 0;
                Some(note)
            }
            None => {
                self.seen.insert(key, (now, 0));
                Some(" (повторы скрываются на 6 ч)".to_string())
            }
        }
    }
}

/// Windows would open a console window for cloudflared (visible when we run in the background).
fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    #[cfg(not(windows))]
    let _ = cmd;
}

fn extract_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let url: String = line[start..].chars().take_while(|c| !c.is_whitespace() && *c != '|').collect();
    url.ends_with(".trycloudflare.com").then_some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_url_in_banner() {
        let line = "2026-10-26T15:00:00Z INF |  https://lazy-river-bold-cat.trycloudflare.com                    |";
        assert_eq!(extract_url(line).as_deref(), Some("https://lazy-river-bold-cat.trycloudflare.com"));
        assert_eq!(extract_url("INF Requesting new quick Tunnel on trycloudflare.com..."), None);
        assert_eq!(extract_url("see https://developers.cloudflare.com/docs"), None);
    }

    #[test]
    fn repeated_errors_are_logged_rarely() {
        let mut f = RepeatFilter::default();
        let t0 = Instant::now();
        let a = |ts: &str| format!("{ts} ERR Failed to refresh DNS local resolver error=\"timeout\"");
        assert!(f.check(&a("2026-10-29T13:29:35Z"), t0).is_some());
        assert!(f.check(&a("2026-10-29T13:34:40Z"), t0 + Duration::from_secs(300)).is_none());
        assert!(f.check(&a("2026-10-29T13:39:45Z"), t0 + Duration::from_secs(600)).is_none());
        assert!(f.check("2026-10-29T13:40:00Z ERR other problem", t0 + Duration::from_secs(601)).is_some());
        let later = f.check(&a("2026-10-29T20:00:00Z"), t0 + REPEAT_WINDOW + Duration::from_secs(1));
        assert_eq!(later.as_deref(), Some(" (повторялось ещё 2 раз)"));
    }
}
