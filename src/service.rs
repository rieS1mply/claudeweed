//! Running in the background: autostart at login, "already running?", --status and --stop.
//!
//! A running cloud writes `status.json` next to the settings: its port, public address
//! and a random secret. Another copy of the program uses it to show the address instead
//! of starting a second tunnel, and `--stop` uses the secret to ask it to quit.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

const STATUS_FILE: &str = "status.json";
const AGENT_ID: &str = "com.local-photo.cloud";
const UNIT_NAME: &str = "local-photo.service";

#[derive(Serialize, Deserialize, Clone)]
pub struct Status {
    pub pid: u32,
    pub port: u16,
    pub url: Option<String>,
    pub secret: String,
    pub storage: PathBuf,
    /// false with --no-tunnel: there will be no public address.
    #[serde(default = "yes")]
    pub tunnel: bool,
}

fn yes() -> bool {
    true
}

pub fn status_path(config_dir: &Path) -> PathBuf {
    config_dir.join(STATUS_FILE)
}

pub fn write_status(config_dir: &Path, status: &Status) {
    let path = status_path(config_dir);
    let tmp = path.with_extension("tmp");
    if let Ok(bytes) = serde_json::to_vec_pretty(status) {
        if std::fs::write(&tmp, bytes).is_ok() {
            crate::config::restrict_permissions(&tmp);
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

pub fn clear_status(config_dir: &Path) {
    let _ = std::fs::remove_file(status_path(config_dir));
}

fn local_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .no_proxy()
        .build()
        .expect("http client")
}

/// The cloud that is running right now, if any (the status file alone may be stale).
pub async fn running(config_dir: &Path) -> Option<Status> {
    let status: Status = serde_json::from_slice(&std::fs::read(status_path(config_dir)).ok()?).ok()?;
    let alive = local_client()
        .get(format!("http://127.0.0.1:{}/api/session", status.port))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success());
    alive.then_some(status)
}

/// Asks the running cloud to shut down. `Ok(false)` if nothing was running.
pub async fn stop(config_dir: &Path) -> Result<bool> {
    let Some(status) = running(config_dir).await else { return Ok(false) };
    local_client()
        .post(format!("http://127.0.0.1:{}/api/local/stop", status.port))
        .header("x-local-secret", &status.secret)
        .send()
        .await?
        .error_for_status()?;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if running(config_dir).await.is_none() {
            return Ok(true);
        }
    }
    bail!("облако не остановилось за 10 секунд")
}

/// Waits (up to `wait`) until a background cloud reports its public address.
pub async fn wait_for_url(config_dir: &Path, wait: Duration) -> Option<String> {
    let deadline = std::time::Instant::now() + wait;
    while std::time::Instant::now() < deadline {
        if let Some(url) = running(config_dir).await.and_then(|s| s.url) {
            return Some(url);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    None
}

/// The address as a QR code drawn with block characters: point the phone camera at it.
pub fn qr(url: &str) -> Option<String> {
    use qrcode::render::unicode::Dense1x2;
    let code = qrcode::QrCode::new(url).ok()?;
    Some(code.render::<Dense1x2>().dark_color(Dense1x2::Light).light_color(Dense1x2::Dark).build())
}

// ---------- Windows: no console windows ----------

/// Windows shows a console window for every console program we start; suppress it.
pub fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Background mode on Windows: let go of the console so its window closes.
pub fn detach_console() {
    #[cfg(windows)]
    {
        extern "system" {
            fn FreeConsole() -> i32;
        }
        unsafe {
            FreeConsole();
        }
    }
}

/// True when the program got its own console window (double-click in Explorer),
/// so the window would vanish before the text in it could be read.
pub fn own_console_window() -> bool {
    #[cfg(windows)]
    {
        extern "system" {
            fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
        }
        let mut pids = [0u32; 4];
        unsafe { GetConsoleProcessList(pids.as_mut_ptr(), 4) <= 1 }
    }
    #[cfg(not(windows))]
    false
}

pub fn pause_if_own_window() {
    if own_console_window() {
        println!();
        println!("Нажмите Enter, чтобы закрыть окно.");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
}

// ---------- autostart ----------

fn home() -> Result<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .context("не найдена домашняя папка")
}

/// The command line autostart runs: this program in background mode (+ portable --dir).
fn launch_args(dir_arg: Option<&Path>) -> Vec<String> {
    let mut args = vec!["--background".to_string()];
    if let Some(d) = dir_arg {
        args.push("--dir".into());
        args.push(d.display().to_string());
    }
    args
}

fn run(cmd: &mut Command) -> Result<()> {
    let out = no_window(cmd).output().with_context(|| format!("не удалось запустить {:?}", cmd.get_program()))?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Registers autostart at login and starts the background cloud right away.
pub fn enable_autostart(exe: &Path, dir_arg: Option<&Path>) -> Result<String> {
    let args = launch_args(dir_arg);
    if cfg!(windows) {
        let quoted: Vec<String> = std::iter::once(exe.display().to_string()).chain(args.clone()).map(|a| format!("\"{a}\"")).collect();
        run(Command::new("reg").args(["add", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "local-photo", "/t", "REG_SZ", "/d"]).arg(quoted.join(" ")).arg("/f"))?;
        spawn_detached(exe, &args)?;
        Ok("автозагрузка Windows (вход в систему)".into())
    } else if cfg!(target_os = "macos") {
        let plist = home()?.join("Library/LaunchAgents").join(format!("{AGENT_ID}.plist"));
        std::fs::create_dir_all(plist.parent().unwrap())?;
        std::fs::write(&plist, launch_agent_plist(exe, &args))?;
        let uid = String::from_utf8(Command::new("id").arg("-u").output()?.stdout)?.trim().to_string();
        let _ = Command::new("launchctl").args(["bootout", &format!("gui/{uid}")]).arg(&plist).output();
        run(Command::new("launchctl").args(["bootstrap", &format!("gui/{uid}")]).arg(&plist))?;
        Ok(format!("LaunchAgent ({})", plist.display()))
    } else {
        let has_systemd = Command::new("systemctl").args(["--user", "show-environment"]).output().is_ok_and(|o| o.status.success());
        if has_systemd {
            let unit = home()?.join(".config/systemd/user").join(UNIT_NAME);
            std::fs::create_dir_all(unit.parent().unwrap())?;
            std::fs::write(&unit, systemd_unit(exe, &args))?;
            run(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
            run(Command::new("systemctl").args(["--user", "enable", "--now", UNIT_NAME]))?;
            // Keep running when nobody is logged in (a home server). May need admin rights.
            let user = std::env::var("USER").unwrap_or_default();
            let linger = Command::new("loginctl").args(["enable-linger", &user]).output().is_ok_and(|o| o.status.success());
            if !linger {
                println!("  Чтобы облако работало и без входа в систему, выполните: sudo loginctl enable-linger {user}");
            }
            Ok(format!("служба systemd ({})", unit.display()))
        } else {
            // Desktop without systemd: XDG autostart, started now by hand.
            let desktop = home()?.join(".config/autostart/local-photo.desktop");
            std::fs::create_dir_all(desktop.parent().unwrap())?;
            let exec: Vec<String> = std::iter::once(exe.display().to_string()).chain(args.clone()).map(|a| format!("\"{a}\"")).collect();
            std::fs::write(&desktop, format!(
                "[Desktop Entry]\nType=Application\nName=Домашнее облако\nExec={}\nX-GNOME-Autostart-enabled=true\n",
                exec.join(" ")
            ))?;
            spawn_detached(exe, &args)?;
            Ok(format!("автозапуск рабочего стола ({})", desktop.display()))
        }
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// macOS LaunchAgent: start at login, restart only after a crash (not after --stop).
fn launch_agent_plist(exe: &Path, args: &[String]) -> String {
    let program: String = std::iter::once(exe.display().to_string())
        .chain(args.iter().cloned())
        .map(|a| format!("    <string>{}</string>\n", xml_escape(&a)))
        .collect();
    format!(
"<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
  <key>Label</key><string>{AGENT_ID}</string>
  <key>ProgramArguments</key>
  <array>
{program}  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>30</integer>
</dict>
</plist>
")
}

/// systemd user unit: restart after a crash or while the storage disk is not mounted yet.
fn systemd_unit(exe: &Path, args: &[String]) -> String {
    let exec: Vec<String> = std::iter::once(exe.display().to_string())
        .chain(args.iter().cloned())
        .map(|a| format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    format!(
"[Unit]
Description=Домашнее облако (local-photo)
Wants=network-online.target
After=network-online.target

[Service]
ExecStart={}
Restart=on-failure
RestartSec=15

[Install]
WantedBy=default.target
", exec.join(" "))
}

pub fn disable_autostart() -> Result<()> {
    if cfg!(windows) {
        let _ = run(Command::new("reg").args(["delete", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "local-photo", "/f"]));
    } else if cfg!(target_os = "macos") {
        let plist = home()?.join("Library/LaunchAgents").join(format!("{AGENT_ID}.plist"));
        if plist.exists() {
            let uid = String::from_utf8(Command::new("id").arg("-u").output()?.stdout)?.trim().to_string();
            let _ = Command::new("launchctl").args(["bootout", &format!("gui/{uid}")]).arg(&plist).output();
            std::fs::remove_file(&plist)?;
        }
    } else {
        let unit = home()?.join(".config/systemd/user").join(UNIT_NAME);
        if unit.exists() {
            let _ = Command::new("systemctl").args(["--user", "disable", "--now", UNIT_NAME]).output();
            std::fs::remove_file(&unit)?;
            let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).output();
        }
        let _ = std::fs::remove_file(home()?.join(".config/autostart/local-photo.desktop"));
    }
    Ok(())
}

pub fn autostart_enabled() -> bool {
    let Ok(home) = home() else { return false };
    if cfg!(windows) {
        no_window(Command::new("reg").args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "local-photo"]))
            .output()
            .is_ok_and(|o| o.status.success())
    } else if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents").join(format!("{AGENT_ID}.plist")).exists()
    } else {
        home.join(".config/systemd/user").join(UNIT_NAME).exists() || home.join(".config/autostart/local-photo.desktop").exists()
    }
}

/// Starts a copy of the program that outlives this one (no window).
fn spawn_detached(exe: &Path, args: &[String]) -> Result<()> {
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);   // not killed together with the terminal
    }
    cmd.spawn().context("не удалось запустить облако в фоне")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_files() {
        let exe = Path::new("/Users/me/Мои программы/local-photo");
        let args = launch_args(Some(Path::new("/data/a&b")));
        let plist = launch_agent_plist(exe, &args);
        assert!(plist.contains("<string>/Users/me/Мои программы/local-photo</string>"));
        assert!(plist.contains("<string>--background</string>") && plist.contains("<string>/data/a&amp;b</string>"));
        // Written out so it can be checked with `plutil -lint`.
        std::fs::write(std::env::temp_dir().join("lp-test.plist"), &plist).unwrap();

        let unit = systemd_unit(exe, &args);
        assert!(unit.contains("ExecStart=\"/Users/me/Мои программы/local-photo\" \"--background\" \"--dir\" \"/data/a&b\""));
        assert!(unit.contains("Restart=on-failure"));
    }

    #[test]
    fn qr_renders() {
        let q = qr("https://example.trycloudflare.com").unwrap();
        assert!(q.lines().count() > 10);
    }
}
