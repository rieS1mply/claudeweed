//! Personal home cloud.
//!
//! Run it on the computer that keeps the photos. It serves a password-protected
//! web interface on 127.0.0.1 and publishes it through a free Cloudflare quick
//! tunnel; the public address is printed and sent to Telegram on every start.

mod auth;
mod backup;
mod bot;
mod config;
mod logging;
mod notes;
mod runner;
mod service;
mod storage;
mod telegram;
mod terminal;
mod thumbs;
mod trash;
mod totp;
mod tunnel;
mod web;
mod zips;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::net::TcpListener;
use tokio::sync::Notify;

use config::{Config, Paths};

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::logging::line(&format!($($arg)*))
    };
}

struct Args {
    dir: Option<PathBuf>,
    setup: bool,
    no_tunnel: bool,
    port: Option<u16>,
    background: bool,
    status: bool,
    stop: bool,
    autostart: Option<bool>,
    terminal: Option<bool>,
    runner: Option<bool>,
    two_factor: Option<bool>,
    /// `Some(None)` = `--backup off`.
    backup: Option<Option<PathBuf>>,
    backup_now: bool,
    /// `Some(true)` = named tunnel (asked interactively), `Some(false)` = back to quick.
    named_tunnel: Option<bool>,
}

fn on_off(value: Option<String>, flag: &str) -> Result<bool> {
    match value.as_deref() {
        Some("on") => Ok(true),
        Some("off") => Ok(false),
        _ => bail!("после {flag} нужно on или off"),
    }
}

const HELP: &str = "\
local-photo — домашнее облако для фото и файлов

Использование: local-photo [параметры]

  (без параметров)    запустить облако (при первом запуске — настройка)
  --status            работает ли облако и по какому адресу
  --stop              остановить облако, работающее в фоне
  --autostart on|off  запускать облако само при входе в систему (в фоне) / не запускать
  --terminal on|off   разрешить / запретить терминал этого компьютера на сайте
  --runner on|off     разрешить / запретить запуск кода на сайте (как онлайн-компилятор)
  --2fa on|off        вход с кодом из приложения (Google Authenticator и др.) / без кода
  --backup <папка>    каждый день копировать хранилище в эту папку (лучше другой диск)
  --backup off        не делать резервную копию
  --backup-now        сделать резервную копию прямо сейчас
  --tunnel named      постоянный адрес: туннель Cloudflare со своим доменом (спросит токен)
  --tunnel quick      снова случайный адрес trycloudflare.com (без аккаунта)
  --setup            выбрать папку хранилища, задать логин, пароль и Telegram заново
  --port <порт>       локальный порт (по умолчанию 8787)
  --no-tunnel         не открывать доступ из интернета (только http://127.0.0.1)
  --dir <папка>       держать настройки в этой папке (переносная/тестовая установка)
  -h, --help          эта справка
";

fn parse_args() -> Result<Args> {
    let mut args = Args {
        dir: None, setup: false, no_tunnel: false, port: None, background: false, status: false, stop: false,
        autostart: None, terminal: None, runner: None, two_factor: None, backup: None, backup_now: false, named_tunnel: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dir" => args.dir = Some(it.next().context("после --dir нужна папка")?.into()),
            "--port" => args.port = Some(it.next().context("после --port нужен номер")?.parse().context("неверный порт")?),
            "--setup" => args.setup = true,
            "--no-tunnel" => args.no_tunnel = true,
            "--background" => args.background = true,
            "--status" => args.status = true,
            "--stop" => args.stop = true,
            "--autostart" => args.autostart = Some(on_off(it.next(), "--autostart")?),
            "--terminal" => args.terminal = Some(on_off(it.next(), "--terminal")?),
            "--runner" => args.runner = Some(on_off(it.next(), "--runner")?),
            "--2fa" => args.two_factor = Some(on_off(it.next(), "--2fa")?),
            "--backup" => {
                let v = it.next().context("после --backup нужна папка или off")?;
                args.backup = Some((v != "off").then(|| config::parse_folder(&v)));
            }
            "--backup-now" => args.backup_now = true,
            "--tunnel" => {
                args.named_tunnel = Some(match it.next().as_deref() {
                    Some("named") => true,
                    Some("quick") => false,
                    _ => bail!("после --tunnel нужно named или quick"),
                })
            }
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            other => bail!("неизвестный параметр {other}\n\n{HELP}"),
        }
    }
    Ok(args)
}

/// Folder of the program itself; only used to pick up the old "everything next to the program" layout.
fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Ошибка: {e:#}");
        logging::line(&format!("Ошибка: {e:#}"));
        // Keep the window open when started by double-click on Windows.
        service::pause_if_own_window();
        std::process::exit(1);
    }
}

/// The address, big enough to notice, plus a QR code for the phone camera.
fn show_url(url: &str) {
    println!();
    println!("  ===============================================================");
    println!("   Облако доступно по адресу:  {url}");
    println!("  ===============================================================");
    if let Some(qr) = service::qr(url) {
        println!("   Наведите камеру телефона, чтобы открыть:");
        for line in qr.lines() {
            println!("   {line}");
        }
    }
    println!();
}

async fn print_status(config_dir: &Path) -> Result<()> {
    let autostart = if service::autostart_enabled() { "включён" } else { "выключен" };
    match service::running(config_dir).await {
        Some(st) => {
            println!("Облако работает.");
            match &st.url {
                Some(url) => show_url(url),
                None if !st.tunnel => println!("  Без туннеля: доступно только с этого компьютера."),
                None => println!("  Туннель ещё открывается — адрес появится через минуту."),
            }
            println!("  Хранилище:   {}", st.storage.display());
            println!("  Локально:    http://127.0.0.1:{}", st.port);
        }
        None => println!("Облако не запущено."),
    }
    println!("  Автозапуск:  {autostart}");
    let config = Config::load(&config_dir.join(config::CONFIG_FILE)).ok().flatten();
    let terminal = config.as_ref().is_some_and(|c| c.terminal);
    println!("  Терминал:    {}", if terminal { "включён (--terminal off — выключить)" } else { "выключен" });
    let runner = config.as_ref().is_some_and(|c| c.runner);
    println!("  Запуск кода: {}", if runner { "включён (--runner off — выключить)" } else { "выключен (--runner on — включить)" });
    let two_factor = config.as_ref().is_some_and(|c| c.totp_secret.is_some());
    println!("  Вход:        {}", if two_factor { "пароль + код из приложения" } else { "только пароль (--2fa on — добавить код)" });
    let address = match config.as_ref().and_then(|c| c.named_tunnel.as_ref()) {
        Some(t) => format!("постоянный, {}", t.url),
        None => "случайный trycloudflare.com (--tunnel named — постоянный)".into(),
    };
    println!("  Адрес:       {address}");
    match config.as_ref().and_then(|c| c.backup_dir.as_ref()) {
        None => println!("  Копия:       не делается (--backup <папка> — включить)"),
        Some(dir) => {
            let st = backup::State::load(config_dir);
            let last = if st.last_ok > 0 { backup::fmt_time(st.last_ok) } else { "ещё не было".into() };
            println!("  Копия:       {} · последняя удачная: {last} · файлов: {}", dir.join(backup::COPY_DIR).display(), st.files);
            if let Some(err) = st.error {
                println!("               последняя ошибка: {err}");
            }
        }
    }
    println!("  Журнал:      {}", config_dir.join("local-photo.log").display());
    service::pause_if_own_window();
    Ok(())
}

/// `--2fa on`: shows a QR for the authenticator app and turns it on only after a correct code,
/// so a mistyped setup can't lock the owner out.
fn set_two_factor(config: &mut Config, on: bool) -> Result<()> {
    if !on {
        config.totp_secret = None;
        println!("Вход по коду выключен: на сайте снова достаточно логина и пароля.");
        return Ok(());
    }
    let secret = totp::new_secret();
    let url = totp::otpauth_url(&secret, &config.username);
    println!("Откройте приложение-аутентификатор (Google Authenticator, Microsoft Authenticator,");
    println!("2FAS, Aegis…), нажмите «+» и отсканируйте этот QR-код:");
    if let Some(qr) = service::qr(&url) {
        for line in qr.lines() {
            println!("   {line}");
        }
    }
    println!("Или введите ключ вручную: {secret}");
    for _ in 0..3 {
        let code = config::ask("Код из приложения (6 цифр): ")?;
        if totp::verify(&secret, &code, totp::unix_now()).is_some() {
            config.totp_secret = Some(secret);
            println!("Готово: вход на сайт теперь требует код из приложения.");
            println!("Уже открытые на устройствах сеансы остаются. Выключить: local-photo --2fa off");
            return Ok(());
        }
        println!("Код не подошёл. Проверьте, что время на телефоне и компьютере выставлено автоматически.");
    }
    bail!("вход по коду не включён: три неверных кода")
}

fn set_backup(config: &mut Config, target: Option<&Path>) -> Result<()> {
    let Some(target) = target else {
        config.backup_dir = None;
        println!("Резервная копия выключена. Уже сделанная копия остаётся на месте.");
        return Ok(());
    };
    let storage = config.storage_dir.clone().context("сначала настройте облако")?;
    if !target.is_dir() {
        bail!("папки {} нет. Создайте её (или подключите диск) и повторите", target.display());
    }
    backup::check_target(&storage, target)?;
    // absolute(), not canonicalize(): no verbatim `\\?\` prefix on Windows in config.json and messages.
    let target = std::path::absolute(target).unwrap_or_else(|_| target.to_path_buf());
    config.backup_dir = Some(target.clone());
    println!("Резервная копия: каждый день в {}", target.join(backup::COPY_DIR).display());
    println!("Копируются новые и изменённые файлы; удалённые в облаке остаются в копии.");
    if backup::same_disk(&storage, &target) {
        println!("Внимание: это тот же диск, что и хранилище. Если диск сломается, пропадёт и копия.");
        println!("Надёжнее — другой диск или внешний диск.");
    }
    println!("Первая копия начнётся через несколько минут после запуска облака (или сейчас: --backup-now).");
    Ok(())
}

fn set_tunnel(config: &mut Config, named: bool) -> Result<()> {
    if !named {
        config.named_tunnel = None;
        println!("Снова случайный адрес trycloudflare.com. Перезапустите облако: --stop, затем --autostart on");
        return Ok(());
    }
    println!("Постоянный адрес — туннель Cloudflare на вашем домене (домен должен быть добавлен в Cloudflare).");
    println!("1. https://one.dash.cloudflare.com → Networks → Tunnels → Create a tunnel → Cloudflared.");
    println!("2. Скопируйте токен из команды установки (длинная строка после --token или «eyJ…»).");
    println!("3. Public Hostname: ваш адрес (например cloud.example.com), Service: HTTP, localhost:{}", config.port);
    let url = config::ask("Адрес (например https://cloud.example.com): ")?;
    let url = if url.starts_with("https://") { url } else { format!("https://{}", url.trim_start_matches("http://")) };
    let url = url.trim_end_matches('/').to_string();
    if url.len() <= "https://".len() || url.contains(char::is_whitespace) {
        bail!("неверный адрес");
    }
    let token = config::ask_password("Токен туннеля (не отображается): ")?;
    // Pasted from "cloudflared service install <token>" or "--token <token>": keep the token only.
    let token = token.split_whitespace().last().unwrap_or("").to_string();
    if token.len() < 40 {
        bail!("это не похоже на токен туннеля (он длинный, обычно начинается с eyJ)");
    }
    config.named_tunnel = Some(config::NamedTunnel { token, url: url.clone() });
    println!("Сохранено. Перезапустите облако (--stop, затем --autostart on): сайт будет на {url}");
    Ok(())
}

async fn backup_now(config: &Config, config_dir: &Path) -> Result<()> {
    let target = config.backup_dir.clone().context("папка копии не задана: local-photo --backup <папка>")?;
    let storage = config.storage_dir.clone().context("сначала настройте облако")?;
    println!("Копирую {} → {} …", storage.display(), target.join(backup::COPY_DIR).display());
    match backup::run_and_record(config_dir, &storage, &target).await.0 {
        Ok(r) => println!("Готово: {}", backup::describe(&r)),
        Err(e) => bail!("{e}"),
    }
    Ok(())
}

async fn run() -> Result<()> {
    let args = parse_args()?;
    let config_dir = match &args.dir {
        Some(d) => d.clone(),
        None => config::default_config_dir()?,
    };
    std::fs::create_dir_all(&config_dir)?;
    let config_dir = config_dir.canonicalize()?;
    let config_file = config_dir.join(config::CONFIG_FILE);
    let exe = std::env::current_exe().context("не удалось определить путь к программе")?;

    // ---- one-shot commands ----
    if args.status {
        return print_status(&config_dir).await;
    }
    if args.stop {
        let stopped = service::stop(&config_dir).await?;
        println!("{}", if stopped { "Облако остановлено." } else { "Облако не было запущено." });
        return Ok(());
    }
    if let Some(on) = args.terminal {
        let mut config = Config::load(&config_file)?.context("сначала настройте облако: запустите программу без параметров")?;
        config.terminal = on;
        config.save(&config_file)?;
        // The running cloud re-reads this on every connection: no restart needed.
        if on {
            println!("Терминал включён: кнопка «Терминал» появится на сайте после обновления страницы.");
            println!("Кто знает пароль — может выполнять любые команды на этом компьютере.");
        } else {
            println!("Терминал выключен. Открытые сеансы закроются в течение 30 секунд.");
        }
        return Ok(());
    }
    if let Some(on) = args.runner {
        let mut config = Config::load(&config_file)?.context("сначала настройте облако: запустите программу без параметров")?;
        config.runner = on;
        config.save(&config_file)?;
        // Read live by the running cloud, like the terminal setting.
        if on {
            println!("Запуск кода включён: в редакторе файлов появится «▶ Запустить», в «Файлах» — «Новый код».");
            println!("Программы работают на этом компьютере с правами этого пользователя: кто знает пароль —");
            println!("может выполнить любой код. Включите вход с кодом из приложения: local-photo --2fa on");
        } else {
            println!("Запуск кода выключен.");
        }
        return Ok(());
    }
    if args.two_factor.is_some() || args.backup.is_some() || args.backup_now || args.named_tunnel.is_some() {
        let mut config = Config::load(&config_file)?.context("сначала настройте облако: запустите программу без параметров")?;
        if let Some(on) = args.two_factor {
            set_two_factor(&mut config, on)?;
        }
        if let Some(target) = &args.backup {
            set_backup(&mut config, target.as_deref())?;
        }
        if let Some(named) = args.named_tunnel {
            set_tunnel(&mut config, named)?;
        }
        config.save(&config_file)?;
        if args.backup_now {
            backup_now(&config, &config_dir).await?;
        }
        service::pause_if_own_window();
        return Ok(());
    }
    if let Some(on) = args.autostart {
        if on {
            if Config::load(&config_file)?.is_none() {
                bail!("сначала настройте облако: запустите программу без параметров");
            }
            let _ = service::stop(&config_dir).await;   // the background copy replaces this one
            let how = service::enable_autostart(&exe, args.dir.as_deref())?;
            println!("Автозапуск включён: {how}. Облако запускается в фоне…");
            match service::wait_for_url(&config_dir, Duration::from_secs(90)).await {
                Some(url) => show_url(&url),
                None => println!("Адрес придёт в Telegram; узнать его: local-photo --status"),
            }
        } else {
            service::disable_autostart()?;
            println!("Автозапуск выключен. (Если облако сейчас работает в фоне: local-photo --stop)");
        }
        return Ok(());
    }

    // ---- only one cloud at a time ----
    if let Some(st) = service::running(&config_dir).await {
        if args.background {
            return Ok(());   // autostart fired while a copy already runs
        }
        println!("Облако уже работает (второй раз запускать не нужно).");
        match &st.url {
            Some(url) => show_url(url),
            None if !st.tunnel => println!("Без туннеля: http://127.0.0.1:{}", st.port),
            None => println!("Туннель ещё открывается — адрес придёт в Telegram."),
        }
        println!("Остановить: local-photo --stop · Подробнее: local-photo --status");
        service::pause_if_own_window();
        return Ok(());
    }

    logging::to_file(&config_dir.join("local-photo.log"));
    if args.background {
        service::detach_console();
    }

    // Older versions kept everything next to the program (or in --dir).
    if let Some(legacy) = args.dir.clone().or_else(exe_dir) {
        config::migrate_legacy(&legacy, &config_dir)?;
    }

    let existing = Config::load(&config_file)?;
    let first_setup = !matches!(&existing, Some(c) if !args.setup && c.storage_dir.is_some());
    let mut config = match existing {
        Some(c) if !first_setup => c,
        prev => {
            // With --dir the suggested storage is inside it, so test setups stay self-contained.
            let suggestion = args.dir.as_ref().map(|d| d.join(config::DEFAULT_STORAGE_NAME));
            config::setup(&config_file, prev.as_ref(), suggestion.as_deref()).await?
        }
    };
    if let Some(port) = args.port {
        config.port = port;
    }
    let storage_dir = config.storage_dir.clone().expect("set by setup");
    let paths = Paths::new(&config_dir, &storage_dir);
    paths.ensure()?;

    // Right after setting up in a window: offer to keep it running without the window.
    if first_setup && !args.background && !args.no_tunnel && !service::autostart_enabled() {
        println!("Облако может запускаться само при входе в систему и работать в фоне —");
        println!("тогда это окно не нужно держать открытым.");
        if config::ask_yes("Включить автозапуск? [Д/н]: ")? {
            let how = service::enable_autostart(&exe, args.dir.as_deref())?;
            println!("Автозапуск включён: {how}. Запускаю облако в фоне…");
            match service::wait_for_url(&config_dir, Duration::from_secs(90)).await {
                Some(url) => show_url(&url),
                None => println!("Адрес придёт в Telegram; узнать его: local-photo --status"),
            }
            println!("Это окно можно закрыть — облако работает в фоне.");
            println!("Узнать адрес: local-photo --status · Остановить: local-photo --stop");
            service::pause_if_own_window();
            return Ok(());
        }
        println!();
    }

    // Unfinished uploads from a previous run can't be resumed (the browser lost its id).
    for entry in std::fs::read_dir(&paths.uploads)?.flatten() {
        let _ = std::fs::remove_file(entry.path());
    }
    let purged = trash::purge_expired(&paths.trash);
    if purged > 0 {
        log!("Корзина: удалено навсегда {purged} (старше {} дней)", trash::RETENTION_DAYS);
    }

    let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, config.port)).await {
        Ok(l) => l,
        Err(_) => {
            log!("Порт {} занят, выбираю свободный", config.port);
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?
        }
    };
    let addr: SocketAddr = listener.local_addr()?;

    log!("Хранилище: {}", paths.storage.display());
    log!("Настройки: {}", paths.config.display());
    log!("Локально:  http://{addr}");

    let status = Arc::new(std::sync::Mutex::new(service::Status {
        pid: std::process::id(),
        port: addr.port(),
        url: None,
        secret: auth::random_hex(32),
        storage: paths.storage.clone(),
        tunnel: !args.no_tunnel,
    }));
    service::write_status(&config_dir, &status.lock().unwrap());

    let stop = Arc::new(Notify::new());
    let auth = auth::Auth::with_store(&config_dir.join("sessions.json"));
    let secret = status.lock().unwrap().secret.clone();
    let app = web::AppState::new(paths.clone(), config.clone(), auth, secret, stop.clone());
    web::spawn_upload_janitor(app.clone());
    let public_url: bot::PublicUrl = Default::default();
    // The bot runs whenever Telegram is set up, also without a tunnel.
    let bot = config.telegram.clone().map(|tg| {
        log!("Telegram-бот слушает чат: фото и файлы оттуда сохраняются в облако");
        tokio::spawn(bot::run(app.clone(), tg, public_url.clone()))
    });
    let terminal = app.terminal.clone();
    let router = web::router(app);
    let backups = tokio::spawn(backup::schedule(config_file.clone(), config_dir.clone(), paths.storage.clone(), config.telegram.clone()));
    let tunnel_mode = match &config.named_tunnel {
        Some(t) => {
            // The dashboard forwards to a fixed port; a different one here would break the address.
            if addr.port() != config.port {
                log!("Внимание: порт {} занят, облако на {} — постоянный адрес не будет работать, пока порт не освободится", config.port, addr.port());
            }
            tunnel::Mode::Named { token: t.token.clone(), url: t.url.clone() }
        }
        None => tunnel::Mode::Quick,
    };

    let tunnel = if args.no_tunnel {
        log!("Туннель отключён (--no-tunnel): сайт доступен только с этого компьютера");
        None
    } else {
        let bin_dir = paths.bin.clone();
        let telegram = config.telegram.clone().map(Arc::new);
        let port = addr.port();
        let public_url = public_url.clone();
        let (status, config_dir) = (status.clone(), config_dir.clone());
        let background = args.background;
        Some(tokio::spawn(async move {
            let bin = match tunnel::ensure_cloudflared(&bin_dir).await {
                Ok(b) => b,
                Err(e) => {
                    log!("Не удалось получить cloudflared: {e:#}. Сайт работает только локально.");
                    return;
                }
            };
            log!("Открываю туннель Cloudflare…");
            tunnel::run(bin, port, tunnel_mode, move |url| {
                *public_url.lock().unwrap() = Some(url.clone());
                {
                    let mut st = status.lock().unwrap();
                    st.url = Some(url.clone());
                    service::write_status(&config_dir, &st);
                }
                log!("Облако доступно по адресу: {url}");
                if !background {
                    show_url(&url);
                }
                if let Some(tg) = telegram.clone() {
                    tokio::spawn(async move {
                        telegram::send_with_retry(&tg, &format!("Облако запущено:\n{url}")).await;
                    });
                }
            })
            .await;
        }))
    };

    // Graceful shutdown waits for open responses; a video left streaming in some tab
    // would keep it waiting forever, so give up after a few seconds.
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    let server = axum::serve(listener, router).with_graceful_shutdown(async move {
        shutdown_signal(stop).await;
        let _ = stop_tx.send(true);
    });
    let deadline = async move {
        let _ = stop_rx.wait_for(|stopped| *stopped).await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    };
    tokio::select! {
        res = std::future::IntoFuture::into_future(server) => res?,
        _ = deadline => log!("Не все соединения закрылись, завершаю принудительно"),
    }

    terminal.shutdown();
    // Dropping the task kills cloudflared (kill_on_drop).
    for t in tunnel.into_iter().chain(bot).chain(Some(backups)) {
        t.abort();
        let _ = t.await;
    }
    service::clear_status(&config_dir);
    log!("Остановлено");
    Ok(())
}

/// Ctrl+C, SIGTERM (systemd/launchd stop) or `local-photo --stop`.
async fn shutdown_signal(stop: Arc<Notify>) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
        _ = stop.notified() => {},
    }
    log!("Останавливаюсь…");
}
