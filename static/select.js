'use strict';
// Selecting several photos/files, like in iOS Photos: "Выбрать" or a long press starts it,
// tap toggles, a swipe across tiles selects the whole range, the bottom bar acts on all.

const sel = {
  on: false,
  keys: new Set(),     // item paths
  drag: null,          // range selection in progress
  suppressClick: false,
};
const nodeItem = new WeakMap();   // tile/row element -> item
const LONG_PRESS_MS = 450;

const selectable = () => state.section === 'photos' || state.section === 'files';
const selNode = (item) => (state.section === 'photos' ? tiles.get(item) : rows.get(item));
const selectedItems = () => (state.flat || []).filter((i) => sel.keys.has(i.path));

/** Tap on a tile or row. */
function onItemTap(section, item) {
  if (sel.suppressClick) { sel.suppressClick = false; return; }
  if (sel.on) return setSelected(item, !sel.keys.has(item.path), true);
  if (section === 'photos') openViewer(item);
  else openFile(section, item);
}

function setSelected(item, on, update = false) {
  if (on) sel.keys.add(item.path);
  else sel.keys.delete(item.path);
  const node = selNode(item);
  if (node && node.classList.contains('selected') !== on) {
    node.classList.toggle('selected', on);
    if (on) animate(node.querySelector('.check'), [{ transform: 'scale(.4)' }, { transform: 'none' }], 'bouncy', { fill: 'none' });
  }
  if (update) updateSelectionUi();
}

function enterSelection(first) {
  if (!selectable() || sel.on) return;
  sel.on = true;
  sel.keys.clear();
  document.body.classList.add('selecting');
  if (first) setSelected(first, true);
  updateSelectionUi();
  const bar = $('#selbar');
  enter(bar, [{ opacity: 0, transform: 'translateY(30px) scale(.94)' }, { opacity: 1, transform: 'none' }], 'bouncy');
  const dock = document.querySelector('.dock');
  animate(dock, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'translateY(30px)' }], 'snappy', { fill: 'forwards' });
  dock.classList.add('away');
}

function exitSelection() {
  if (!sel.on) return;
  sel.on = false;
  sel.keys.clear();
  sel.drag = null;
  document.body.classList.remove('selecting');
  document.querySelectorAll('.tile.selected, .file-row.selected').forEach((n) => n.classList.remove('selected'));
  leave($('#selbar'), [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'translateY(30px) scale(.94)' }]);
  const dock = document.querySelector('.dock');
  dock.classList.remove('away');
  dock.getAnimations().forEach((a) => a.cancel());
  animate(dock, [{ opacity: 0, transform: 'translateY(30px)' }, { opacity: 1, transform: 'none' }], 'bouncy', { fill: 'none' });
  $('#select-btn').textContent = 'Выбрать';
  showSectionHeader();
  if (state.data[state.section]) render();
}

/** Title, buttons and bar while selecting. Called after every change. */
function updateSelectionUi() {
  if (!sel.on) return;
  const n = sel.keys.size;
  const all = (state.flat || []).length;
  $('#page-title').textContent = n ? `Выбрано: ${n}` : 'Выберите объекты';
  $('#nav-title').textContent = $('#page-title').textContent;
  $('#select-btn').textContent = 'Отменить';
  const action = $('#head-action');
  action.hidden = !all;
  action.textContent = n === all ? 'Снять все' : 'Выбрать все';
  headAction = () => {
    const on = sel.keys.size !== all;
    for (const item of state.flat || []) setSelected(item, on);
    updateSelectionUi();
  };
  const size = selectedItems().reduce((s, i) => s + i.size, 0);
  $('#summary').textContent = n ? fmtSize(size) : 'Нажмите на фото или проведите по ним пальцем';
  for (const b of ['#sel-share', '#sel-download', '#sel-delete']) $(b).disabled = !n;
  $('#sel-share').hidden = !CAN_SHARE_FILES;
}

/** After a re-render (reload, sort change) keep what is still there selected. */
function reapplySelection() {
  if (!sel.on) return;
  const present = new Set((state.flat || []).map((i) => i.path));
  for (const key of [...sel.keys]) if (!present.has(key)) sel.keys.delete(key);
  for (const item of state.flat || []) selNode(item)?.classList.toggle('selected', sel.keys.has(item.path));
  updateSelectionUi();
}

// ---------- gestures: long press, swipe to select a range ----------

function itemAt(x, y) {
  const node = document.elementFromPoint(x, y)?.closest('.tile, .file-row');
  return node ? nodeItem.get(node) : null;
}

function bindSelection() {
  const content = $('#content');
  let press = null;

  content.addEventListener('pointerdown', (e) => {
    sel.suppressClick = false;   // a long press may not be followed by a click (iOS)
    if (!selectable() || e.button > 0) return;
    const node = e.target.closest('.tile, .file-row');
    if (!node || e.target.closest('.btn')) return;
    const item = nodeItem.get(node);
    if (!item) return;
    press = { x: e.clientX, y: e.clientY, item, id: e.pointerId, type: e.pointerType, timer: null };
    if (!sel.on) {
      // Long press starts selecting with this item.
      press.timer = setTimeout(() => {
        if (!press) return;
        sel.suppressClick = true;
        navigator.vibrate?.(10);
        enterSelection(press.item);
        press = null;
      }, LONG_PRESS_MS);
      return;
    }
    const flat = state.flat || [];
    sel.drag = {
      started: false, id: e.pointerId,
      anchor: flat.indexOf(item),
      value: !sel.keys.has(item.path),   // the first tile decides: select or deselect
      base: new Set(sel.keys),
    };
  });

  content.addEventListener('pointermove', (e) => {
    if (press && press.timer && Math.hypot(e.clientX - press.x, e.clientY - press.y) > 8) {
      clearTimeout(press.timer);
      press = null;
    }
    const d = sel.drag;
    if (!d || e.pointerId !== d.id || !press) return;
    const dx = e.clientX - press.x, dy = e.clientY - press.y;
    if (!d.started) {
      if (Math.hypot(dx, dy) < 10) return;
      // On touch, a vertical move is a scroll; a sideways one starts the range selection.
      if (press.type === 'touch' && Math.abs(dy) > Math.abs(dx)) { sel.drag = null; return; }
      d.started = true;
      sel.suppressClick = true;
      content.setPointerCapture?.(e.pointerId);
    }
    const item = itemAt(e.clientX, e.clientY);
    const flat = state.flat || [];
    const idx = item ? flat.indexOf(item) : -1;
    if (idx >= 0 && idx !== d.last) {
      d.last = idx;
      const [a, b] = d.anchor < idx ? [d.anchor, idx] : [idx, d.anchor];
      flat.forEach((it, i) => setSelected(it, i >= a && i <= b ? d.value : d.base.has(it.path)));
      updateSelectionUi();
    }
    // Scroll along when the finger reaches the top/bottom edge.
    if (e.clientY > window.innerHeight - 90) window.scrollBy(0, 14);
    else if (e.clientY < 120) window.scrollBy(0, -14);
  });

  const end = () => {
    if (press?.timer) clearTimeout(press.timer);
    press = null;
    if (sel.drag?.started) setTimeout(() => { sel.suppressClick = false; }, 0);
    sel.drag = null;
  };
  content.addEventListener('pointerup', end);
  content.addEventListener('pointercancel', end);
  // Long press must not open the browser's image menu.
  content.addEventListener('contextmenu', (e) => { if (e.target.closest('.tile') && e.pointerType !== 'mouse') e.preventDefault(); });

  $('#select-btn').addEventListener('click', () => (sel.on ? exitSelection() : enterSelection()));
  $('#sel-delete').addEventListener('click', deleteSelected);
  $('#sel-download').addEventListener('click', downloadSelected);
  $('#sel-share').addEventListener('click', shareSelected);
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && sel.on && $('#sheet').hidden && $('#modal').hidden) exitSelection();
  });
}

// ---------- actions on the selection ----------

async function deleteSelected() {
  const section = state.section;
  const items = selectedItems();
  const n = items.length;
  if (!n) return;
  const what = section === 'photos' ? plural(n, 'объект', 'объекта', 'объектов') : plural(n, 'файл', 'файла', 'файлов');
  if (!(await confirmDialog(`${n} ${what} будут перемещены в корзину на 30 дней.`, { title: 'Удалить выбранное?' }))) return;

  const ids = [];
  let failed = 0;
  // A few at a time: fast, without flooding a home connection.
  const queue = [...items];
  await Promise.all([1, 2, 3, 4].map(async () => {
    while (queue.length) {
      const item = queue.shift();
      try {
        ids.push((await api.remove(section, item)).trash_id);
        const node = selNode(item);
        if (node) animate(node, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'scale(.6)' }], 'snappy');
      } catch {
        failed++;
      }
    }
  }));
  state.data.trash = null;
  exitSelection();
  await load(section);
  const done = ids.length;
  toast(failed ? `В корзину: ${done}, не удалось: ${failed}` : `Перемещено в корзину: ${done}`, {
    action: 'Отменить',
    onAction: async () => {
      await Promise.all(ids.map((id) => api.restore(id).catch(() => {})));
      state.data.trash = null;
      await load(section);
      toast('Восстановлено');
    },
  });
}

/** One file downloads directly; several are packed into a ZIP on the server. */
async function downloadSelected() {
  const section = state.section;
  const items = selectedItems();
  if (!items.length) return;
  if (items.length === 1) return triggerDownload(api.fileUrl(section, items[0], true), items[0].name);
  try {
    const { id } = await api.zipStart(section, items.map((i) => i.path));
    for (;;) {
      const st = await api.zipStatus(id);
      if (st.state === 'failed') throw new Error(st.error || 'Не удалось собрать архив');
      if (st.state === 'ready') break;
      toast(`Собираю архив… ${st.total ? Math.floor((st.done / st.total) * 100) : 0}%`);
      await new Promise((r) => setTimeout(r, 700));
    }
    toast('Архив готов');
    triggerDownload(api.zipFileUrl(id), 'Облако.zip');
    exitSelection();
  } catch (e) {
    toast(e.message);
  }
}

/** Phones: download everything, then one share sheet — "Сохранить N объектов" puts them in the gallery. */
async function shareSelected() {
  const section = state.section;
  const items = selectedItems();
  if (!items.length) return;
  const total = items.reduce((s, i) => s + i.size, 0);
  if (total > 1.5 * 1024 ** 3) return toast('Слишком много за один раз — выберите меньше (до 1,5 ГБ)');
  try {
    const files = [];
    for (const [i, item] of items.entries()) {
      toast(`Подготовка ${i + 1} из ${items.length}…`);
      files.push(await prepareShare(section, item));
    }
    if (!navigator.canShare?.({ files })) throw new Error('Телефон не может поделиться этими файлами');
    toast(`Готово: ${files.length}`, {
      action: 'Сохранить',
      onAction: async () => {
        try {
          await navigator.share({ files });
          exitSelection();
        } catch (e) {
          if (e.name !== 'AbortError') toast('Не удалось открыть меню «Поделиться»');
        }
      },
    });
  } catch (e) {
    toast(e.message);
  }
}
