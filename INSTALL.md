# Установка — инструкция для агента Claude

Этот файл для агента (Claude Code и подобных), которого попросили установить домашнее облако
`local-photo` на компьютер пользователя. Человеку достаточно сказать агенту:
«установи облако по INSTALL.md из https://github.com/rieS1mply/greenclaude».

Что получится: программа собирается из исходников, кладётся в постоянное место, человек сам проходит
первую настройку (папка, логин, пароль, Telegram, терминал, автозапуск), облако работает в фоне
и открывается по ссылке `https://…trycloudflare.com`, которая приходит в Telegram.

## Правила для агента

- **Пароли и токены вводит человек, не агент.** Первая настройка интерактивная (пароль вводится
  скрыто). Не подставляй пароль через `printf … |`, не придумывай его и не записывай в файлы и чат.
  Попроси человека запустить настройку в своём окне терминала (шаг 5).
- **Терминал на сайте включается только по явному согласию владельца компьютера.** Он даёт любому,
  кто знает пароль, выполнять команды на этом компьютере. Объясни это своими словами и спроси.
  Если владелец — не тот, кто с тобой говорит (облако ставят «для друга»), включать терминал должен
  сам владелец.
- **`sudo` и системные установщики** могут спросить пароль администратора — это тоже вводит человек.
  Предупреди заранее, что сейчас появится запрос.
- Не публикуй адрес облака нигде, кроме ответа человеку.
- Если шаг упал — покажи человеку точный текст ошибки и не обходи проверки программы
  (например, отказ запускаться без папки хранилища — это защита, а не баг).

## 1. Определи систему

```sh
uname -sm            # macOS / Linux: Darwin arm64, Linux x86_64, Linux aarch64 …
```

На Windows работай в PowerShell (`$env:PROCESSOR_ARCHITECTURE`). Дальше шаги разделены по системам.

## 2. Компилятор, git и Rust

Проверь, что уже есть: `git --version`, `cargo --version`. Ставь только недостающее.

**macOS**

```sh
xcode-select -p || xcode-select --install   # откроется окно — человек нажимает «Установить», ждём окончания
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
. "$HOME/.cargo/env"
```

**Linux** (нужен C-компилятор; команда зависит от дистрибутива, `sudo` спросит пароль)

```sh
sudo apt install -y build-essential git curl        # Debian, Ubuntu, Raspberry Pi OS, Mint
sudo dnf install -y gcc git curl                     # Fedora
sudo pacman -S --needed base-devel git curl          # Arch, Manjaro
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
. "$HOME/.cargo/env"
```

**Windows** (PowerShell; Rust для Windows нужны Build Tools от Microsoft)

```powershell
winget install --id Git.Git -e
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --id Rustlang.Rustup -e
```

После установки открой **новое** окно PowerShell (иначе `cargo` не найдётся в PATH) и проверь
`cargo --version`.

## 3. Исходники

```sh
git clone https://github.com/rieS1mply/greenclaude.git
cd claudeweed
```

Если папка уже есть — `git pull` в ней.

## 4. Сборка и установка в постоянное место

```sh
cargo build --release      # 2–5 минут; на Raspberry Pi — до получаса
```

Автозапуск запоминает **полный путь** к программе, поэтому не запускай её из `target/` (папку можно
случайно удалить `cargo clean`). Скопируй:

**macOS / Linux**

```sh
mkdir -p ~/.local/bin
cp target/release/local-photo ~/.local/bin/local-photo
~/.local/bin/local-photo --help     # проверка: печатает справку
```

**Windows**

```powershell
$dst = "$env:LOCALAPPDATA\Programs\local-photo"
New-Item -ItemType Directory -Force $dst | Out-Null
Copy-Item target\release\local-photo.exe $dst\
& "$dst\local-photo.exe" --help
```

Дальше `local-photo` — это этот установленный путь.

## 5. Первая настройка — делает человек

Скажи человеку открыть **отдельное окно терминала** (Терминал на macOS, PowerShell на Windows)
и выполнить:

```sh
~/.local/bin/local-photo                                   # macOS / Linux
& "$env:LOCALAPPDATA\Programs\local-photo\local-photo.exe"   # Windows
```

Заранее перескажи, о чём программа спросит:

1. **Папка для фото и файлов** — Enter оставляет `~/Облако`; можно перетащить папку в окно.
2. **Логин и пароль** для сайта — пароль не короче 10 символов, сайт открыт в интернет.
3. **Токен Telegram-бота** (можно пропустить Enter): в Telegram написать @BotFather → `/newbot`,
   вставить токен, затем отправить новому боту `/start`. Без бота адрес сайта виден только
   в `local-photo --status` на самом компьютере.
4. **Терминал на сайте** — по умолчанию «нет». См. правила выше.
5. **Автозапуск** — рекомендуй «да»: облако уйдёт в фон, переживёт перезагрузку, окно можно закрыть.

Подожди, пока человек скажет, что закончил.

## 6. Проверка

```sh
local-photo --status
```

Должно быть «Облако работает», адрес `https://…trycloudflare.com` (туннель открывается до минуты),
папка хранилища, «Автозапуск: включён», состояние терминала. Попроси человека открыть адрес на
телефоне и войти.

Если не работает — смотри журнал:

| Система | Журнал и настройки |
|---|---|
| macOS | `~/Library/Application Support/local-photo/local-photo.log` |
| Linux | `~/.config/local-photo/local-photo.log` |
| Windows | `%APPDATA%\local-photo\local-photo.log` |

Частые причины: нет интернета (не скачался `cloudflared`), папка хранилища на отключённом внешнем
диске, корпоративный файрвол режет Cloudflare.

## 7. Обновление до новой версии

```sh
cd claudeweed && git pull && cargo build --release
local-photo --stop                     # сначала остановить: на Windows запущенный файл не заменить
cp target/release/local-photo ~/.local/bin/local-photo      # на Windows — Copy-Item как в шаге 4
local-photo --autostart on             # снова запускает в фоне (если автозапуск был включён)
```

Если автозапуск не используется — вместо последней команды человек запускает `local-photo` сам.
Настройки, пароль и файлы при обновлении сохраняются. Ссылка на сайт после перезапуска новая —
придёт в Telegram.

## 8. Полезные команды

| Команда | Что делает |
|---|---|
| `local-photo --status` | работает ли облако, адрес, QR-код |
| `local-photo --stop` | остановить |
| `local-photo --terminal on` / `off` | включить / выключить терминал на сайте (без перезапуска) — только с согласия владельца |
| `local-photo --autostart on` / `off` | включить / выключить автозапуск |
| `local-photo --setup` | заново пройти настройку (делает человек, как в шаге 5) |

## 9. Удаление

```sh
local-photo --autostart off
local-photo --stop
rm ~/.local/bin/local-photo          # Windows: удалить папку %LOCALAPPDATA%\Programs\local-photo
```

Папку настроек (таблица в шаге 6) удаляй только если человек просит забыть логин и бота.
**Папку хранилища с фото не трогай** — удалить её может только сам человек.

## Что сообщить человеку в конце

- адрес сайта (или что он придёт в Telegram) и что ссылка меняется при каждом перезапуске;
- включён ли автозапуск и терминал;
- где лежат фото (папка хранилища) и как посмотреть адрес: `local-photo --status`.
