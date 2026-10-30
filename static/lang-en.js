'use strict';
// English texts. Key: the Russian text as written in the code (see i18n.js).
// {0}, {n}…: placeholders; keep them. Server error texts use {x} (several: {a}, {b}).

I18N.en = {
  // months, days, units
  'Январь': 'January', 'Февраль': 'February', 'Март': 'March', 'Апрель': 'April', 'Май': 'May', 'Июнь': 'June',
  'Июль': 'July', 'Август': 'August', 'Сентябрь': 'September', 'Октябрь': 'October', 'Ноябрь': 'November', 'Декабрь': 'December',
  'января': 'January', 'февраля': 'February', 'марта': 'March', 'апреля': 'April', 'мая': 'May', 'июня': 'June',
  'июля': 'July', 'августа': 'August', 'сентября': 'September', 'октября': 'October', 'ноября': 'November', 'декабря': 'December',
  'воскресенье': 'Sunday', 'понедельник': 'Monday', 'вторник': 'Tuesday', 'среда': 'Wednesday', 'четверг': 'Thursday',
  'пятница': 'Friday', 'суббота': 'Saturday',
  'Б': 'B', 'КБ': 'KB', 'МБ': 'MB', 'ГБ': 'GB', 'ТБ': 'TB',
  '{0} мс': '{0} ms', '{0} с': '{0} s',

  // plural forms (one / many)
  'файл': 'file', 'файлов': 'files', 'объект': 'item', 'объектов': 'items', 'заметка': 'note', 'заметок': 'notes',
  'день': 'day', 'дней': 'days', 'объект будет удалён': 'item will be deleted', 'объектов будут удалены': 'items will be deleted',

  // common
  'Облако': 'Cloud', 'Войти': 'Log in', 'Выйти': 'Log out', 'Логин': 'Username', 'Пароль': 'Password',
  'Код из приложения': 'Code from the app', 'демо': 'demo', 'Язык': 'Language',
  'Удалить?': 'Delete?', 'Удалить': 'Delete', 'Отмена': 'Cancel', 'Готово': 'Done', 'Ошибка': 'Error', 'Ошибка {0}': 'Error {0}',
  'Сохранить': 'Save', 'Закрыть': 'Close', 'Закрыть?': 'Close?', 'Скачать': 'Download', 'Открыть': 'Open',
  'Переименовать': 'Rename', 'Переименовано': 'Renamed', 'Отменить': 'Undo', 'Понятно': 'OK', 'Ещё': 'More',
  'Назад': 'Back', 'Вперёд': 'Forward', 'Скрыть': 'Hide', 'Поиск': 'Search', 'Действия': 'Actions',
  'Нет соединения с сервером': 'No connection to the server', 'Нет соединения': 'No connection', 'Требуется вход': 'Please log in',
  'Неверный логин или пароль': 'Wrong username or password',

  // sections and lists
  'Фото': 'Photos', 'Файлы': 'Files', 'Заметки': 'Notes', 'Корзина': 'Trash', 'Загрузить': 'Upload',
  'Ничего не найдено': 'Nothing found', 'По запросу «{0}»': 'For “{0}”', 'Другое': 'Other',
  'Сортировка': 'Sort', 'По дате съёмки': 'By date taken', 'По дате загрузки': 'By date uploaded', 'По дате создания': 'By date created',
  'фото и видео': 'photos and videos', 'свободно {0}': '{0} free', 'Здесь пока пусто': 'Nothing here yet',
  'Нажмите «+» или перетащите файлы в окно': 'Tap “+” or drop files into the window',
  'Найдено {0} из {1}': 'Found {0} of {1}', 'загружен {0}': 'uploaded {0}', 'загружено {0}': 'uploaded {0}',
  'Сохранить в галерею': 'Save to gallery', 'Поделиться': 'Share',
  'в меню выберите «Сохранить изображение/видео»': 'in the menu choose “Save Image/Video”',
  'Не удалось открыть меню «Поделиться»': 'Could not open the Share menu', 'Подготовка файла…': 'Preparing the file…', 'Файл готов': 'File ready',
  'Редактировать': 'Edit', 'Редактировать фото': 'Edit photo',
  '«{0}» будет перемещён в корзину. Его можно восстановить в течение 30 дней.': '“{0}” will be moved to the trash. You can restore it within 30 days.',
  'Перемещено в корзину': 'Moved to the trash',
  'Этот браузер не показывает {ext}. Превью появится, когда галерею откроют на iPhone или в Safari.':
    'This browser can’t show {ext}. A preview appears once the gallery is opened on an iPhone or in Safari.',
  'Этот формат браузер показать не может — скачайте файл.': 'The browser can’t show this format — download the file.',
  'Выбрать': 'Select', 'Выбрано: {0}': 'Selected: {0}', 'Выберите объекты': 'Select items', 'Снять все': 'Deselect all',
  'Выбрать все': 'Select all', 'Нажмите на фото или проведите по ним пальцем': 'Tap photos or swipe across them',
  '{0} {1} будут перемещены в корзину на 30 дней.': '{0} {1} will be moved to the trash for 30 days.',
  'Удалить выбранное?': 'Delete selected?', 'В корзину: {0}, не удалось: {1}': 'To the trash: {0}, failed: {1}',
  'Перемещено в корзину: {0}': 'Moved to the trash: {0}', 'Восстановлено': 'Restored',
  'Не удалось собрать архив': 'Could not build the archive', 'Собираю архив… {0}%': 'Building the archive… {0}%',
  'Архив готов': 'Archive ready', 'Облако.zip': 'Cloud.zip',
  'Слишком много за один раз — выберите меньше (до 1,5 ГБ)': 'Too much at once — select less (up to 1.5 GB)',
  'Подготовка {0} из {1}…': 'Preparing {0} of {1}…', 'Телефон не может поделиться этими файлами': 'The phone can’t share these files',
  'В галерею': 'To gallery', 'В папку': 'To folder',

  // uploads
  'в очереди': 'queued', 'Загрузка {0} из {1}': 'Uploading {0} of {1}', 'Загружено {0} из {1}': 'Uploaded {0} of {1}',
  'Загрузка ещё идёт': 'Still uploading', '→ Фото': '→ Photos', '→ Файлы': '→ Files', 'Вставка': 'Pasted',
  'Загрузка': 'Upload', 'Отпустите, чтобы загрузить': 'Drop to upload',

  // own folders
  'Все': 'All', 'Новая папка': 'New folder', 'Новый альбом': 'New album', 'Создать': 'Create',
  'Переименовать папку': 'Rename folder', 'Удалить папку': 'Delete folder', 'только пустую': 'only if empty', 'Папка удалена': 'Folder deleted',
  'Новая папка…': 'New folder…', 'Убрать из папки': 'Take out of the folder', 'обратно в раздел по дате': 'back to the date view',
  'Переложить в папку': 'Move to folder', 'Не удалось переложить: {n}': 'Could not move: {n}', 'Переложено в «{f}»': 'Moved to “{f}”',
  'Убрано из папки': 'Taken out of the folder', 'Папка пуста': 'The folder is empty',
  'Загрузите файлы сюда («+») или переложите: «⋯» у файла → «Переложить в папку»':
    'Upload files here (“+”) or move them: “⋯” next to a file → “Move to folder”',

  // share links
  'Ссылка без пароля': 'Link without password', 'на 1, 7 или 30 дней': 'for 1, 7 or 30 days', 'Действует {x}': 'Valid for {x}',
  '1 день': '1 day', '7 дней': '7 days', '30 дней': '30 days',
  'Ссылка скопирована': 'Link copied', 'Ссылка': 'Link', 'Отправить': 'Send', 'Ссылка готова': 'Link ready', 'Скопировать': 'Copy',
  'Ссылок нет. Создать: «⋯» у файла → «Ссылка без пароля»': 'No links. To make one: “⋯” next to a file → “Link without password”',
  'Ссылки без пароля': 'Links without password', '{n} файлов': '{n} files', 'до {d}': 'until {d}',
  'Скопировать ссылку': 'Copy link', 'Отозвать': 'Revoke', 'Ссылка больше не действует': 'The link no longer works',
  'ссылка действует до {d}': 'link valid until {d}', 'Ссылка не открывается': 'The link doesn’t open',
  'Файлы по этой ссылке больше не лежат там, где были.': 'The files of this link are no longer where they were.',

  // viewers
  'Править': 'Edit',

  // photo editor
  'Яркость': 'Brightness', 'Контраст': 'Contrast', 'Насыщенность': 'Saturation', 'Теплота': 'Warmth', 'Свободно': 'Free',
  'Оригинал': 'Original', 'Сбросить': 'Reset', 'Коррекция': 'Adjust', 'Обрезка': 'Crop', 'Повернуть': 'Rotate', 'Отразить': 'Flip',
  'Браузер не может открыть этот формат': 'The browser can’t open this format',
  'Фото слишком большое для этого браузера — сохранится уменьшенная копия, оригинал не тронется':
    'The photo is too big for this browser — a smaller copy will be saved, the original stays as is',
  'Этот формат браузер сохранить не может — будет создана копия в JPG': 'The browser can’t save this format — a JPG copy will be made',
  'оригинал уйдёт в корзину': 'the original goes to the trash', 'Сохранить как копию': 'Save as a copy',
  'Не удалось подготовить изображение': 'Could not prepare the image', 'Сохранение…': 'Saving…', 'ред.': 'edited',
  'Копия сохранена': 'Copy saved', 'Сохранено. Оригинал — в корзине': 'Saved. The original is in the trash',
  'Конвертировать в {0}': 'Convert to {0}', 'без сжатия — файл станет в несколько раз больше': 'lossless — the file gets several times bigger',
  'Конвертирую в {0}…': 'Converting to {0}…', 'Этот браузер не умеет сохранять {0}': 'This browser can’t save {0}',
  'Готово: {0} · {1} (было {2})': 'Done: {0} · {1} (was {2})', 'Готово: {0}': 'Done: {0}', 'Не получилось прочитать JSON': 'Could not read the JSON',

  // text editor
  'Текст': 'Text', '{0} из {1}': '{0} of {1}', 'нет': 'none', 'Заменено: {0}': 'Replaced: {0}', 'Стр {0}, кол {1}': 'Ln {0}, Col {1}',
  'выделено {0}': '{0} selected', 'Перейти к строке (1–{0})': 'Go to line (1–{0})', 'Перейти': 'Go',
  'Прошлых версий нет: они появляются после сохранения': 'No earlier versions: they appear after saving',
  'Версия до {0}': 'Version before {0}', 'Открыть в редакторе': 'Open in the editor',
  'текст заменится, сохранить — кнопкой «Сохранить»': 'replaces the text; keep it with “Save”',
  'Версия загружена — Ctrl+Z вернёт текущий текст': 'Version loaded — Ctrl+Z brings back the current text',
  'История: {0}': 'History: {0}', 'До {0}': 'Before {0}', 'Файл слишком большой для редактора — скачайте его': 'The file is too big for the editor — download it',
  'Сохранено': 'Saved', 'История': 'History', 'Прошлые версии файла': 'Earlier versions of the file', 'Изменено': 'Edited',
  'Изменения не сохранены.': 'Changes are not saved.', 'Закрыть без сохранения?': 'Close without saving?',
  'Найти': 'Find', 'Строка': 'Line', 'Перенос': 'Wrap', 'Заменить': 'Replace', 'Отступ': 'Indent',
  'Найти и заменить (Ctrl+F, Ctrl+H)': 'Find and replace (Ctrl+F, Ctrl+H)', 'Перейти к строке (Ctrl+G)': 'Go to line (Ctrl+G)',
  'Перенос строк (Alt+Z)': 'Wrap lines (Alt+Z)', 'Мельче (Ctrl+−)': 'Smaller (Ctrl+−)', 'Крупнее (Ctrl+=)': 'Bigger (Ctrl+=)',
  'Мельче': 'Smaller', 'Крупнее': 'Bigger', 'Предыдущее (Shift+Enter)': 'Previous (Shift+Enter)', 'Следующее (Enter)': 'Next (Enter)',
  'Учитывать регистр': 'Match case', 'Регулярное выражение': 'Regular expression', 'Заменить (Ctrl+H)': 'Replace (Ctrl+H)',
  'Закрыть (Esc)': 'Close (Esc)', 'Заменить на': 'Replace with', 'Заменить это совпадение': 'Replace this match',
  'Заменить все совпадения': 'Replace all matches',

  // code runner
  'Остановить (Ctrl+Enter)': 'Stop (Ctrl+Enter)', 'Запустить (Ctrl+Enter)': 'Run (Ctrl+Enter)', 'Стоп': 'Stop', 'Запустить': 'Run',
  'Превышено время — программа остановлена': 'Time limit — the program was stopped',
  'Слишком много вывода — программа остановлена': 'Too much output — the program was stopped',
  'Ошибка компиляции': 'Compilation error', 'Остановлено': 'Stopped', 'Не запущено': 'Not started',
  '{0} не установлен': '{0} is not installed', '{0} не найден на компьютере.\n{1}\n\nПосле установки обновите страницу.':
    '{0} was not found on the computer.\n{1}\n\nReload the page after installing it.',
  'Запуск…': 'Starting…', 'Соединение прервалось': 'Connection lost', 'Остановлено · {0}': 'Stopped · {0}', '[остановлено]': '[stopped]',
  'Компиляция…': 'Compiling…', 'Работает…': 'Running…', 'Готово · код {0} · {1}': 'Done · exit code {0} · {1}',
  '(программа ничего не вывела)': '(the program printed nothing)', 'Развернуть': 'Expand', 'Свернуть': 'Collapse',
  '{0} — не установлен': '{0} — not installed', 'Обновить список (после установки)': 'Refresh the list (after installing)',
  'Новый код — выберите язык': 'New code — choose a language', 'В Файлы': 'To Files', '{0} · черновик': '{0} · draft',
  'Сохранить в «Файлы» как': 'Save to Files as', 'Черновик не сохранён в «Файлы».': 'The draft is not saved to Files.',
  'Новый код — написать и запустить': 'New code — write and run', 'Ввод': 'Input',
  'Что программа прочитает из стандартного ввода': 'What the program reads from standard input', 'Очистить вывод': 'Clear output',
  'Скрыть вывод': 'Hide output', 'Ввод для программы (stdin): каждая строка — одна строка ввода': 'Program input (stdin): one line per line of input',

  // notes
  'Новая заметка': 'New note', 'Заметка не найдена': 'Note not found', 'Сегодня': 'Today', 'Вчера': 'Yesterday',
  'Последние 7 дней': 'Previous 7 days', 'Последние 30 дней': 'Previous 30 days', 'Нет заметок': 'No notes',
  'Нажмите «+», чтобы создать. Заметки доступны с любого устройства': 'Tap “+” to create one. Notes are available on every device',
  'Нет дополнительного текста': 'No additional text', 'Не сохранено': 'Not saved',
  'Пока вы писали, заметку изменили на другом устройстве.': 'While you were typing, the note was changed on another device.',
  'Заметка изменена': 'Note changed', 'Оставить мою': 'Keep mine', 'Загрузить новую': 'Load the new one', 'Обновлено': 'Updated',
  'Удалить заметку': 'Delete note', 'Заметка будет перемещена в корзину.': 'The note will be moved to the trash.',
  'Удалить заметку?': 'Delete note?', 'Заметка удалена': 'Note deleted', '‹ Заметки': '‹ Notes',

  // trash
  'Объекты удаляются навсегда через {0} дней': 'Items are deleted for good after {0} days', 'Очистить': 'Empty',
  'Корзина пуста': 'Trash is empty', 'Удалённые фото, файлы и заметки хранятся здесь 30 дней': 'Deleted photos, files and notes stay here for 30 days',
  'Фото и видео': 'Photos and videos', 'Файлы и заметки': 'Files and notes', 'Заметка': 'Note',
  'прошлая версия до {0}': 'earlier version before {0}', '{0} — удалится через {1}': '{0} — deleted in {1}',
  'Восстановить': 'Restore', 'Удалить навсегда': 'Delete for good', 'Восстановлено в «{0}»': 'Restored to “{0}”',
  'Это действие нельзя отменить.': 'This can’t be undone.', 'Удалить навсегда?': 'Delete for good?',
  'навсегда. Это нельзя отменить.': 'for good. This can’t be undone.', 'Очистить корзину?': 'Empty the trash?', 'Корзина очищена': 'Trash emptied',

  // terminal
  'Терминал': 'Terminal', 'сжатие': 'compression', 'Получено {0}, без сжатия было бы {1}': 'Received {0}, without compression it would be {1}',
  'Открыто максимум сеансов ({0}). Закройте ненужный.': 'The maximum number of sessions ({0}) is open. Close one you don’t need.',
  'Сеанс завершён.': 'Session ended.', 'Начать заново': 'Start again', 'Закрыть сеанс': 'Close session', 'Новый сеанс': 'New session',
  'Пустое окно. Откройте в нём сеанс:': 'Empty window. Open a session in it:', 'подключено окон: {0}': 'windows attached: {0}',
  'Двойное нажатие — переименовать': 'Double-click to rename', 'Название сеанса': 'Session name',
  'Закрыть «{0}» и всё, что в нём запущено?': 'Close “{0}” and everything running in it?',
  'Терминал выключен. Включить его можно только на самом компьютере:<br><code>local-photo --terminal on</code>':
    'The terminal is off. It can only be turned on at the computer itself:<br><code>local-photo --terminal on</code>',
  'Закрыть оболочку в этом окне и всё, что в ней запущено, и начать заново?': 'Close the shell in this window and everything running in it, and start again?',
  'Перезапустить': 'Restart', 'Окна: 1 / 2 / 4': 'Windows: 1 / 2 / 4', 'Перезапустить оболочку в этом окне': 'Restart the shell in this window',
  'Сеансы': 'Sessions', 'Клавиши': 'Keys',

  // demo mode
  'В демо-режиме архивы не создаются': 'Archives are not made in the demo', 'В демо-режиме недоступно': 'Not available in the demo',
  'В демо-режиме ссылки не создаются': 'Links are not made in the demo',

  // server errors (tServer matches {x}-style keys as patterns)
  'Файл не найден': 'File not found', 'Запрос отклонён': 'Request refused',
  'Слишком много попыток. Попробуйте через {x} мин.': 'Too many attempts. Try again in {x} min.',
  'Введите код из приложения': 'Enter the code from the app', 'Неверный или устаревший код': 'Wrong or expired code',
  'Не удалось открыть сеанс': 'Could not open a session', 'Сеанс не найден или пустое имя': 'Session not found or empty name',
  'Сеанс не найден': 'Session not found', 'Слишком большой файл для запуска': 'The file is too big to run', 'Неизвестный раздел': 'Unknown section',
  'Не удалось удалить: {x}': 'Could not delete: {x}', 'Не удалось удалить': 'Could not delete',
  'Не удалось переименовать: {x}': 'Could not rename: {x}', 'Не удалось сохранить: {x}': 'Could not save: {x}', 'Не удалось сохранить': 'Could not save',
  'Выберите от 1 до 5000 файлов': 'Select 1 to 5000 files', 'Некоторые файлы не найдены — обновите страницу': 'Some files were not found — reload the page',
  'Архив не найден': 'Archive not found', 'Архив ещё не готов': 'The archive is not ready yet',
  'Не удалось создать заметку: {x}': 'Could not create the note: {x}', 'Заметка слишком большая': 'The note is too big',
  'Заметка изменена на другом устройстве': 'The note was changed on another device', 'В корзине такого нет': 'Not in the trash',
  'Не удалось восстановить: {x}': 'Could not restore: {x}',
  'Не хватает места на диске: свободно {a} ГБ, файл {b} ГБ': 'Not enough disk space: {a} GB free, the file is {b} GB',
  'Папка не найдена': 'Folder not found', 'Не удалось начать загрузку: {x}': 'Could not start the upload: {x}',
  'Загрузка не найдена': 'Upload not found', 'Смещение не совпадает': 'Offset mismatch', 'Данных больше, чем заявлено': 'More data than announced',
  'Загрузка закрыта': 'Upload closed', 'Ошибка записи: {x}': 'Write error: {x}', 'Файл загружен не полностью': 'The file was not uploaded completely',
  'Такое имя папки нельзя: оно пустое или похоже на год': 'This folder name is not allowed: it is empty or looks like a year',
  'Папка с таким именем уже есть': 'A folder with this name already exists', 'Не удалось создать папку: {x}': 'Could not create the folder: {x}',
  'В папке есть файлы: сначала переложите или удалите их': 'The folder has files in it: move or delete them first',
  'Не удалось удалить папку: {x}': 'Could not delete the folder: {x}', 'Не удалось переложить: {x}': 'Could not move: {x}',
  'Выберите от 1 до 500 файлов': 'Select 1 to 500 files', 'Ссылка не найдена': 'Link not found',
  'Ссылка не действует: срок истёк или её отозвали': 'The link no longer works: it expired or was revoked',
  'Превью нужно только для HEIC и похожих форматов': 'Previews are only for HEIC and similar formats',
  'Превью не принято: {x}': 'Preview refused: {x}', 'Превью не принято': 'Preview refused', 'Неверный ключ загрузки': 'Wrong upload key',
  'Не хватает места на диске': 'Not enough disk space', 'Пустой файл': 'Empty file', 'Файл не принят: {x}': 'File refused: {x}',
  'Шпаргалка команд': 'Command cheat sheet', 'Поиск: git, удалить, папка…': 'Search: git, delete, folder…',
  'Нажмите на команду — она появится в терминале. В самом терминале то же: cmds': 'Tap a command to type it into the terminal. In the terminal itself: cmds',
};
