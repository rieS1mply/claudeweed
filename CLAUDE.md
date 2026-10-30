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

**Если Claude запущен в веб-терминале самого облака** (оболочка — дочерний процесс
local-photo.exe), `--stop` убьёт и терминал, и самого Claude: облако останется выключенным
(так случилось 2026-10-30). Тогда не выкатывать самому, а попросить владельца выкатить
из обычного окна, или запустить выкатку отдельным процессом, который переживёт остановку:
`Start-Process powershell -WindowStyle Hidden -ArgumentList '-File','deploy.ps1'` со всеми
тремя шагами внутри. Проверено 2026-10-30: запускать через WMI
(`Invoke-CimMethod Win32_Process -MethodName Create`, родитель — WmiPrvSE, вне дерева терминала),
каждую команду облака в скрипте — через `cmd /c "… < NUL >> deploy.log 2>&1"`: в скрытом окне
`--autostart on` иначе вечно ждёт «Нажмите Enter». Запущенное так облако наследует файл
вывода `--autostart on` и держит его открытым до остановки — журнал выкатки каждый раз новый
(`.dev/deploy-<время>.log`), иначе следующая выкатка молча ничего не сделает (так было
2026-10-30). Готовый скрипт — `.dev/deploy.ps1`
(ждёт 60 с, чтобы успеть ответить; после перезапуска разговор восстанавливается в терминале).

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
| `src/telegram.rs`, `src/bot.rs` | Bot API, приём файлов из чата владельца, команды бота |
| `src/agent.rs`, `src/agent_text.rs` | Claude в Telegram (`--agent on`): текст владельца → `claude -p --permission-mode auto --resume <id>` (stream-json), шаги в обновляемом сообщении, очередь, `/stop`, `/new`, `/dir`; разговор и папка в `agent.json`. `agent_text` — Markdown → HTML Telegram, разбиение на части, подписи к шагам |
| `src/terminal.rs` | до 8 именованных PTY-сеансов (ConPTY на Windows), история, deflate-сжатие вывода; REST `/api/terminal/sessions` + WebSocket `/api/terminal?id=N` |
| `src/terminal_store.rs` | сеансы переживают перезапуск: раз в 15 с пишутся в `<настройки>/terminal/` (имя, папка, вывод, запущенный Claude); при старте оболочки возвращаются, в них вводится `claude --resume <id>`. Папку PowerShell сообщает своим приглашением (OSC 7). Убирает переменные `CLAUDE_CODE_*` из окружения оболочек |
| шпаргалка команд | `static/commands.json` — один список для `cmds` в оболочке (`src/cmds.ps1`, входит в `-EncodedCommand`, список — файл `LP_CMDS` из `terminal_store::commands_file`) и для кнопки «?» на странице терминала. Новая команда — строка в JSON (поля g, ge, cmd, ru, en) |
| `src/runner.rs` | запуск кода (как онлайн-компилятор): таблица `LANGS`, поиск инструментов `which`, лимиты, NDJSON-поток `/api/run`, `/api/run/langs` |
| `src/trash.rs`, `src/notes.rs`, `src/zips.rs`, `src/thumbs.rs` | корзина 30 дней, заметки, ZIP, превью (`store_preview` — JPEG для HEIC, присланный браузером) |
| `src/search.rs` | поиск по тексту заметок и текстовых файлов (`/api/search`) |
| `src/shares.rs` | ссылки без пароля: `shares.json` в папке настроек; публичные маршруты `/s/{token}`, `/api/s/{token}/…` в `web.rs` (вне `protected`) |
| `src/funnel.rs` | постоянный адрес через Tailscale Funnel (`--tunnel funnel`, `config.funnel` = https://…ts.net): `tailscale funnel --bg` раз в 10 мин, без cloudflared. Запросы через Funnel приходят с 127.0.0.1 с `X-Forwarded-For` — `auth::via_tunnel`/`client_ip` их учитывают |
| `src/i18n.rs`, `src/lang_en.rs` | язык консоли/Telegram (`--lang`), `say!`, `tr()` |
| свои папки | `storage::folders`/`folder_name` (папки верхнего уровня, кроме годов), `/api/folder`, `/api/file/move`; `prune_empty_dirs` их не удаляет |
| автозагрузка | `POST /api/auto-upload` с `Authorization: Bearer` (ключ `--upload-token new`, в конфиге SHA-256), вне `protected`; повтор (имя+размер в месяце) пропускается |
| `static/app.js` | ядро интерфейса, поиск, клавиши; `editor.js` (фото + редактор кода), `runner.js` (запуск кода, песочница, шаблоны `RUN_TEMPLATES`), `notes.js`, `trash.js`, `select.js`, `terminal.js` — разделы; `highlight.js` — сторонний, не править |
| `static/i18n.js`, `static/lang-en.js` | подключаются первыми: `t()`, `tServer()`, `plural()`, `translatePage()` (переводит тексты из HTML) |
| `static/extras.js` | свои папки, ссылки, HEIC-превью из браузера, просмотр PDF/аудио/Markdown, поиск по содержимому; `markdown.js` — Markdown → DOM без innerHTML |
| `static/share.html`, `share.js`, `share.css` | страница для получателя ссылки |
| `static/start.js` | подключается последним и запускает приложение |

Новый API-маршрут: обработчик в `web.rs`, регистрация в `router()` внутри `protected`
(требует сессию; не-GET автоматически проверяются на CSRF). Клиент ходит через
`request()` в `app.js`, который ставит заголовок `X-Cloud: 1`.

## Правила, которые нельзя ломать

- Сервер слушает только `127.0.0.1`. Всё снаружи идёт через туннель.
- Любой путь от клиента — только через `storage::resolve` / `safe_join`; имена — через `sanitize_filename`.
- Отдача файлов: inline только фото/видео, остальное `attachment` + `CSP sandbox` (загруженный .html не должен исполняться).
  Исключение — PDF с `view=1` (`serve_pdf`): без sandbox (иначе браузер его не покажет), но
  тип жёстко `application/pdf` + `nosniff`.
- Markdown рендерится только через DOM (`markdown.js`), никакого innerHTML с текстом файла.

## Проверка интерфейса без браузерной панели

Node есть в VS Code: `ELECTRON_RUN_AS_NODE=1 "%LOCALAPPDATA%\Programs\Microsoft VS Code\Code.exe"`
(`--check file.js` — синтаксис). Headless Edge (`msedge --headless=new --remote-debugging-port=…`)
управляется через CDP по WebSocket из этого Node — так проверялась сессия 11 (2026-10-30) (скрипт `cdp.js` был
во временной папке; при нужде написать заново: evaluate + captureScreenshot). Локаль headless Edge
английская — сайт открывается на английском, для русского `localStorage['lp-lang']='ru'`.
Кириллица в аргументах curl из Git Bash портится — запросы к API слать из Python.
- Удаление = перенос в корзину, не `remove_file`.
- Тяжёлая работа с диском/картинками — в `spawn_blocking`; размещение файлов под `place_lock`.
- CSP запрещает inline-скрипты и стили (кроме страницы терминала): никаких `onclick=""` и `<style>` в HTML.
- Терминал, запуск кода и Claude в Telegram включает только владелец на самом компьютере
  (`--terminal on`, `--runner on`, `--agent on`), не код и не агент.
- Бот слушает только чат владельца (`chat_id` из настройки); текст разговора с Claude не
  писать в журнал.
- Новый язык для запуска: строка в `LANGS` (`src/runner.rs`), расширение в `RUN_LANG`,
  подсветка в `RUN_HL` и шаблон в `RUN_TEMPLATES` (`static/runner.js`). Шаблон должен
  печатать «Привет из облака!», «Ввод: …» и «2 + 3 = 5» — так его проверяет тест-скрипт.
- Команды в Bash-инструменте Claude: `$` и `\` внутри perl/sed портятся — правки с ними
  делать через Edit, не через однострочники.

## Стиль

- Тексты для пользователя пишутся по-русски, но **через словарь**: на сайте `t('Текст')` /
  `t('Найдено {0} из {1}', { 0: a, 1: b })` (i18n.js), английский — в `static/lang-en.js`
  (ключ — русский текст). Ошибки API остаются русским текстом, страница переводит их через
  `tServer()` (шаблоны `{x}` в lang-en.js). Консоль — `say!` вместо `println!`, Telegram —
  `i18n::tr()`; английские тексты в `src/lang_en.rs` (`{}` на месте значений). Новый текст —
  сразу строка в словаре. Журнал (`log!`) — только по-русски. Не называть локальные переменные `t`.
- Комментарии и имена в коде — по-английски, коротко, объясняют «почему».
- Окончания строк LF (`.gitattributes`). Новые зависимости — только при реальной нужде.
- Перед коммитом: `cargo test --release` и `cargo clippy --release --all-targets` без предупреждений.
