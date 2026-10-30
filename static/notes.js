'use strict';
// Notes, like iOS Notes: a list grouped by date, a full-screen editor that saves by itself.

const WEEKDAYS = [t('воскресенье'), t('понедельник'), t('вторник'), t('среда'), t('четверг'), t('пятница'), t('суббота')];
const NOTE_SAVE_DELAY = 700;

function startOfDay(d) {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** "Сегодня", "Вчера", "Последние 7 дней", "Последние 30 дней", then "Март 2026". */
function noteGroup(ms) {
  const today = startOfDay(new Date());
  const day = 86400e3;
  if (ms >= today) return t('Сегодня');
  if (ms >= today - day) return t('Вчера');
  if (ms >= today - 7 * day) return t('Последние 7 дней');
  if (ms >= today - 30 * day) return t('Последние 30 дней');
  const d = new Date(ms);
  return `${MONTHS[d.getMonth()]} ${d.getFullYear()}`;
}

function noteDate(ms) {
  const d = new Date(ms);
  const today = startOfDay(new Date());
  if (ms >= today) return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  if (ms >= today - 6 * 86400e3) return WEEKDAYS[d.getDay()];
  return `${pad(d.getDate())}.${pad(d.getMonth() + 1)}.${d.getFullYear()}`;
}

function renderNotes(list) {
  const content = $('#content');
  const n = list.items.length;
  $('#search-box').hidden = !n;
  $('#summary').textContent = n ? `${n} ${plural(n, 'заметка', 'заметки', 'заметок')}` : '';
  if (!n) {
    content.replaceChildren(el('div', { class: 'empty' },
      icon('note'),
      el('div', { text: t('Нет заметок') }),
      el('div', { class: 'hint', text: t('Нажмите «+», чтобы создать. Заметки доступны с любого устройства') })));
    return;
  }
  const found = filterByQuery(list.items, (note) => `${note.title}\n${note.preview || ''}`, (note) => contentSnippet('notes', note.id));
  if (found.length !== n) $('#summary').textContent = t('Найдено {0} из {1}', { 0: found.length, 1: n });
  if (!found.length) return content.replaceChildren(nothingFound());
  const groups = new Map();
  for (const note of found) {
    const g = noteGroup(note.updated);
    if (!groups.has(g)) groups.set(g, []);
    groups.get(g).push(note);
  }
  const frag = document.createDocumentFragment();
  for (const [title, notes] of groups) {
    frag.append(el('section', { class: 'month-block' },
      el('div', { class: 'month' }, el('span', { class: 'month-chip glass' }, title)),
      el('div', { class: 'files notes' }, ...notes.map((note) => el('div', { class: 'note-row', onclick: () => openNote(note.id) },
        el('div', { class: 'note-title', text: note.title === 'Новая заметка' ? t('Новая заметка') : note.title }),
        el('div', { class: 'note-meta' },
          el('span', { class: 'note-date', text: noteDate(note.updated) }),
          el('span', { class: 'note-preview', text: contentSnippet('notes', note.id) ?? (note.preview || t('Нет дополнительного текста')) })))))));
  }
  content.replaceChildren(frag);
}

async function newNote() {
  try {
    const { id } = await api.noteCreate('');
    await openNote(id, { fresh: true });
  } catch (e) {
    toast(e.message);
  }
}

async function openNote(id, { fresh = false } = {}) {
  let note;
  try {
    note = await api.noteGet(id);
  } catch (e) {
    toast(e.message);
    return load('notes');
  }
  const s = { id, text: note.content, base: note.updated, saved: note.content, timer: null, saving: null };

  const save = async (force = false, keepalive = false) => {
    clearTimeout(s.timer);
    if (s.text === s.saved && !force) return;
    const text = s.text;
    $('#doc-status').textContent = t('Сохранение…');
    s.saving = (async () => {
      try {
        const res = await api.noteSave(id, text, force ? null : s.base, keepalive);
        s.base = res.updated;
        s.saved = text;
        if (s.text === text) $('#doc-status').textContent = t('Сохранено');
      } catch (e) {
        if (e.status === 409) return conflict(e.data);
        $('#doc-status').textContent = t('Не сохранено');
        toast(e.message);
      }
    })();
    await s.saving;
  };

  // The same note was changed on another device since we opened it.
  const conflict = async (server) => {
    const keepMine = await confirmDialog(t('Пока вы писали, заметку изменили на другом устройстве.'),
      { title: t('Заметка изменена'), ok: t('Оставить мою'), cancel: t('Загрузить новую'), destructive: false });
    if (keepMine) return save(true);
    s.text = s.saved = server.content;
    s.base = server.updated;
    $('#doc-text').value = server.content;
    $('#doc-status').textContent = t('Обновлено');
  };

  const remove = el('button', { class: 'doc-btn icon-only', title: t('Удалить заметку') }, icon('trash'));
  remove.onclick = async () => {
    if (!(await confirmDialog(t('Заметка будет перемещена в корзину.'), { title: t('Удалить заметку?') }))) return;
    clearTimeout(s.timer);
    try {
      const res = await api.noteDelete(id);
      flushNote = null;
      await closeDocView();
      state.data.trash = null;
      toast(t('Заметка удалена'), { action: t('Отменить'), onAction: () => undoDelete(res.trash_id, 'notes') });
      load('notes');
    } catch (e) {
      toast(e.message);
    }
  };

  // Phones may freeze or kill the tab right after switching apps: save at once, not after the delay.
  // (keepalive requests are limited to 64 KB by browsers)
  flushNote = () => { if (s.text !== s.saved && s.text.trim()) save(false, s.text.length < 60000); };

  await openDocView({
    title: '', text: note.content, mono: false, left: t('‹ Заметки'), right: [remove], slide: 'left',
    status: fresh ? '' : t('Сохранено'),
    onInput: (v) => {
      s.text = v;
      $('#doc-status').textContent = '';
      clearTimeout(s.timer);
      s.timer = setTimeout(save, NOTE_SAVE_DELAY);
    },
    onClose: async () => {
      flushNote = null;
      await s.saving;
      // Like iOS: a note left empty is simply dropped.
      if (!s.text.trim()) {
        clearTimeout(s.timer);
        await api.noteDelete(id, true).catch(() => {});
      } else {
        await save();
      }
      await closeDocView();
      load('notes');
    },
  });
  if (fresh) $('#doc-text').focus();
}

document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'hidden') flushNote?.(); });
window.addEventListener('pagehide', () => flushNote?.());
