//! Code runner: compiles and runs one small source file with the compilers and interpreters
//! installed on this computer, like an online compiler. Off unless `"runner": true` in
//! config.json (`--runner on`): like the terminal, it runs anything on this computer.
//!
//! Each run gets a fresh temporary folder, a time limit and an output limit; the whole
//! process tree is killed on timeout, on "Стоп" or when the page goes away. Output streams
//! to the page as it is printed (NDJSON events, see [`Event`]).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Semaphore};

pub const MAX_SOURCE: usize = 256 * 1024;
pub const MAX_STDIN: usize = 64 * 1024;
const MAX_OUTPUT: usize = 1024 * 1024;
const COMPILE_TIMEOUT: Duration = Duration::from_secs(60);
const RUN_TIMEOUT: Duration = Duration::from_secs(15);
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
/// Probing runs every candidate's `--version`; once in a while is enough.
const PROBE_CACHE: Duration = Duration::from_secs(300);
/// Two devices may run at once; more would mostly be a mistake (or abuse).
const PARALLEL_RUNS: usize = 2;

/// One way to run a language. Arguments use `{tool}` (the found program), `{src}` (source
/// file name) and `{exe}` (absolute path of the compiled program).
struct Toolchain {
    tool: &'static str,
    probe: &'static [&'static str],
    compile: Option<&'static [&'static str]>,
    run: &'static [&'static str],
}

struct Lang {
    id: &'static str,
    name: &'static str,
    file: &'static str,
    /// PowerShell 5.1 reads BOM-less files as ANSI and breaks Cyrillic.
    bom: bool,
    hint: &'static str,
    toolchains: &'static [Toolchain],
}

const V: &[&str] = &["--version"];

const LANGS: &[Lang] = &[
    Lang {
        id: "python", name: "Python", file: "main.py", bom: false, hint: "Установите Python с python.org (галочка «Add to PATH»)",
        toolchains: &[
            Toolchain { tool: "python3", probe: V, compile: None, run: &["{tool}", "-X", "utf8", "-u", "{src}"] },
            Toolchain { tool: "python", probe: V, compile: None, run: &["{tool}", "-X", "utf8", "-u", "{src}"] },
            Toolchain { tool: "py", probe: V, compile: None, run: &["{tool}", "-X", "utf8", "-u", "{src}"] },
        ],
    },
    Lang {
        id: "javascript", name: "JavaScript (Node.js)", file: "main.js", bom: false, hint: "Установите Node.js с nodejs.org",
        toolchains: &[Toolchain { tool: "node", probe: V, compile: None, run: &["{tool}", "{src}"] }],
    },
    Lang {
        id: "typescript", name: "TypeScript (Node.js)", file: "main.ts", bom: false, hint: "Нужен Node.js 23 или новее (nodejs.org)",
        toolchains: &[Toolchain { tool: "node", probe: V, compile: None, run: &["{tool}", "--experimental-strip-types", "--no-warnings", "{src}"] }],
    },
    Lang {
        id: "c", name: "C", file: "main.c", bom: false, hint: "Установите компилятор: MSYS2 (gcc) или LLVM (clang)",
        toolchains: &[
            Toolchain { tool: "gcc", probe: V, compile: Some(&["{tool}", "{src}", "-O2", "-o", "{exe}"]), run: &["{exe}"] },
            Toolchain { tool: "clang", probe: V, compile: Some(&["{tool}", "{src}", "-O2", "-o", "{exe}"]), run: &["{exe}"] },
        ],
    },
    Lang {
        id: "cpp", name: "C++", file: "main.cpp", bom: false, hint: "Установите компилятор: MSYS2 (g++) или LLVM (clang++)",
        toolchains: &[
            Toolchain { tool: "g++", probe: V, compile: Some(&["{tool}", "{src}", "-O2", "-std=c++20", "-o", "{exe}"]), run: &["{exe}"] },
            Toolchain { tool: "clang++", probe: V, compile: Some(&["{tool}", "{src}", "-O2", "-std=c++20", "-o", "{exe}"]), run: &["{exe}"] },
        ],
    },
    Lang {
        id: "csharp", name: "C#", file: "main.cs", bom: false, hint: "Нужен .NET 10 SDK (dotnet.microsoft.com): он запускает один файл .cs",
        toolchains: &[Toolchain { tool: "dotnet", probe: V, compile: None, run: &["{tool}", "run", "{src}"] }],
    },
    Lang {
        id: "java", name: "Java", file: "Main.java", bom: false, hint: "Установите JDK 11 или новее (adoptium.net)",
        toolchains: &[Toolchain {
            tool: "java", probe: &["-version"], compile: None,
            run: &["{tool}", "-Dfile.encoding=UTF-8", "-Dsun.stdout.encoding=UTF-8", "-Dsun.stderr.encoding=UTF-8", "-Dstdout.encoding=UTF-8", "-Dstderr.encoding=UTF-8", "{src}"],
        }],
    },
    Lang {
        id: "go", name: "Go", file: "main.go", bom: false, hint: "Установите Go с go.dev",
        toolchains: &[Toolchain { tool: "go", probe: &["version"], compile: Some(&["{tool}", "build", "-o", "{exe}", "{src}"]), run: &["{exe}"] }],
    },
    Lang {
        id: "rust", name: "Rust", file: "main.rs", bom: false, hint: "Установите Rust с rustup.rs",
        toolchains: &[Toolchain { tool: "rustc", probe: V, compile: Some(&["{tool}", "--edition", "2021", "{src}", "-o", "{exe}"]), run: &["{exe}"] }],
    },
    Lang {
        id: "php", name: "PHP", file: "main.php", bom: false, hint: "Установите PHP (php.net) и добавьте в PATH",
        toolchains: &[Toolchain { tool: "php", probe: V, compile: None, run: &["{tool}", "{src}"] }],
    },
    Lang {
        id: "ruby", name: "Ruby", file: "main.rb", bom: false, hint: "Установите Ruby (rubyinstaller.org)",
        toolchains: &[Toolchain { tool: "ruby", probe: V, compile: None, run: &["{tool}", "{src}"] }],
    },
    Lang {
        id: "lua", name: "Lua", file: "main.lua", bom: false, hint: "Установите Lua и добавьте в PATH",
        toolchains: &[
            Toolchain { tool: "lua", probe: &["-v"], compile: None, run: &["{tool}", "{src}"] },
            Toolchain { tool: "luajit", probe: &["-v"], compile: None, run: &["{tool}", "{src}"] },
        ],
    },
    Lang {
        id: "powershell", name: "PowerShell", file: "main.ps1", bom: true, hint: "PowerShell есть в Windows; на Mac/Linux — pwsh",
        toolchains: &[
            Toolchain { tool: "pwsh", probe: V, compile: None, run: PS_RUN },
            Toolchain { tool: "powershell", probe: &["-NoProfile", "-Command", "$PSVersionTable.PSVersion.ToString()"], compile: None, run: PS_RUN },
        ],
    },
    Lang {
        id: "bash", name: "Bash", file: "main.sh", bom: false, hint: "На Windows: Git Bash или WSL",
        toolchains: &[Toolchain { tool: "bash", probe: V, compile: None, run: &["{tool}", "{src}"] }],
    },
];

/// Output as UTF-8 whatever the console code page, then the script itself.
const PS_RUN: &[&str] = &[
    "{tool}", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command",
    "[Console]::OutputEncoding = [Text.Encoding]::UTF8; [Console]::InputEncoding = [Text.Encoding]::UTF8; $OutputEncoding = [Text.Encoding]::UTF8; & ./main.ps1; exit $LASTEXITCODE",
];

#[derive(Serialize, Clone)]
pub struct LangInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub file: &'static str,
    pub available: bool,
    /// First line of the tool's version, when found.
    pub version: Option<String>,
    pub hint: &'static str,
    #[serde(skip)]
    toolchain: Option<usize>,
}

/// What the page receives, one JSON object per line.
#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Event {
    /// "compile" or "run": a new step starts.
    Phase { v: &'static str },
    Out { v: String },
    Err { v: String },
    Exit {
        code: Option<i32>,
        ms: u64,
        /// None = finished by itself; "timeout", "limit" (too much output), "compile" (didn't compile), "error".
        reason: Option<&'static str>,
    },
}

pub struct Runner {
    slots: Semaphore,
    probed: Mutex<Option<(Instant, Vec<LangInfo>)>>,
}

impl Default for Runner {
    fn default() -> Self {
        Runner { slots: Semaphore::new(PARALLEL_RUNS), probed: Mutex::new(None) }
    }
}

impl Runner {
    /// Every language with what was found for it; cached for a few minutes.
    pub async fn languages(&self, refresh: bool) -> Vec<LangInfo> {
        if !refresh {
            if let Some((at, list)) = self.probed.lock().unwrap().as_ref() {
                if at.elapsed() < PROBE_CACHE {
                    return list.clone();
                }
            }
        }
        // All at once: a missing tool can take seconds to fail, and there are a dozen of them.
        let list = futures_util::future::join_all(LANGS.iter().map(|lang| async move {
            let mut info = LangInfo { id: lang.id, name: lang.name, file: lang.file, available: false, version: None, hint: lang.hint, toolchain: None };
            for (i, tc) in lang.toolchains.iter().enumerate() {
                if let Some(version) = probe(tc).await {
                    if lang.id == "csharp" && !dotnet_runs_files(&version) {
                        info.version = Some(format!("{version} — нужен .NET 10"));
                        break;
                    }
                    (info.available, info.version, info.toolchain) = (true, Some(version), Some(i));
                    break;
                }
            }
            info
        }))
        .await;
        *self.probed.lock().unwrap() = Some((Instant::now(), list.clone()));
        list
    }

    /// A free slot, or None when two runs are already going.
    pub fn try_slot(&self) -> Option<tokio::sync::SemaphorePermit<'_>> {
        self.slots.try_acquire().ok()
    }

    /// Compiles (if needed) and runs `code`; events go to `tx`. Ends early when `tx` is closed.
    pub async fn run(&self, lang_id: &str, code: &str, stdin: &str, tx: mpsc::Sender<Event>) {
        let started = Instant::now();
        let exit = |code, reason| Event::Exit { code, ms: started.elapsed().as_millis() as u64, reason };
        let langs = self.languages(false).await;
        let (Some(lang), Some(info)) = (LANGS.iter().find(|l| l.id == lang_id), langs.iter().find(|l| l.id == lang_id)) else {
            let _ = tx.send(Event::Err { v: "Неизвестный язык\n".into() }).await;
            let _ = tx.send(exit(None, Some("error"))).await;
            return;
        };
        let Some(tc) = info.toolchain.map(|i| &lang.toolchains[i]) else {
            let _ = tx.send(Event::Err { v: format!("{} не найден на компьютере. {}\n", lang.name, lang.hint) }).await;
            let _ = tx.send(exit(None, Some("error"))).await;
            return;
        };
        let dir = match work_dir() {
            Ok(d) => d,
            Err(e) => {
                let _ = tx.send(Event::Err { v: format!("Не удалось создать временную папку: {e}\n") }).await;
                let _ = tx.send(exit(None, Some("error"))).await;
                return;
            }
        };
        let mut source = Vec::with_capacity(code.len() + 3);
        if lang.bom {
            source.extend_from_slice(b"\xef\xbb\xbf");
        }
        source.extend_from_slice(code.as_bytes());
        let result: Result<(Option<i32>, Option<&str>), String> = async {
            std::fs::write(dir.join(lang.file), &source).map_err(|e| format!("Не удалось записать файл: {e}"))?;
            let exe = dir.join(if cfg!(windows) { "main.exe" } else { "main" });
            let mut budget = MAX_OUTPUT;
            if let Some(compile) = tc.compile {
                let _ = tx.send(Event::Phase { v: "compile" }).await;
                let step = step(&argv(compile, tc, lang, &exe), &dir, None, COMPILE_TIMEOUT, &tx, &mut budget).await;
                if step.reason.is_some() || step.code != Some(0) {
                    return Ok((step.code, Some(step.reason.unwrap_or("compile"))));
                }
            }
            let _ = tx.send(Event::Phase { v: "run" }).await;
            let step = step(&argv(tc.run, tc, lang, &exe), &dir, Some(stdin.as_bytes().to_vec()), RUN_TIMEOUT, &tx, &mut budget).await;
            Ok((step.code, step.reason))
        }
        .await;
        let _ = tokio::task::spawn_blocking({
            let dir = dir.clone();
            move || std::fs::remove_dir_all(dir)
        })
        .await;
        match result {
            Ok((code, reason)) => {
                let _ = tx.send(exit(code, reason)).await;
            }
            Err(e) => {
                let _ = tx.send(Event::Err { v: format!("{e}\n") }).await;
                let _ = tx.send(exit(None, Some("error"))).await;
            }
        }
    }
}

/// .NET runs a single .cs file since version 10.
fn dotnet_runs_files(version: &str) -> bool {
    version.split('.').next().and_then(|m| m.trim().parse::<u32>().ok()).is_some_and(|major| major >= 10)
}

fn argv(template: &[&str], tc: &Toolchain, lang: &Lang, exe: &Path) -> Vec<String> {
    template
        .iter()
        .map(|a| a.replace("{tool}", tc.tool).replace("{src}", lang.file).replace("{exe}", &exe.to_string_lossy()))
        .collect()
}

fn work_dir() -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("local-photo-run").join(crate::auth::random_hex(8));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Full path of a program found in PATH. Done here, not by std, for two Windows cases met on
/// the owner's computer: a stray `"` in PATH makes std's parser glue every later folder into
/// one, and the Microsoft Store python.exe is an "app execution alias" that file checks
/// can't open (it is found by listing its folder instead).
pub(crate) fn which(program: &str) -> Option<PathBuf> {
    let path = Path::new(program);
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }
    let path_var = std::env::var_os("PATH")?;
    let path_var = path_var.to_string_lossy();
    let dirs = path_var.split(if cfg!(windows) { ';' } else { ':' }).map(|d| d.trim().trim_matches('"')).filter(|d| !d.is_empty());
    if cfg!(windows) {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        let names: Vec<String> = pathext.split(';').filter(|e| !e.is_empty()).map(|e| format!("{program}{e}").to_lowercase()).collect();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            let found: Vec<String> = entries.flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).filter(|n| names.contains(n)).collect();
            // In PATHEXT order, as cmd.exe does: python.exe before python.bat.
            if let Some(name) = names.iter().find(|n| found.contains(n)) {
                return Some(Path::new(dir).join(name));
            }
        }
        None
    } else {
        dirs.map(|d| Path::new(d).join(program)).find(|p| p.is_file())
    }
}

fn command(argv: &[String], dir: &Path) -> Command {
    let program = which(&argv[0]).unwrap_or_else(|| PathBuf::from(&argv[0]));
    let mut cmd = Command::new(program);
    cmd.args(&argv[1..])
        .current_dir(dir)
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .kill_on_drop(true);
    for var in crate::terminal_store::inherited_agent_vars() {
        cmd.env_remove(var);
    }
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    #[cfg(unix)]
    cmd.process_group(0); // so the whole group can be killed
    cmd
}

/// First line of `--version` (stderr for java), if the tool runs.
async fn probe(tc: &Toolchain) -> Option<String> {
    let mut args = vec![tc.tool.to_string()];
    args.extend(tc.probe.iter().map(|s| s.to_string()));
    let mut cmd = command(&args, &std::env::temp_dir());
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = tokio::time::timeout(PROBE_TIMEOUT, cmd.output()).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    let text = if out.stdout.iter().any(|b| !b.is_ascii_whitespace()) { out.stdout } else { out.stderr };
    let line = String::from_utf8_lossy(&text).lines().find(|l| !l.trim().is_empty())?.trim().to_string();
    // The Windows Store "python" placeholder prints nothing useful.
    (!line.is_empty()).then(|| line.chars().take(80).collect())
}

struct StepResult {
    code: Option<i32>,
    reason: Option<&'static str>,
}

/// Runs one process to the end, streaming its output, within `timeout` and the output budget.
async fn step(argv: &[String], dir: &Path, stdin: Option<Vec<u8>>, timeout: Duration, tx: &mpsc::Sender<Event>, budget: &mut usize) -> StepResult {
    let mut cmd = command(argv, dir);
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Event::Err { v: format!("Не удалось запустить {}: {e}\n", argv[0]) }).await;
            return StepResult { code: None, reason: Some("error") };
        }
    };
    if let (Some(mut pipe), Some(data)) = (child.stdin.take(), stdin) {
        // A program that never reads its input must not block us: write in the background.
        tokio::spawn(async move {
            let _ = pipe.write_all(&data).await;
        });
    }
    let (mut out, mut err) = (child.stdout.take().expect("piped"), child.stderr.take().expect("piped"));
    let (mut out_buf, mut err_buf) = ([0u8; 8192], [0u8; 8192]);
    let (mut out_text, mut err_text) = (Utf8Stream::default(), Utf8Stream::default());
    let (mut out_open, mut err_open) = (true, true);
    let deadline = tokio::time::Instant::now() + timeout;
    let mut reason = None;
    while out_open || err_open {
        tokio::select! {
            n = out.read(&mut out_buf), if out_open => match n {
                Ok(0) | Err(_) => out_open = false,
                Ok(n) => {
                    if !forward(&mut out_text, &out_buf[..n], false, tx, budget).await {
                        reason = Some("limit");
                        break;
                    }
                }
            },
            n = err.read(&mut err_buf), if err_open => match n {
                Ok(0) | Err(_) => err_open = false,
                Ok(n) => {
                    if !forward(&mut err_text, &err_buf[..n], true, tx, budget).await {
                        reason = Some("limit");
                        break;
                    }
                }
            },
            _ = tokio::time::sleep_until(deadline) => {
                reason = Some("timeout");
                break;
            }
            // The page was closed or "Стоп" pressed: nobody reads any more.
            _ = tx.closed() => {
                reason = Some("stopped");
                break;
            }
        }
    }
    if reason.is_some() {
        kill_tree(&mut child).await;
    }
    for (text, is_err) in [(out_text.finish(), false), (err_text.finish(), true)] {
        if !text.is_empty() {
            let _ = tx.send(if is_err { Event::Err { v: text } } else { Event::Out { v: text } }).await;
        }
    }
    let code = match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
        Ok(Ok(status)) => status.code(),
        _ => None,
    };
    StepResult { code, reason }
}

/// Sends a chunk; false once the output budget is spent.
async fn forward(stream: &mut Utf8Stream, bytes: &[u8], is_err: bool, tx: &mpsc::Sender<Event>, budget: &mut usize) -> bool {
    let over = bytes.len() > *budget;
    let take = bytes.len().min(*budget);
    *budget -= take;
    let text = stream.push(&bytes[..take]);
    if !text.is_empty() {
        let _ = tx.send(if is_err { Event::Err { v: text } } else { Event::Out { v: text } }).await;
    }
    if over {
        let _ = tx.send(Event::Err { v: format!("\n[вывод больше {} МБ — программа остановлена]\n", MAX_OUTPUT >> 20) }).await;
    }
    !over
}

/// Kills the process and everything it started (`go run`, `dotnet run` start children).
pub(crate) async fn kill_tree(child: &mut Child) {
    if let Some(pid) = child.id() {
        #[cfg(windows)]
        {
            let mut kill = Command::new("taskkill");
            kill.args(["/PID", &pid.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null());
            kill.creation_flags(0x0800_0000);
            let _ = kill.status().await;
        }
        #[cfg(unix)]
        {
            let _ = Command::new("kill").args(["-9", "--", &format!("-{pid}")]).stdout(Stdio::null()).stderr(Stdio::null()).status().await;
        }
    }
    let _ = child.start_kill();
}

/// Turns byte chunks into text without breaking a character split between two chunks.
#[derive(Default)]
struct Utf8Stream {
    pending: Vec<u8>,
}

impl Utf8Stream {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&self.pending) {
            Ok(_) => self.pending.len(),
            // An incomplete character at the end waits for the next chunk; real garbage doesn't.
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(_) => self.pending.len(),
        };
        let rest = self.pending.split_off(valid);
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending = rest;
        text
    }

    fn finish(&mut self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut self.pending)).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_split_between_chunks() {
        let mut s = Utf8Stream::default();
        let bytes = "Привет".as_bytes();
        assert_eq!(s.push(&bytes[..3]), "П");
        assert_eq!(s.push(&bytes[3..]), "ривет");
        assert_eq!(s.push(&[0xff, b'a']), "\u{fffd}a");
        assert_eq!(s.push(&[0xd0]), "");
        assert_eq!(s.finish(), "\u{fffd}");
    }

    #[test]
    fn dotnet_version_check() {
        assert!(dotnet_runs_files("10.0.100"));
        assert!(!dotnet_runs_files("8.0.403"));
        assert!(!dotnet_runs_files("garbage"));
    }

    #[test]
    fn every_language_has_a_file_and_toolchain() {
        for l in LANGS {
            assert!(!l.toolchains.is_empty() && l.file.contains('.'), "{}", l.id);
            for tc in l.toolchains {
                assert!(tc.run.iter().any(|a| a.contains("{src}") || a.contains("{exe}") || a.contains("main.ps1")), "{}", l.id);
            }
        }
    }

    /// Uses whatever is installed; checks streaming, stdin, exit codes and the time limit.
    #[tokio::test]
    async fn runs_a_script_end_to_end() {
        let runner = Runner::default();
        let langs = runner.languages(true).await;
        let Some(lang) = ["python", "powershell", "bash"].into_iter().find(|id| langs.iter().any(|l| l.id == *id && l.available)) else {
            return; // nothing to run here
        };
        let (code, sleep) = match lang {
            "python" => ("import sys\nprint('Привет', sys.stdin.readline().strip())\nsys.exit(3)\n", "import time\ntime.sleep(60)\n"),
            "powershell" => ("Write-Output ('Привет ' + [Console]::In.ReadLine())\nexit 3\n", "Start-Sleep 60\n"),
            _ => ("read x; echo \"Привет $x\"; exit 3\n", "sleep 60\n"),
        };
        let collect = |code: &'static str| {
            let runner = &runner;
            async move {
                let (tx, mut rx) = mpsc::channel(64);
                let run = runner.run(lang, code, "мир\n", tx);
                let mut events = Vec::new();
                let gather = async {
                    while let Some(e) = rx.recv().await {
                        events.push(e);
                    }
                };
                tokio::join!(run, gather);
                events
            }
        };
        let events = collect(code).await;
        let out: String = events.iter().filter_map(|e| if let Event::Out { v } = e { Some(v.as_str()) } else { None }).collect();
        assert!(out.contains("Привет мир"), "{lang}: {events:?}");
        assert!(matches!(events.last(), Some(Event::Exit { code: Some(3), reason: None, .. })), "{lang}: {events:?}");

        // Dropping the receiver ("Стоп") ends a program that would sleep for a minute.
        let (tx, rx) = mpsc::channel(64);
        drop(rx);
        let t = Instant::now();
        runner.run(lang, sleep, "", tx).await;
        assert!(t.elapsed() < Duration::from_secs(20), "{lang}: stop took {:?}", t.elapsed());
    }
}


