//! Terminal sessions survive a restart of the cloud or of the computer.
//!
//! Every few seconds the open sessions are written to `<settings>/terminal/`: names, the
//! folder each shell is in, the recent screen output, and whether Claude Code runs in it.
//! On the next start the shells come back in the same folders with their old output, and a
//! session that had `claude` running gets `claude --resume <conversation>` typed into it.
//!
//! The folder of a PowerShell can't be read from outside (`cd` doesn't change the process's
//! own directory), so its prompt reports it with an OSC 7 sequence ([`PS_HOOK`]); on
//! macOS/Linux the shell's real working directory is used.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const STATE_FILE: &str = "sessions.json";

#[derive(Serialize, Deserialize, Default)]
pub struct Saved {
    pub last_id: u32,
    pub sessions: Vec<SavedSession>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct SavedSession {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub claude: Option<ClaudeRun>,
}

/// Claude Code running in a session, enough to start it again.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct ClaudeRun {
    /// Its options worth keeping (model, permission mode…), see [`keep_args`].
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Conversation id (`~/.claude/projects/<folder>/<id>.jsonl`); None → nothing was said yet.
    pub conversation: Option<String>,
    /// Process start time (s): the same process keeps the conversation it was matched with.
    pub started: u64,
}

pub fn load(dir: &Path) -> Saved {
    std::fs::read(dir.join(STATE_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(dir: &Path, saved: &Saved) {
    let _ = std::fs::create_dir_all(dir);
    let tmp = dir.join(format!("{STATE_FILE}.tmp"));
    if let Ok(bytes) = serde_json::to_vec_pretty(saved) {
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, dir.join(STATE_FILE));
        }
    }
    // Output of sessions closed since.
    let keep: Vec<String> = saved.sessions.iter().map(|s| output_name(s.id)).collect();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".out") && !keep.contains(&name) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn output_name(id: u32) -> String {
    format!("{id}.out")
}

pub fn save_output(dir: &Path, id: u32, bytes: &[u8]) {
    let path = dir.join(output_name(id));
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

pub fn load_output(dir: &Path, id: u32) -> Vec<u8> {
    std::fs::read(dir.join(output_name(id))).unwrap_or_default()
}

/// Everything saved goes (the terminal was turned off: nothing should come back).
pub fn forget(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

// ---------- a clean environment ----------

/// Variables a Claude Code session sets for the programs it starts. A cloud started from
/// such a session (an update rolled out by an agent) passes them on to its shells, and a
/// `claude` run there then thinks it is a sub-agent: it doesn't save its conversation, so it
/// could never be resumed. Shells and programs started by the cloud drop them.
pub fn inherited_agent_vars() -> Vec<std::ffi::OsString> {
    let from_agent = std::env::var_os("CLAUDECODE").is_some() || std::env::var_os("CLAUDE_CODE_ENTRYPOINT").is_some();
    std::env::vars_os()
        .map(|(k, _)| k)
        .filter(|k| {
            let k = k.to_string_lossy();
            k.starts_with("CLAUDE_CODE_")
                || k.starts_with("CLAUDE_AGENT_SDK")
                || ["CLAUDECODE", "CLAUDE_PID", "CLAUDE_EFFORT", "AI_AGENT"].contains(&k.as_ref())
                || k.starts_with("CLAUDE_PREVIEW_")
                // Points at the agent's own proxy; the user's own setting would come with no agent around.
                || (from_agent && k == "ANTHROPIC_BASE_URL")
        })
        .collect()
}

// ---------- the PowerShell prompt reports its folder ----------

/// Wraps whatever prompt the profile set up; xterm.js ignores the OSC 7 it prints.
const PS_HOOK: &str = "$global:__lpPrompt = $function:prompt; \
    function global:prompt { [Console]::Write([char]27 + ']7;' + $PWD.ProviderPath + [char]7); & $global:__lpPrompt }";

/// The `cmds` function: the cheat sheet of [`COMMANDS`] in the shell.
const PS_CMDS: &str = include_str!("cmds.ps1");

/// `-EncodedCommand` argument for [`PS_HOOK`] and [`PS_CMDS`]: base64 of UTF-16LE, no
/// quoting troubles, and no execution policy (a `.ps1` file is refused by default).
pub fn ps_hook_encoded() -> String {
    let script = format!("{PS_HOOK}\n{PS_CMDS}");
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64(&utf16)
}

/// Terminal commands with descriptions; the terminal page shows the same list (button "?").
const COMMANDS: &str = include_str!("../static/commands.json");

/// The list for `cmds` (`LP_CMDS`), written once per run; None if that failed.
pub fn commands_file() -> Option<&'static Path> {
    static FILE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    FILE.get_or_init(|| {
        let path = std::env::temp_dir().join("local-photo-commands.json");
        std::fs::write(&path, COMMANDS).ok().map(|_| path)
    })
    .as_deref()
}

pub(crate) fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Finds the last complete OSC 7 (`ESC ] 7 ; <folder> BEL`) in a stream split into chunks.
#[derive(Default)]
pub struct CwdTracker {
    tail: Vec<u8>,
}

impl CwdTracker {
    pub fn push(&mut self, bytes: &[u8]) -> Option<PathBuf> {
        const START: &[u8] = b"]7;";
        self.tail.extend_from_slice(bytes);
        let mut found = None;
        let mut done = 0;
        loop {
            let Some(start) = find(&self.tail[done..], START).map(|i| i + done) else {
                // Keep only what could be the beginning of the next sequence.
                let keep_from = self.tail.len().saturating_sub(START.len() - 1).max(done);
                self.tail.drain(..keep_from);
                break;
            };
            let body = start + START.len();
            let Some(len) = self.tail[body..].iter().position(|&b| b == 0x07) else {
                // Unfinished: wait for the rest (unless it is clearly not a real sequence).
                self.tail.drain(..start);
                if self.tail.len() > 4096 {
                    self.tail.clear();
                }
                break;
            };
            let text = String::from_utf8_lossy(&self.tail[body..body + len]).into_owned();
            if !text.is_empty() {
                found = Some(PathBuf::from(text));
            }
            done = body + len + 1;
        }
        found
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

// ---------- what runs in the shells ----------

/// What a look at the process tree found for one shell.
pub struct Found {
    /// The shell's own working directory (macOS/Linux; PowerShell reports via OSC 7).
    pub cwd: Option<PathBuf>,
    pub claude: Option<ClaudeRun>,
}

/// Looks at the processes under each shell. `shells`: session id → (shell pid, Claude run
/// matched before, if any). Conversations already matched are not given to another session.
pub fn scan(shells: &[(u32, u32, Option<ClaudeRun>)]) -> HashMap<u32, Found> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut result = HashMap::new();
    let Ok(mut sys) = System::new() else { return result };
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always).with_cwd(UpdateKind::Always),
    );
    let procs = sys.processes();
    let mut children: HashMap<sysinfo::Pid, Vec<sysinfo::Pid>> = HashMap::new();
    for (pid, p) in procs {
        if let Some(parent) = p.parent() {
            children.entry(parent).or_default().push(*pid);
        }
    }
    let mut claimed: Vec<String> = shells.iter().filter_map(|(_, _, c)| c.as_ref()?.conversation.clone()).collect();
    for (id, shell_pid, before) in shells {
        let shell_pid = sysinfo::Pid::from_u32(*shell_pid);
        let cwd = procs.get(&shell_pid).and_then(|p| p.cwd()).map(Path::to_path_buf);
        // Breadth first: the claude closest to the shell is the one the user started.
        let mut queue = std::collections::VecDeque::from([shell_pid]);
        let mut claude = None;
        let mut seen = 0;
        while let Some(pid) = queue.pop_front() {
            seen += 1;
            if seen > 500 {
                break;
            }
            if pid != shell_pid {
                if let Some(p) = procs.get(&pid) {
                    let cmd: Vec<String> = p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect();
                    let name = p.name().to_string_lossy().to_lowercase();
                    // The desktop app is also Claude.exe; it is not something to resume.
                    let desktop = cmd.first().is_some_and(|a| a.contains("WindowsApps")) || cmd.iter().any(|a| a.starts_with("--type="));
                    if let Some(args) = claude_args(&name, &cmd).filter(|_| !desktop) {
                        let cwd = p.cwd().map(Path::to_path_buf);
                        let started = p.start_time();
                        let conversation = match before {
                            Some(b) if b.started == started && b.conversation.is_some() => b.conversation.clone(),
                            _ => cwd.as_deref().and_then(|d| newest_conversation(d, started, &claimed)),
                        };
                        if let Some(c) = &conversation {
                            claimed.push(c.clone());
                        }
                        claude = Some(ClaudeRun { args: keep_args(&args), cwd, conversation, started });
                        break;
                    }
                }
            }
            queue.extend(children.get(&pid).into_iter().flatten().copied());
        }
        result.insert(*id, Found { cwd, claude });
    }
    result
}

/// Claude Code's own arguments if this process is it: the native `claude(.exe)`, or Node
/// running its `cli.js`.
fn claude_args(name: &str, cmd: &[String]) -> Option<Vec<String>> {
    if name == "claude.exe" || name == "claude" {
        return Some(cmd.iter().skip(1).cloned().collect());
    }
    if name.starts_with("node") {
        let i = cmd.iter().position(|a| {
            let a = a.replace('\\', "/").to_lowercase();
            a.contains("claude-code/") && a.ends_with(".js")
        })?;
        return Some(cmd[i + 1..].to_vec());
    }
    None
}

/// Options that make sense again on resume. Everything else (a first prompt, `--print`,
/// `--resume` of an older conversation) is dropped.
pub fn keep_args(args: &[String]) -> Vec<String> {
    const FLAGS: &[&str] = &["--dangerously-skip-permissions", "--verbose", "--ide"];
    const WITH_VALUE: &[&str] = &["--model", "--permission-mode", "--add-dir", "--fallback-model"];
    let mut out = Vec::new();
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        if FLAGS.contains(&a.as_str()) {
            out.push(a.clone());
        } else if let Some((flag, value)) = a.split_once('=').filter(|(f, _)| WITH_VALUE.contains(f)) {
            out.extend([flag.to_string(), value.to_string()]);
        } else if WITH_VALUE.contains(&a.as_str()) {
            if let Some(v) = it.next_if(|v| !v.starts_with('-')) {
                out.extend([a.clone(), v.clone()]);
            }
        }
    }
    out
}

/// Claude Code keeps each conversation in `<config>/projects/<folder with every
/// non-alphanumeric character as '-'>/<id>.jsonl`.
fn projects_dir(cwd: &Path) -> Option<PathBuf> {
    let config = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(|h| PathBuf::from(h).join(".claude")))?;
    let path = cwd.to_string_lossy();
    // "C:\work\" and "C:\work" are the same project.
    let folder: String = path.trim_end_matches(['\\', '/']).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    Some(config.join("projects").join(folder))
}

/// The conversation written to most recently since the process started, not taken by another session.
fn newest_conversation(cwd: &Path, started: u64, claimed: &[String]) -> Option<String> {
    let dir = projects_dir(cwd)?;
    let mut best: Option<(std::time::SystemTime, String)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".jsonl") else { continue };
        if claimed.iter().any(|c| c == id) || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else { continue };
        let secs = modified.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        if secs + 5 < started {
            continue; // older than this claude
        }
        if best.as_ref().is_none_or(|(t, _)| modified > *t) {
            best = Some((modified, id.to_string()));
        }
    }
    best.map(|(_, id)| id)
}

/// The line typed into the restored shell to bring Claude back.
pub fn resume_command(run: &ClaudeRun) -> String {
    let mut cmd = String::from("claude");
    // Without a conversation of its own a fresh start is right: `--continue` would pick up
    // whatever was discussed last in that folder.
    if let Some(id) = &run.conversation {
        cmd.push_str(&format!(" --resume {id}"));
    }
    for a in &run.args {
        cmd.push(' ');
        cmd.push_str(&quote(a));
    }
    cmd
}

/// Safe as one argument in PowerShell and in sh: plain words as is, the rest single-quoted.
fn quote(arg: &str) -> String {
    if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@".contains(c)) {
        return arg.to_string();
    }
    if cfg!(windows) {
        format!("'{}'", arg.replace('\'', "''"))
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_values() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    /// `cmds` and the page's cheat sheet both read these fields; a typo would hide an entry.
    #[test]
    fn command_list_is_complete() {
        let list: Vec<HashMap<String, String>> = serde_json::from_str(COMMANDS).unwrap();
        assert!(list.len() > 50);
        for entry in &list {
            for field in ["g", "ge", "cmd", "ru", "en"] {
                assert!(entry.get(field).is_some_and(|v| !v.trim().is_empty()), "{field} in {entry:?}");
            }
            assert_eq!(entry.len(), 5, "{entry:?}");
        }
    }

    #[test]
    fn cwd_is_found_across_chunks() {
        let mut t = CwdTracker::default();
        assert_eq!(t.push(b"PS> ls\r\n\x1b]7;C:\\Users\\"), None);
        assert_eq!(t.push(b"admin\x07PS C:\\Users\\admin> "), Some(PathBuf::from("C:\\Users\\admin")));
        assert_eq!(t.push(b"\x1b]7;D:\\a\x07\x1b]7;D:\\b\x07"), Some(PathBuf::from("D:\\b")));
        assert_eq!(t.push(b"plain output\r\n"), None);
        assert!(t.tail.len() < 4);
        assert_eq!(t.push(b"\x1b"), None);
        assert_eq!(t.push(b"]7;/home/u\x07"), Some(PathBuf::from("/home/u")));
    }

    #[test]
    fn claude_is_recognised() {
        let s = |v: &[&str]| v.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert_eq!(claude_args("claude.exe", &s(&["C:\\x\\claude.exe", "--model", "opus"])), Some(s(&["--model", "opus"])));
        let node = s(&["node", "C:\\npm\\node_modules\\@anthropic-ai\\claude-code\\cli.js", "--verbose"]);
        assert_eq!(claude_args("node.exe", &node), Some(s(&["--verbose"])));
        assert_eq!(claude_args("node.exe", &s(&["node", "server.js"])), None);
        assert_eq!(claude_args("powershell.exe", &s(&["powershell"])), None);
    }

    #[test]
    fn only_lasting_options_are_kept() {
        let s = |v: &[&str]| v.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        let args = s(&["--resume", "abc", "fix the bug", "--model", "opus", "--permission-mode=plan", "-p", "--dangerously-skip-permissions", "--add-dir"]);
        assert_eq!(keep_args(&args), s(&["--model", "opus", "--permission-mode", "plan", "--dangerously-skip-permissions"]));
    }

    #[test]
    fn resume_line_is_quoted() {
        let run = ClaudeRun { args: vec!["--add-dir".into(), "C:\\My Docs".into()], cwd: None, conversation: Some("1b2c-3d".into()), started: 0 };
        assert_eq!(resume_command(&run), "claude --resume 1b2c-3d --add-dir 'C:\\My Docs'");
        let run = ClaudeRun { conversation: None, args: vec![], cwd: None, started: 0 };
        assert_eq!(resume_command(&run), "claude");
    }

    #[test]
    fn project_folder_naming() {
        let dir = projects_dir(Path::new("C:\\Users\\admin\\Documents\\claudeweed-main")).unwrap();
        assert!(dir.ends_with(Path::new("projects").join("C--Users-admin-Documents-claudeweed-main")));
        assert_eq!(Some(dir), projects_dir(Path::new("C:\\Users\\admin\\Documents\\claudeweed-main\\")));
    }
}
