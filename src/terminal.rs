//! Remote terminal: up to `MAX_SESSIONS` named shells on this computer. Every browser
//! tab or pane attached to the same session sees the same screen.
//!
//! A shell keeps running when the browser disconnects (a phone switching networks,
//! a closed laptop lid); reconnecting replays the recent output, so the screen comes
//! back as it was. Off unless `"terminal": true` in config.json — whoever knows the
//! password controls the computer.
//!
//! Sessions are listed, created, renamed and closed over REST (`/api/terminal/sessions`);
//! `/api/terminal?id=N` attaches to one. Protocol over that WebSocket, binary frames only:
//! - server → client: terminal output, raw-deflate compressed as one continuous stream
//!   (`?z=1`, sync flush per frame), or plain bytes;
//! - client → server: `0` + input bytes, `1` + cols(u16 BE) + rows(u16 BE), `2` = kill the shell.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use flate2::{Compress, Compression, FlushCompress};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tokio::sync::{broadcast, watch};

use crate::terminal_store::{self as store, ClaudeRun, CwdTracker};

/// Output kept for replay on reconnect.
const HISTORY: usize = 128 * 1024;
/// Output chunks a slow client may fall behind before it is resynced from the history.
const BACKLOG: usize = 512;
/// Below this, output goes out at once (typing echo); above, wait briefly to batch a burst.
const INTERACTIVE: usize = 1024;
const BATCH_WAIT: Duration = Duration::from_millis(5);
const MAX_FRAME: usize = 64 * 1024;
/// Cloudflare drops WebSockets idle for ~100 s.
const PING_EVERY: Duration = Duration::from_secs(25);
const PONG_TIMEOUT: Duration = Duration::from_secs(70);
/// One Telegram message per address per this long, not one per reconnect.
const NOTIFY_EVERY: Duration = Duration::from_secs(3600);
/// Each shell is a process with its own history buffer; keep the total bounded.
pub const MAX_SESSIONS: usize = 8;
const MAX_NAME: usize = 40;

/// WebSocket close codes the page understands.
pub const CLOSE_RESYNC: u16 = 4000;
pub const CLOSE_EXITED: u16 = 4001;
pub const CLOSE_LOGGED_OUT: u16 = 4401;
pub const CLOSE_DISABLED: u16 = 4403;
/// The session was closed (from another device, or before this one reconnected).
pub const CLOSE_GONE: u16 = 4404;

#[derive(Default)]
pub struct Terminal {
    sessions: Mutex<Sessions>,
    notified: Mutex<HashMap<String, Instant>>,
    /// Folder the sessions are saved to (see `terminal_store`); None in tests.
    store: Mutex<Option<PathBuf>>,
}

#[derive(Default)]
struct Sessions {
    last_id: u32,
    /// In creation order: that's the order of the tabs.
    list: Vec<Session>,
}

struct Session {
    id: u32,
    name: String,
    shell: Arc<Shell>,
    /// Claude Code seen running in the shell at the last save.
    claude: Option<ClaudeRun>,
    /// `Shell::generation` of the output saved last time.
    saved_output: u64,
}

#[derive(Serialize)]
pub struct SessionInfo {
    pub id: u32,
    pub name: String,
    pub alive: bool,
    /// Browser connections attached right now (other devices included).
    pub clients: usize,
}

impl Session {
    fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id,
            name: self.name.clone(),
            alive: !self.shell.dead.load(Ordering::SeqCst),
            clients: self.shell.clients.load(Ordering::SeqCst),
        }
    }

    /// The shell, started afresh if it exited ("new session" in the same tab).
    fn live_shell(&mut self) -> Result<Arc<Shell>> {
        if self.shell.dead.load(Ordering::SeqCst) {
            // The folder may be gone since (or not a folder: `cd HKCU:` reports a registry path).
            let cwd = self.shell.cwd.lock().unwrap().clone().filter(|d| d.is_dir());
            self.shell = Shell::spawn(default_shell(cwd.as_deref()))?;
            self.claude = None;
        }
        Ok(self.shell.clone())
    }
}

impl Sessions {
    fn create(&mut self, name: Option<&str>) -> Result<&mut Session> {
        if self.list.len() >= MAX_SESSIONS {
            anyhow::bail!("Открыто максимум сеансов ({MAX_SESSIONS}). Закройте ненужный.");
        }
        let shell = Shell::spawn(default_shell(None))?;
        self.last_id += 1;
        let id = self.last_id;
        let name = name.and_then(clean_name).unwrap_or_else(|| format!("Терминал {id}"));
        self.list.push(Session { id, name, shell, claude: None, saved_output: 0 });
        Ok(self.list.last_mut().unwrap())
    }
}

/// Trimmed, single-line, at most `MAX_NAME` characters; None if nothing is left.
fn clean_name(name: &str) -> Option<String> {
    let name: String = name.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

struct Shell {
    input: std::sync::mpsc::Sender<Vec<u8>>,
    pid: Option<u32>,
    /// Folder the shell is in (from its prompt on Windows, from the process elsewhere).
    cwd: Mutex<Option<PathBuf>>,
    cwd_tracker: Mutex<CwdTracker>,
    /// Bumped on every output, so unchanged sessions aren't written again.
    generation: AtomicU64,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    /// Recent output; locked together with `subscribe` so a new client sees every byte exactly once.
    history: Mutex<VecDeque<u8>>,
    output: broadcast::Sender<Arc<[u8]>>,
    exited: watch::Sender<bool>,
    dead: AtomicBool,
    /// Closed on purpose: attached pages are told the session is gone, not that it exited.
    gone: AtomicBool,
    clients: AtomicUsize,
}

/// Counts a connection in `Shell::clients` while it lives.
struct Attached(Arc<Shell>);

impl Drop for Attached {
    fn drop(&mut self) {
        self.0.clients.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Terminal {
    pub fn list(&self) -> Vec<SessionInfo> {
        self.sessions.lock().unwrap().list.iter().map(Session::info).collect()
    }

    pub fn create(&self, name: Option<&str>) -> Result<SessionInfo> {
        let mut sessions = self.sessions.lock().unwrap();
        Ok(sessions.create(name)?.info())
    }

    pub fn rename(&self, id: u32, name: &str) -> Option<SessionInfo> {
        let mut sessions = self.sessions.lock().unwrap();
        let s = sessions.list.iter_mut().find(|s| s.id == id)?;
        s.name = clean_name(name)?;
        Some(s.info())
    }

    /// Kills the shell and everything running in it; attached pages see `CLOSE_GONE`.
    pub fn close(&self, id: u32) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        let Some(i) = sessions.list.iter().position(|s| s.id == id) else {
            return false;
        };
        let s = sessions.list.remove(i);
        s.shell.gone.store(true, Ordering::SeqCst);
        s.shell.kill();
        true
    }

    /// Session `id`, or without one (a page from an older version) the first session,
    /// created if there is none. None if `id` no longer exists.
    fn shell(&self, id: Option<u32>) -> Result<Option<Arc<Shell>>> {
        let mut sessions = self.sessions.lock().unwrap();
        let session = match id {
            Some(id) => sessions.list.iter_mut().find(|s| s.id == id),
            None if sessions.list.is_empty() => Some(sessions.create(None)?),
            None => sessions.list.first_mut(),
        };
        session.map(Session::live_shell).transpose()
    }

    /// Ends every shell (and what runs in them) when the cloud stops. Saved sessions stay
    /// on disk and come back on the next start.
    pub fn shutdown(&self) {
        for s in self.sessions.lock().unwrap().list.drain(..) {
            s.shell.gone.store(true, Ordering::SeqCst);
            s.shell.kill();
        }
    }

    /// The terminal was turned off: end every shell and forget the saved ones too.
    pub fn disable(&self) {
        self.shutdown();
        if let Some(dir) = self.store.lock().unwrap().as_ref() {
            store::forget(dir);
        }
    }

    /// Sets where sessions are saved and, if `start`, brings back the ones saved last time
    /// (each in its folder, with its old output; Claude Code resumed where it was).
    pub fn restore(&self, dir: &Path, start: bool) {
        *self.store.lock().unwrap() = Some(dir.to_path_buf());
        if !start {
            return;
        }
        let saved = store::load(dir);
        let mut sessions = self.sessions.lock().unwrap();
        if !sessions.list.is_empty() {
            return;
        }
        sessions.last_id = sessions.last_id.max(saved.last_id);
        for s in saved.sessions.into_iter().take(MAX_SESSIONS) {
            let cwd = s.claude.as_ref().and_then(|c| c.cwd.clone()).or(s.cwd).filter(|d| d.is_dir());
            let shell = match Shell::spawn(default_shell(cwd.as_deref())) {
                Ok(sh) => sh,
                Err(e) => {
                    crate::log!("Терминал: сеанс «{}» не восстановлен: {e:#}", s.name);
                    continue;
                }
            };
            let mut replay = store::load_output(dir, s.id);
            if !replay.is_empty() {
                // Reset colours, leave a full-screen program's screen, then a marker line.
                replay.extend_from_slice("\x1b[0m\x1b[?1049l\r\n\x1b[2m--- облако перезапущено, сеанс восстановлен ---\x1b[0m\r\n".as_bytes());
                shell.prepend_history(&replay);
            }
            if let Some(run) = &s.claude {
                let line = store::resume_command(run);
                crate::log!("Терминал: в сеансе «{}» снова запускаю Claude ({line})", s.name);
                let _ = shell.input.send(format!("{line}\r").into_bytes());
            }
            sessions.last_id = sessions.last_id.max(s.id);
            sessions.list.push(Session { id: s.id, name: s.name, shell, claude: s.claude, saved_output: 0 });
        }
        if !sessions.list.is_empty() {
            crate::log!("Терминал: восстановлено сеансов: {}", sessions.list.len());
        }
    }

    /// Writes the open sessions to disk (blocking: looks at the process tree). Called every
    /// few seconds, so a power cut or a reboot loses only the last moments.
    pub fn save(&self) {
        let Some(dir) = self.store.lock().unwrap().clone() else { return };
        let shells: Vec<(u32, u32, Option<ClaudeRun>)> = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .list
                .iter()
                .filter(|s| !s.shell.dead.load(Ordering::SeqCst))
                .filter_map(|s| Some((s.id, s.shell.pid?, s.claude.clone())))
                .collect()
        };
        let mut found = if shells.is_empty() { HashMap::new() } else { store::scan(&shells) };
        let mut sessions = self.sessions.lock().unwrap();
        let mut saved = store::Saved { last_id: sessions.last_id, sessions: Vec::new() };
        for s in sessions.list.iter_mut() {
            if let Some(f) = found.remove(&s.id) {
                if !cfg!(windows) && f.cwd.is_some() {
                    *s.shell.cwd.lock().unwrap() = f.cwd;
                }
                s.claude = f.claude;
            }
            let generation = s.shell.generation.load(Ordering::SeqCst);
            if generation != s.saved_output {
                store::save_output(&dir, s.id, &s.shell.attach().0);
                s.saved_output = generation;
            }
            saved.sessions.push(store::SavedSession {
                id: s.id,
                name: s.name.clone(),
                cwd: s.shell.cwd.lock().unwrap().clone(),
                claude: s.claude.clone(),
            });
        }
        drop(sessions);
        store::save(&dir, &saved);
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
}

/// The user's login shell (macOS/Linux), PowerShell on Windows. Starts in `cwd`, or the home folder.
fn default_shell(cwd: Option<&Path>) -> CommandBuilder {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = CommandBuilder::new("powershell.exe");
        // The prompt reports its folder, so the session can come back there after a restart.
        c.args(["-NoLogo", "-NoExit", "-EncodedCommand", &store::ps_hook_encoded()]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = CommandBuilder::new_default_prog();
    match cwd {
        Some(dir) => cmd.cwd(dir),
        None => {
            if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
                cmd.cwd(home);
            }
        }
    }
    for var in store::inherited_agent_vars() {
        cmd.env_remove(var);
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    // Started by launchd/systemd there may be no locale, and Cyrillic would show as "?".
    #[cfg(not(windows))]
    if std::env::var_os("LANG").is_none() && std::env::var_os("LC_ALL").is_none() {
        cmd.env("LANG", if cfg!(target_os = "macos") { "en_US.UTF-8" } else { "C.UTF-8" });
    }
    cmd
}

impl Shell {
    fn spawn(cmd: CommandBuilder) -> Result<Arc<Shell>> {
        let pair = native_pty_system()
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .context("не удалось открыть терминал")?;
        let start_dir = cmd.get_cwd().map(PathBuf::from);
        let mut child = pair.slave.spawn_command(cmd).context("не удалось запустить оболочку")?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;
        let (input, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();

        let shell = Arc::new(Shell {
            input,
            pid: child.process_id(),
            cwd: Mutex::new(start_dir),
            cwd_tracker: Default::default(),
            generation: AtomicU64::new(0),
            killer: Mutex::new(child.clone_killer()),
            master: Mutex::new(Some(pair.master)),
            history: Mutex::new(VecDeque::with_capacity(HISTORY)),
            output: broadcast::channel(BACKLOG).0,
            exited: watch::channel(false).0,
            dead: AtomicBool::new(false),
            gone: AtomicBool::new(false),
            clients: AtomicUsize::new(0),
        });

        // Writes to the shell can block (it isn't reading), so they get their own thread.
        std::thread::spawn(move || {
            while let Ok(bytes) = input_rx.recv() {
                if writer.write_all(&bytes).and_then(|_| writer.flush()).is_err() {
                    break;
                }
            }
        });

        let out = shell.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => out.push_output(&buf[..n]),
                }
            }
            out.dead.store(true, Ordering::SeqCst);
            out.exited.send_replace(true);
        });

        // When the shell exits, close the PTY so the reader sees the end even if a
        // background job still holds it open (and on Windows, where it never would).
        let waiter = shell.clone();
        std::thread::spawn(move || {
            let _ = child.wait();
            std::thread::sleep(Duration::from_millis(300));
            waiter.master.lock().unwrap().take();
        });

        crate::log!("Терминал: запущена новая оболочка");
        Ok(shell)
    }

    fn push_output(&self, bytes: &[u8]) {
        if let Some(dir) = self.cwd_tracker.lock().unwrap().push(bytes) {
            *self.cwd.lock().unwrap() = Some(dir);
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        let stripped = strip_cursor_queries(bytes);
        // ConPTY asks for the cursor position at start and waits for the answer: with nobody
        // watching (a session restored after a restart) the shell would hang until a browser
        // connects, so it is answered here.
        if stripped.len() != bytes.len() && self.clients.load(Ordering::SeqCst) == 0 {
            let _ = self.input.send(CURSOR_REPLY.to_vec());
        }
        let mut history = self.history.lock().unwrap();
        history.extend(stripped.iter());
        if history.len() > HISTORY {
            let mut cut = history.len() - HISTORY;
            // Start the replay at a line boundary rather than mid-escape-sequence.
            if let Some(nl) = history.range(cut..).take(4096).position(|&b| b == b'\n') {
                cut += nl + 1;
            }
            history.drain(..cut);
        }
        let _ = self.output.send(bytes.into());
    }

    /// Output from before a restart, shown above what the new shell prints.
    fn prepend_history(&self, old: &[u8]) {
        let mut history = self.history.lock().unwrap();
        let mut start = old.len().saturating_sub(HISTORY / 2);
        if start > 0 {
            if let Some(nl) = old[start..].iter().take(4096).position(|&b| b == b'\n') {
                start += nl + 1;
            }
        }
        for &b in strip_cursor_queries(&old[start..]).iter().rev() {
            history.push_front(b);
        }
    }

    fn attach(&self) -> (Vec<u8>, broadcast::Receiver<Arc<[u8]>>) {
        let history = self.history.lock().unwrap();
        let rx = self.output.subscribe();
        (history.iter().copied().collect(), rx)
    }

    fn resize(&self, cols: u16, rows: u16) {
        if let Some(m) = self.master.lock().unwrap().as_ref() {
            let _ = m.resize(PtySize { rows: rows.max(2), cols: cols.max(2), pixel_width: 0, pixel_height: 0 });
        }
    }

    fn kill(&self) {
        let _ = self.killer.lock().unwrap().kill();
    }
}

/// "Where is the cursor?" (DSR 6). A browser answers it as typed input.
const CURSOR_QUERY: &[u8] = b"\x1b[6n";
const CURSOR_REPLY: &[u8] = b"\x1b[1;1R";

/// Output without cursor queries, for the replay: a query there was answered when it was
/// live, and xterm.js answering it again on every reconnect sends a stray reply to the
/// shell, which then swallows the next key typed (`claude` ran as `laude`).
fn strip_cursor_queries(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if !bytes.windows(CURSOR_QUERY.len()).any(|w| w == CURSOR_QUERY) {
        return bytes.into();
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(CURSOR_QUERY) {
            i += CURSOR_QUERY.len();
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out.into()
}

/// Raw deflate, one context per connection: repeated prompts and colors compress to almost nothing.
pub struct Deflater(Compress);

impl Deflater {
    pub fn new() -> Self {
        Deflater(Compress::new(Compression::fast(), false))
    }

    /// Compresses `data` and sync-flushes, so the browser can show it right away.
    pub fn frame(&mut self, mut data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() / 2 + 64);
        loop {
            if out.capacity() - out.len() < 64 {
                out.reserve(out.capacity().max(1024));
            }
            let before = self.0.total_in();
            self.0.compress_vec(data, &mut out, FlushCompress::Sync).expect("deflate");
            data = &data[(self.0.total_in() - before) as usize..];
            // Done when all input is consumed and the flush fit in the spare room.
            if data.is_empty() && out.len() < out.capacity() {
                return out;
            }
        }
    }
}

pub struct Client {
    pub compress: bool,
    pub ip: String,
    /// Still logged in and the terminal still enabled; checked on every ping.
    pub still_allowed: Box<dyn Fn() -> Option<u16> + Send>,
}

/// Runs one browser connection until it closes.
pub async fn serve(term: Arc<Terminal>, mut ws: WebSocket, client: Client, id: Option<u32>) {
    let shell = match term.shell(id) {
        Ok(Some(s)) => s,
        Ok(None) => {
            let _ = ws.send(close(CLOSE_GONE, "gone")).await;
            return;
        }
        Err(e) => {
            crate::log!("Терминал: {e:#}");
            let _ = ws.send(close(CLOSE_EXITED, "не удалось запустить оболочку")).await;
            return;
        }
    };
    shell.clients.fetch_add(1, Ordering::SeqCst);
    let _attached = Attached(shell.clone());
    let (history, mut rx) = shell.attach();
    let mut exited = shell.exited.subscribe();
    let mut deflate = client.compress.then(Deflater::new);
    let mut encode = move |data: &[u8]| match deflate.as_mut() {
        Some(d) => d.frame(data),
        None => data.to_vec(),
    };

    if !history.is_empty() && ws.send(Message::Binary(encode(&history).into())).await.is_err() {
        return;
    }
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    let mut last_heard = Instant::now();

    let reason = loop {
        tokio::select! {
            out = rx.recv() => {
                let mut batch = match out {
                    Ok(chunk) => chunk.to_vec(),
                    // Too far behind: reconnecting replays the history, cheaper than the backlog.
                    Err(broadcast::error::RecvError::Lagged(_)) => break close(CLOSE_RESYNC, "resync"),
                    Err(broadcast::error::RecvError::Closed) => break exit_reason(&shell),
                };
                let deadline = tokio::time::Instant::now() + BATCH_WAIT;
                while batch.len() < MAX_FRAME {
                    let next = if batch.len() < INTERACTIVE {
                        rx.try_recv().ok()
                    } else {
                        tokio::time::timeout_at(deadline, rx.recv()).await.ok().and_then(Result::ok)
                    };
                    match next {
                        Some(chunk) => batch.extend_from_slice(&chunk),
                        None => break,
                    }
                }
                if ws.send(Message::Binary(encode(&batch).into())).await.is_err() {
                    return;
                }
            }
            msg = ws.recv() => {
                last_heard = Instant::now();
                match msg {
                    Some(Ok(Message::Binary(b))) => match b.first() {
                        Some(0) => {
                            let _ = shell.input.send(b[1..].to_vec());
                        }
                        Some(1) if b.len() == 5 => {
                            shell.resize(u16::from_be_bytes([b[1], b[2]]), u16::from_be_bytes([b[3], b[4]]));
                        }
                        Some(2) => {
                            crate::log!("Терминал: оболочка перезапущена из браузера (адрес: {})", client.ip);
                            shell.kill();
                        }
                        _ => {}
                    },
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                    Some(Ok(_)) => {}
                }
            }
            _ = exited.changed() => {
                // Let the last output reach this client first.
                while let Ok(chunk) = rx.try_recv() {
                    if ws.send(Message::Binary(encode(&chunk).into())).await.is_err() {
                        return;
                    }
                }
                break exit_reason(&shell);
            }
            _ = ping.tick() => {
                if let Some(code) = (client.still_allowed)() {
                    if code == CLOSE_DISABLED {
                        // Turned off while running: nobody should keep a shell.
                        term.disable();
                    }
                    break close(code, "");
                }
                if last_heard.elapsed() > PONG_TIMEOUT || ws.send(Message::Ping(Default::default())).await.is_err() {
                    return;
                }
            }
        }
    };
    let _ = ws.send(reason).await;
}

fn exit_reason(shell: &Shell) -> Message {
    if shell.gone.load(Ordering::SeqCst) {
        close(CLOSE_GONE, "gone")
    } else {
        close(CLOSE_EXITED, "exited")
    }
}

fn close(code: u16, reason: &'static str) -> Message {
    Message::Close(Some(CloseFrame { code, reason: reason.into() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Decompress, FlushDecompress};

    #[test]
    fn deflate_frames_decode_one_by_one() {
        let mut d = Deflater::new();
        let mut inflate = Decompress::new(false);
        let big = "строка вывода 0123456789\r\n".repeat(20_000);
        for msg in ["$ ", "ls\r\n", big.as_str(), "$ "] {
            let frame = d.frame(msg.as_bytes());
            let mut out = Vec::with_capacity(msg.len() + 1024);
            inflate.decompress_vec(&frame, &mut out, FlushDecompress::Sync).unwrap();
            assert_eq!(out, msg.as_bytes(), "each frame must decode fully on its own");
        }
        assert!(d.frame(big.as_bytes()).len() < big.len() / 20, "repeated output should shrink a lot");
    }

    #[cfg(unix)]
    #[test]
    fn shell_output_reaches_history_and_exit_is_seen() {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args(["-c", "echo hello-from-pty"]);
        let shell = Shell::spawn(cmd).unwrap();
        let mut exited = shell.exited.subscribe();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !*exited.borrow_and_update() {
            assert!(Instant::now() < deadline, "shell did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
        let (history, _) = shell.attach();
        assert!(String::from_utf8_lossy(&history).contains("hello-from-pty"));
        assert!(shell.dead.load(Ordering::SeqCst));
    }

    #[test]
    fn cursor_queries_are_left_out_of_the_replay() {
        assert_eq!(&*strip_cursor_queries(b"plain"), b"plain");
        assert_eq!(&*strip_cursor_queries(b"\x1b[6n\x1b[?25lPS> \x1b[6n"), b"\x1b[?25lPS> ");
        assert_eq!(&*strip_cursor_queries(b"\x1b[6"), b"\x1b[6");
    }

    #[test]
    fn session_names_are_cleaned() {
        assert_eq!(clean_name("  сборка\n\t "), Some("сборка".into()));
        assert_eq!(clean_name(" \x1b "), None);
        assert_eq!(clean_name(&"я".repeat(100)).unwrap().chars().count(), MAX_NAME);
    }

    #[test]
    fn sessions_are_limited_and_closed() {
        let term = Terminal::default();
        for _ in 0..MAX_SESSIONS {
            term.create(None).unwrap();
        }
        assert!(term.create(Some("лишний")).is_err());
        let ids: Vec<u32> = term.list().iter().map(|s| s.id).collect();
        assert_eq!(ids, (1..=MAX_SESSIONS as u32).collect::<Vec<_>>());
        assert_eq!(term.list()[0].name, "Терминал 1");
        assert_eq!(term.rename(2, "логи").unwrap().name, "логи");
        assert!(term.rename(2, "  ").is_none());
        assert!(term.close(3));
        assert!(!term.close(3));
        assert!(term.shell(Some(3)).unwrap().is_none());
        assert_eq!(term.create(None).unwrap().id, MAX_SESSIONS as u32 + 1, "ids are never reused");
        term.shutdown();
        assert!(term.list().is_empty());
    }

    #[test]
    fn history_is_capped_at_a_line_start() {
        let mut cmd = CommandBuilder::new(if cfg!(windows) { "cmd.exe" } else { "/bin/sh" });
        cmd.args(if cfg!(windows) { ["/c", "rem"] } else { ["-c", "true"] });
        let shell = Shell::spawn(cmd).unwrap();
        for i in 0..20_000 {
            shell.push_output(format!("line {i}\r\n").as_bytes());
        }
        let (history, _) = shell.attach();
        assert!(history.len() <= HISTORY);
        assert!(history.starts_with(b"line "));
        assert!(history.ends_with(b"line 19999\r\n"));
    }
}
