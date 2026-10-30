//! Claude Code in the Telegram chat. The owner's text messages go to `claude -p` running on
//! this computer; the steps it takes show up in one message that updates as it works, and its
//! answer comes back as a reply.
//!
//! Off unless `"agent": true` in config.json (`--agent on`, only on the computer itself): like
//! the terminal, whoever holds the owner's Telegram can then act on this computer, within what
//! Claude's auto mode allows (it refuses destructive or risky actions by itself).
//!
//! One conversation, resumed for every message (`--resume`) and remembered across restarts in
//! `agent.json` next to the settings. Messages sent while Claude works wait their turn.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, watch};

use crate::agent_text::{describe_tool, markdown_to_html, split_message};
use crate::config::{Config, TelegramConfig};
use crate::telegram;

/// A runaway task is stopped after this.
const MAX_RUN: Duration = Duration::from_secs(60 * 60);
/// Telegram allows 4096 characters; the HTML tags make a converted piece a little longer.
const PIECE: usize = 3500;
/// How often the progress message is updated and "typing…" renewed.
const TICK: Duration = Duration::from_secs(4);
const STEPS_SHOWN: usize = 8;

pub const OFF_TEXT: &str = "Чат с Claude выключен. Включить его можно только на самом компьютере:\nlocal-photo --agent on";

/// Where the answers go, and the one thing not to do from inside the cloud.
const SYSTEM_HINT: &str = "Ты отвечаешь владельцу этого компьютера через Telegram: он пишет с телефона и видит \
только твой итоговый ответ, а каждый твой шаг — одной короткой строкой. Отвечай на его языке, коротко и по делу; \
широкие таблицы на телефоне не читаются — лучше списки. Ты запущен домашним облаком local-photo как его дочерний \
процесс: если остановить или перезапустить облако (local-photo --stop, выкатка нового local-photo.exe), ты \
завершишься на середине работы. Такую выкатку запускай отдельным процессом, который переживёт остановку \
(Start-Process powershell с готовым скриптом), и заранее предупреди владельца.";

#[derive(Serialize, Deserialize, Default, Clone)]
struct State {
    /// Conversation id for `--resume`; None → the next message starts a new conversation.
    session: Option<String>,
    /// Folder Claude works in (`/dir`); None → the home folder.
    cwd: Option<PathBuf>,
}

struct Request {
    text: String,
    reply_to: i64,
    epoch: u64,
}

pub struct Agent {
    tg: TelegramConfig,
    config_file: PathBuf,
    state_file: PathBuf,
    state: Mutex<State>,
    queue: mpsc::UnboundedSender<Request>,
    waiting: AtomicUsize,
    busy: AtomicBool,
    /// Bumped by /stop: the running request is killed and the queued ones are dropped.
    epoch: watch::Sender<u64>,
}

impl Agent {
    pub fn start(tg: TelegramConfig, config_file: PathBuf) -> Arc<Agent> {
        let state_file = config_file.with_file_name("agent.json");
        let state = std::fs::read(&state_file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let (queue, rx) = mpsc::unbounded_channel();
        let agent = Arc::new(Agent {
            tg,
            config_file,
            state_file,
            state: Mutex::new(state),
            queue,
            waiting: AtomicUsize::new(0),
            busy: AtomicBool::new(false),
            epoch: watch::channel(0).0,
        });
        tokio::spawn(agent.clone().work(rx));
        agent
    }

    /// Read on every use, so `--agent off` takes effect without a restart.
    pub fn enabled(&self) -> bool {
        Config::load(&self.config_file).ok().flatten().is_some_and(|c| c.agent)
    }

    /// Queues a message for Claude, and says so if it has to wait.
    pub async fn ask(&self, text: String, reply_to: i64) {
        if !self.enabled() {
            let _ = telegram::reply(&self.tg, reply_to, OFF_TEXT).await;
            return;
        }
        let ahead = self.waiting.fetch_add(1, Ordering::SeqCst) + usize::from(self.busy.load(Ordering::SeqCst));
        let epoch = *self.epoch.borrow();
        let _ = self.queue.send(Request { text, reply_to, epoch });
        if ahead > 0 {
            let text = format!("📥 В очереди: Claude сначала закончит {ahead} предыдущ{}. /stop — прервать всё.", if ahead == 1 { "ее" } else { "их" });
            let _ = telegram::reply(&self.tg, reply_to, &text).await;
        }
    }

    /// Stops what runs and drops what waits. False if there was nothing to stop.
    pub fn stop(&self) -> bool {
        let had = self.busy.load(Ordering::SeqCst) || self.waiting.load(Ordering::SeqCst) > 0;
        self.epoch.send_modify(|e| *e += 1);
        had
    }

    pub fn new_conversation(&self) {
        self.update(|s| s.session = None);
    }

    /// `/dir <folder>`: absolute, `~/…` or relative to the current folder.
    pub fn set_dir(&self, input: &str) -> Result<PathBuf, String> {
        let input = input.trim().trim_matches('"');
        let path = match input.strip_prefix('~') {
            Some(rest) => home().join(rest.trim_start_matches(['/', '\\'])),
            None if Path::new(input).is_absolute() => PathBuf::from(input),
            None => self.dir().join(input),
        };
        if !path.is_dir() {
            return Err(format!("Нет такой папки: {}", path.display()));
        }
        self.update(|s| s.cwd = Some(path.clone()));
        Ok(path)
    }

    pub fn dir(&self) -> PathBuf {
        dir_of(&self.state.lock().unwrap())
    }

    pub fn status(&self) -> String {
        let session = self.state.lock().unwrap().session.clone();
        let now = if self.busy.load(Ordering::SeqCst) { "работает" } else { "свободен" };
        let waiting = self.waiting.load(Ordering::SeqCst);
        format!(
            "Claude в Telegram: {}\nСейчас: {now}{}\nПапка: {}\nРазговор: {}",
            if self.enabled() { "включён" } else { "выключен (на компьютере: local-photo --agent on)" },
            if waiting > 0 { format!(", в очереди {waiting}") } else { String::new() },
            self.dir().display(),
            match session {
                Some(id) => format!("продолжается ({}…), /new — начать новый", id.chars().take(8).collect::<String>()),
                None => "новый".into(),
            }
        )
    }

    fn update(&self, f: impl FnOnce(&mut State)) {
        let mut st = self.state.lock().unwrap();
        f(&mut st);
        if let Ok(bytes) = serde_json::to_vec_pretty(&*st) {
            let _ = std::fs::write(&self.state_file, bytes);
        }
    }

    async fn work(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<Request>) {
        while let Some(req) = rx.recv().await {
            self.busy.store(true, Ordering::SeqCst);
            self.waiting.fetch_sub(1, Ordering::SeqCst);
            if req.epoch == *self.epoch.borrow() {
                if self.enabled() {
                    self.answer(&req).await;
                } else {
                    let _ = telegram::reply(&self.tg, req.reply_to, OFF_TEXT).await;
                }
            }
            self.busy.store(false, Ordering::SeqCst);
        }
    }

    async fn answer(&self, req: &Request) {
        let started = Instant::now();
        crate::log!("Claude в Telegram: запрос ({} симв.)", req.text.chars().count());
        let progress = telegram::send_message(&self.tg, "⏳ Claude думает…", Some(req.reply_to), false).await.ok();
        let resumed = self.state.lock().unwrap().session.is_some();
        let mut run = self.run(req, progress, started).await;
        if resumed && run.lost_session {
            // The conversation is gone (deleted on the computer, say): carry on in a new one.
            crate::log!("Claude в Telegram: прошлый разговор не открылся, начинаю новый");
            self.new_conversation();
            run = self.run(req, progress, started).await;
        }

        let took = fmt_duration(started.elapsed());
        let n = run.steps.len();
        let (head, log) = match &run.end {
            End::Done(_) => (format!("✅ Готово за {took} · шагов: {n}"), format!("готово за {took}, шагов: {n}")),
            End::Failed(_) => (format!("❌ Не получилось ({took})"), format!("ошибка через {took}")),
            End::Stopped => (format!("⏹ Остановлено через {took}"), "остановлено".into()),
            End::TurnedOff => ("⏹ Остановлено: чат с Claude выключили на компьютере".into(), "выключен во время работы".into()),
            End::TimedOut => (format!("⏱ Остановлено: работа шла дольше {}", fmt_duration(MAX_RUN)), "превышено время".into()),
        };
        crate::log!("Claude в Telegram: {log}");
        if let Some(id) = progress {
            if matches!(run.end, End::Done(_)) && run.steps.is_empty() {
                let _ = telegram::delete_message(&self.tg, id).await;
            } else {
                let _ = telegram::edit_message(&self.tg, id, &progress_text(&head, &run.steps)).await;
            }
        }
        match run.end {
            End::Done(answer) => self.send_answer(&answer, req.reply_to).await,
            End::Failed(error) => {
                let _ = telegram::send_message(&self.tg, &format!("❌ {error}"), Some(req.reply_to), false).await;
            }
            _ => {}
        }
    }

    /// Markdown → Telegram HTML, in pieces; a piece Telegram refuses to parse goes as plain text.
    async fn send_answer(&self, answer: &str, reply_to: i64) {
        let pieces = split_message(answer, PIECE);
        if pieces.is_empty() {
            let _ = telegram::send_message(&self.tg, "(Claude ничего не ответил)", Some(reply_to), false).await;
            return;
        }
        for (i, piece) in pieces.iter().enumerate() {
            let to = (i == 0).then_some(reply_to);
            if telegram::send_message(&self.tg, &markdown_to_html(piece), to, true).await.is_err() {
                let _ = telegram::send_message(&self.tg, piece, to, false).await;
            }
        }
    }

    async fn run(&self, req: &Request, progress: Option<i64>, started: Instant) -> Run {
        let mut seen = Seen::default();
        let Some(program) = crate::runner::which("claude") else {
            return Run::failed("Claude Code не найден на этом компьютере. Установите его (https://claude.com/claude-code) и один раз войдите: claude", seen);
        };
        let (session, cwd) = {
            let st = self.state.lock().unwrap();
            (st.session.clone(), dir_of(&st))
        };
        let mut cmd = Command::new(program);
        cmd.args(["-p", "--output-format", "stream-json", "--verbose", "--permission-mode", "auto"])
            .args(["--append-system-prompt", SYSTEM_HINT]);
        if let Some(id) = &session {
            cmd.args(["--resume", id]);
        }
        cmd.current_dir(&cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        // A cloud rolled out by an agent inherits its variables; Claude would think it is a sub-agent.
        for var in crate::terminal_store::inherited_agent_vars() {
            cmd.env_remove(var);
        }
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        #[cfg(unix)]
        cmd.process_group(0); // so kill_tree gets what it started too
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return Run::failed(format!("не удалось запустить Claude: {e}"), seen),
        };
        // The prompt goes through stdin: no command-line length limit and no quoting.
        if let Some(mut stdin) = child.stdin.take() {
            let text = req.text.clone();
            tokio::spawn(async move {
                let _ = stdin.write_all(text.as_bytes()).await;
            });
        }
        let stderr = tokio::spawn(read_tail(child.stderr.take()));
        let Some(stdout) = child.stdout.take() else {
            return Run::failed("нет вывода Claude", seen);
        };
        let mut lines = BufReader::new(stdout).lines();
        let mut epoch = self.epoch.subscribe();
        let deadline = tokio::time::sleep(MAX_RUN.saturating_sub(started.elapsed()));
        tokio::pin!(deadline);
        let mut tick = tokio::time::interval(TICK);
        let mut shown = String::new();

        let interrupted = loop {
            tokio::select! {
                line = lines.next_line() => match line {
                    Ok(Some(line)) => {
                        seen.take(&line);
                        if let Some(id) = seen.new_session.take() {
                            // Saved right away: a stop or crash mid-way still resumes this conversation.
                            self.update(|s| s.session = Some(id));
                        }
                    }
                    _ => break None,
                },
                // The borrow `wait_for` returns can't cross an await: dropped inside.
                _ = async { drop(epoch.wait_for(|e| *e != req.epoch).await) } => break Some(End::Stopped),
                _ = &mut deadline => break Some(End::TimedOut),
                _ = tick.tick() => {
                    if !self.enabled() {
                        break Some(End::TurnedOff);
                    }
                    telegram::typing(&self.tg).await;
                    if let Some(id) = progress {
                        let text = progress_text(&format!("⏳ Claude работает · {}", fmt_duration(started.elapsed())), &seen.steps);
                        if text != shown {
                            let _ = telegram::edit_message(&self.tg, id, &text).await;
                            shown = text;
                        }
                    }
                }
            }
        };

        if interrupted.is_some() {
            crate::runner::kill_tree(&mut child).await;
        } else if tokio::time::timeout(Duration::from_secs(10), child.wait()).await.is_err() {
            // Output closed but the process hangs on: nothing more will come from it.
            crate::runner::kill_tree(&mut child).await;
        }
        let code = child.wait().await.ok().and_then(|s| s.code());
        let stderr = stderr.await.unwrap_or_default();
        let lost_session = session.is_some() && !seen.started && seen.result.is_none();
        let end = match (interrupted, seen.result.take()) {
            (Some(end), _) => end,
            (None, Some((false, text))) => End::Done(if text.trim().is_empty() { std::mem::take(&mut seen.last_text) } else { text }),
            (None, Some((true, text))) => End::Failed(explain(&text)),
            (None, None) if !stderr.is_empty() => End::Failed(explain(&stderr)),
            (None, None) => End::Failed(format!("Claude завершился без ответа (код {})", code.map_or("?".into(), |c| c.to_string()))),
        };
        Run { end, steps: seen.steps, lost_session }
    }
}

#[derive(Debug)]
enum End {
    Done(String),
    Failed(String),
    Stopped,
    TurnedOff,
    TimedOut,
}

struct Run {
    end: End,
    steps: Vec<String>,
    /// `--resume` was given but Claude never started: the conversation no longer exists.
    lost_session: bool,
}

impl Run {
    fn failed(error: impl Into<String>, seen: Seen) -> Run {
        Run { end: End::Failed(error.into()), steps: seen.steps, lost_session: false }
    }
}

/// What came out of Claude's `stream-json` output so far.
#[derive(Default)]
struct Seen {
    steps: Vec<String>,
    /// The `init` event arrived: the conversation opened.
    started: bool,
    new_session: Option<String>,
    /// The final `result` event: (is an error, text).
    result: Option<(bool, String)>,
    /// Latest text block, in case the result comes without one.
    last_text: String,
}

impl Seen {
    fn take(&mut self, line: &str) {
        let Ok(ev) = serde_json::from_str::<Value>(line) else { return };
        let session = ev["session_id"].as_str().map(str::to_string);
        match ev["type"].as_str() {
            Some("system") if ev["subtype"] == "init" => {
                self.started = true;
                self.new_session = session;
            }
            Some("assistant") => {
                for block in ev["message"]["content"].as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("tool_use") => self.steps.push(describe_tool(block["name"].as_str().unwrap_or("?"), &block["input"])),
                        Some("text") => {
                            if let Some(t) = block["text"].as_str().filter(|t| !t.trim().is_empty()) {
                                self.last_text = t.to_string();
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("result") => {
                let is_error = ev["is_error"].as_bool().unwrap_or(false) || ev["subtype"].as_str().is_some_and(|s| s != "success");
                let text = ev["result"].as_str().unwrap_or_default().to_string();
                self.result = Some((is_error, text));
                if session.is_some() {
                    self.new_session = session;
                }
            }
            _ => {}
        }
    }
}

fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn dir_of(st: &State) -> PathBuf {
    st.cwd.clone().filter(|d| d.is_dir()).unwrap_or_else(home)
}

/// The last couple of kilobytes of stderr: enough to explain a failure, bounded however much it prints.
async fn read_tail(stderr: Option<tokio::process::ChildStderr>) -> String {
    const KEEP: usize = 4096;
    let Some(mut stderr) = stderr else { return String::new() };
    let mut tail = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = stderr.read(&mut buf).await {
        if n == 0 {
            break;
        }
        tail.extend_from_slice(&buf[..n]);
        if tail.len() > KEEP {
            tail.drain(..tail.len() - KEEP);
        }
    }
    String::from_utf8_lossy(&tail).trim().to_string()
}

/// Adds what to do about the failures that need the owner at the computer.
fn explain(error: &str) -> String {
    let error: String = error.trim().chars().take(1500).collect();
    let lower = error.to_lowercase();
    if lower.contains("/login") || lower.contains("api key") || lower.contains("not logged in") {
        format!("{error}\n\nВойдите в Claude на компьютере: запустите claude и выполните /login.")
    } else {
        error
    }
}

fn progress_text(head: &str, steps: &[String]) -> String {
    let mut text = head.to_string();
    let skipped = steps.len().saturating_sub(STEPS_SHOWN);
    if skipped > 0 {
        text.push_str(&format!("\n…ещё шагов: {skipped}"));
    }
    for step in &steps[skipped..] {
        text.push('\n');
        text.push_str(step);
    }
    text
}

fn fmt_duration(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s} с"),
        60..3600 => format!("{} мин {} с", s / 60, s % 60),
        _ => format!("{} ч {} мин", s / 3600, s % 3600 / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_events_are_read() {
        let mut seen = Seen::default();
        seen.take(r#"{"type":"system","subtype":"init","session_id":"abc-1","tools":[]}"#);
        assert!(seen.started);
        assert_eq!(seen.new_session.take().as_deref(), Some("abc-1"));
        seen.take(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Смотрю"},{"type":"tool_use","name":"Read","input":{"file_path":"C:\\a\\main.rs"}}]}}"#);
        assert_eq!(seen.last_text, "Смотрю");
        assert_eq!(seen.steps.len(), 1);
        seen.take("not json");
        seen.take(r#"{"type":"rate_limit_event"}"#);
        seen.take(r#"{"type":"result","subtype":"success","is_error":false,"result":"Готово","session_id":"abc-2"}"#);
        assert_eq!(seen.result, Some((false, "Готово".into())));
        assert_eq!(seen.new_session.as_deref(), Some("abc-2"));

        let mut failed = Seen::default();
        failed.take(r#"{"type":"result","subtype":"error_max_turns","is_error":false,"result":""}"#);
        assert_eq!(failed.result.map(|r| r.0), Some(true));
    }

    #[test]
    fn progress_shows_the_last_steps() {
        let steps: Vec<String> = (1..=10).map(|i| format!("шаг {i}")).collect();
        let text = progress_text("⏳", &steps);
        assert!(text.starts_with("⏳\n…ещё шагов: 2\nшаг 3"));
        assert!(text.ends_with("шаг 10"));
        assert_eq!(progress_text("✅", &[]), "✅");
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(fmt_duration(Duration::from_secs(5)), "5 с");
        assert_eq!(fmt_duration(Duration::from_secs(65)), "1 мин 5 с");
        assert_eq!(fmt_duration(Duration::from_secs(3720)), "1 ч 2 мин");
    }

    /// Runs the real `claude` (costs a little of the owner's usage): `cargo test -- --ignored real_claude`.
    /// Telegram calls go to a fake bot and fail quietly.
    #[tokio::test]
    #[ignore]
    async fn real_claude_answers_resumes_and_stops() {
        let dir = std::env::temp_dir().join(format!("lp-agent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.json");
        std::fs::write(&config, r#"{"username":"t","password_hash":"x","agent":true}"#).unwrap();
        let agent = Agent::start(TelegramConfig { token: "0:fake".into(), chat_id: 1 }, config);
        agent.set_dir(dir.to_str().unwrap()).unwrap();
        let req = |text: &str| Request { text: text.into(), reply_to: 0, epoch: *agent.epoch.borrow() };

        let first = agent.run(&req("Запомни кодовое слово «пеликан». Ответь одним словом: «запомнил»."), None, Instant::now()).await;
        let End::Done(answer) = &first.end else { panic!("first run failed: {:?}", first.end) };
        assert!(answer.to_lowercase().contains("запомнил"), "{answer}");
        assert!(agent.state.lock().unwrap().session.is_some());

        let second = agent.run(&req("Какое кодовое слово я просил запомнить? Ответь одним словом."), None, Instant::now()).await;
        let End::Done(answer) = &second.end else { panic!("resume failed: {:?}", second.end) };
        assert!(answer.to_lowercase().contains("пеликан"), "{answer}");

        let stopper = agent.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(8)).await;
            stopper.stop();
        });
        let t = Instant::now();
        let third = agent.run(&req("Выполни в PowerShell команду Start-Sleep -Seconds 120 и потом скажи «проснулся»."), None, Instant::now()).await;
        assert!(matches!(third.end, End::Stopped), "{:?}", third.end);
        assert!(t.elapsed() < Duration::from_secs(30));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn login_failures_get_a_hint() {
        assert!(explain("Invalid API key · Please run /login").contains("выполните /login"));
        assert_eq!(explain("  что-то ещё  "), "что-то ещё");
    }
}
