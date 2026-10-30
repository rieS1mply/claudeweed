'use strict';
// "Недавно удалённые": everything deleted waits here 30 days.

const SECTION_NAMES = { photos: t('Фото'), files: t('Файлы'), notes: t('Заметки') };

function renderTrash(list) {
  const content = $('#content');
  const items = list.items;
  $('#summary').textContent = t('Объекты удаляются навсегда через {0} дней', { 0: list.retention_days });
  const action = $('#head-action');
  action.hidden = !items.length;
  action.textContent = t('Очистить');
  headAction = emptyTrash;

  if (!items.length) {
    content.replaceChildren(el('div', { class: 'empty' },
      icon('trash'),
      el('div', { text: t('Корзина пуста') }),
      el('div', { class: 'hint', text: t('Удалённые фото, файлы и заметки хранятся здесь 30 дней') })));
    return;
  }

  const media = items.filter((entry) => entry.section === 'photos');
  const other = items.filter((entry) => entry.section !== 'photos');
  const frag = document.createDocumentFragment();
  if (media.length) {
    const grid = el('div', { class: 'grid' });
    for (const entry of media) {
      const tile = el('button', { class: 'tile', title: entry.name, onclick: () => trashActions(entry) });
      if (entry.kind === 'image') {
        const img = el('img', { src: api.trashThumbUrl(entry), alt: entry.name, loading: 'lazy', decoding: 'async', draggable: 'false' });
        img.onload = () => img.classList.add('loaded');
        img.onerror = () => img.replaceWith(el('div', { class: 'placeholder' }, icon('image'), splitName(entry.name)[1].toUpperCase()));
        tile.append(img);
      } else {
        tile.append(el('div', { class: 'placeholder' }, icon(entry.kind === 'video' ? 'play' : 'image'), splitName(entry.name)[1].toUpperCase()));
      }
      tile.append(el('span', { class: 'days-left', text: daysLeft(entry.days_left) }));
      grid.append(tile);
    }
    frag.append(el('section', { class: 'month-block' },
      el('div', { class: 'month' }, el('span', { class: 'month-chip glass' }, t('Фото и видео'), el('span', { class: 'count', text: String(media.length) }))),
      grid));
  }
  if (other.length) {
    frag.append(el('section', { class: 'month-block' },
      el('div', { class: 'month' }, el('span', { class: 'month-chip glass' }, t('Файлы и заметки'), el('span', { class: 'count', text: String(other.length) }))),
      el('div', { class: 'files' }, ...other.map((entry) => {
        const ext = splitName(entry.name)[1];
        return el('div', { class: 'file-row', onclick: () => trashActions(entry) },
          el('div', { class: 'file-icon', text: entry.section === 'notes' ? '' : ext.slice(0, 4) || '—' },
            entry.section === 'notes' ? icon('note') : null),
          el('div', { class: 'file-main' },
            el('div', { class: 'file-name', text: entry.section === 'notes' ? t('Заметка') : entry.name }),
            el('div', { class: 'file-meta', text: `${entry.version ? t('прошлая версия до {0}', { 0: fmtDate(entry.deleted_at) }) : SECTION_NAMES[entry.section]} · ${fmtSize(entry.size)} · ${daysLeft(entry.days_left)}` })),
          el('button', { class: 'btn round', title: t('Действия'), onclick: (e) => { e.stopPropagation(); trashActions(entry); } }, icon('more')));
      }))));
  }
  content.replaceChildren(frag);
}

function daysLeft(n) {
  return `${n} ${plural(n, 'день', 'дня', 'дней')}`;
}

function trashActions(entry) {
  return actionSheet(t('{0} — удалится через {1}', { 0: entry.section === 'notes' ? t('Заметка') : entry.name, 1: daysLeft(entry.days_left) }), [
    { label: t('Восстановить'), run: () => restoreFromTrash(entry) },
    { label: t('Удалить навсегда'), destructive: true, run: () => purgeFromTrash(entry) },
  ]);
}

async function restoreFromTrash(entry) {
  try {
    await api.restore(entry.id);
  } catch (e) {
    return toast(e.message);
  }
  toast(t('Восстановлено в «{0}»', { 0: SECTION_NAMES[entry.section] }));
  state.data[entry.section] = null;
  await load('trash');
}

async function purgeFromTrash(entry) {
  if (!(await confirmDialog(t('Это действие нельзя отменить.'), { title: t('Удалить навсегда?') }))) return;
  try {
    await api.purge(entry.id);
  } catch (e) {
    return toast(e.message);
  }
  await load('trash');
}

async function emptyTrash() {
  const n = state.data.trash?.items.length || 0;
  if (!n) return;
  if (!(await confirmDialog(`${n} ${plural(n, 'объект будет удалён', 'объекта будут удалены', 'объектов будут удалены')} ${t('навсегда. Это нельзя отменить.')}`,
    { title: t('Очистить корзину?'), ok: t('Очистить') }))) return;
  try {
    await api.emptyTrash();
  } catch (e) {
    return toast(e.message);
  }
  toast(t('Корзина очищена'));
  await load('trash');
}
