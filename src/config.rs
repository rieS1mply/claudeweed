//! Where things live, the settings file, and the interactive first-run setup.
//!
//! ```text
//! <settings folder>/config.json      login, password hash, Telegram, storage_dir
//! <settings folder>/bin/cloudflared   downloaded on first run
//!
//! <storage_dir>/Фото/2024/03-Март/IMG_0001.jpg      chosen by the user (any disk)
//! <storage_dir>/Файлы/2024/03-Март/report.pdf
//! <storage_dir>/Заметки/<id>.md
//! <storage_dir>/.local-photo/trash, thumbs, uploads  same disk as the photos, so moving
//!                                                   into the trash is a rename, not a copy
//! ```
//!
//! The settings folder is the OS's usual place for app settings
//! (Windows `%APPDATA%\local-photo`, macOS `~/Library/Application Support/local-photo`,
//! Linux `~/.config/local-photo`), or `--dir` for a portable/test setup.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use argon2::password_hash::{PasswordHasher, SaltString};
use argon2::Argon2;
use serde::{Deserialize, Serialize};

use crate::telegram;

pub const APP_DIR: &str = "local-photo";
pub const CONFIG_FILE: &str = "config.json";
pub const DEFAULT_STORAGE_NAME: &str = "Облако";
pub const NOTES_DIR: &str = "Заметки";
pub const SYSTEM_DIR: &str = ".local-photo";
/// Layout before the storage location became configurable: everything next to the program.
const LEGACY_STORAGE_DIR: &str = "Хранилище";
pub const DEFAULT_PORT: u16 = 8787;

#[derive(Clone)]
pub struct Paths {
    pub config: PathBuf,
    pub bin: PathBuf,
    pub storage: PathBuf,
    pub notes: PathBuf,
    pub trash: PathBuf,
    pub thumbs: PathBuf,
    pub uploads: PathBuf,
}

impl Paths {
    pub fn new(config_dir: &Path, storage: &Path) -> Self {
        let system = storage.join(SYSTEM_DIR);
        Paths {
            config: config_dir.join(CONFIG_FILE),
            bin: config_dir.join("bin"),
            storage: storage.to_path_buf(),
            notes: storage.join(NOTES_DIR),
            trash: system.join("trash"),
            thumbs: system.join("thumbs"),
            uploads: system.join("uploads"),
        }
    }

    pub fn ensure(&self) -> Result<()> {
        // A missing storage folder usually means an unplugged disk. Creating an empty one
        // elsewhere would quietly split the photos over two places, so refuse instead.
        if !self.storage.is_dir() {
            bail!(
                "папка хранилища недоступна: {}\n\
                 Если она на внешнем диске — подключите его. Выбрать другую папку: запустите с флагом --setup",
                self.storage.display()
            );
        }
        for dir in [&self.notes, &self.trash, &self.bin, &self.thumbs, &self.uploads] {
            std::fs::create_dir_all(dir).with_context(|| format!("не удалось создать {}", dir.display()))?;
        }
        for section in crate::storage::Section::ALL {
            std::fs::create_dir_all(self.storage.join(section.dir_name()))?;
        }
        hide_on_windows(&self.storage.join(SYSTEM_DIR));
        Ok(())
    }
}

#[cfg(windows)]
fn hide_on_windows(path: &Path) {
    // Dot-folders are only hidden by convention on Unix; mark it hidden for Explorer too.
    let _ = crate::service::no_window(std::process::Command::new("attrib").arg("+h").arg(path)).status();
}

#[cfg(not(windows))]
fn hide_on_windows(_path: &Path) {}

/// The OS's usual folder for app settings.
pub fn default_config_dir() -> Result<PathBuf> {
    let home = || std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let dir = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library").join("Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".config")))
    };
    Ok(dir.context("не удалось найти папку для настроек (нет HOME/APPDATA)")?.join(APP_DIR))
}

fn default_storage_dir() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(DEFAULT_STORAGE_NAME)
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    /// Folder with Фото/Файлы/Заметки. Can be edited here by hand.
    #[serde(default)]
    pub storage_dir: Option<PathBuf>,
    pub username: String,
    /// Argon2id PHC string; the password itself is never stored.
    pub password_hash: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,
    /// Terminal of this computer on the site. Off unless the owner turns it on.
    #[serde(default)]
    pub terminal: bool,
    /// Running code files from the site (`--runner on`): executes anything, like the terminal.
    #[serde(default)]
    pub runner: bool,
    /// Base32 secret of the authenticator app (`--2fa on`); login then also asks for its code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totp_secret: Option<String>,
    /// Daily copy of the storage goes here (`--backup <folder>`), ideally another disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_dir: Option<PathBuf>,
    /// A named Cloudflare tunnel with a permanent address instead of a random trycloudflare one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub named_tunnel: Option<NamedTunnel>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct NamedTunnel {
    /// From the Cloudflare dashboard (Zero Trust → Networks → Tunnels). Secret: never logged.
    pub token: String,
    /// The public hostname set up for that tunnel, e.g. https://cloud.example.com.
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TelegramConfig {
    pub token: String,
    pub chat_id: i64,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

impl Config {
    pub fn load(path: &Path) -> Result<Option<Config>> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(serde_json::from_str(&text).with_context(|| format!("{} повреждён", path.display()))?)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        restrict_permissions(&tmp);
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(unix)]
pub fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
pub fn restrict_permissions(_path: &Path) {}

/// Older versions kept `.local-photo/config.json` and `Хранилище/` next to the program.
/// Move that into the new layout once, keeping the photos where they are.
pub fn migrate_legacy(legacy_base: &Path, config_dir: &Path) -> Result<()> {
    let old_system = legacy_base.join(SYSTEM_DIR);
    let old_config = old_system.join(CONFIG_FILE);
    let new_config = config_dir.join(CONFIG_FILE);
    if new_config.exists() || !old_config.is_file() {
        return Ok(());
    }
    let mut config = Config::load(&old_config)?.context("старый config.json пуст")?;
    let storage = legacy_base.join(LEGACY_STORAGE_DIR);
    config.storage_dir = Some(storage.clone());
    config.save(&new_config)?;

    // Trash and thumbnails move next to the photos; the rest is either re-created or downloaded again.
    let new_system = storage.join(SYSTEM_DIR);
    std::fs::create_dir_all(&new_system)?;
    for sub in ["trash", "thumbs"] {
        let from = old_system.join(sub);
        let to = new_system.join(sub);
        if from.is_dir() && !to.exists() {
            move_tree(&from, &to)?;
        }
    }
    let _ = std::fs::remove_dir_all(&old_system);
    crate::log!("Настройки перенесены в {}; хранилище осталось в {}", new_config.display(), storage.display());
    Ok(())
}

/// rename, or copy + delete when it's another disk.
fn move_tree(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_tree(from, to)?;
    std::fs::remove_dir_all(from)?;
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        std::fs::copy(from, to).with_context(|| format!("не удалось скопировать {}", from.display()))?;
        // Keep the capture date the sorting relies on.
        if let Ok(meta) = std::fs::metadata(from) {
            let ft = filetime::FileTime::from_last_modification_time(&meta);
            let _ = filetime::set_file_mtime(to, ft);
        }
    }
    Ok(())
}

pub fn hash_password(password: &str) -> Result<String> {
    let mut salt_bytes = [0u8; 16];
    getrandom::getrandom(&mut salt_bytes).map_err(|e| anyhow::anyhow!("random: {e}"))?;
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|e| anyhow::anyhow!("argon2: {e}"))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("argon2: {e}"))
}

pub fn ask(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line)? == 0 {
        // Started as a service (systemd, autostart) before the first setup was done.
        bail!("нет ввода с клавиатуры — первую настройку нужно пройти, запустив программу в терминале");
    }
    // Windows pipes and some pastes start with a byte-order mark; it is not part of the answer.
    Ok(line.trim().trim_start_matches('\u{feff}').trim().to_string())
}

pub fn ask_password(prompt: &str) -> Result<String> {
    // Hidden input in a real terminal; plain line when input is piped (scripts, tests).
    if io::stdin().is_terminal() {
        Ok(rpassword::prompt_password(prompt)?)
    } else {
        ask(prompt)
    }
}

pub fn ask_yes(prompt: &str) -> Result<bool> {
    let answer = ask(prompt)?.to_lowercase();
    Ok(answer.is_empty() || answer.starts_with('д') || answer.starts_with('y'))
}

/// A folder typed or dragged into the terminal: quotes, `\ ` escapes and `~` are handled.
pub fn parse_folder(input: &str) -> PathBuf {
    let s = input.trim().trim_matches(|c| c == '"' || c == '\'');
    let s = if cfg!(windows) { s.to_string() } else { s.replace("\\ ", " ") };
    let path = match s.strip_prefix("~") {
        Some(rest) => {
            let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default();
            home.join(rest.trim_start_matches(['/', '\\']))
        }
        None => PathBuf::from(s),
    };
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir().map(|d| d.join(&path)).unwrap_or(path)
    }
}

fn has_storage(dir: &Path) -> bool {
    crate::storage::Section::ALL
        .iter()
        .any(|s| std::fs::read_dir(dir.join(s.dir_name())).is_ok_and(|mut it| it.next().is_some()))
}

/// Asks where to keep the photos, creates the folder and checks it is writable.
fn choose_storage(previous: Option<&Path>, suggestion: &Path) -> Result<PathBuf> {
    println!("Где хранить фото и файлы?");
    println!("Можно выбрать любую папку, в том числе на другом или внешнем диске.");
    println!("Впишите путь или перетащите папку в это окно.");
    let default = previous.unwrap_or(suggestion);
    loop {
        let answer = ask(&format!("Папка (Enter — {}): ", default.display()))?;
        let dir = if answer.is_empty() { default.to_path_buf() } else { parse_folder(&answer) };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            println!("  Не удалось создать папку: {e}. Попробуйте другую.");
            continue;
        }
        let probe = dir.join(".local-photo-write-test");
        if std::fs::write(&probe, b"ok").and_then(|_| std::fs::remove_file(&probe)).is_err() {
            println!("  В эту папку нельзя записывать. Выберите другую.");
            continue;
        }
        let dir = dir.canonicalize().unwrap_or(dir);
        if has_storage(&dir) {
            println!("  В папке уже есть хранилище — оно и будет использоваться.");
        }
        return Ok(dir);
    }
}

/// When the storage folder changes, offer to bring the existing photos along.
fn offer_move(old: &Path, new: &Path) -> Result<()> {
    if old == new || !has_storage(old) {
        return Ok(());
    }
    if has_storage(new) {
        println!("  В старой папке тоже есть файлы ({}). Они не тронуты — перенесите вручную, если нужно.", old.display());
        return Ok(());
    }
    if !ask_yes(&format!("Перенести фото и файлы из {} в новую папку? [Д/н]: ", old.display()))? {
        println!("  Старые файлы остались в {}.", old.display());
        return Ok(());
    }
    println!("  Переношу… (с одного диска на другой это может занять время)");
    for name in crate::storage::Section::ALL.iter().map(|s| s.dir_name()).chain([NOTES_DIR, SYSTEM_DIR]) {
        let from = old.join(name);
        if !from.exists() {
            continue;
        }
        let to = new.join(name);
        if to.exists() {
            // Empty skeleton folders created earlier: replace them.
            let _ = std::fs::remove_dir_all(&to);
        }
        move_tree(&from, &to).with_context(|| format!("перенос {}", from.display()))?;
    }
    println!("  Готово.");
    Ok(())
}

/// Interactive setup: storage folder, login, password and (optionally) the Telegram bot.
pub async fn setup(path: &Path, previous: Option<&Config>, suggestion: Option<&Path>) -> Result<Config> {
    println!();
    println!("=== Настройка домашнего облака ===");
    println!();

    let old_storage = previous.and_then(|c| c.storage_dir.clone());
    let fallback = default_storage_dir();
    let storage_dir = choose_storage(old_storage.as_deref(), suggestion.unwrap_or(&fallback))?;
    if let Some(old) = &old_storage {
        offer_move(old, &storage_dir)?;
    }
    println!();

    let username = loop {
        let name = ask("Логин для входа на сайт: ")?;
        if name.is_empty() {
            println!("  Логин не может быть пустым.");
        } else {
            break name;
        }
    };

    let password = loop {
        let p1 = ask_password("Пароль (минимум 10 символов): ")?;
        if p1.chars().count() < 10 {
            println!("  Слишком короткий пароль — сайт будет открыт в интернет, нужен надёжный.");
            continue;
        }
        let p2 = ask_password("Повторите пароль: ")?;
        if p1 != p2 {
            println!("  Пароли не совпадают, попробуйте ещё раз.");
            continue;
        }
        break p1;
    };

    println!();
    println!("Telegram: программа будет присылать адрес сайта при каждом запуске,");
    println!("а фото и файлы, отправленные боту, сохранятся в облако.");
    println!("Создайте бота у @BotFather (команда /newbot) и вставьте сюда его токен.");
    let telegram = loop {
        let token = ask("Токен бота (Enter — пропустить): ")?;
        if token.is_empty() {
            break None;
        }
        match connect_telegram(&token).await {
            Ok(tg) => break Some(tg),
            Err(e) => println!("  Не получилось: {e:#}. Попробуйте снова или нажмите Enter, чтобы пропустить."),
        }
    };

    println!();
    let terminal = ask_terminal()?;

    let config = Config {
        storage_dir: Some(storage_dir.clone()),
        username,
        password_hash: hash_password(&password)?,
        port: previous.map_or(DEFAULT_PORT, |c| c.port),
        telegram,
        terminal,
        // Re-running setup changes the login, not these (each has its own command).
        runner: previous.is_some_and(|c| c.runner),
        totp_secret: previous.and_then(|c| c.totp_secret.clone()),
        backup_dir: previous.and_then(|c| c.backup_dir.clone()),
        named_tunnel: previous.and_then(|c| c.named_tunnel.clone()),
    };
    config.save(path)?;
    println!();
    println!("Хранилище:  {}", storage_dir.display());
    println!("Настройки:  {}", path.display());
    println!("Поменять папку, пароль или бота: запустите программу с флагом --setup");
    println!("(или исправьте storage_dir в файле настроек).");
    println!();
    Ok(config)
}

/// Shown at setup; the owner of the computer decides.
fn ask_terminal() -> Result<bool> {
    println!("Терминал: с сайта можно будет выполнять любые команды на этом компьютере");
    println!("(с ноутбука или телефона). Кто знает пароль — управляет компьютером.");
    println!("О каждом входе в терминал придёт сообщение в Telegram.");
    let on = ask("Включить терминал на сайте? [д/Н]: ")?;
    Ok(matches!(on.to_lowercase().as_str(), "д" | "да" | "y" | "yes"))
}

async fn connect_telegram(token: &str) -> Result<TelegramConfig> {
    let bot = telegram::get_me(token).await.context("токен не подходит")?;
    println!("  Бот найден: @{bot}");
    println!("  Откройте этого бота в Telegram и отправьте ему /start (жду до 10 минут)…");

    // Skip whatever the bot received before this moment.
    let mut offset = telegram::latest_update_id(token).await?.map_or(0, |id| id + 1);
    let deadline = Instant::now() + Duration::from_secs(600);
    while Instant::now() < deadline {
        let (next, chat) = telegram::wait_for_chat(token, offset).await?;
        offset = next;
        if let Some((chat_id, who)) = chat {
            println!("  Получено сообщение от {who}.");
            let tg = TelegramConfig { token: token.to_string(), chat_id };
            telegram::send(&tg, "Бот подключён. Сюда будет приходить адрес облака при каждом запуске.").await?;
            return Ok(tg);
        }
    }
    bail!("сообщение /start не пришло за 10 минут")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn folder_input_forms_windows() {
        let home = PathBuf::from(std::env::var_os("USERPROFILE").unwrap());
        assert_eq!(parse_folder("~/Облако"), home.join("Облако"));
        assert_eq!(parse_folder(r#"  "D:\Мой диск\Фото"  "#), PathBuf::from(r"D:\Мой диск\Фото"));
        assert!(parse_folder("relative").is_absolute());
    }

    #[test]
    #[cfg(unix)]
    fn folder_input_forms() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(parse_folder("~/Облако"), home.join("Облако"));
        assert_eq!(parse_folder("  \"/Volumes/Disk/Фото\"  "), PathBuf::from("/Volumes/Disk/Фото"));
        assert_eq!(parse_folder("/Volumes/My\\ Disk/x"), PathBuf::from("/Volumes/My Disk/x"));
        assert!(parse_folder("relative").is_absolute());
    }

    #[test]
    fn legacy_layout_is_migrated() {
        let base = std::env::temp_dir().join(format!("lp-legacy-{}", std::process::id()));
        let old_system = base.join(SYSTEM_DIR);
        std::fs::create_dir_all(old_system.join("trash/abc")).unwrap();
        std::fs::create_dir_all(base.join(LEGACY_STORAGE_DIR).join("Фото")).unwrap();
        let cfg = Config { storage_dir: None, username: "u".into(), password_hash: "h".into(), port: 1, telegram: None, terminal: false, runner: false, totp_secret: None, backup_dir: None, named_tunnel: None };
        cfg.save(&old_system.join(CONFIG_FILE)).unwrap();
        let config_dir = base.join("settings");

        migrate_legacy(&base, &config_dir).unwrap();

        let moved = Config::load(&config_dir.join(CONFIG_FILE)).unwrap().unwrap();
        assert_eq!(moved.storage_dir.unwrap(), base.join(LEGACY_STORAGE_DIR));
        assert!(base.join(LEGACY_STORAGE_DIR).join(SYSTEM_DIR).join("trash/abc").is_dir());
        assert!(!old_system.exists());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
