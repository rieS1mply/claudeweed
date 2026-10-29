'use strict';
// "Недавно удалённые": everything deleted waits here 30 days.

const SECTION_NAMES = { photos: 'Фото', files: 'Файлы', notes: 'Заметки' };

function renderTrash(list) {
  const content = $('#content');
  const items = list.items;
  $('#summary').textContent = `Объекты удаляются навсегда через ${list.retention_days} дней`;
  const action = $('#head-action');
  action.hidden = !items.length;
  action.textContent = 'Очистить';
  headAction = emptyTrash;

  if (!items.length) {
    content.replaceChildren(el('div', { class: 'empty' },
      icon('trash'),
      el('div', { text: 'Корзина пуста' }),
      el('div', { class: 'hint', text: 'Удалённые фото, файлы и заметки хранятся здесь 30 дней' })));
    return;
  }

  const media = items.filter((t) => t.section === 'photos');
  const other = items.filter((t) => t.section !== 'photos');
  const frag = document.createDocumentFragment();
  if (media.length) {
    const grid = el('div', { class: 'grid' });
    for (const t of media) {
      const tile = el('button', { class: 'tile', title: t.name, onclick: () => trashActions(t) });
      if (t.kind === 'image') {
        const img = el('img', { src: api.trashThumbUrl(t), alt: t.name, loading: 'lazy', decoding: 'async', draggable: 'false' });
        img.onload = () => img.classList.add('loaded');
        img.onerror = () => img.replaceWith(el('div', { class: 'placeholder' }, icon('image'), splitName(t.name)[1].toUpperCase()));
        tile.append(img);
      } else {
        tile.append(el('div', { class: 'placeholder' }, icon(t.kind === 'video' ? 'play' : 'image'), splitName(t.name)[1].toUpperCase()));
      }
      tile.append(el('span', { class: 'days-left', text: daysLeft(t.days_left) }));
      grid.append(tile);
    }
    frag.append(el('section', { class: 'month-block' },
      el('div', { class: 'month' }, el('span', { class: 'month-chip glass' }, 'Фото и видео', el('span', { class: 'count', text: String(media.length) }))),
      grid));
  }
  if (other.length) {
    frag.append(el('section', { class: 'month-block' },
      el('div', { class: 'month' }, el('span', { class: 'month-chip glass' }, 'Файлы и заметки', el('span', { class: 'count', text: String(other.length) }))),
      el('div', { class: 'files' }, ...other.map((t) => {
        const ext = splitName(t.name)[1];
        return el('div', { class: 'file-row', onclick: () => trashActions(t) },
          el('div', { class: 'file-icon', text: t.section === 'notes' ? '' : ext.slice(0, 4) || '—' },
            t.section === 'notes' ? icon('note') : null),
          el('div', { class: 'file-main' },
            el('div', { class: 'file-name', text: t.section === 'notes' ? 'Заметка' : t.name }),
            el('div', { class: 'file-meta', text: `${t.version ? `прошлая версия до ${fmtDate(t.deleted_at)}` : SECTION_NAMES[t.section]} · ${fmtSize(t.size)} · ${daysLeft(t.days_left)}` })),
          el('button', { class: 'btn round', title: 'Действия', onclick: (e) => { e.stopPropagation(); trashActions(t); } }, icon('more')));
      }))));
  }
  content.replaceChildren(frag);
}

function daysLeft(n) {
  return `${n} ${plural(n, 'день', 'дня', 'дней')}`;
}

function trashActions(t) {
  return actionSheet(`${t.section === 'notes' ? 'Заметка' : t.name} — удалится через ${daysLeft(t.days_left)}`, [
    { label: 'Восстановить', run: () => restoreFromTrash(t) },
    { label: 'Удалить навсегда', destructive: true, run: () => purgeFromTrash(t) },
  ]);
}

async function restoreFromTrash(t) {
  try {
    await api.restore(t.id);
  } catch (e) {
    return toast(e.message);
  }
  toast(`Восстановлено в «${SECTION_NAMES[t.section]}»`);
  state.data[t.section] = null;
  await load('trash');
}

async function purgeFromTrash(t) {
  if (!(await confirmDialog('Это действие нельзя отменить.', { title: 'Удалить навсегда?' }))) return;
  try {
    await api.purge(t.id);
  } catch (e) {
    return toast(e.message);
  }
  await load('trash');
}

async function emptyTrash() {
  const n = state.data.trash?.items.length || 0;
  if (!n) return;
  if (!(await confirmDialog(`${n} ${plural(n, 'объект будет удалён', 'объекта будут удалены', 'объектов будут удалены')} навсегда. Это нельзя отменить.`,
    { title: 'Очистить корзину?', ok: 'Очистить' }))) return;
  try {
    await api.emptyTrash();
  } catch (e) {
    return toast(e.message);
  }
  toast('Корзина очищена');
  await load('trash');
}
