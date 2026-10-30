'use strict';
// «Компьютер»: the whole computer, like Finder on a Mac (and like Files on an iPhone).
// Sidebar of places; four views (icons, list with open folders, columns, gallery); Quick Look
// on Space; selection by click, Shift/Ctrl and rubber band; context menu; drag and drop;
// Finder's keys. Server side: src/pc.rs, /api/pc/*.

const $ = (s, root = document) => root.querySelector(s);
const coarse = matchMedia('(pointer: coarse)').matches;
const phoneQuery = matchMedia('(max-width: 720px)');
const isPhone = () => phoneQuery.matches;

const load = (k, fallback) => { try { const v = localStorage.getItem(k); return v === null ? fallback : JSON.parse(v); } catch { return fallback; } };
const save = (k, v) => { try { localStorage.setItem(k, JSON.stringify(v)); } catch { /* private mode */ } };

const VIEWS = ['icons', 'list', 'columns', 'gallery'];

const S = {
  places: null,
  sep: '\\',
  cwd: null,          // folder shown; null = the "Browse" screen (phone)
  entries: [],        // items of cwd
  writable: true,
  free: null,
  view: VIEWS.includes(load('fd-view')) ? load('fd-view') : 'icons',
  sort: load('fd-sort', { key: 'name', dir: 1 }),
  hidden: load('fd-hidden', false) === true,
  preview: load('fd-preview', true) !== false,
  sel: new Set(),     // selected paths
  anchor: null,       // Shift+click starts here
  focus: null,        // keyboard focus (path)
  search: null,       // { q, hits, more } while searching
  expanded: new Set(),  // list view: open folders
  children: new Map(),  // folder path → items (open folders, columns)
  columns: [],        // columns view: folders opened to the right of cwd
  clip: null,         // { mode: 'copy' | 'cut', paths }
  selecting: false,   // phone: select mode
  index: new Map(),   // path → item, for everything on screen
  hist: 0,            // position in the page's history (for the back/forward buttons)
  histMax: 0,
};

// ---------- helpers ----------

function h(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v == null || v === false) continue;
    if (k === 'class') node.className = v;
    else if (k === 'text') node.textContent = v;
    else if (k.startsWith('on')) node.addEventListener(k.slice(2), v);
    else node.setAttribute(k, v === true ? '' : v);
  }
  for (const c of children.flat()) if (c != null && c !== false) node.append(c);
  return node;
}
/** markdown.js builds its nodes with the main app's `el()`; the same thing here. */
const el = h;

const SVG_NS = 'http://www.w3.org/2000/svg';
function svg(paths, cls = 'icon', viewBox = '0 0 24 24') {
  const node = document.createElementNS(SVG_NS, 'svg');
  node.setAttribute('viewBox', viewBox);
  node.setAttribute('class', cls);
  node.setAttribute('aria-hidden', 'true');
  for (const d of [].concat(paths)) {
    const p = document.createElementNS(SVG_NS, 'path');
    p.setAttribute('d', d);
    node.append(p);
  }
  return node;
}

const ICON = {
  home: 'M4 11l8-7 8 7v8a1 1 0 0 1-1 1h-4v-6h-6v6H5a1 1 0 0 1-1-1z',
  desktop: ['M3 5a1 1 0 0 1 1-1h16a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z', 'M8 20h8M12 16v4'],
  doc: ['M6 3h9l4 4v14H6z', 'M9 12h7M9 16h5'],
  download: ['M12 4v12m0 0-5-5m5 5 5-5', 'M4 20h16'],
  image: ['M4 5h16v14H4z', 'M4 16l5-5 4 4 3-3 4 4'],
  video: ['M3 6h13v12H3z', 'M16 10l5-3v10l-5-3'],
  music: ['M9 18V5l11-2v13', 'M9 18a3 3 0 1 1-3-3 3 3 0 0 1 3 3zM20 16a3 3 0 1 1-3-3 3 3 0 0 1 3 3z'],
  drive: ['M3 13l2.5-7h13L21 13v5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z', 'M3 13h18M17 16h.01'],
  cloud: 'M7 18a5 5 0 0 1-.6-9.96A6 6 0 0 1 18 9a4.5 4.5 0 0 1-.5 9H7z',
  folder: 'M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z',
  chevron: 'M9 5l7 7-7 7',
  check: 'M5 12l5 5 9-10',
};

const pad = (n) => String(n).padStart(2, '0');

function fmtSize(bytes) {
  if (bytes == null) return '—';
  const units = [t('Б'), t('КБ'), t('МБ'), t('ГБ'), t('ТБ')];
  let v = bytes, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  const num = i === 0 ? String(v) : v.toFixed(v < 10 ? 1 : 0);
  return `${LANG === 'ru' ? num.replace('.', ',') : num} ${units[i]}`;
}

/** "Сегодня, 14:03" / "Вчера, 09:10" / "3 мар. 2024, 18:00", like Finder. */
function fmtDate(ms) {
  if (!ms) return '—';
  const d = new Date(ms);
  const time = `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  const today = new Date();
  const day = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(today) - day(d)) / 86400000);
  if (diff === 0) return t('Сегодня, {x}', { x: time });
  if (diff === 1) return t('Вчера, {x}', { x: time });
  const date = d.toLocaleDateString(LANG === 'ru' ? 'ru-RU' : 'en-US', { day: 'numeric', month: 'short', year: 'numeric' });
  return `${date}, ${time}`;
}

const extOf = (name) => { const i = name.lastIndexOf('.'); return i > 0 ? name.slice(i + 1).toLowerCase() : ''; };

const KINDS = {
  image: 'jpg jpeg png gif webp bmp tif tiff ico svg',
  raw: 'heic heif avif dng cr2 cr3 nef arw raf orf rw2',
  video: 'mp4 mov m4v webm mkv avi wmv 3gp',
  audio: 'mp3 m4a aac wav ogg oga opus flac weba wma',
  pdf: 'pdf',
  archive: 'zip rar 7z tar gz tgz bz2 xz iso',
  code: 'js mjs ts jsx tsx json rs py c h cpp hpp cs java go rb php sh ps1 psm1 bat cmd html htm css scss xml yml yaml toml ini cfg conf sql lua kt swift vue gradle',
  text: 'txt log csv tsv md markdown rtf nfo env gitignore',
  doc: 'doc docx odt pages',
  sheet: 'xls xlsx ods numbers',
  slides: 'ppt pptx odp key',
  app: 'exe msi appx msix dll lnk url apk dmg',
};
const KIND_OF = {};
for (const [k, list] of Object.entries(KINDS)) for (const e of list.split(' ')) KIND_OF[e] = k;

function kindOf(item) {
  if (item.dir) return 'folder';
  return KIND_OF[extOf(item.name)] || 'file';
}

const KIND_NAMES = {
  folder: 'Папка', image: 'Изображение', raw: 'Фото', video: 'Видео', audio: 'Аудио', pdf: 'PDF',
  archive: 'Архив', code: 'Код', text: 'Текст', doc: 'Документ', sheet: 'Таблица', slides: 'Презентация',
  app: 'Программа', file: 'Файл',
};
function kindName(item) {
  const k = kindOf(item);
  if (k === 'file' && extOf(item.name)) return t('Файл {x}', { x: extOf(item.name).toUpperCase() });
  return t(KIND_NAMES[k]);
}

/** Thumbnails exist for these (the server decodes them). */
const thumbable = (item) => !item.dir && 'jpg jpeg png gif webp bmp tif tiff'.split(' ').includes(extOf(item.name));
/** Opened in Quick Look rather than downloaded. */
const previewable = (item) => !item.dir && ['image', 'raw', 'video', 'audio', 'pdf', 'code', 'text'].includes(kindOf(item)) && extOf(item.name) !== 'rtf';

const enc = encodeURIComponent;
const fileUrl = (item, extra = '') => `/api/pc/file?path=${enc(item.path)}&v=${item.mtime || 0}${extra}`;
const thumbUrl = (item) => `/api/pc/thumb?path=${enc(item.path)}&v=${item.mtime || 0}`;

function join(dir, name) {
  return dir.endsWith(S.sep) ? dir + name : dir + S.sep + name;
}

/** Parent folder, or null at a disk's root. */
function parentOf(path) {
  if (!path) return null;
  const trimmed = path.length > 3 ? path.replace(/[\\/]+$/, '') : path;
  const i = trimmed.lastIndexOf(S.sep);
  if (i < 0 || trimmed === S.sep || /^[A-Za-z]:\\?$/.test(trimmed)) return null;
  const up = trimmed.slice(0, i);
  if (/^[A-Za-z]:$/.test(up)) return up + '\\';
  return up || S.sep;
}

function baseName(path) {
  if (/^[A-Za-z]:\\?$/.test(path) || path === S.sep) return placeName(path) || path;
  const trimmed = path.replace(/[\\/]+$/, '');
  return trimmed.slice(trimmed.lastIndexOf(S.sep) + 1);
}

function placeName(path) {
  const all = S.places ? [...S.places.favorites, ...S.places.cloud] : [];
  const p = all.find((x) => x.path === path);
  if (p) return t(p.name);
  const d = S.places?.drives.find((x) => x.path === path);
  return d ? t('Диск {x}', { x: d.name.replace(/[\\:]/g, '') }) : null;
}

const sameDrive = (a, b) => (/^[A-Za-z]:/.test(a) ? a.slice(0, 2).toUpperCase() === b.slice(0, 2).toUpperCase() : true);

// ---------- server ----------

async function api(method, url, body, raw = false) {
  const opts = { method, credentials: 'same-origin', headers: { 'X-Cloud': '1' } };
  if (body !== undefined) {
    if (raw) opts.body = body;
    else { opts.headers['Content-Type'] = 'application/json'; opts.body = JSON.stringify(body); }
  }
  const res = await fetch(url, opts);
  let data = null;
  try { data = await res.json(); } catch { /* empty answer */ }
  if (!res.ok) {
    if (res.status === 401) location.replace('/');
    const err = new Error(tServer(data?.error) || t('Ошибка {0}', { 0: res.status }));
    err.status = res.status;
    err.data = data;
    throw err;
  }
  return data;
}

async function listDir(path) {
  const data = await api('GET', `/api/pc/list?path=${enc(path)}`);
  data.entries = data.entries.map((e) => ({ ...e, path: join(data.path, e.name) }));
  return data;
}

// ---------- toast, dialogs, menu ----------

let toastTimer = 0;
function toast(msg, isError = false) {
  const box = $('#fd-toast');
  box.textContent = msg;
  box.classList.toggle('error', isError);
  box.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { box.hidden = true; }, isError ? 6000 : 3000);
}
const fail = (e) => toast(e.message || String(e), true);

let dialogOpen = false;
/** Asks for a line of text (opts.value) or just OK/Cancel; resolves the text, true, or null. */
function dialog({ title, text = '', value = null, ok = t('OK'), cancel = true, danger = false, select = null, body = null }) {
  return new Promise((resolve) => {
    const modal = $('#fd-modal'), form = $('#fd-dialog'), input = $('#fd-dialog-input');
    $('#fd-dialog-title').textContent = title;
    const textBox = $('#fd-dialog-text');
    textBox.replaceChildren(...(body ? [body] : []));
    if (!body) textBox.textContent = text;
    textBox.hidden = !text && !body;
    input.hidden = value === null;
    input.value = value ?? '';
    const okBtn = $('#fd-dialog-ok');
    okBtn.textContent = ok;
    okBtn.classList.toggle('danger', danger);
    $('#fd-dialog-cancel').hidden = !cancel || ok === null;
    okBtn.hidden = ok === null;
    modal.hidden = false;
    dialogOpen = true;
    const done = (result) => {
      modal.hidden = true;
      dialogOpen = false;
      form.onsubmit = null;
      $('#fd-dialog-cancel').onclick = null;
      modal.onclick = null;
      document.removeEventListener('keydown', onKey, true);
      $('#fd-view').focus({ preventScroll: true });
      resolve(result);
    };
    const onKey = (e) => { if (e.key === 'Escape') { e.preventDefault(); done(null); } };
    document.addEventListener('keydown', onKey, true);
    form.onsubmit = (e) => { e.preventDefault(); done(value === null ? true : input.value.trim()); };
    $('#fd-dialog-cancel').onclick = () => done(null);
    modal.onclick = (e) => { if (e.target === modal) done(null); };
    if (value !== null) {
      setTimeout(() => {
        input.focus();
        // Like Finder: the name is selected without its extension.
        const [from, to] = select || [0, input.value.length];
        input.setSelectionRange(from, to);
      }, 30);
    } else if (ok !== null) {
      setTimeout(() => okBtn.focus(), 30);
    }
  });
}

/**
 * Context menu at (x, y), or an action sheet from the bottom on a phone.
 * `items`: { label, act, danger, disabled, checked } or '-' for a separator.
 */
function showMenu(items, x, y) {
  const menu = $('#fd-menu'), shade = $('#fd-menu-shade');
  const sheet = isPhone() || (coarse && x == null);
  menu.classList.toggle('fd-sheet', sheet);   // not 'sheet': style.css has its own
  const nodes = [];
  for (const it of items) {
    if (it === '-') { if (nodes.length && nodes.at(-1).className !== 'fd-menu-sep') nodes.push(h('div', { class: 'fd-menu-sep' })); continue; }
    if (it.title) { nodes.push(h('div', { class: 'fd-menu-title', text: it.title })); continue; }
    nodes.push(h('button', {
      type: 'button', role: 'menuitem',
      class: `fd-menu-item${it.danger ? ' danger' : ''}${it.checked ? ' checked' : ''}`,
      disabled: it.disabled,
      onclick: () => { hideMenu(); it.act(); },
    }, it.checked ? svg(ICON.check, 'icon fd-check') : h('span', { class: 'fd-check' }), h('span', { text: it.label }), it.hint ? h('kbd', { text: it.hint }) : null));
  }
  if (nodes.at(-1)?.className === 'fd-menu-sep') nodes.pop();
  if (sheet) nodes.push(h('button', { type: 'button', class: 'fd-menu-item fd-menu-cancel', onclick: hideMenu, text: t('Отмена') }));
  menu.replaceChildren(...nodes);
  menu.hidden = false;
  shade.hidden = false;
  if (!sheet) {
    const r = menu.getBoundingClientRect();
    menu.style.left = `${Math.max(6, Math.min(x, innerWidth - r.width - 6))}px`;
    menu.style.top = `${Math.max(6, Math.min(y, innerHeight - r.height - 6))}px`;
  } else {
    menu.style.left = '';
    menu.style.top = '';
  }
}

function hideMenu() {
  $('#fd-menu').hidden = true;
  $('#fd-menu-shade').hidden = true;
}
$('#fd-menu-shade').addEventListener('pointerdown', (e) => { e.preventDefault(); hideMenu(); });
$('#fd-menu-shade').addEventListener('contextmenu', (e) => { e.preventDefault(); hideMenu(); });

// ---------- what is on screen ----------

function sorted(items) {
  const { key, dir } = S.sort;
  const coll = new Intl.Collator(LANG === 'ru' ? 'ru' : 'en', { numeric: true, sensitivity: 'base' });
  const list = items.filter((i) => S.hidden || !i.hidden);
  return list.sort((a, b) => {
    if (a.dir !== b.dir) return a.dir ? -1 : 1;   // folders first
    let r = 0;
    if (key === 'size') r = a.size - b.size;
    else if (key === 'mtime') r = a.mtime - b.mtime;
    else if (key === 'kind') r = coll.compare(kindName(a), kindName(b));
    return (r || coll.compare(a.name, b.name)) * (key === 'name' ? dir : (r ? dir : 1));
  });
}

/** Items in screen order: what arrows, Shift-ranges and Ctrl+A work on. */
function visibleItems() {
  if (S.search) return S.search.hits;
  if (S.view === 'columns') {
    const col = activeColumn();
    return sorted(col === S.cwd ? S.entries : (S.children.get(col) || []));
  }
  if (S.view === 'list') return flatList().map((r) => r.item).filter(Boolean);
  return sorted(S.entries);
}

/** List view rows with open folders unfolded: { item, depth } (or { loading, depth }). */
function flatList() {
  const rows = [];
  const walk = (items, depth) => {
    for (const item of sorted(items)) {
      rows.push({ item, depth });
      if (item.dir && S.expanded.has(item.path)) {
        const kids = S.children.get(item.path);
        if (kids) walk(kids, depth + 1);
        else rows.push({ loading: true, depth: depth + 1 });
      }
    }
  };
  walk(S.entries, 0);
  return rows;
}

/** The folder new things go into: in columns view the column with the focus. */
function here() {
  if (S.view === 'columns' && !S.search) return activeColumn();
  return S.cwd;
}

function activeColumn() {
  const cols = [S.cwd, ...S.columns];
  if (!S.focus) return cols.at(-1);
  for (let i = cols.length - 1; i >= 0; i--) {
    const items = i === 0 ? S.entries : S.children.get(cols[i]);
    if (items?.some((x) => x.path === S.focus)) return cols[i];
  }
  return cols.at(-1);
}

const selectedItems = () => [...S.sel].map((p) => S.index.get(p)).filter(Boolean);

// ---------- navigation ----------

async function go(path, { push = true, keepSearch = false } = {}) {
  hideMenu();
  if (!keepSearch && S.search) { S.search = null; $('#fd-q').value = ''; }
  if (path === null) {
    S.cwd = null;
    if (push) pushState(null);
    render();
    return;
  }
  const view = $('#fd-view');
  view.classList.add('loading');
  try {
    const data = await listDir(path);
    S.cwd = data.path;
    S.entries = data.entries;
    S.writable = data.writable;
    S.free = data.free;
    S.sel.clear();
    S.anchor = S.focus = null;
    S.columns = [];
    S.expanded.clear();
    S.children.clear();
    if (push) pushState(S.cwd);
    else updateHistoryButtons();
    render();
    view.scrollTop = 0;
  } catch (e) {
    fail(e);
    if (!S.cwd && !isPhone()) go(S.places?.home || S.places?.drives[0]?.path, { push: false });
  } finally {
    view.classList.remove('loading');
  }
}

function pushState(path) {
  S.hist += 1;
  S.histMax = S.hist;
  history.pushState({ path, i: S.hist }, '', path ? `#${enc(path)}` : '#');
  updateHistoryButtons();
}

function updateHistoryButtons() {
  $('#fd-back').disabled = S.hist <= 1;
  $('#fd-fwd').disabled = S.hist >= S.histMax;
}

addEventListener('popstate', (e) => {
  const st = e.state;
  if (st && typeof st.i === 'number') S.hist = st.i;
  go(st ? st.path : pathFromHash(), { push: false });
});

const pathFromHash = () => { try { return decodeURIComponent(location.hash.slice(1)) || null; } catch { return null; } };

/** Reloads what is on screen after a change (keeps selection where items still exist). */
async function refresh() {
  if (!S.cwd) return;
  try {
    const data = await listDir(S.cwd);
    S.entries = data.entries;
    S.writable = data.writable;
    S.free = data.free;
    for (const path of [...S.children.keys()]) {
      try { S.children.set(path, (await listDir(path)).entries); } catch { S.children.delete(path); S.expanded.delete(path); }
    }
    S.columns = S.columns.filter((c) => S.children.has(c));
    if (S.search) await runSearch(S.search.q);
    render();
    const alive = new Set(S.index.keys());
    for (const p of [...S.sel]) if (!alive.has(p)) S.sel.delete(p);
    renderSelection();
  } catch (e) { fail(e); }
}

// ---------- rendering ----------

function render() {
  const phone = isPhone();
  document.body.classList.toggle('fd-phone', phone);
  document.body.classList.toggle('fd-browse', phone && S.cwd === null);
  document.body.classList.toggle('fd-selecting', S.selecting);
  renderPlaces();
  if (S.cwd === null) {
    if (!phone) return;
    $('#fd-title').textContent = t('Обзор');
    return;
  }
  const title = S.search ? t('Поиск: «{x}»', { x: S.search.q }) : baseName(S.cwd);
  $('#fd-title').textContent = title;
  document.title = `${baseName(S.cwd)} — ${t('Компьютер')}`;
  for (const b of document.querySelectorAll('#fd-views button')) {
    b.classList.toggle('on', b.dataset.view === S.view);
    b.setAttribute('aria-checked', String(b.dataset.view === S.view));
  }
  $('#fd-newdir').disabled = !S.writable || Boolean(S.search);
  $('#fd-upload').disabled = !S.writable || Boolean(S.search);
  $('#fd-up').title = parentOf(S.cwd) ? t('Наверх') : t('Обзор');
  renderPath();
  S.index.clear();
  const view = $('#fd-view');
  view.dataset.view = S.search ? 'list' : S.view;
  if (S.search) renderSearch(view);
  else if (S.view === 'list') renderList(view);
  else if (S.view === 'columns' && !phone) renderColumns(view);
  else if (S.view === 'gallery') renderGallery(view);
  else renderIcons(view);
  renderSelection();
  renderPaste();
}

function renderPlaces() {
  if (!S.places) return;
  const group = (title, items) => items.length ? [h('div', { class: 'fd-side-title', text: title }), ...items.map(placeRow)] : [];
  $('#fd-places').replaceChildren(
    ...group(t('Избранное'), S.places.favorites),
    ...group(t('Места'), S.places.drives),
    ...group(t('Облако'), S.places.cloud),
  );
}

function placeRow(p) {
  const active = S.cwd && (S.cwd === p.path);
  const label = p.icon === 'drive' ? t('Диск {x}', { x: p.name.replace(/[\\:]/g, '') }) : t(p.name);
  const row = h('button', { type: 'button', class: `fd-place${active ? ' on' : ''}`, 'data-drop': p.path, title: p.path, onclick: () => go(p.path) },
    svg(ICON[p.icon] || ICON.folder, `icon fd-place-icon pi-${p.icon}`),
    h('span', { class: 'fd-place-name', text: label }));
  if (p.total) {
    const used = 1 - p.free / p.total;
    row.append(h('span', { class: 'fd-place-meta', text: t('{x} свободно', { x: fmtSize(p.free) }) }));
    const bar = h('span', { class: 'fd-place-bar' }, h('span'));
    bar.firstChild.style.width = `${Math.round(used * 100)}%`;
    if (used > 0.9) bar.classList.add('full');
    row.append(bar);
  }
  if (isPhone()) row.append(svg(ICON.chevron, 'icon fd-row-chevron'));
  return row;
}

function renderPath() {
  const crumbs = [];
  for (let p = here() || S.cwd; p; p = parentOf(p)) crumbs.unshift(p);
  const nodes = [];
  crumbs.forEach((p, i) => {
    if (i) nodes.push(svg(ICON.chevron, 'icon fd-crumb-sep'));
    nodes.push(h('button', { type: 'button', class: 'fd-crumb', 'data-drop': p, title: p, onclick: () => go(p) },
      svg(i === 0 ? ICON.drive : ICON.folder, 'icon'), h('span', { text: baseName(p) })));
  });
  const bar = $('#fd-path');
  bar.replaceChildren(...nodes);
  bar.scrollLeft = bar.scrollWidth;
}

/** The icon of an item: a thumbnail for photos, otherwise a folder or a page with its type. */
function itemIcon(item, big) {
  const kind = kindOf(item);
  if (kind === 'folder') {
    const node = svg(ICON.folder, `fd-folder${item.link ? ' link' : ''}`);
    return node;
  }
  const page = h('span', { class: `fd-doc k-${kind}` }, h('span', { class: 'fd-doc-ext', text: (extOf(item.name) || '').slice(0, 4).toUpperCase() }));
  if (big && thumbable(item)) {
    const box = h('span', { class: 'fd-thumb' });
    const img = h('img', { src: thumbUrl(item), alt: '', loading: 'lazy', decoding: 'async', draggable: 'false' });
    img.addEventListener('error', () => box.replaceWith(page), { once: true });
    box.append(img);
    return box;
  }
  return page;
}

function itemAttrs(item, cls) {
  S.index.set(item.path, item);
  return {
    class: `${cls}${item.hidden ? ' dim' : ''}`,
    'data-path': item.path,
    'data-drop': item.dir ? item.path : null,
    draggable: coarse ? null : 'true',
    title: coarse ? null : item.name,
  };
}

function renderIcons(view) {
  const items = sorted(S.entries);
  const grid = h('div', { class: 'fd-grid' });
  for (const item of items) {
    grid.append(h('div', itemAttrs(item, 'fd-item fd-tile'),
      h('div', { class: 'fd-tile-icon' }, itemIcon(item, true)),
      h('div', { class: 'fd-tile-name', text: item.name }),
      isPhone() ? h('div', { class: 'fd-tile-meta', text: item.dir ? fmtDate(item.mtime) : fmtSize(item.size) }) : null));
  }
  view.replaceChildren(items.length ? grid : emptyNote());
}

function emptyNote() {
  return h('div', { class: 'fd-empty' }, svg(ICON.folder, 'fd-empty-icon'), h('div', { text: t('Папка пуста') }),
    S.writable ? h('div', { class: 'fd-empty-hint', text: coarse ? t('Нажмите «⋯», чтобы создать папку или загрузить файлы') : t('Перетащите сюда файлы, чтобы загрузить') }) : null);
}

const COLS = [
  { key: 'name', label: 'Имя' },
  { key: 'mtime', label: 'Дата изменения' },
  { key: 'size', label: 'Размер' },
  { key: 'kind', label: 'Тип' },
];

function listHead() {
  return h('div', { class: 'fd-head' }, ...COLS.map((c) => h('button', {
    type: 'button',
    class: `fd-col-${c.key}${S.sort.key === c.key ? ` on ${S.sort.dir > 0 ? 'up' : 'down'}` : ''}`,
    onclick: () => setSort(c.key),
  }, h('span', { text: t(c.label) }))));
}

function setSort(key) {
  S.sort = S.sort.key === key ? { key, dir: -S.sort.dir } : { key, dir: key === 'name' || key === 'kind' ? 1 : -1 };
  save('fd-sort', S.sort);
  render();
}

function listRow(item, depth, extra = null) {
  const disclosure = item.dir && !S.search && !isPhone()
    ? h('button', { type: 'button', class: `fd-disc${S.expanded.has(item.path) ? ' open' : ''}`, tabindex: '-1', 'aria-label': t('Раскрыть'), onclick: (e) => { e.stopPropagation(); toggleExpand(item); } }, svg(ICON.chevron))
    : h('span', { class: 'fd-disc' });
  const name = h('div', { class: 'fd-col-name' }, disclosure, h('span', { class: 'fd-row-icon' }, itemIcon(item, isPhone())),
    h('span', { class: 'fd-row-text' }, h('span', { class: 'fd-name', text: item.name }),
      isPhone() ? h('span', { class: 'fd-row-sub', text: item.dir ? fmtDate(item.mtime) : `${fmtDate(item.mtime)} — ${fmtSize(item.size)}` }) : null,
      extra ? h('span', { class: 'fd-row-sub', text: extra }) : null));
  name.style.paddingLeft = `${depth * 18}px`;
  return h('div', itemAttrs(item, 'fd-item fd-row'), name,
    h('div', { class: 'fd-col-mtime', text: fmtDate(item.mtime) }),
    h('div', { class: 'fd-col-size', text: item.dir ? '—' : fmtSize(item.size) }),
    h('div', { class: 'fd-col-kind', text: kindName(item) }),
    isPhone() && item.dir ? svg(ICON.chevron, 'icon fd-row-chevron') : null);
}

function renderList(view) {
  const rows = flatList();
  const body = h('div', { class: 'fd-rows' });
  for (const r of rows) {
    if (r.loading) {
      const row = h('div', { class: 'fd-row fd-row-loading' }, h('div', { class: 'fd-col-name', text: t('Загрузка…') }));
      row.firstChild.style.paddingLeft = `${r.depth * 18 + 46}px`;
      body.append(row);
    } else body.append(listRow(r.item, r.depth));
  }
  view.replaceChildren(listHead(), rows.length ? body : emptyNote());
}

async function toggleExpand(item, open = !S.expanded.has(item.path)) {
  if (open === S.expanded.has(item.path)) return;
  if (!open) {
    S.expanded.delete(item.path);
    renderKeepScroll();
    return;
  }
  S.expanded.add(item.path);
  renderKeepScroll();
  if (!S.children.has(item.path)) {
    try { S.children.set(item.path, (await listDir(item.path)).entries); } catch (e) { S.expanded.delete(item.path); fail(e); }
    renderKeepScroll();
  }
}

function renderKeepScroll() {
  const view = $('#fd-view');
  const top = view.scrollTop, left = view.scrollLeft;
  render();
  view.scrollTop = top;
  view.scrollLeft = left;
}

function renderSearch(view) {
  const hits = S.search.hits;
  const body = h('div', { class: 'fd-rows' });
  for (const item of hits) body.append(listRow(item, 0, parentOf(item.path)));
  const note = S.search.busy ? t('Ищу…') : S.search.more ? t('Показаны первые {x} — уточните запрос', { x: hits.length }) : null;
  view.replaceChildren(listHead(), hits.length ? body : h('div', { class: 'fd-empty' }, h('div', { text: S.search.busy ? t('Ищу…') : t('Ничего не найдено') })),
    note && hits.length ? h('div', { class: 'fd-search-note', text: note }) : '');
}

function renderColumns(view) {
  const cols = [S.cwd, ...S.columns];
  const wrap = h('div', { class: 'fd-cols' });
  cols.forEach((path, i) => {
    const items = i === 0 ? S.entries : S.children.get(path);
    const col = h('div', { class: 'fd-colm', 'data-col': path, 'data-drop': path });
    if (!items) col.append(h('div', { class: 'fd-col-loading', text: t('Загрузка…') }));
    else {
      const next = cols[i + 1];
      for (const item of sorted(items)) {
        col.append(h('div', itemAttrs(item, `fd-item fd-crow${item.path === next ? ' open' : ''}`),
          h('span', { class: 'fd-row-icon' }, itemIcon(item, false)),
          h('span', { class: 'fd-name', text: item.name }),
          item.dir ? svg(ICON.chevron, 'icon fd-crow-chevron') : null));
      }
      if (!items.length) col.append(h('div', { class: 'fd-col-loading', text: t('Пусто') }));
    }
    wrap.append(col);
  });
  // A single file chosen in the last column: its preview, like Finder.
  const last = selectedItems();
  if (last.length === 1 && !last[0].dir) wrap.append(h('div', { class: 'fd-colm fd-col-preview' }, previewCard(last[0])));
  view.replaceChildren(wrap);
  requestAnimationFrame(() => { view.scrollLeft = view.scrollWidth; });
}

function renderGallery(view) {
  const items = sorted(S.entries);
  if (!items.length) { view.replaceChildren(emptyNote()); return; }
  if (!S.focus || !items.some((i) => i.path === S.focus)) S.focus = items[0].path;
  const current = items.find((i) => i.path === S.focus);
  const stage = h('div', { class: 'fd-stage' }, stageContent(current), h('div', { class: 'fd-stage-name', text: current.name }),
    h('div', { class: 'fd-stage-meta', text: current.dir ? kindName(current) : `${kindName(current)} — ${fmtSize(current.size)}` }));
  const strip = h('div', { class: 'fd-strip' });
  for (const item of items) strip.append(h('div', itemAttrs(item, 'fd-item fd-strip-item'), itemIcon(item, true)));
  view.replaceChildren(h('div', { class: 'fd-gal' }, stage, strip));
  requestAnimationFrame(() => strip.querySelector('.focus')?.scrollIntoView({ inline: 'center', block: 'nearest' }));
}

function stageContent(item) {
  const kind = kindOf(item);
  if (kind === 'image' || (kind === 'raw' && /safari/i.test(navigator.userAgent) && !/chrome/i.test(navigator.userAgent))) {
    return h('img', { class: 'fd-stage-media', src: fileUrl(item), alt: '', draggable: 'false' });
  }
  if (kind === 'video') return h('video', { class: 'fd-stage-media', src: fileUrl(item), controls: true, preload: 'metadata', playsinline: true });
  return h('div', { class: 'fd-stage-icon' }, itemIcon(item, true));
}

/** Big preview with facts: the preview pane and the last column. */
function previewCard(item) {
  const card = h('div', { class: 'fd-card' });
  card.append(h('div', { class: 'fd-card-media' }, kindOf(item) === 'image' ? h('img', { src: thumbable(item) ? thumbUrl(item) : fileUrl(item), alt: '' }) : itemIcon(item, true)));
  card.append(h('div', { class: 'fd-card-name', text: item.name }));
  card.append(h('div', { class: 'fd-card-kind', text: item.dir ? kindName(item) : `${kindName(item)} — ${fmtSize(item.size)}` }));
  const facts = h('dl', { class: 'fd-facts' });
  const fact = (k, v) => facts.append(h('dt', { text: t(k) }), h('dd', { text: v }));
  fact('Изменён', fmtDate(item.mtime));
  if (item.dir) {
    const size = h('button', { type: 'button', class: 'btn link fd-calc', text: t('Посчитать'), onclick: () => calcSize(item, size) });
    facts.append(h('dt', { text: t('Размер') }), h('dd', {}, size));
  }
  fact('Где', parentOf(item.path) || item.path);
  card.append(facts);
  const actions = h('div', { class: 'fd-card-actions' });
  if (item.dir) actions.append(h('button', { type: 'button', class: 'btn pill', text: t('Открыть'), onclick: () => openItem(item) }));
  else {
    if (previewable(item)) actions.append(h('button', { type: 'button', class: 'btn pill', text: t('Просмотр'), onclick: () => quickLook(item) }));
    actions.append(h('button', { type: 'button', class: 'btn pill', text: t('Скачать'), onclick: () => download([item]) }));
  }
  card.append(actions);
  return card;
}

async function calcSize(item, node) {
  node.disabled = true;
  node.textContent = t('Считаю…');
  try {
    const info = await api('GET', `/api/pc/info?path=${enc(item.path)}`);
    const text = t('{size} · файлов: {files}', { size: fmtSize(info.size), files: info.files.toLocaleString() });
    node.replaceWith(document.createTextNode(info.complete ? text : `${text} ${t('(не до конца — папка очень большая)')}`));
  } catch (e) { node.textContent = t('Посчитать'); node.disabled = false; fail(e); }
}

function renderPreviewPane() {
  const pane = $('#fd-preview');
  const show = S.preview && !isPhone() && S.cwd !== null && S.view !== 'columns' && S.view !== 'gallery';
  pane.hidden = !show;
  $('#fd-info-btn').classList.toggle('on', S.preview);
  if (!show) return;
  const items = selectedItems();
  if (items.length === 1) pane.replaceChildren(previewCard(items[0]));
  else if (items.length > 1) {
    const size = items.reduce((s, i) => s + (i.dir ? 0 : i.size), 0);
    pane.replaceChildren(h('div', { class: 'fd-card' },
      h('div', { class: 'fd-card-media' }, h('span', { class: 'fd-stack' }, ...items.slice(0, 3).map((i) => itemIcon(i, true)))),
      h('div', { class: 'fd-card-name', text: t('Выбрано: {x}', { x: items.length }) }),
      h('div', { class: 'fd-card-kind', text: items.some((i) => i.dir) ? t('{x} в файлах, не считая папок', { x: fmtSize(size) }) : fmtSize(size) })));
  } else {
    const folder = { name: baseName(S.cwd), dir: true, path: S.cwd, mtime: 0 };
    pane.replaceChildren(h('div', { class: 'fd-card' },
      h('div', { class: 'fd-card-media' }, itemIcon(folder, true)),
      h('div', { class: 'fd-card-name', text: baseName(S.cwd) }),
      h('div', { class: 'fd-card-kind', text: countText() })));
  }
}

function countText() {
  const n = sorted(S.entries).length;
  return LANG === 'ru' ? `${n} ${plural(n, 'объект', 'объекта', 'объектов')}` : `${n} ${n === 1 ? 'item' : 'items'}`;
}

function renderSelection() {
  for (const node of document.querySelectorAll('#fd-view .fd-item')) {
    node.classList.toggle('sel', S.sel.has(node.dataset.path));
    node.classList.toggle('focus', node.dataset.path === S.focus);
    node.classList.toggle('cut', S.clip?.mode === 'cut' && S.clip.paths.includes(node.dataset.path));
  }
  const n = S.sel.size;
  const status = [S.search ? t('Найдено: {x}', { x: S.search.hits.length }) : countText()];
  if (n) status.push(t('выбрано: {x}', { x: n }));
  if (S.free != null && !S.search) status.push(t('{x} свободно', { x: fmtSize(S.free) }));
  if (!S.writable && !S.search) status.push(t('только просмотр'));
  $('#fd-status').textContent = status.join(' · ');
  const bar = $('#fd-selbar');
  bar.hidden = !(isPhone() && S.selecting);
  for (const b of bar.querySelectorAll('button')) b.disabled = !n || (b.dataset.act !== 'download' && b.dataset.act !== 'copy' && !S.writable && !S.search);
  $('#fd-select').textContent = S.selecting ? t('Готово') : t('Выбрать');
  if (isPhone() && S.selecting) $('#fd-title').textContent = n ? t('Выбрано: {x}', { x: n }) : t('Выберите объекты');
  renderPreviewPane();
}

function renderPaste() {
  const box = $('#fd-paste');
  box.hidden = !S.clip || S.cwd === null;
  if (box.hidden) return;
  const n = S.clip.paths.length;
  const what = n === 1 ? `«${baseName(S.clip.paths[0])}»` : t('{x} объектов', { x: n });
  $('#fd-paste-text').textContent = S.clip.mode === 'cut' ? t('Переместить {x}', { x: what }) : t('Скопировать {x}', { x: what });
  $('#fd-paste-go').disabled = !S.writable || Boolean(S.search);
  $('#fd-paste-go').textContent = t('Вставить сюда');
}

// ---------- selection and opening ----------

function select(path, mode = 'only') {
  if (mode === 'only') { S.sel.clear(); S.sel.add(path); S.anchor = path; }
  else if (mode === 'toggle') { if (S.sel.has(path)) S.sel.delete(path); else S.sel.add(path); S.anchor = path; }
  else if (mode === 'range') {
    const items = visibleItems().map((i) => i.path);
    const a = items.indexOf(S.anchor ?? path), b = items.indexOf(path);
    if (a < 0 || b < 0) { S.sel.add(path); }
    else {
      S.sel.clear();
      for (let i = Math.min(a, b); i <= Math.max(a, b); i++) S.sel.add(items[i]);
    }
  }
  S.focus = path;
  if (S.view === 'gallery' && !S.search) { renderKeepScroll(); return; }
  renderSelection();
  document.querySelector(`#fd-view .fd-item.focus`)?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
}

async function openItem(item) {
  if (item.dir) {
    if (S.view === 'columns' && !S.search && !isPhone()) { await openColumn(item); return; }
    go(item.path);
  } else if (previewable(item)) quickLook(item);
  else download([item]);
}

async function openColumn(item) {
  const cols = [S.cwd, ...S.columns];
  const owner = cols.findIndex((c, i) => (i === 0 ? S.entries : S.children.get(c) || []).some((x) => x.path === item.path));
  S.columns = S.columns.slice(0, Math.max(0, owner));
  S.sel.clear();
  S.sel.add(item.path);
  S.focus = item.path;
  if (item.dir) {
    S.columns.push(item.path);
    render();
    if (!S.children.has(item.path)) {
      try { S.children.set(item.path, (await listDir(item.path)).entries); } catch (e) { S.columns.pop(); fail(e); }
      render();
    }
  } else render();
}

// Clicks, double clicks, taps and long presses on items; everything is delegated.
const view = $('#fd-view');

let longPress = null;
view.addEventListener('pointerdown', (e) => {
  if (e.pointerType !== 'touch') return;
  const node = e.target.closest('.fd-item');
  if (!node) return;
  const start = { x: e.clientX, y: e.clientY };
  longPress = {
    start,
    fired: false,
    timer: setTimeout(() => {
      longPress.fired = true;
      navigator.vibrate?.(10);
      const item = S.index.get(node.dataset.path);
      if (!S.sel.has(item.path) || !S.selecting) { S.sel.clear(); S.sel.add(item.path); S.focus = item.path; renderSelection(); }
      itemMenu(e.clientX, e.clientY);
    }, 480),
  };
});
view.addEventListener('pointermove', (e) => {
  if (longPress && Math.hypot(e.clientX - longPress.start.x, e.clientY - longPress.start.y) > 10) clearTimeout(longPress.timer);
});
const endPress = () => { if (longPress) clearTimeout(longPress.timer); };
view.addEventListener('pointerup', endPress);
view.addEventListener('pointercancel', () => { endPress(); longPress = null; });

view.addEventListener('click', (e) => {
  if (longPress?.fired) { longPress = null; return; }
  longPress = null;
  if (e.target.closest('.fd-head, .fd-card, .fd-stage, video, .fd-disc')) return;
  const node = e.target.closest('.fd-item');
  if (!node) {
    if (!bandMoved) { S.sel.clear(); S.focus = null; renderSelection(); }
    return;
  }
  const item = S.index.get(node.dataset.path);
  if (!item) return;
  if (coarse && e.pointerType !== 'mouse') {
    // Touch: a tap opens; in select mode it ticks.
    if (S.selecting) { select(item.path, 'toggle'); return; }
    if (S.view === 'gallery' && S.focus !== item.path) { select(item.path); return; }
    select(item.path);
    openItem(item);
    return;
  }
  if (e.shiftKey) select(item.path, 'range');
  else if (e.metaKey || e.ctrlKey) select(item.path, 'toggle');
  else if (S.view === 'columns' && !S.search) openColumn(item);
  else select(item.path);
});

view.addEventListener('dblclick', (e) => {
  if (coarse) return;
  const node = e.target.closest('.fd-item');
  const item = node && S.index.get(node.dataset.path);
  if (item) {
    if (S.view === 'columns' && item.dir && !S.search) go(item.path);
    else openItem(item);
  }
});

view.addEventListener('contextmenu', (e) => {
  if (e.target.closest('input, textarea, video')) return;
  e.preventDefault();
  // A long press on Android also sends contextmenu: one menu, and no click after it.
  if (longPress) {
    clearTimeout(longPress.timer);
    if (longPress.fired) return;
    longPress.fired = true;
  }
  const node = e.target.closest('.fd-item');
  if (node) {
    if (!S.sel.has(node.dataset.path)) select(node.dataset.path);
    itemMenu(e.clientX, e.clientY);
  } else {
    S.sel.clear();
    renderSelection();
    folderMenu(e.clientX, e.clientY);
  }
});

// ---------- rubber band selection (mouse) ----------

let band = null;
let bandMoved = false;
view.addEventListener('mousedown', (e) => {
  bandMoved = false;
  if (e.button !== 0 || e.target.closest('.fd-item, .fd-head, .fd-card, .fd-strip, .fd-stage, .fd-cols')) return;
  const rect = view.getBoundingClientRect();
  if (e.clientX > rect.left + view.clientWidth) return;   // the scrollbar
  band = { x: e.clientX, y: e.clientY + view.scrollTop, base: e.metaKey || e.ctrlKey || e.shiftKey ? new Set(S.sel) : new Set() };
  e.preventDefault();
  view.focus({ preventScroll: true });
});
addEventListener('mousemove', (e) => {
  if (!band) return;
  const box = $('#fd-band');
  const vr = view.getBoundingClientRect();
  const y1 = band.y - view.scrollTop;
  const left = Math.min(band.x, e.clientX), top = Math.max(vr.top, Math.min(y1, e.clientY));
  const right = Math.max(band.x, e.clientX), bottom = Math.min(vr.bottom, Math.max(y1, e.clientY));
  if (!bandMoved && Math.hypot(e.clientX - band.x, e.clientY - y1) < 4) return;
  bandMoved = true;
  box.hidden = false;
  box.style.left = `${left}px`;
  box.style.top = `${top}px`;
  box.style.width = `${right - left}px`;
  box.style.height = `${bottom - top}px`;
  S.sel = new Set(band.base);
  for (const node of view.querySelectorAll('.fd-item')) {
    const r = node.getBoundingClientRect();
    if (r.right > left && r.left < right && r.bottom > top && r.top < bottom) S.sel.add(node.dataset.path);
  }
  renderSelection();
  // Near the edge: scroll along.
  if (e.clientY > vr.bottom - 20) view.scrollTop += 12;
  else if (e.clientY < vr.top + 20) view.scrollTop -= 12;
});
addEventListener('mouseup', () => {
  if (!band) return;
  band = null;
  $('#fd-band').hidden = true;
  setTimeout(() => { bandMoved = false; }, 0);
});

// ---------- drag and drop ----------

let dragPaths = null;
view.addEventListener('dragstart', (e) => {
  const node = e.target.closest('.fd-item');
  if (!node) return;
  if (!S.sel.has(node.dataset.path)) select(node.dataset.path);
  dragPaths = [...S.sel];
  e.dataTransfer.effectAllowed = 'copyMove';
  e.dataTransfer.setData('text/plain', dragPaths.join('\n'));
  e.dataTransfer.setData('application/x-lp-paths', JSON.stringify(dragPaths));
  if (dragPaths.length > 1) {
    const ghost = h('div', { class: 'fd-ghost', text: t('{x} объектов', { x: dragPaths.length }) });
    document.body.append(ghost);
    e.dataTransfer.setDragImage(ghost, 20, 20);
    setTimeout(() => ghost.remove(), 0);
  }
});
addEventListener('dragend', () => { dragPaths = null; clearDropMarks(); });

function dropTarget(e) {
  const node = e.target.closest?.('[data-drop]');
  if (node) return { node, path: node.dataset.drop };
  if (e.target.closest?.('#fd-view') && S.cwd) return { node: null, path: here() };
  return null;
}

function clearDropMarks() {
  for (const n of document.querySelectorAll('.drop-over')) n.classList.remove('drop-over');
  $('#fd-drop').hidden = true;
}

addEventListener('dragover', (e) => {
  const target = dropTarget(e);
  const files = e.dataTransfer.types.includes('Files') && !dragPaths;
  if (!target || (!dragPaths && !files)) return;
  if (dragPaths?.includes(target.path)) return;
  e.preventDefault();
  const copying = files || e.altKey || e.ctrlKey || (dragPaths && !sameDrive(dragPaths[0], target.path));
  e.dataTransfer.dropEffect = copying ? 'copy' : 'move';
  clearDropMarks();
  if (target.node) target.node.classList.add('drop-over');
  else if (files) $('#fd-drop').hidden = false;
});
addEventListener('dragleave', (e) => { if (!e.relatedTarget) clearDropMarks(); });
addEventListener('drop', (e) => {
  const target = dropTarget(e);
  clearDropMarks();
  if (!target) return;
  e.preventDefault();
  if (dragPaths) {
    const paths = dragPaths.filter((p) => p !== target.path && parentOf(p) !== target.path);
    dragPaths = null;
    if (!paths.length) return;
    const copying = e.altKey || e.ctrlKey || !sameDrive(paths[0], target.path);
    transfer(paths, target.path, !copying);
  } else if (e.dataTransfer.files.length) {
    upload([...e.dataTransfer.files], target.path);
  }
});

// ---------- actions ----------

function download(items) {
  if (items.length === 1 && !items[0].dir) {
    const a = h('a', { href: fileUrl(items[0], '&download=1'), download: items[0].name });
    document.body.append(a);
    a.click();
    a.remove();
    return;
  }
  zipDownload(items.map((i) => i.path), items.length === 1 ? items[0].name : t('Архив'));
}

async function zipDownload(paths, label) {
  let id;
  try { ({ id } = await api('POST', '/api/pc/zip', { paths })); } catch (e) { fail(e); return; }
  const job = jobRow(t('ZIP: {x}', { x: label }), null);
  const tick = async () => {
    let st;
    try { st = await api('GET', `/api/zip/${id}`); } catch (e) { job.fail(e.message); return; }
    if (st.state === 'failed') { job.fail(st.error); return; }
    job.progress(st.done, st.total);
    if (st.state === 'ready') {
      job.finish(t('Готово — скачивание началось'));
      location.href = `/api/zip/${id}/file`;
      return;
    }
    setTimeout(tick, 500);
  };
  tick();
}

async function newFolder() {
  const dir = here();
  if (!dir || !S.writable) return;
  const name = await dialog({ title: t('Новая папка'), value: t('Новая папка'), ok: t('Создать') });
  if (!name) return;
  try {
    const { path } = await api('POST', '/api/pc/mkdir', { path: dir, name });
    await refresh();
    if (S.index.has(path)) select(path);
  } catch (e) { fail(e); }
}

async function renameItem(item) {
  const dot = item.dir ? -1 : item.name.lastIndexOf('.');
  const name = await dialog({ title: t('Переименовать'), value: item.name, ok: t('Переименовать'), select: [0, dot > 0 ? dot : item.name.length] });
  if (!name || name === item.name) return;
  try {
    const { path } = await api('POST', '/api/pc/rename', { path: item.path, name });
    if (S.clip) S.clip.paths = S.clip.paths.map((p) => (p === item.path ? path : p));
    await refresh();
    if (S.index.has(path)) select(path);
  } catch (e) { fail(e); }
}

async function trash(items, now = false) {
  if (!items.length) return;
  if (now) {
    const what = items.length === 1 ? `«${items[0].name}»` : t('{x} объектов', { x: items.length });
    const ok = await dialog({ title: t('Удалить {x} навсегда?', { x: what }), text: t('Это нельзя отменить: мимо Корзины.'), ok: t('Удалить'), danger: true });
    if (!ok) return;
  }
  try {
    const { job } = await api('POST', '/api/pc/delete', { paths: items.map((i) => i.path), now });
    const label = items.length === 1 ? items[0].name : t('{x} объектов', { x: items.length });
    trackJob(job, now ? t('Удаление: {x}', { x: label }) : t('В Корзину: {x}', { x: label }), 'items',
      now ? t('Удалено') : t('Перемещено в Корзину — восстановить можно из Корзины Windows'));
    S.sel.clear();
  } catch (e) { fail(e); }
}

async function transfer(paths, to, moving) {
  try {
    const { job } = await api('POST', '/api/pc/transfer', { paths, to, move: moving });
    const what = paths.length === 1 ? baseName(paths[0]) : t('{x} объектов', { x: paths.length });
    trackJob(job, moving ? t('Перемещение: {x}', { x: what }) : t('Копирование: {x}', { x: what }), 'bytes', moving ? t('Перемещено') : t('Скопировано'));
  } catch (e) { fail(e); }
}

function setClip(mode, items) {
  if (!items.length) return;
  S.clip = { mode, paths: items.map((i) => i.path) };
  toast(mode === 'cut' ? t('Откройте нужную папку и нажмите «Вставить сюда»') : t('Скопировано. Откройте папку и нажмите «Вставить сюда»'));
  if (S.selecting) { S.selecting = false; }
  render();
}

function paste(to = here()) {
  if (!S.clip || !to) return;
  const { mode, paths } = S.clip;
  const moving = mode === 'cut';
  if (moving) S.clip = null;
  transfer(paths, to, moving);
  renderPaste();
  renderSelection();
}

async function properties(item) {
  const facts = h('dl', { class: 'fd-facts wide' });
  const fact = (k, v) => facts.append(h('dt', { text: t(k) }), h('dd', { text: v }));
  fact('Тип', kindName(item));
  fact('Где', parentOf(item.path) || item.path);
  const sizeCell = h('dd', { text: t('Считаю…') });
  facts.append(h('dt', { text: t('Размер') }), sizeCell);
  const created = h('dd', { text: '…' });
  facts.append(h('dt', { text: t('Создан') }), created);
  fact('Изменён', fmtDate(item.mtime));
  dialog({ title: item.name, body: facts, ok: t('Готово'), cancel: false });
  try {
    const info = await api('GET', `/api/pc/info?path=${enc(item.path)}`);
    sizeCell.textContent = info.dir
      ? `${fmtSize(info.size)} · ${t('файлов: {x}', { x: info.files.toLocaleString() })} · ${t('папок: {x}', { x: info.dirs.toLocaleString() })}${info.complete ? '' : ` ${t('(не до конца — папка очень большая)')}`}`
      : `${fmtSize(info.size)} (${info.size.toLocaleString()} ${t('байт')})`;
    created.textContent = fmtDate(info.created);
  } catch (e) { sizeCell.textContent = e.message; }
}

async function copyText(text) {
  try { await navigator.clipboard.writeText(text); toast(t('Путь скопирован')); } catch { dialog({ title: t('Путь'), value: text, ok: t('Готово') }); }
}

function itemMenu(x, y) {
  const items = selectedItems();
  if (!items.length) return;
  const one = items.length === 1 ? items[0] : null;
  const canChange = S.writable || Boolean(S.search);
  const menu = [];
  if (isPhone() || (coarse && x != null)) menu.push({ title: one ? one.name : t('Выбрано: {x}', { x: items.length }) });
  if (one) {
    menu.push({ label: one.dir ? t('Открыть') : previewable(one) ? t('Просмотр') : t('Скачать'), act: () => openItem(one), hint: one.dir ? '' : 'Space' });
    if (S.search) menu.push({ label: t('Показать в папке'), act: () => revealItem(one) });
  }
  menu.push({ label: items.length === 1 && !one.dir ? t('Скачать') : t('Скачать как ZIP'), act: () => download(items) });
  if (one && kindOf(one) === 'pdf') menu.push({ label: t('Открыть в новой вкладке'), act: () => open(fileUrl(one, '&view=1'), '_blank', 'noopener') });
  menu.push('-');
  if (one) menu.push({ label: t('Переименовать'), act: () => renameItem(one), disabled: !canChange, hint: 'Enter' });
  menu.push({ label: t('Дублировать'), act: () => transfer(items.map((i) => i.path), parentOf(items[0].path), false), disabled: !canChange, hint: 'Ctrl+D' });
  menu.push({ label: t('Копировать'), act: () => setClip('copy', items), hint: 'Ctrl+C' });
  menu.push({ label: t('Вырезать'), act: () => setClip('cut', items), disabled: !canChange, hint: 'Ctrl+X' });
  if (one?.dir && S.clip) menu.push({ label: t('Вставить в «{x}»', { x: one.name }), act: () => paste(one.path) });
  menu.push('-');
  if (one) menu.push({ label: t('Свойства'), act: () => properties(one), hint: 'Ctrl+I' });
  if (one) menu.push({ label: t('Скопировать путь'), act: () => copyText(one.path) });
  menu.push('-');
  menu.push({ label: t('В Корзину'), act: () => trash(items), danger: true, disabled: !canChange, hint: 'Delete' });
  menu.push({ label: t('Удалить сразу…'), act: () => trash(items, true), danger: true, disabled: !canChange, hint: 'Shift+Delete' });
  showMenu(menu, x, y);
}

function folderMenu(x, y) {
  const menu = [];
  const writable = S.writable && !S.search;
  menu.push({ label: t('Новая папка'), act: newFolder, disabled: !writable, hint: 'Ctrl+Shift+N' });
  menu.push({ label: t('Загрузить файлы…'), act: () => $('#fd-file').click(), disabled: !writable });
  if (S.clip) menu.push({ label: t('Вставить сюда'), act: () => paste(), disabled: !writable, hint: 'Ctrl+V' });
  menu.push('-');
  for (const [v, label] of [['icons', 'Значки'], ['list', 'Список'], ['columns', 'Колонки'], ['gallery', 'Галерея']]) {
    if (v === 'columns' && isPhone()) continue;
    menu.push({ label: t(label), checked: S.view === v, act: () => setView(v) });
  }
  menu.push('-');
  menu.push({ title: t('Сортировка') });
  for (const c of COLS) menu.push({ label: t(c.label), checked: S.sort.key === c.key, act: () => setSort(c.key) });
  menu.push('-');
  menu.push({ label: t('Показывать скрытые файлы'), checked: S.hidden, act: toggleHidden, hint: 'Ctrl+Shift+.' });
  menu.push({ label: t('Перейти к папке…'), act: goToFolder, hint: 'Ctrl+Shift+G' });
  menu.push({ label: t('Скопировать путь папки'), act: () => copyText(here()) });
  menu.push({ label: t('Свойства папки'), act: () => properties({ name: baseName(here()), dir: true, path: here(), mtime: 0 }) });
  menu.push({ label: t('Обновить'), act: refresh, hint: 'F5' });
  showMenu(menu, x, y);
}

async function revealItem(item) {
  await go(parentOf(item.path));
  if (S.index.has(item.path)) select(item.path);
}

function setView(v) {
  if (v === 'columns' && isPhone()) return;
  S.view = v;
  save('fd-view', v);
  S.columns = [];
  render();
}

function toggleHidden() {
  S.hidden = !S.hidden;
  save('fd-hidden', S.hidden);
  toast(S.hidden ? t('Скрытые файлы показаны') : t('Скрытые файлы спрятаны'));
  render();
}

async function goToFolder() {
  const path = await dialog({ title: t('Перейти к папке'), text: t('Например C:\\Users или D:\\Фото'), value: here() || '', ok: t('Перейти') });
  if (path) go(path);
}

// ---------- jobs and uploads ----------

/** A row in the progress panel; returns controls for it. */
function jobRow(label, onCancel) {
  const bar = h('span', { class: 'fd-job-bar' }, h('span'));
  const text = h('span', { class: 'fd-job-text', text: label });
  const meta = h('span', { class: 'fd-job-meta', text: t('Начинаю…') });
  const cancel = onCancel ? h('button', { type: 'button', class: 'btn round small', title: t('Отменить'), onclick: onCancel }, svg('M6 6l12 12M18 6L6 18')) : null;
  const row = h('div', { class: 'fd-job glass' }, h('div', { class: 'fd-job-top' }, text, cancel), bar, meta);
  $('#fd-jobs').append(row);
  const close = (delay) => setTimeout(() => row.remove(), delay);
  return {
    progress(done, total, unit = 'bytes', current = '') {
      const share = total ? Math.min(1, done / total) : 0;
      bar.firstChild.style.width = `${Math.round(share * 100)}%`;
      meta.textContent = unit === 'items'
        ? `${done} / ${total}${current ? ` · ${current}` : ''}`
        : `${fmtSize(done)} / ${fmtSize(total)}${current ? ` · ${current}` : ''}`;
    },
    finish(msg) { row.classList.add('done'); bar.firstChild.style.width = '100%'; meta.textContent = msg; cancel?.remove(); close(2500); },
    fail(msg) { row.classList.add('failed'); meta.textContent = tServer(msg) || t('Ошибка'); cancel?.remove(); close(9000); },
  };
}

function trackJob(id, label, unit, doneText) {
  const row = jobRow(label, () => api('DELETE', `/api/pc/jobs/${id}`).catch(fail));
  const tick = async () => {
    let st;
    try { st = await api('GET', `/api/pc/jobs/${id}`); } catch (e) { row.fail(e.message); refresh(); return; }
    row.progress(st.done, st.total, unit, st.current);
    if (st.state === 'running') { setTimeout(tick, 400); return; }
    if (st.state === 'done') row.finish(doneText);
    else row.fail(st.error);
    refresh();
  };
  tick();
}

const CHUNK = 8 * 1024 * 1024;

/** Uploads files one after another, in pieces (the tunnel limits one request to 100 MB). */
async function upload(files, dir) {
  if (!files.length || !dir) return;
  for (const file of files) {
    let stopped = false;
    const row = jobRow(t('Загрузка: {x}', { x: file.name }), () => { stopped = true; });
    let offset = 0, tries = 0;
    try {
      while (true) {
        if (stopped) throw new Error(t('Отменено'));
        const piece = file.slice(offset, offset + CHUNK);
        const url = `/api/pc/upload?${new URLSearchParams({ path: dir, name: file.name, offset, total: file.size })}`;
        let res;
        try {
          res = await api('PUT', url, piece, true);
        } catch (e) {
          if (e.status === 409 && typeof e.data?.received === 'number') { offset = e.data.received; continue; }
          if ((e.status === undefined || e.status >= 500) && tries++ < 4) { await new Promise((r) => setTimeout(r, 1500 * tries)); continue; }
          throw e;
        }
        tries = 0;
        if (res.done) { row.finish(t('Загружено')); break; }
        offset = res.received;
        row.progress(offset, file.size);
      }
    } catch (e) {
      row.fail(e.message);
    }
  }
  if (dir === here() || S.children.has(dir) || dir === S.cwd) refresh();
}

$('#fd-file').addEventListener('change', (e) => {
  upload([...e.target.files], here());
  e.target.value = '';
});

// ---------- Quick Look ----------

const QL = { items: [], i: 0, editing: false, dirty: false, text: null, mtime: 0 };

function quickLook(item) {
  const list = (S.search ? S.search.hits : visibleItems()).filter((x) => !x.dir);
  QL.items = list.length ? list : [item];
  QL.i = Math.max(0, QL.items.findIndex((x) => x.path === item.path));
  $('#ql').hidden = false;
  showQl();
}

async function showQl() {
  const item = QL.items[QL.i];
  QL.editing = false;
  QL.dirty = false;
  QL.text = null;
  S.sel.clear();
  S.sel.add(item.path);
  S.focus = item.path;
  renderSelection();
  $('#ql-name').textContent = item.name;
  $('#ql-prev').hidden = QL.items.length < 2;
  $('#ql-next').hidden = QL.items.length < 2;
  const tools = [];
  const kind = kindOf(item);
  if (kind === 'pdf') tools.push(h('a', { class: 'btn pill', href: fileUrl(item, '&view=1'), target: '_blank', rel: 'noopener', text: t('Открыть') }));
  tools.push(h('button', { type: 'button', class: 'btn pill', text: t('Скачать'), onclick: () => download([item]) }));
  $('#ql-tools').replaceChildren(...tools);
  const body = $('#ql-body');
  body.className = `ql-body k-${kind}`;
  if (kind === 'image' || kind === 'raw') {
    const img = h('img', { src: fileUrl(item), alt: item.name, draggable: 'false' });
    img.addEventListener('error', () => body.replaceChildren(qlCard(item, t('Этот браузер не умеет показывать такой формат'))), { once: true });
    body.replaceChildren(img);
  } else if (kind === 'video') {
    body.replaceChildren(h('video', { src: fileUrl(item), controls: true, autoplay: true, playsinline: true }));
  } else if (kind === 'audio') {
    body.replaceChildren(h('div', { class: 'ql-audio' }, itemIcon(item, true), h('audio', { src: fileUrl(item), controls: true, autoplay: true })));
  } else if (kind === 'pdf') {
    body.replaceChildren(qlCard(item, t('PDF откроется во встроенном просмотрщике браузера'),
      h('a', { class: 'btn primary', href: fileUrl(item, '&view=1'), target: '_blank', rel: 'noopener', text: t('Открыть PDF') })));
  } else {
    body.replaceChildren(h('div', { class: 'ql-loading', text: t('Загрузка…') }));
    try {
      const data = await api('GET', `/api/pc/text?path=${enc(item.path)}`);
      if (QL.items[QL.i] !== item) return;
      QL.text = data.text;
      QL.mtime = data.mtime;
      showText(item, data);
    } catch (e) {
      if (QL.items[QL.i] === item) body.replaceChildren(qlCard(item, e.message));
    }
  }
}

function qlCard(item, note, ...extra) {
  return h('div', { class: 'ql-card' }, itemIcon(item, true), h('div', { class: 'ql-card-name', text: item.name }),
    h('div', { class: 'ql-card-meta', text: `${kindName(item)} — ${fmtSize(item.size)}` }), note ? h('div', { class: 'ql-card-note', text: note }) : null, ...extra);
}

function showText(item, data) {
  const body = $('#ql-body');
  const isMd = ['md', 'markdown'].includes(extOf(item.name));
  const tools = $('#ql-tools');
  if (!data.exact) tools.prepend(h('span', { class: 'ql-warn', text: t('Не UTF-8 — показан приблизительно, сохранить нельзя') }));
  if (data.writable) {
    tools.prepend(h('button', { type: 'button', class: 'btn pill', id: 'ql-edit', text: t('Изменить'), onclick: () => editText(item) }));
  }
  if (isMd) {
    const doc = h('article', { class: 'ql-md' });
    doc.append(renderMarkdown(data.text));
    body.replaceChildren(doc);
  } else {
    body.replaceChildren(h('pre', { class: 'ql-text', text: data.text }));
  }
}

function editText(item) {
  QL.editing = true;
  const area = h('textarea', { class: 'ql-edit', spellcheck: 'false', autocapitalize: 'off', autocomplete: 'off' });
  area.value = QL.text;
  area.addEventListener('input', () => { QL.dirty = true; });
  area.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') { e.preventDefault(); saveText(item, area.value); }
    if (e.key === 'Tab' && !e.shiftKey) { e.preventDefault(); area.setRangeText('    ', area.selectionStart, area.selectionEnd, 'end'); QL.dirty = true; }
  });
  $('#ql-body').replaceChildren(area);
  $('#ql-tools').replaceChildren(
    h('button', { type: 'button', class: 'btn pill', text: t('Отмена'), onclick: async () => { if (await leaveEditing()) showQl(); } }),
    h('button', { type: 'button', class: 'btn pill primary', text: t('Сохранить'), title: 'Ctrl+S', onclick: () => saveText(item, area.value) }));
  area.focus();
}

async function saveText(item, text) {
  try {
    const res = await api('PUT', `/api/pc/text?path=${enc(item.path)}&mtime=${QL.mtime}`, new Blob([text], { type: 'text/plain;charset=utf-8' }), true);
    QL.mtime = res.mtime;
    QL.text = text;
    QL.dirty = false;
    item.mtime = res.mtime;
    item.size = new Blob([text]).size;
    toast(t('Сохранено'));
  } catch (e) { fail(e); }
}

async function leaveEditing() {
  if (!QL.editing || !QL.dirty) return true;
  return Boolean(await dialog({ title: t('Выйти без сохранения?'), text: t('Изменения пропадут.'), ok: t('Выйти'), danger: true }));
}

async function closeQl() {
  if (!(await leaveEditing())) return;
  $('#ql').hidden = true;
  const body = $('#ql-body');
  for (const m of body.querySelectorAll('video, audio')) m.pause();
  body.replaceChildren();
  QL.editing = false;
  view.focus({ preventScroll: true });
  renderSelection();
}

async function qlStep(d) {
  if (QL.items.length < 2 || !(await leaveEditing())) return;
  for (const m of $('#ql-body').querySelectorAll('video, audio')) m.pause();
  QL.i = (QL.i + d + QL.items.length) % QL.items.length;
  showQl();
}

$('#ql-close').addEventListener('click', closeQl);
$('#ql-prev').addEventListener('click', () => qlStep(-1));
$('#ql-next').addEventListener('click', () => qlStep(1));
$('#ql').addEventListener('click', (e) => { if (e.target.id === 'ql') closeQl(); });

// Swipe left/right through photos on a phone.
let qlSwipe = null;
$('#ql-body').addEventListener('touchstart', (e) => { if (e.touches.length === 1 && !QL.editing) qlSwipe = { x: e.touches[0].clientX, y: e.touches[0].clientY }; }, { passive: true });
$('#ql-body').addEventListener('touchend', (e) => {
  if (!qlSwipe) return;
  const dx = e.changedTouches[0].clientX - qlSwipe.x, dy = e.changedTouches[0].clientY - qlSwipe.y;
  qlSwipe = null;
  if (Math.abs(dx) > 60 && Math.abs(dx) > Math.abs(dy) * 1.5 && !$('#ql-body').querySelector('pre, article, textarea')) qlStep(dx < 0 ? 1 : -1);
});

// ---------- search ----------

let searchTimer = 0;
let searchSeq = 0;
$('#fd-q').addEventListener('input', (e) => {
  clearTimeout(searchTimer);
  const q = e.target.value.trim();
  if (!q) { if (S.search) { S.search = null; render(); } return; }
  searchTimer = setTimeout(() => runSearch(q), 450);
});
$('#fd-q').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') { clearTimeout(searchTimer); const q = e.target.value.trim(); if (q) runSearch(q); }
  if (e.key === 'Escape') { e.target.value = ''; S.search = null; render(); view.focus(); }
  if (e.key === 'ArrowDown') { e.preventDefault(); view.focus(); moveFocus(1, e.shiftKey); }
});

async function runSearch(q) {
  if (!S.cwd) return;
  const seq = ++searchSeq;
  S.search = { q, hits: S.search?.q === q ? S.search.hits : [], more: false, busy: true };
  S.sel.clear();
  render();
  try {
    const data = await api('GET', `/api/pc/search?${new URLSearchParams({ path: S.cwd, q, ...(S.hidden ? { hidden: '1' } : {}) })}`);
    if (seq !== searchSeq || !S.search) return;
    S.search = { q, hits: data.hits, more: data.more, busy: false };
  } catch (e) {
    if (seq === searchSeq) { S.search = null; fail(e); }
  }
  render();
}

// ---------- keys (Finder's) ----------

function moveFocus(step, extend) {
  const items = visibleItems();
  if (!items.length) return;
  let i = items.findIndex((x) => x.path === S.focus);
  i = i < 0 ? (step > 0 ? 0 : items.length - 1) : Math.max(0, Math.min(items.length - 1, i + step));
  const path = items[i].path;
  if (extend) { S.anchor ??= S.focus ?? path; select(path, 'range'); }
  else if (S.view === 'columns' && !S.search) openColumn(items[i]);
  else select(path);
}

/** Tiles per row in the icon grid (arrows up/down jump a row). */
function gridColumns() {
  const tiles = [...view.querySelectorAll('.fd-tile')];
  if (tiles.length < 2) return 1;
  const top = tiles[0].offsetTop;
  const n = tiles.findIndex((x) => x.offsetTop !== top);
  return n < 0 ? tiles.length : n;
}

let typed = '';
let typedTimer = 0;

addEventListener('keydown', async (e) => {
  if (dialogOpen) return;
  if (!$('#fd-menu').hidden) { if (e.key === 'Escape') hideMenu(); return; }
  const cmd = e.ctrlKey || e.metaKey;
  const key = e.key;
  const inField = e.target.closest?.('input, textarea, [contenteditable]');

  if (!$('#ql').hidden) {
    if (QL.editing) { if (key === 'Escape') { e.preventDefault(); closeQl(); } return; }
    if (key === 'Escape' || key === ' ') { e.preventDefault(); closeQl(); }
    else if (key === 'ArrowLeft' || key === 'ArrowUp') { e.preventDefault(); qlStep(-1); }
    else if (key === 'ArrowRight' || key === 'ArrowDown') { e.preventDefault(); qlStep(1); }
    return;
  }
  if (inField) return;
  if (S.cwd === null) return;

  const items = selectedItems();
  const done = () => { e.preventDefault(); e.stopPropagation(); };

  if (cmd && key.toLowerCase() === 'a') { done(); for (const i of visibleItems()) S.sel.add(i.path); renderSelection(); return; }
  if (cmd && key.toLowerCase() === 'c') { if (items.length) { done(); setClip('copy', items); } return; }
  if (cmd && key.toLowerCase() === 'x') { if (items.length && (S.writable || S.search)) { done(); setClip('cut', items); } return; }
  if (cmd && key.toLowerCase() === 'v') { if (S.clip && S.writable) { done(); paste(); } return; }
  if (cmd && key.toLowerCase() === 'd') { if (items.length) { done(); transfer(items.map((i) => i.path), parentOf(items[0].path), false); } return; }
  if (cmd && key.toLowerCase() === 'i') { if (items.length === 1) { done(); properties(items[0]); } return; }
  if (cmd && key.toLowerCase() === 'f') { done(); $('#fd-q').focus(); $('#fd-q').select(); return; }
  if (cmd && key.toLowerCase() === 'o') { if (items.length === 1) { done(); openItem(items[0]); } return; }
  if (cmd && e.shiftKey && key.toLowerCase() === 'n') { done(); newFolder(); return; }
  if (cmd && e.shiftKey && key.toLowerCase() === 'g') { done(); goToFolder(); return; }
  if (cmd && e.shiftKey && key.toLowerCase() === 'p') { done(); S.preview = !S.preview; save('fd-preview', S.preview); renderPreviewPane(); return; }
  if (cmd && e.shiftKey && (key === '.' || key === '>' || e.code === 'Period')) { done(); toggleHidden(); return; }
  if (cmd && ['1', '2', '3', '4'].includes(key)) { done(); setView(VIEWS[Number(key) - 1]); return; }
  if ((cmd && key === 'ArrowUp') || (e.altKey && key === 'ArrowUp')) { done(); const up = parentOf(S.cwd); if (up) go(up); return; }
  if (cmd && key === 'ArrowDown') { if (items.length === 1) { done(); openItem(items[0]); } return; }
  if ((e.altKey && key === 'ArrowLeft') || (cmd && key === '[')) { done(); history.back(); return; }
  if ((e.altKey && key === 'ArrowRight') || (cmd && key === ']')) { done(); history.forward(); return; }
  if (key === 'F5') { done(); refresh(); return; }
  if ((key === 'Delete' || (cmd && key === 'Backspace')) && items.length) { done(); trash(items, e.shiftKey || (e.altKey && cmd)); return; }
  if ((key === 'Enter' || key === 'F2') && items.length === 1 && (S.writable || S.search)) { done(); renameItem(items[0]); return; }
  if (key === ' ' && items.length) { done(); const f = items.find((i) => !i.dir); if (f) quickLook(f); else properties(items[0]); return; }
  if (key === 'Escape') { if (S.search) { $('#fd-q').value = ''; S.search = null; render(); } else { S.sel.clear(); renderSelection(); } return; }
  if (key === 'Backspace' && !cmd) { done(); history.back(); return; }

  const listLike = S.view === 'list' || S.view === 'columns' || S.search;
  if (key === 'ArrowDown' || key === 'ArrowUp') {
    done();
    const step = listLike || S.view === 'gallery' ? 1 : gridColumns();
    moveFocus(key === 'ArrowDown' ? step : -step, e.shiftKey);
    return;
  }
  if (key === 'ArrowRight' || key === 'ArrowLeft') {
    const right = key === 'ArrowRight';
    if (S.view === 'list' && !S.search) {
      const item = S.index.get(S.focus);
      if (item?.dir) { done(); toggleExpand(item, right); }
      return;
    }
    if (S.view === 'columns' && !S.search) {
      done();
      const cols = [S.cwd, ...S.columns];
      const col = activeColumn();
      const ci = cols.indexOf(col);
      if (right) {
        const item = S.index.get(S.focus);
        const kids = item?.dir && S.children.get(item.path);
        if (kids?.length) select(sorted(kids)[0].path);
      } else if (ci > 0) {
        // Back to the folder this column shows; it stays open, like in Finder.
        const folder = cols[ci];
        S.columns = S.columns.slice(0, ci);
        S.sel = new Set([folder]);
        S.focus = folder;
        render();
      }
      return;
    }
    done();
    moveFocus(right ? 1 : -1, e.shiftKey);
    return;
  }
  // Typing a name jumps to it, like in Finder.
  if (key.length === 1 && !cmd && !e.altKey && key !== ' ') {
    typed += key.toLowerCase();
    clearTimeout(typedTimer);
    typedTimer = setTimeout(() => { typed = ''; }, 900);
    const hit = visibleItems().find((i) => i.name.toLowerCase().startsWith(typed));
    if (hit) { done(); select(hit.path); }
  }
});

// ---------- buttons ----------

for (const b of document.querySelectorAll('#fd-views button')) b.addEventListener('click', () => setView(b.dataset.view));
$('#fd-newdir').addEventListener('click', newFolder);
$('#fd-upload').addEventListener('click', () => $('#fd-file').click());
$('#fd-info-btn').addEventListener('click', () => { S.preview = !S.preview; save('fd-preview', S.preview); renderPreviewPane(); });
$('#fd-more').addEventListener('click', (e) => {
  const r = e.currentTarget.getBoundingClientRect();
  if (S.sel.size && !isPhone()) itemMenu(r.left, r.bottom + 6);
  else folderMenu(r.right - 240, r.bottom + 6);
});
$('#fd-back').addEventListener('click', () => history.back());
$('#fd-fwd').addEventListener('click', () => history.forward());
$('#fd-up').addEventListener('click', () => {
  if (S.selecting) { S.selecting = false; S.sel.clear(); render(); return; }
  if (S.search) { $('#fd-q').value = ''; S.search = null; render(); return; }
  const up = parentOf(S.cwd);
  const isPlace = S.places && [...S.places.favorites, ...S.places.cloud, ...S.places.drives].some((p) => p.path === S.cwd);
  go(up && !isPlace ? up : null);
});
$('#fd-select').addEventListener('click', () => {
  S.selecting = !S.selecting;
  S.sel.clear();
  render();
});
$('#fd-selbar').addEventListener('click', (e) => {
  const b = e.target.closest('button');
  const items = selectedItems();
  if (!b || !items.length) return;
  const act = b.dataset.act;
  if (act === 'download') download(items);
  else if (act === 'copy') setClip('copy', items);
  else if (act === 'cut') setClip('cut', items);
  else if (act === 'trash') trash(items);
  S.selecting = false;
  render();
});
$('#fd-paste-go').addEventListener('click', () => paste());
$('#fd-paste-x').addEventListener('click', () => { S.clip = null; renderPaste(); renderSelection(); });
phoneQuery.addEventListener('change', () => {
  if (!isPhone() && S.cwd === null) go(S.places?.home || S.places?.drives[0]?.path, { push: false });
  else render();
});

// ---------- start ----------

(async () => {
  translatePage();
  let session = null;
  try { session = await (await fetch('/api/session', { credentials: 'same-origin' })).json(); } catch { /* offline */ }
  if (!session?.authenticated) { location.replace('/'); return; }
  if (!session.files) { $('#fd-off').hidden = false; return; }
  try {
    S.places = await api('GET', '/api/pc/places');
  } catch (e) {
    $('#fd-off').hidden = false;
    if (e.status !== 404) fail(e);
    return;
  }
  S.sep = S.places.sep;
  $('#fd').hidden = false;
  const start = pathFromHash() || (isPhone() ? null : S.places.home || S.places.drives[0]?.path);
  S.hist = 1;
  S.histMax = 1;
  history.replaceState({ path: start, i: 1 }, '', start ? `#${enc(start)}` : '#');
  updateHistoryButtons();
  if (start) await go(start, { push: false });
  else render();
  view.focus({ preventScroll: true });
})();
