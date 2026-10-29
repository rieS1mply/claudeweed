# local-photo (claudeweed) — домашнее облако

Rust-сервер (axum + tokio) + веб-интерфейс на чистом JS без сборщика. Один бинарник,
интерфейс из `static/` встраивается через `rust-embed`. Наружу публикуется туннелем
Cloudflare (quick tunnel), адрес приходит в Telegram. Описание для людей — README.md,
установка на чужой компьютер — INSTALL.md.

**PROJECT_LOG.md** — журнал проекта: экскурс, идеи в очереди и отклонённые, история сессий.
Читать в начале сессии, обновлять в конце каждой.

## Порядок после каждой задачи (просьба владельца, обязательно)

1. Когда задача сделана и проверена (`cargo test`, `cargo clippy`), **сразу выкатить её на
   настоящее облако** командами из раздела «Выкатить новую версию» ниже: остановить программу,
   скопировать новый exe, запустить (`--autostart on`), проверить `--status` и что сайт отвечает
   (`http://127.0.0.1:8787` и адрес trycloudflare). Владелец сам заходит на сайт и проверяет.
   В ответе назвать новый адрес trycloudflare.
2. **Коммит и пуш — самостоятельно**, без отдельной команды, после каждых 2–3 выполненных задач
   (и всегда в конце сессии, если есть незакоммиченное): `git add` нужных файлов,
   коммит с понятным русским описанием, `git push -u origin <ветка>`. Не коммитить `.dev/`,
   `target/`, настройки и секреты из `%APPDATA%\local-photo\`.

## Команды

```sh
cargo build --release                        # → target/release/local-photo(.exe)
cargo test --release                         # юнит-тесты (быстрые, без сети)
cargo clippy --release --all-targets         # должно быть без предупреждений

cargo run --release --example dev_setup      # один раз: тестовая установка в .dev/
cargo run --release -- --dir .dev --no-tunnel --port 8788
# → http://127.0.0.1:8788, логин dev / пароль devpassword, терминал включён
cargo run --release -- --dir .dev --stop     # остановить тестовый экземпляр
```

Правки в `static/` попадают в бинарник только после пересборки. Для быстрой проверки
вёрстки можно открыть `static/index.html` прямо в браузере — будет демо-режим без сервера.

## Настоящее облако на этом компьютере — не трогать при разработке

- Настройки, пароль, токен бота, журнал: `%APPDATA%\local-photo\` (`config.json`, `local-photo.log`).
- Хранилище с фото: `D:\Cloud` (`storage_dir` в config.json).
- Установленная программа: `%LOCALAPPDATA%\Programs\local-photo\local-photo.exe`, автозапуск
  (HKCU\…\Run) указывает на неё, а не на `target/`.
- Для экспериментов всегда `--dir .dev`. Не читать и не выводить `password_hash` и `telegram.token`.

Выкатить новую версию на настоящее облако (Windows, PowerShell):

```powershell
cargo build --release
& "$env:LOCALAPPDATA\Programs\local-photo\local-photo.exe" --stop
Copy-Item target\release\local-photo.exe "$env:LOCALAPPDATA\Programs\local-photo\" -Force
& "$env:LOCALAPPDATA\Programs\local-photo\local-photo.exe" --autostart on   # запускает в фоне
# Не добавлять к этой команде `| Out-Null` и другие конвейеры: фоновое облако держит вывод и команда зависнет.
# Адрес туннеля после запуска: `%APPDATA%/local-photo/status.json` (адрес trycloudflare).
```

После перезапуска адрес trycloudflare меняется (придёт в Telegram).

## Устройство

| Файл | Что внутри |
|---|---|
| `src/main.rs` | разбор флагов, запуск сервера, туннеля и бота, остановка |
| `src/web.rs` | все HTTP-маршруты (`router()`), CSRF, заголовки безопасности, загрузка по частям |
| `src/auth.rs` | сессии (на диске только SHA-256 токенов), защита от подбора пароля |
| `src/totp.rs` | второй фактор входа: коды из приложения (TOTP), `--2fa on` в `main.rs` |
| `src/backup.rs` | ежедневная резервная копия хранилища (`--backup`), состояние в `backup.json` |
| `src/storage.rs` | разделы Фото/Файлы, даты из EXIF/MP4, `sanitize_filename`, `resolve` (защита путей) |
| `src/config.rs` | папки, `config.json`, интерактивная первая настройка |
| `src/service.rs` | автозапуск (Windows Run / launchd / systemd), `status.json`, `--stop` |
| `src/tunnel.rs` | скачивание и перезапуск cloudflared: быстрый туннель или именованный (`--tunnel named`), фильтр повторяющихся ошибок |
| `src/telegram.rs`, `src/bot.rs` | Bot API и приём файлов из чата владельца |
| `src/terminal.rs` | до 8 именованных PTY-сеансов (ConPTY на Windows), история, deflate-сжатие вывода; REST `/api/terminal/sessions` + WebSocket `/api/terminal?id=N` |
| `src/trash.rs`, `src/notes.rs`, `src/zips.rs`, `src/thumbs.rs` | корзина 30 дней, заметки, ZIP, превью |
| `static/app.js` | ядро интерфейса, поиск, клавиши; `editor.js` (фото + редактор кода), `notes.js`, `trash.js`, `select.js`, `terminal.js` — разделы; `highlight.js` — сторонний, не править |
| `static/start.js` | подключается последним и запускает приложение |

Новый API-маршрут: обработчик в `web.rs`, регистрация в `router()` внутри `protected`
(требует сессию; не-GET автоматически проверяются на CSRF). Клиент ходит через
`request()` в `app.js`, который ставит заголовок `X-Cloud: 1`.

## Правила, которые нельзя ломать

- Сервер слушает только `127.0.0.1`. Всё снаружи идёт через туннель.
- Любой путь от клиента — только через `storage::resolve` / `safe_join`; имена — через `sanitize_filename`.
- Отдача файлов: inline только фото/видео, остальное `attachment` + `CSP sandbox` (загруженный .html не должен исполняться).
- Удаление = перенос в корзину, не `remove_file`.
- Тяжёлая работа с диском/картинками — в `spawn_blocking`; размещение файлов под `place_lock`.
- CSP запрещает inline-скрипты и стили (кроме страницы терминала): никаких `onclick=""` и `<style>` в HTML.
- Терминал включает только владелец на самом компьютере (`--terminal on`), не код и не агент.

## Стиль

- Тексты для пользователя (интерфейс, ошибки API, журнал, Telegram) — по-русски.
- Комментарии и имена в коде — по-английски, коротко, объясняют «почему».
- Окончания строк LF (`.gitattributes`). Новые зависимости — только при реальной нужде.
- Перед коммитом: `cargo test --release` и `cargo clippy --release --all-targets` без предупреждений.
