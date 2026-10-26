//! Personal home cloud.
//!
//! Run it on the computer that keeps the photos. It serves a password-protected
//! web interface on 127.0.0.1 and publishes it through a free Cloudflare quick
//! tunnel; the public address is printed and sent to Telegram on every start.

mod auth;
mod bot;
mod config;
mod logging;
mod notes;
mod service;
mod storage;
mod telegram;
mod thumbs;
mod trash;
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
}

const HELP: &str = "\
local-photo — домашнее облако для фото и файлов

Использование: local-photo [параметры]

  (без параметров)    запустить облако (при первом запуске — настройка)
  --status            работает ли облако и по какому адресу
  --stop              остановить облако, работающее в фоне
  --autostart on|off  запускать облако само при входе в систему (в фоне) / не запускать
  --setup             выбрать папку хранилища, задать логин, пароль и Telegram заново
  --port <порт>       локальный порт (по умолчанию 8787)
  --no-tunnel         не открывать доступ из интернета (только http://127.0.0.1)
  --dir <папка>       держать настройки в этой папке (переносная/тестовая установка)
  -h, --help          эта справка
";

fn parse_args() -> Result<Args> {
    let mut args = Args { dir: None, setup: false, no_tunnel: false, port: None, background: false, status: false, stop: false, autostart: None };
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
            "--autostart" => {
                args.autostart = Some(match it.next().as_deref() {
                    Some("on") => true,
                    Some("off") => false,
                    _ => bail!("после --autostart нужно on или off"),
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
    println!("  Журнал:      {}", config_dir.join("local-photo.log").display());
    service::pause_if_own_window();
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
    let router = web::router(app);

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
            tunnel::run(bin, port, move |url| {
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

    // Dropping the task kills cloudflared (kill_on_drop).
    for t in tunnel.into_iter().chain(bot) {
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
