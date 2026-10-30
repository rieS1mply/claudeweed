'use strict';
// Own folders, share links, HEIC previews made by the browser, viewers for PDF / audio /
// Markdown, and search inside notes and text files. Hooks are called from app.js.

// ---------- search inside notes and text files ----------

/** Matches found by the server inside files: 'files/<path>' or 'notes/<id>' -> snippet. */
const contentHits = { query: '', map: new Map(), timer: 0, seq: 0 };

/** Called on every keystroke in the search field: asks the server after a short pause. */
function searchContent() {
  clearTimeout(contentHits.timer);
  const q = $('#search').value.trim();
  if (!(state.section === 'files' || state.section === 'notes') || q.length < 2) {
    contentHits.query = '';
    contentHits.map.clear();
    return;
  }
  if (q === contentHits.query) return;
  contentHits.timer = setTimeout(async () => {
    const seq = ++contentHits.seq;
    try {
      const { hits } = await api.search(q);
      if (seq !== contentHits.seq) return;   // a newer query is on its way
      contentHits.query = q;
      contentHits.map = new Map(hits.map((h) => [`${h.section}/${h.path}`, h.snippet]));
      if ($('#search').value.trim() === q) render();
    } catch { /* name search still works */ }
  }, 350);
}

function contentSnippet(section, key) {
  return searchWords().length && contentHits.query ? contentHits.map.get(`${section}/${key}`) : undefined;
}

// ---------- own folders ----------

/** Folder open in each section: null = the top. In Files a path, like "Работа/Отчёты". */
const openFolder = { photos: null, files: null };

/** First path part when it is an own folder, not a year of the date layout. */
function folderOf(item) {
  const first = item.path.split('/')[0];
  return item.path.includes('/') && !/^\d{4}$/.test(first) ? first : null;
}

/** The own folder a file lies in directly ("Работа/Отчёты"), or null in the date layout. */
const dirOf = (item) => folderOf(item) && item.path.slice(0, item.path.lastIndexOf('/'));
const leafName = (path) => path.slice(path.lastIndexOf('/') + 1);
const parentOf = (path) => (path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : null);
const inFolder = (path, folder) => path.startsWith(`${folder}/`);

/** Folders a section offers: Files has nested ones, Photos albums are top-level only. */
function sectionFolders(section) {
  const all = state.data[section]?.folders || [];
  return section === 'files' ? all : all.filter((f) => !f.includes('/'));
}

/**
 * What the list shows for the open folder. `below`: every file under it (for the summary);
 * `items`: the files listed; `folders`: folder rows (Files only). In Files a folder shows its
 * own files and subfolders, like a file manager; a search looks through everything below.
 */
function folderView(section, list, folder) {
  if (section !== 'files') {
    const below = folder ? list.items.filter((i) => folderOf(i) === folder) : list.items;
    return { below, items: below, folders: [] };
  }
  const below = folder ? list.items.filter((i) => inFolder(i.path, folder)) : list.items;
  const under = (list.folders || []).filter((f) => !folder || inFolder(f, folder));
  const words = searchWords();
  if (words.length) {
    return { below, items: below, folders: under.filter((f) => words.every((w) => normalize(leafName(f)).includes(w))) };
  }
  const depth = folder ? folder.split('/').length + 1 : 1;
  const direct = (path) => path.split('/').length === depth;
  return { below, items: below.filter((i) => (folder ? direct(i.path) : !folderOf(i))), folders: under.filter(direct) };
}

/** Folder rows at the top of the Files list: a tap opens the folder, "⋯" renames or deletes it. */
function renderFolderRows(paths, below, withParent) {
  const box = el('div', { class: 'files folder-rows' });
  for (const path of paths) {
    const n = below.filter((i) => inFolder(i.path, path)).length;
    const parent = withParent && parentOf(path);
    box.append(el('div', { class: 'file-row folder-row', title: path, onclick: () => chooseFolder('files', path) },
      el('div', { class: 'file-icon folder-icon' }, icon('folder')),
      el('div', { class: 'file-main' },
        el('div', { class: 'file-name', text: leafName(path) }),
        el('div', { class: 'file-meta' },
          el('span', { class: 'file-size', text: n ? `${n} ${plural(n, 'файл', 'файла', 'файлов')}` : t('пусто') }),
          el('span', { class: 'file-date', text: parent || '' }))),
      el('button', { class: 'btn round', title: t('Действия'), onclick: (e) => { e.stopPropagation(); folderActions('files', path); } }, icon('more'))));
  }
  return box;
}

/** Above the list: chips "Все · Альбом · ＋" in Photos, the path "Файлы › Работа › ＋" in Files. */
function renderFolderBar(section, folders) {
  const bar = $('#folders');
  if (!(section === 'photos' || section === 'files')) { bar.hidden = true; return; }
  if (section === 'photos') folders = folders.filter((f) => !f.includes('/'));
  // The open folder may be gone (renamed or deleted elsewhere): go to what is left of its path.
  let open = openFolder[section];
  while (open && !folders.includes(open)) open = section === 'files' ? parentOf(open) : null;
  openFolder[section] = open;
  if (sel.on) { bar.hidden = true; return; }
  if (section === 'files') return renderCrumbs(bar);
  const chip = (label, folder, cls = '') => el('button', {
    class: `folder-chip ${cls} ${openFolder[section] === folder ? 'active' : ''}`, type: 'button',
    onclick: () => (folder !== null && openFolder[section] === folder ? folderActions(section, folder) : chooseFolder(section, folder)),
  }, label);
  bar.replaceChildren(
    chip(t('Все'), null),
    ...folders.map((f) => chip(f, f, 'own')),
    el('button', { class: 'folder-chip add', type: 'button', title: t('Новая папка'), onclick: () => createFolder(section) }, '+'),
  );
  bar.hidden = false;
}

/** Files: "Файлы › Работа › Отчёты" — a tap goes there; a tap on the open folder shows its actions. */
function renderCrumbs(bar) {
  const open = openFolder.files;
  const parts = open ? open.split('/') : [];
  const crumb = (label, path) => el('button', {
    class: `folder-chip crumb ${path ? 'own' : ''} ${path === open ? 'active' : ''}`, type: 'button',
    onclick: () => (path && path === open ? folderActions('files', path) : chooseFolder('files', path)),
  }, label);
  bar.replaceChildren(
    crumb(t('Файлы'), null),
    ...parts.flatMap((part, i) => [el('span', { class: 'crumb-sep', text: '›' }), crumb(part, parts.slice(0, i + 1).join('/'))]),
    el('button', { class: 'folder-chip add', type: 'button', title: t('Новая папка'), onclick: () => createFolder('files') }, '+'),
  );
  bar.hidden = false;
  bar.scrollLeft = bar.scrollWidth;   // a deep path keeps the open folder in view
}

function chooseFolder(section, folder) {
  openFolder[section] = folder;
  exitSelection();
  window.scrollTo(0, 0);
  render();
}

/** New folder; in Files inside the open one. Returns its path (null when cancelled). */
async function createFolder(section, thenMove) {
  const name = await promptDialog(section === 'photos' ? t('Новый альбом') : t('Новая папка'), '', { ok: t('Создать') });
  if (!name) return null;
  try {
    const res = await api.folderCreate(section, name, section === 'files' ? openFolder.files : null);
    await load(section);
    if (!thenMove) chooseFolder(section, res.path);
    return res.path;
  } catch (e) {
    toast(tServer(e.message));
    return null;
  }
}

/** "⋯" of a folder, or a tap on the open one: rename or delete it. */
function folderActions(section, folder) {
  return actionSheet(leafName(folder), [
    { label: t('Переименовать'), run: async () => {
      const name = await promptDialog(t('Переименовать папку'), leafName(folder));
      if (!name || name === leafName(folder)) return;
      try {
        const res = await api.folderRename(section, folder, name);
        // The open folder may be this one or inside it.
        const open = openFolder[section];
        if (open === folder || (open && inFolder(open, folder))) openFolder[section] = res.path + open.slice(folder.length);
        await load(section);
      } catch (e) { toast(tServer(e.message)); }
    } },
    { label: t('Удалить папку'), destructive: true, hint: t('только пустую'), run: async () => {
      try {
        await api.folderDelete(section, folder);
        const open = openFolder[section];
        if (open === folder || (open && inFolder(open, folder))) openFolder[section] = parentOf(folder);
        await load(section);
        toast(t('Папка удалена'));
      } catch (e) { toast(tServer(e.message)); }
    } },
  ]);
}

/** "Переложить в папку…" for one item or the selection. */
async function moveToFolder(section, items) {
  const folders = sectionFolders(section);
  const dir = section === 'files' ? dirOf : folderOf;
  const here = items.length === 1 ? dir(items[0]) : undefined;
  let target;
  await actionSheet(t('Переложить в папку'), [
    ...folders.filter((f) => f !== here).map((f) => ({ label: f.replaceAll('/', ' › '), run: () => { target = f; } })),
    { label: t('Новая папка…'), hint: section === 'files' && openFolder.files ? t('в «{f}»', { f: leafName(openFolder.files) }) : '', run: () => { target = createFolder(section, true); } },
    here !== null && { label: t('Убрать из папки'), hint: t('обратно в раздел по дате'), run: () => { target = ''; } },
  ]);
  // actionSheet runs the choice right after it closes.
  await new Promise((r) => setTimeout(r, 0));
  target = await target;
  if (target == null) return;
  let failed = 0;
  for (const item of items) {
    try { await api.move(section, item, target); } catch { failed++; }
  }
  if (sel.on) exitSelection();
  await load(section);
  toast(failed ? t('Не удалось переложить: {n}', { n: failed }) : target ? t('Переложено в «{f}»', { f: leafName(target) }) : t('Убрано из папки'));
}

// ---------- share links ----------

const SHARE_DAYS = [[1, '1 день'], [7, '7 дней'], [30, '30 дней']];

async function createShareLink(section, items) {
  let days = 0;
  await actionSheet(t('Ссылка без пароля'), SHARE_DAYS.map(([d, label]) => ({
    label: t('Действует {x}', { x: t(label) }), run: () => { days = d; },
  })));
  await new Promise((r) => setTimeout(r, 0));
  if (!days) return;
  try {
    const share = await api.shareCreate(section, items.map((i) => i.path), days);
    if (sel.on) exitSelection();
    offerLink(shareUrl(share));
  } catch (e) {
    toast(tServer(e.message));
  }
}

const shareUrl = (share) => `${location.origin}/s/${share.token}`;

/** Copies right away when the browser allows it; otherwise a button does it within a tap. */
async function offerLink(url) {
  const copy = () => navigator.clipboard?.writeText(url).then(() => toast(t('Ссылка скопирована')), () => promptDialog(t('Ссылка'), url, { ok: t('Готово') }));
  try {
    await navigator.clipboard.writeText(url);
    toast(t('Ссылка скопирована'), CAN_SHARE_FILES ? { action: t('Отправить'), onAction: () => navigator.share({ url }).catch(() => {}) } : {});
  } catch {
    toast(t('Ссылка готова'), { action: t('Скопировать'), onAction: copy });
  }
}

async function manageLinks() {
  let list;
  try { list = (await api.shares()).items; } catch (e) { return toast(tServer(e.message)); }
  if (!list.length) return toast(t('Ссылок нет. Создать: «⋯» у файла → «Ссылка без пароля»'));
  await actionSheet(t('Ссылки без пароля'), list.map((s) => ({
    label: s.paths.length > 1 ? t('{n} файлов', { n: s.paths.length }) : s.name,
    hint: t('до {d}', { d: fmtDate(s.expires) }),
    run: () => actionSheet(s.name, [
      { label: t('Скопировать ссылку'), run: () => offerLink(shareUrl(s)) },
      { label: t('Отозвать'), destructive: true, run: async () => {
        try { await api.shareRevoke(s.token); toast(t('Ссылка больше не действует')); } catch (e) { toast(tServer(e.message)); }
      } },
    ]),
  })));
}

// ---------- HEIC: previews made by the browser ----------
// The server can't decode HEIC; Safari can. Whoever opens the gallery in a browser that
// decodes it makes a JPEG once and sends it, and from then on every browser has it.

const PREVIEW_SIDE = 2048;
const decodeKey = (ext) => `lp-decode-${ext}`;
function canDecode(ext) {
  try { return sessionStorage.getItem(decodeKey(ext)); } catch { return null; }   // '1', '0' or null = unknown
}
function rememberDecode(ext, ok) {
  try { sessionStorage.setItem(decodeKey(ext), ok ? '1' : '0'); } catch { /* private mode */ }
}

function decodeBlob(blob) {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(blob);
    const img = new Image();
    img.onload = () => { URL.revokeObjectURL(url); resolve(img); };
    img.onerror = () => { URL.revokeObjectURL(url); reject(new Error('decode')); };
    img.src = url;
  });
}

async function jpegFrom(blob) {
  const img = await decodeBlob(blob);
  const k = Math.min(1, PREVIEW_SIDE / Math.max(img.naturalWidth, img.naturalHeight));
  const c = document.createElement('canvas');
  c.width = Math.max(1, Math.round(img.naturalWidth * k));
  c.height = Math.max(1, Math.round(img.naturalHeight * k));
  c.getContext('2d').drawImage(img, 0, 0, c.width, c.height);
  const out = await new Promise((r) => c.toBlob(r, 'image/jpeg', 0.85));
  if (!out) throw new Error('encode');
  return out;
}

const previewQueue = [];
let previewBusy = false;

/** Tile of a HEIC without a thumbnail: make one in the background if this browser can. */
function queuePreview(item, tile) {
  if (MOCK || canDecode(splitName(item.name)[1]) === '0') return;
  previewQueue.push({ item, tile, file: null });
  runPreviews();
}

async function runPreviews() {
  if (previewBusy) return;
  previewBusy = true;
  while (previewQueue.length) {
    const job = previewQueue.shift();
    const ext = splitName(job.item.name)[1];
    if (canDecode(ext) === '0') continue;
    try {
      const blob = job.file || await (await fetch(api.fileUrl('photos', job.item, false), { credentials: 'same-origin' })).blob();
      let jpeg;
      try {
        jpeg = await jpegFrom(blob);
        rememberDecode(ext, true);
      } catch {
        rememberDecode(ext, false);   // this browser can't: don't download the others
        continue;
      }
      await api.putPreview('photos', job.item, jpeg);
      if (job.tile?.isConnected) {
        const img = el('img', { src: `${api.thumbUrl('photos', job.item)}&p=1`, alt: job.item.name, draggable: 'false' });
        img.onload = () => img.classList.add('loaded');
        job.tile.querySelector('.placeholder')?.replaceWith(img);
      }
    } catch { /* try again next time the gallery shows it */ }
  }
  previewBusy = false;
}

/** Right after uploading a HEIC from this browser: the file is still here, no download needed. */
function previewAfterUpload(file, res) {
  if (MOCK || res.section !== 'photos' || kindOf(file.name) !== 'raw') return;
  if (canDecode(splitName(file.name)[1]) === '0') return;
  previewQueue.push({ item: { path: res.path, name: file.name, date: 0, size: file.size }, tile: null, file });
  runPreviews();
}

/** What the viewer loads for a photo: HEIC as is only where the browser shows it. */
function viewerUrl(item) {
  if (item.kind === 'raw' && canDecode(splitName(item.name)[1]) !== '1') return api.previewUrl('photos', item);
  return api.fileUrl('photos', item, false);
}

// ---------- viewers: PDF, audio, Markdown ----------

const AUDIO_EXT = ['mp3', 'm4a', 'aac', 'wav', 'ogg', 'oga', 'opus', 'flac', 'weba'];
const isPdf = (name) => splitName(name)[1] === 'pdf';
const isAudio = (name) => AUDIO_EXT.includes(splitName(name)[1]);
const isMarkdown = (name) => ['md', 'markdown'].includes(splitName(name)[1]);

/** Opens what the browser can show; returns false for everything else. */
function viewFile(section, item) {
  if (isPdf(item.name)) {
    window.open(`${api.fileUrl(section, item, false)}&view=1`, '_blank', 'noopener');
    return true;
  }
  if (isAudio(item.name)) { openPlayer(section, item); return true; }
  if (isMarkdown(item.name)) { openMarkdown(section, item); return true; }
  return false;
}

function openPlayer(section, item) {
  const box = $('#player');
  $('#player-title').textContent = item.name;
  $('#player-meta').textContent = `${fmtSize(item.size)} · ${fmtDate(item.date)}`;
  const audio = $('#player-audio');
  audio.src = api.fileUrl(section, item, false);
  audio.play().catch(() => {});
  $('#player-download').onclick = () => triggerDownload(api.fileUrl(section, item, true), item.name);
  if (box.hidden) enter(box, [{ opacity: 0, transform: 'translateY(30px) scale(.94)' }, { opacity: 1, transform: 'none' }], 'bouncy');
}

async function closePlayer() {
  const audio = $('#player-audio');
  audio.pause();
  await leave($('#player'), [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'translateY(30px) scale(.94)' }]);
  audio.removeAttribute('src');
  audio.load();
}

async function openMarkdown(section, item) {
  let text;
  try { text = await api.readText(section, item); } catch (e) { return toast(tServer(e.message)); }
  const view = $('#md-view');
  view.replaceChildren(renderMarkdown(text));
  view.scrollTop = 0;
  const edit = el('button', { class: 'doc-btn strong', text: t('Править'), onclick: async () => {
    await closeMdView();
    openTextEditor(section, item);
  } });
  $('#md-title').textContent = item.name;
  $('#md-right').replaceChildren(edit);
  await enter($('#md'), [{ transform: 'translateY(100%)' }, { transform: 'none' }], 'smooth');
}

function closeMdView() {
  return leave($('#md'), [{ transform: 'none' }, { transform: 'translateY(100%)' }], 'smooth', { duration: 380 });
}

// ---------- wiring ----------

function bindExtras() {
  $('#search').addEventListener('input', searchContent);
  $('#links-btn').addEventListener('click', manageLinks);
  $('#more-btn').addEventListener('click', () => actionSheet('', [
    { label: t('Ссылки без пароля'), run: manageLinks },
    { label: `${t('Язык')}: ${LANGS[LANG]}`, run: () => $('#lang-btn').click() },
    { label: t('Выйти'), destructive: true, run: () => $('#logout-btn').click() },
  ]));
  $('#lang-btn').addEventListener('click', () => actionSheet(t('Язык'), Object.entries(LANGS).map(([code, name]) => ({
    label: code === LANG ? `✓ ${name}` : name, run: () => { if (code !== LANG) setLang(code); },
  }))));
  $('#login-lang').addEventListener('click', () => setLang(LANG === 'ru' ? 'en' : 'ru'));
  $('#login-lang').textContent = LANG === 'ru' ? 'English' : 'Русский';
  $('#player-close').addEventListener('click', closePlayer);
  $('#md-close').addEventListener('click', closeMdView);
  $('#sel-link').addEventListener('click', () => createShareLink(state.section, selectedItems()));
  $('#sel-move').addEventListener('click', () => moveToFolder(state.section, selectedItems()));
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !$('#md').hidden && $('#sheet').hidden) closeMdView();
  });
}
