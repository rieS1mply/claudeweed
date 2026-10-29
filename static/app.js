'use strict';

// Demo mode: page opened as a local file (no server) or with ?mock in the URL.
const MOCK = location.protocol === 'file:' || new URLSearchParams(location.search).has('mock');
const CHUNK_SIZE = 16 * 1024 * 1024;
const MONTHS = ['Январь', 'Февраль', 'Март', 'Апрель', 'Май', 'Июнь', 'Июль', 'Август', 'Сентябрь', 'Октябрь', 'Ноябрь', 'Декабрь'];
const MONTHS_GEN = ['января', 'февраля', 'марта', 'апреля', 'мая', 'июня', 'июля', 'августа', 'сентября', 'октября', 'ноября', 'декабря'];
const SVG_NS = 'http://www.w3.org/2000/svg';

const ICONS = {
  download: '<path d="M12 4v12m0 0-5-5m5 5 5-5M4 20h16"/>',
  trash: '<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/>',
  check: '<path d="M6 12.5l4 4 8-9"/>',
  share: '<path d="M12 15V3m0 0L8 7m4-4 4 4"/><path d="M8 10H6a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-7a2 2 0 0 0-2-2h-2"/>',
  play: '<path d="M7 4v16l13-8z"/>',
  image: '<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-5-5L5 21"/>',
  cloud: '<path d="M12 16V4m0 0-5 5m5-5 5 5M4 20h16"/>',
  more: '<circle cx="5" cy="12" r="1.3"/><circle cx="12" cy="12" r="1.3"/><circle cx="19" cy="12" r="1.3"/>',
  note: '<path d="M6 3h9l4 4v14H6z"/><path d="M9 12h7M9 16h5"/>',
};

// ---------- helpers ----------

const $ = (sel) => document.querySelector(sel);

function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === 'class') node.className = v;
    else if (k === 'text') node.textContent = v;
    else if (k.startsWith('on')) node.addEventListener(k.slice(2), v);
    else if (v !== false && v != null) node.setAttribute(k, v === true ? '' : v);
  }
  for (const c of children) if (c != null) node.append(c);
  return node;
}

function icon(name) {
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('class', 'icon');
  svg.innerHTML = ICONS[name];
  return svg;
}

const pad = (n) => String(n).padStart(2, '0');

function fmtSize(bytes) {
  const units = ['Б', 'КБ', 'МБ', 'ГБ', 'ТБ'];
  let v = bytes, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  const num = i === 0 ? String(v) : v.toFixed(v < 10 ? 1 : 0).replace('.', ',');
  return `${num} ${units[i]}`;
}

function noteDateShort(ms) {
  const d = new Date(ms);
  return `${pad(d.getDate())}.${pad(d.getMonth() + 1)}.${d.getFullYear()}`;
}

function fmtDate(ms) {
  const d = new Date(ms);
  return `${d.getDate()} ${MONTHS_GEN[d.getMonth()]} ${d.getFullYear()}, ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function plural(n, one, few, many) {
  const m10 = n % 10, m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return one;
  if (m10 >= 2 && m10 <= 4 && (m100 < 10 || m100 >= 20)) return few;
  return many;
}

const qs = (obj) => new URLSearchParams(obj).toString();

// ---------- motion ----------
// Spring curves live in style.css (--ease-*) so CSS transitions and JS animations match.

const REDUCED = matchMedia('(prefers-reduced-motion: reduce)').matches;
const HAS_LINEAR = typeof CSS !== 'undefined' && CSS.supports('transition-timing-function', 'linear(0, 1)');
const SPRINGS = { smooth: 560, bouncy: 640, snappy: 420 };

function springEasing(name) {
  const fallback = { smooth: 'cubic-bezier(.22,.9,.3,1)', bouncy: 'cubic-bezier(.3,1.35,.5,1)', snappy: 'cubic-bezier(.25,.85,.3,1)' };
  if (!HAS_LINEAR) return fallback[name];
  return getComputedStyle(document.documentElement).getPropertyValue(`--ease-${name}`).trim() || fallback[name];
}

/** Runs a Web Animation with a spring; resolves when done (or immediately with reduced motion). */
function animate(node, keyframes, spring = 'smooth', opts = {}) {
  if (!node || REDUCED || !node.animate) return Promise.resolve();
  const a = node.animate(keyframes, { duration: SPRINGS[spring], easing: springEasing(spring), fill: 'both', ...opts });
  return a.finished.then(() => a, () => a);
}

/** Show `node` and play an entrance; the animation is dropped afterwards so CSS owns the final state. */
async function enter(node, keyframes, spring = 'smooth', opts = {}) {
  node.hidden = false;
  const a = await animate(node, keyframes, spring, { fill: 'backwards', ...opts });
  a?.cancel?.();
}

/** Play an exit, then hide `node`. */
async function leave(node, keyframes, spring = 'snappy', opts = {}) {
  if (node.hidden) return;
  const a = await animate(node, keyframes, spring, { fill: 'forwards', ...opts });
  node.hidden = true;
  a?.cancel?.();
}

const shake = (node) => animate(node,
  [{ transform: 'translateX(0)' }, { transform: 'translateX(-12px)' }, { transform: 'translateX(10px)' },
   { transform: 'translateX(-7px)' }, { transform: 'translateX(4px)' }, { transform: 'translateX(0)' }],
  'smooth', { duration: 420, easing: 'ease-out', fill: 'none' });

// ---------- toast, alerts, action sheet ----------

let toastTimer;
/** Short message at the top. `opts.action` adds a button (e.g. "Отменить"). */
function toast(msg, opts = {}) {
  const t = $('#toast');
  t.replaceChildren(el('span', { text: msg }));
  if (opts.action) {
    t.append(el('button', {
      class: 'toast-action', text: opts.action,
      onclick: () => { hideToast(); opts.onAction(); },
    }));
  }
  clearTimeout(toastTimer);
  if (t.hidden) enter(t, [{ opacity: 0, transform: 'translateY(14px) scale(.85)' }, { opacity: 1, transform: 'none' }], 'bouncy');
  toastTimer = setTimeout(hideToast, opts.action ? 5000 : 3000);
}

function hideToast() {
  clearTimeout(toastTimer);
  leave($('#toast'), [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'translateY(10px) scale(.9)' }]);
}

function showAlert(modal, box) {
  enter(modal, [{ opacity: 0 }, { opacity: 1 }], 'snappy');
  enter(box, [{ opacity: 0, transform: 'scale(1.15)' }, { opacity: 1, transform: 'none' }], 'smooth');
}

function hideAlert(modal, box) {
  animate(box, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'scale(.94)' }], 'snappy', { fill: 'none', duration: 200 });
  leave(modal, [{ opacity: 1 }, { opacity: 0 }], 'snappy', { duration: 220 });
}

/** iOS-style two-button alert. Resolves true for the main button. */
function confirmDialog(text, opts = {}) {
  const { title = 'Удалить?', ok = 'Удалить', cancel = 'Отмена', destructive = true } = opts;
  return new Promise((resolve) => {
    const modal = $('#modal'), box = $('#alert');
    $('#alert-title').textContent = title;
    $('#modal-text').textContent = text;
    $('#modal-text').hidden = !text;
    $('#modal-ok').textContent = ok;
    $('#modal-ok').classList.toggle('destructive', destructive);
    $('#modal-cancel').textContent = cancel;
    showAlert(modal, box);
    const done = (v) => {
      $('#modal-ok').onclick = $('#modal-cancel').onclick = modal.onclick = null;
      hideAlert(modal, box);
      resolve(v);
    };
    $('#modal-ok').onclick = () => done(true);
    $('#modal-cancel').onclick = () => done(false);
    modal.onclick = (e) => { if (e.target === modal) done(false); };
  });
}

/** Alert with a text field. Resolves the entered text, or null if cancelled. */
function promptDialog(title, value, opts = {}) {
  return new Promise((resolve) => {
    const modal = $('#prompt'), box = $('#prompt-box'), input = $('#prompt-input');
    $('#prompt-title').textContent = title;
    $('#prompt-ok').textContent = opts.ok || 'Готово';
    input.value = value;
    showAlert(modal, box);
    // Select the name without its extension, as Finder/Files do.
    const dot = opts.selectStem ? value.lastIndexOf('.') : -1;
    setTimeout(() => { input.focus(); input.setSelectionRange(0, dot > 0 ? dot : value.length); }, 50);
    const done = (v) => {
      $('#prompt-ok').onclick = $('#prompt-cancel').onclick = input.onkeydown = modal.onclick = null;
      hideAlert(modal, box);
      resolve(v);
    };
    $('#prompt-ok').onclick = () => done(input.value.trim() || null);
    $('#prompt-cancel').onclick = () => done(null);
    input.onkeydown = (e) => {
      if (e.key === 'Enter') done(input.value.trim() || null);
      if (e.key === 'Escape') done(null);
    };
    modal.onclick = (e) => { if (e.target === modal) done(null); };
  });
}

/**
 * iOS action sheet. `actions`: [{ label, run, destructive?, hint? }] (null entries are skipped).
 * Resolves once closed; the chosen action runs after the sheet is gone.
 */
function actionSheet(title, actions) {
  return new Promise((resolve) => {
    const sheet = $('#sheet'), panel = $('#sheet-panel'), cancel = $('#sheet-cancel');
    $('#sheet-title').textContent = title || '';
    $('#sheet-title').hidden = !title;
    const list = $('#sheet-actions');
    list.replaceChildren();
    let chosen = null;
    const close = async () => {
      sheet.onclick = cancel.onclick = null;
      animate(panel, [{ transform: 'none', opacity: 1 }, { transform: 'translateY(40px)', opacity: 0 }], 'snappy', { fill: 'none', duration: 220 });
      animate(cancel, [{ transform: 'none', opacity: 1 }, { transform: 'translateY(40px)', opacity: 0 }], 'snappy', { fill: 'none', duration: 220 });
      await leave(sheet, [{ opacity: 1 }, { opacity: 0 }], 'snappy', { duration: 230 });
      resolve();
      chosen?.run();
    };
    for (const a of actions.filter(Boolean)) {
      list.append(el('button', { class: a.destructive ? 'destructive' : '', onclick: () => { chosen = a; close(); } },
        el('span', { text: a.label }), a.hint ? el('span', { class: 'hint', text: a.hint }) : null));
    }
    enter(sheet, [{ opacity: 0 }, { opacity: 1 }], 'snappy', { duration: 200 });
    enter(panel, [{ transform: 'translateY(60px)', opacity: 0 }, { transform: 'none', opacity: 1 }], 'smooth');
    enter(cancel, [{ transform: 'translateY(60px)', opacity: 0 }, { transform: 'none', opacity: 1 }], 'smooth', { delay: 30 });
    cancel.onclick = close;
    sheet.onclick = (e) => { if (e.target === sheet) close(); };
  });
}

function triggerDownload(url, name) {
  const a = el('a', { href: url, download: name });
  document.body.append(a);
  a.click();
  a.remove();
}

/** "IMG_0001.JPG" → ["IMG_0001", "jpg"] */
function splitName(name) {
  const i = name.lastIndexOf('.');
  return i > 0 ? [name.slice(0, i), name.slice(i + 1).toLowerCase()] : [name, ''];
}

// ---------- save to the phone's gallery ----------
// Browsers can't write to Photos directly. The system share sheet can: it has
// "Сохранить изображение / видео", which puts the original file into the gallery.

const CAN_SHARE_FILES = (() => {
  try {
    return matchMedia('(pointer: coarse)').matches
      && Boolean(navigator.canShare?.({ files: [new File([''], 'x.jpg', { type: 'image/jpeg' })] }));
  } catch {
    return false;
  }
})();
const sharePending = new Map();   // key -> Promise<File>
const shareReady = new Map();     // key -> File, for sharing within the same tap

function shareKey(section, item) { return `${section}/${item.path}/${item.size}`; }

function prepareShare(section, item) {
  const key = shareKey(section, item);
  if (!sharePending.has(key)) {
    if (sharePending.size >= 4) {
      const oldest = sharePending.keys().next().value;
      sharePending.delete(oldest);
      shareReady.delete(oldest);
    }
    const p = fetch(api.fileUrl(section, item, false), { credentials: 'same-origin' })
      .then((r) => { if (!r.ok) throw new HttpError(r.status, `Ошибка ${r.status}`); return r.blob(); })
      .then((b) => {
        const file = new File([b], item.name, { type: b.type || 'application/octet-stream' });
        shareReady.set(key, file);
        return file;
      });
    p.catch(() => sharePending.delete(key));
    sharePending.set(key, p);
  }
  return sharePending.get(key);
}

async function openShareSheet(file) {
  try {
    await navigator.share({ files: [file] });
  } catch (e) {
    if (e.name !== 'AbortError') toast('Не удалось открыть меню «Поделиться»');
  }
}

/** Opens the share sheet for a photo/file. Safari only allows it right inside the tap,
 *  so a file that is not downloaded yet takes a second tap once it is ready. */
async function shareItem(section, item) {
  const ready = shareReady.get(shareKey(section, item));
  if (ready) return openShareSheet(ready);
  toast('Подготовка файла…');
  try {
    const file = await prepareShare(section, item);
    toast('Файл готов', { action: 'Сохранить', onAction: () => openShareSheet(file) });
  } catch (e) {
    toast(e.message);
  }
}

/** Mirrors Kind::of on the server. */
function kindOf(name) {
  const ext = splitName(name)[1];
  if (['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'tif', 'tiff'].includes(ext)) return 'image';
  if (['heic', 'heif', 'avif', 'dng', 'cr2', 'cr3', 'nef', 'arw', 'raf', 'orf', 'rw2'].includes(ext)) return 'raw';
  if (['mp4', 'mov', 'm4v', '3gp', 'avi', 'mkv', 'webm'].includes(ext)) return 'video';
  return 'file';
}

// ---------- real API ----------

class HttpError extends Error {
  constructor(status, message, data) { super(message); this.status = status; this.data = data; }
}

async function request(url, opts = {}) {
  const headers = { 'X-Cloud': '1', ...(opts.headers || {}) };
  let body = opts.body;
  if (opts.json !== undefined) {
    headers['Content-Type'] = 'application/json';
    body = JSON.stringify(opts.json);
  }
  let res;
  try {
    res = await fetch(url, { method: opts.method || 'GET', headers, body, credentials: 'same-origin', keepalive: Boolean(opts.keepalive) });
  } catch {
    throw new HttpError(0, 'Нет соединения с сервером');
  }
  let data = null;
  if ((res.headers.get('content-type') || '').includes('application/json')) data = await res.json().catch(() => null);
  if (res.status === 401 && !opts.allow401) {
    showLogin();
    throw new HttpError(401, 'Требуется вход');
  }
  if (!res.ok) throw new HttpError(res.status, (data && data.error) || `Ошибка ${res.status}`, data);
  return data;
}

function putChunk(url, blob, onProgress) {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open('PUT', url);
    xhr.setRequestHeader('X-Cloud', '1');
    xhr.setRequestHeader('Content-Type', 'application/octet-stream');
    xhr.upload.onprogress = (e) => onProgress(e.loaded);
    xhr.onload = () => {
      let data = null;
      try { data = JSON.parse(xhr.responseText); } catch { /* not json */ }
      if (xhr.status >= 200 && xhr.status < 300) resolve(data);
      else reject(new HttpError(xhr.status, (data && data.error) || `Ошибка ${xhr.status}`, data));
    };
    xhr.onerror = () => reject(new HttpError(0, 'Нет соединения'));
    xhr.send(blob);
  });
}

const realApi = {
  session: () => request('/api/session', { allow401: true }),
  login: (username, password) => request('/api/login', { method: 'POST', json: { username, password }, allow401: true }),
  logout: () => request('/api/logout', { method: 'POST' }),
  list: (section) => request(`/api/list?${qs({ section })}`),
  remove: (section, item) => request(`/api/file?${qs({ section, path: item.path })}`, { method: 'DELETE' }),
  // `v` changes when a file is edited, so browsers don't show a cached old version.
  thumbUrl: (section, item) => `/api/thumb?${qs({ section, path: item.path, v: `${item.date}-${item.size}` })}`,
  fileUrl: (section, item, download) =>
    `/api/file?${qs({ section, path: item.path, v: `${item.date}-${item.size}`, ...(download ? { download: '1' } : {}) })}`,
  rename: (section, item, name) => request(`/api/file/rename?${qs({ section, path: item.path })}`, { method: 'POST', json: { name } }),
  replace: (section, item, blob, keepOriginal) => request(
    `/api/file/content?${qs({ section, path: item.path, ...(keepOriginal ? { keep_original: '1' } : {}) })}`,
    { method: 'PUT', body: blob, headers: { 'Content-Type': 'application/octet-stream' } }),
  copy: (section, item, name, blob) => request(`/api/file/copy?${qs({ section, path: item.path, name })}`,
    { method: 'POST', body: blob, headers: { 'Content-Type': 'application/octet-stream' } }),
  async readText(section, item) {
    const res = await fetch(realApi.fileUrl(section, item, true), { credentials: 'same-origin' });
    if (res.status === 401) { showLogin(); throw new HttpError(401, 'Требуется вход'); }
    if (!res.ok) throw new HttpError(res.status, `Ошибка ${res.status}`);
    return res.text();
  },

  notes: () => request('/api/notes'),
  noteCreate: (content) => request('/api/notes', { method: 'POST', json: { content } }),
  noteGet: (id) => request(`/api/notes/${id}`),
  noteSave: (id, content, base, keepalive) => request(`/api/notes/${id}`, { method: 'PUT', json: { content, base }, keepalive }),
  noteDelete: (id, permanent) => request(`/api/notes/${id}${permanent ? '?permanent=1' : ''}`, { method: 'DELETE' }),

  zipStart: (section, paths) => request('/api/zip', { method: 'POST', json: { section, paths } }),
  zipStatus: (id) => request(`/api/zip/${id}`),
  zipFileUrl: (id) => `/api/zip/${id}/file`,

  trash: () => request('/api/trash'),
  restore: (id) => request(`/api/trash/${id}/restore`, { method: 'POST' }),
  purge: (id) => request(`/api/trash/${id}`, { method: 'DELETE' }),
  emptyTrash: () => request('/api/trash', { method: 'DELETE' }),
  trashThumbUrl: (t) => `/api/trash/${t.id}/thumb`,
  trashFileUrl: (t) => `/api/trash/${t.id}/file`,

  // Upload in chunks: Cloudflare limits one request body to ~100 MB,
  // and chunks let a dropped connection resume instead of starting over.
  async upload(file, onProgress) {
    const { id } = await request('/api/upload/init', {
      method: 'POST',
      json: { name: file.name, size: file.size, last_modified: file.lastModified || 0 },
    });
    let offset = 0, failures = 0;
    while (offset < file.size) {
      const chunk = file.slice(offset, offset + CHUNK_SIZE);
      try {
        const res = await putChunk(`/api/upload/${id}?offset=${offset}`, chunk, (loaded) => onProgress((offset + loaded) / file.size));
        offset = res.received;
        failures = 0;
      } catch (e) {
        if (e.status === 401) { showLogin(); throw e; }
        if (++failures > 5) throw e;
        await new Promise((r) => setTimeout(r, 1000 * failures));
        // The server knows how much it actually got; resume from there.
        const st = await request(`/api/upload/${id}`).catch(() => null);
        if (st) offset = st.received;
      }
    }
    onProgress(1);
    return request(`/api/upload/${id}/finish`, { method: 'POST' });
  },
};

// ---------- demo API (no server) ----------

function rng(seed) {
  // Spread neighbouring seeds apart, otherwise the first values are almost equal.
  let s = Math.imul(seed ^ 0x9e3779b9, 2654435761) >>> 0;
  return () => { s = (s * 1664525 + 1013904223) >>> 0; return s / 4294967296; };
}

function paintLandscape(seed, w, h) {
  const r = rng(seed);
  const c = document.createElement('canvas');
  c.width = w; c.height = h;
  const g = c.getContext('2d');
  const hue = Math.floor(r() * 360);
  const sky = g.createLinearGradient(0, 0, 0, h);
  sky.addColorStop(0, `hsl(${hue}, 60%, ${30 + r() * 30}%)`);
  sky.addColorStop(1, `hsl(${(hue + 40) % 360}, 70%, ${60 + r() * 20}%)`);
  g.fillStyle = sky;
  g.fillRect(0, 0, w, h);
  g.fillStyle = `hsla(${(hue + 180) % 360}, 90%, 85%, .9)`;
  g.beginPath();
  g.arc(w * (0.2 + r() * 0.6), h * (0.2 + r() * 0.25), Math.min(w, h) * (0.06 + r() * 0.08), 0, Math.PI * 2);
  g.fill();
  for (let layer = 0; layer < 3; layer++) {
    g.fillStyle = `hsl(${(hue + 200 + layer * 15) % 360}, 30%, ${35 - layer * 10}%)`;
    g.beginPath();
    g.moveTo(0, h);
    let y = h * (0.45 + layer * 0.15 + r() * 0.1);
    for (let x = 0; x <= w; x += w / 8) {
      y += (r() - 0.5) * h * 0.18;
      g.lineTo(x, y);
    }
    g.lineTo(w, h);
    g.fill();
  }
  return c.toDataURL('image/jpeg', 0.8);
}

const mockApi = (() => {
  const data = { photos: [], files: [], notes: [], trash: [] };
  const urls = new Map();
  // ?mock=in skips the login screen (demo only, no real data involved).
  let authed = new URLSearchParams(location.search).get('mock') === 'in';

  const r = rng(42);
  const now = Date.now();
  const DAY = 86400e3;
  const folder = (ms) => { const d = new Date(ms); return `${d.getFullYear()}/${pad(d.getMonth() + 1)}-${MONTHS[d.getMonth()]}`; };
  const rid = () => Math.random().toString(16).slice(2, 14);
  const rename = (path, name) => path.replace(/[^/]+$/, name);

  for (let i = 0; i < 64; i++) {
    const date = now - Math.floor(r() * r() * 420 * DAY);
    const roll = r();
    const kind = roll < 0.08 ? 'video' : roll < 0.13 ? 'raw' : 'image';
    const ext = kind === 'video' ? 'MOV' : kind === 'raw' ? 'HEIC' : 'JPG';
    const name = `IMG_${String(1000 + i * 7)}.${ext}`;
    const added = Math.max(date, now - Math.floor(r() * 90 * DAY));
    data.photos.push({ path: `${folder(date)}/${name}`, name, size: Math.floor(1.5e6 + r() * 4e6) * (kind === 'video' ? 20 : 1), date, added, kind, seed: i + 1 });
  }
  const texts = {
    'Бюджет.csv': 'Месяц;Доходы;Расходы\nЯнварь;120000;85000\nФевраль;118000;91000\nМарт;125000;79000\n',
    'Список дел.md': '# Дела\n\n- [x] Настроить облако\n- [ ] Загрузить фото с отпуска\n- [ ] Разобрать документы\n',
    'config.json': '[\n  { "name": "camera", "enabled": true },\n  { "name": "backup", "enabled": false }\n]\n',
    'Заметки.txt': 'Пароль от Wi-Fi на даче — спросить у соседа.\nКупить батарейки для пульта.\n',
  };
  const docs = ['Договор аренды.pdf', 'Отчёт за квартал.xlsx', 'Резюме.docx', 'Паспорт скан.pdf', 'Бэкап настроек.zip', 'Презентация.pptx', 'Билеты.pdf', 'Музыка.mp3', ...Object.keys(texts)];
  for (const name of docs) {
    const date = now - Math.floor(r() * 600 * DAY);
    const text = texts[name];
    const added = Math.max(date, now - Math.floor(r() * 120 * DAY));
    data.files.push({ path: `${folder(date)}/${name}`, name, size: text ? new Blob([text]).size : Math.floor(2e4 + r() * 8e6), date, added, kind: 'file', text });
  }
  for (const [content, ago] of [
    ['Идеи для отпуска\n\nГрузия в мае, взять палатку. Посмотреть билеты заранее.', 0.1],
    ['Покупки\n\nМолоко, хлеб, кофе в зёрнах, фильтры для воды', 1.2],
    ['Книги на осень\n\n«Мастер и Маргарита», «Пикник на обочине», что-нибудь Пелевина', 12],
  ]) data.notes.push({ id: rid(), content, updated: now - ago * DAY });

  const sortAll = () => { for (const k of ['photos', 'files']) data[k].sort((a, b) => b.date - a.date); };
  sortAll();

  const toTrash = (section, item, deletedAt = Date.now()) => {
    const t = { id: rid(), section, path: item.path, name: item.name, size: item.size, kind: item.kind, deleted_at: deletedAt, date: item.date, item };
    data.trash.unshift(t);
    return t;
  };
  // A few things already in the trash so the demo shows it.
  for (const [i, ago] of [[5, 3], [17, 12], [30, 27]]) {
    const item = data.photos[i];
    data.photos.splice(i, 1);
    toTrash('photos', item, now - ago * DAY);
  }
  toTrash('files', data.files.pop(), now - 6 * DAY);
  data.trash.sort((a, b) => b.deleted_at - a.deleted_at);

  const noteMeta = (n) => {
    const lines = n.content.split('\n').map((l) => l.trim()).filter(Boolean);
    return {
      id: n.id, updated: n.updated,
      title: (lines[0] || '').replace(/^#+\s*/, '').slice(0, 80) || 'Новая заметка',
      preview: lines.slice(1).join(' ').slice(0, 140),
    };
  };
  const delay = (ms) => new Promise((res) => setTimeout(res, ms));

  const api = {
    async session() { return { authenticated: authed }; },
    async login(u, p) {
      await delay(400);
      if (!u || !p) throw new HttpError(401, 'Неверный логин или пароль');
      authed = true;
      return { ok: true };
    },
    async logout() { authed = false; },
    async list(section) {
      await delay(150);
      const items = data[section].slice();
      return { items, total_size: items.reduce((s, x) => s + x.size, 0) };
    },
    async remove(section, item) {
      data[section] = data[section].filter((x) => x !== item);
      return { trash_id: toTrash(section, item).id };
    },
    async rename(section, item, name) {
      item.name = name;
      item.path = rename(item.path, name);
      item.kind = kindOf(name);
      return { path: item.path };
    },
    async replace(section, item, blob, keepOriginal) {
      if (keepOriginal) toTrash(section, { ...item });
      if (blob.type.startsWith('image/')) item.blob = URL.createObjectURL(blob);
      else item.text = await blob.text();
      item.size = blob.size;
      return { path: item.path };
    },
    async copy(section, item, name, blob) {
      const isImage = blob.type.startsWith('image/');
      const copy = { ...item, name, path: rename(item.path, name), size: blob.size, kind: kindOf(name), added: Date.now(),
        blob: isImage ? URL.createObjectURL(blob) : null, text: isImage ? undefined : await blob.text() };
      data[section].push(copy);
      sortAll();
      return { section, path: copy.path };
    },
    async readText(section, item) { await delay(100); return item.text ?? ''; },
    thumbUrl(section, item) {
      if (item.blob) return item.blob;
      const key = `t${item.seed}`;
      if (!urls.has(key)) urls.set(key, paintLandscape(item.seed, 360, 240));
      return urls.get(key);
    },
    fileUrl(section, item) {
      if (item.blob) return item.blob;
      if (item.kind !== 'image') return `data:text/plain;charset=utf-8,${encodeURIComponent(item.text ?? 'demo')}`;
      const key = `f${item.seed}`;
      if (!urls.has(key)) urls.set(key, paintLandscape(item.seed, 1620, 1080));
      return urls.get(key);
    },
    async upload(file, onProgress) {
      for (let p = 0; p <= 1; p += 0.1) { onProgress(p); await delay(120); }
      const kind = kindOf(file.name);
      const section = kind === 'file' ? 'files' : 'photos';
      const date = file.lastModified || Date.now();
      const item = { path: `${folder(date)}/${file.name}`, name: file.name, size: file.size, date, added: Date.now(), kind,
        blob: kind === 'file' ? null : URL.createObjectURL(file), text: kind === 'file' ? await file.text() : undefined };
      data[section].push(item);
      sortAll();
      return { section, path: item.path };
    },

    async notes() { await delay(100); return { items: data.notes.map(noteMeta).sort((a, b) => b.updated - a.updated) }; },
    async noteCreate(content) {
      const n = { id: rid(), content, updated: Date.now() };
      data.notes.push(n);
      return { id: n.id, updated: n.updated };
    },
    async noteGet(id) {
      const n = data.notes.find((x) => x.id === id);
      if (!n) throw new HttpError(404, 'Заметка не найдена');
      return { ...n };
    },
    async noteSave(id, content) {
      const n = data.notes.find((x) => x.id === id);
      n.content = content;
      n.updated = Date.now();
      return { updated: n.updated };
    },
    async noteDelete(id, permanent) {
      const n = data.notes.find((x) => x.id === id);
      data.notes = data.notes.filter((x) => x !== n);
      if (permanent) return { ok: true };
      const t = toTrash('notes', { path: `${id}.md`, name: `${id}.md`, size: n.content.length, kind: 'file', date: n.updated });
      t.note = n;
      return { trash_id: t.id };
    },

    async zipStart() { throw new HttpError(0, 'В демо-режиме архивы не создаются'); },

    async trash() {
      await delay(120);
      const items = data.trash.map(({ item, note, ...t }) => ({ ...t, days_left: Math.max(1, 30 - Math.floor((Date.now() - t.deleted_at) / DAY)) }));
      return { items, retention_days: 30 };
    },
    async restore(id) {
      const t = data.trash.find((x) => x.id === id);
      data.trash = data.trash.filter((x) => x !== t);
      if (t.note) data.notes.push(t.note);
      else data[t.section].push(t.item);
      sortAll();
      return { section: t.section, path: t.path };
    },
    async purge(id) { data.trash = data.trash.filter((x) => x.id !== id); },
    async emptyTrash() { const n = data.trash.length; data.trash = []; return { removed: n }; },
    trashThumbUrl(t) {
      const full = data.trash.find((x) => x.id === t.id);
      return full?.item ? api.thumbUrl(t.section, full.item) : '';
    },
    trashFileUrl(t) {
      const full = data.trash.find((x) => x.id === t.id);
      return full?.item ? api.fileUrl(t.section, full.item) : '';
    },
  };
  return api;
})();

const api = MOCK ? mockApi : realApi;

/** What the small button next to the large title does (e.g. "Очистить" in the trash). */
let headAction = null;
/** Saves the open note right now (set by notes.js while a note is open). */
let flushNote = null;

// ---------- state ----------

const state = {
  section: 'photos',
  data: { photos: null, files: null, notes: null, trash: null },
  viewer: [],   // items shown in the lightbox, in display order
  index: -1,
};

const tiles = new WeakMap();       // item -> its tile in the current grid
const rows = new WeakMap();        // item -> its row in the current file list
const shownThumbs = new Set();     // thumbnails already faded in once; don't fade again on re-render

// ---------- login / session ----------

function showLogin() {
  flushNote?.();   // keep what was typed in an open note
  if (sel.on) { sel.on = false; sel.keys.clear(); document.body.classList.remove('selecting'); $('#selbar').hidden = true; }
  for (const id of ['#app', '#lightbox', '#editor', '#doc', '#sheet', '#prompt', '#modal']) $(id).hidden = true;
  viewer.open = false;
  const form = $('#login-form');
  $('#login').hidden = false;
  animate(form, [{ opacity: 0, transform: 'scale(.96)' }, { opacity: 1, transform: 'none' }], 'smooth', { fill: 'none' });
  form.username.focus();
}

function showApp() {
  $('#login').hidden = true;
  $('#demo-chip').hidden = !MOCK;
  state.data = { photos: null, files: null, notes: null, trash: null };
  showSectionHeader();
  enter($('#app'), [{ opacity: 0 }, { opacity: 1 }], 'smooth', { duration: 400, easing: 'ease-out' });
  animate($('#content'), [{ transform: 'translateY(16px)' }, { transform: 'none' }], 'smooth', { fill: 'none' });
  onScroll();
  load(state.section);
  // The terminal is off unless the computer's owner turned it on (local-photo --terminal on).
  $('#term-btn').hidden = true;
  if (!MOCK) api.session().then((s) => { $('#term-btn').hidden = !s?.terminal; }).catch(() => {});
}

async function onLogin(e) {
  e.preventDefault();
  const form = e.target;
  const err = $('#login-error');
  const btn = $('#login-submit');
  err.hidden = true;
  btn.disabled = true;
  try {
    await api.login(form.username.value.trim(), form.password.value);
    form.password.value = '';
    const a = await animate(form, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'scale(1.06)' }], 'snappy', { duration: 260 });
    showApp();
    a?.cancel?.();
  } catch (ex) {
    err.textContent = ex.message;
    err.hidden = false;
    shake(form);
  } finally {
    btn.disabled = false;
  }
}

// ---------- sections ----------

const SECTIONS = ['photos', 'files', 'notes', 'trash'];
const TITLES = { photos: 'Фото', files: 'Файлы', notes: 'Заметки', trash: 'Корзина' };

// Top segmented control, the phone tab bar and the trash button all switch sections.
function setSection(section) {
  exitSelection();
  if (section === state.section) {
    window.scrollTo({ top: 0, behavior: REDUCED ? 'auto' : 'smooth' });   // iOS: tap the active tab to go to top
    return;
  }
  const dir = SECTIONS.indexOf(section) > SECTIONS.indexOf(state.section) ? 1 : -1;
  for (const group of [$('#seg'), $('#tabbar')]) {
    group.dataset.active = section;
    const thumb = group.querySelector('.thumb');
    thumb.classList.remove('stretch');
    void thumb.offsetWidth;   // restart the stretch animation
    thumb.classList.add('stretch');
  }
  document.querySelectorAll('.tab[data-section]').forEach((t) => t.classList.toggle('active', t.dataset.section === section));
  $('#trash-btn').classList.toggle('active', section === 'trash');
  state.section = section;
  updateAddButtons();

  const moving = [$('#content'), $('.page-head')];
  Promise.all(moving.map((n) => animate(n,
    [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: `translateX(${-dir * 28}px)` }],
    'snappy', { duration: 150, easing: 'ease-in' }))).then((done) => {
    if (state.section !== section) return done.forEach((a) => a?.cancel?.());   // switched again meanwhile
    window.scrollTo(0, 0);
    showSectionHeader();
    if (state.data[section]) render();
    else $('#content').replaceChildren(el('div', { class: 'loading' }));
    done.forEach((a) => a?.cancel?.());
    moving.forEach((n) => animate(n, [{ opacity: 0, transform: `translateX(${dir * 28}px)` }, { opacity: 1, transform: 'none' }], 'smooth', { fill: 'none' }));
    load(section);
  });
}

// "+" uploads in Photos/Files, creates a note in Notes, and is not needed in the trash.
function updateAddButtons() {
  const fab = $('#upload-fab');
  const show = state.section !== 'trash';
  if (show && fab.hidden) enter(fab, [{ opacity: 0, transform: 'scale(.6)' }, { opacity: 1, transform: 'none' }], 'bouncy');
  if (!show) leave(fab, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'scale(.6)' }]);
  $('#upload-btn').hidden = !show;
  const label = state.section === 'notes' ? 'Новая заметка' : 'Загрузить';
  $('#upload-btn').title = fab.title = label;
}

function onAdd() {
  if (state.section === 'notes') newNote();
  else $('#file-input').click();
}

function showSectionHeader() {
  const name = TITLES[state.section];
  $('#page-title').textContent = name;
  $('#nav-title').textContent = name;
  $('#head-action').hidden = true;
  $('#select-btn').hidden = !(state.section === 'photos' || state.section === 'files');
  // Each section starts unfiltered; render() shows the field once there is something to search.
  $('#search').value = '';
  $('#search-box').hidden = true;
}

// ---------- search ----------

const normalize = (s) => s.toLowerCase().replace(/ё/g, 'е');

/** Words typed in the search field; an item matches if its text contains all of them. */
function searchWords() {
  return normalize($('#search').value).split(/\s+/).filter(Boolean);
}

function filterByQuery(items, textOf) {
  const words = searchWords();
  if (!words.length) return items;
  return items.filter((item) => {
    const text = normalize(textOf(item));
    return words.every((w) => text.includes(w));
  });
}

function nothingFound() {
  return el('div', { class: 'empty' },
    el('div', { text: 'Ничего не найдено' }),
    el('div', { class: 'hint', text: `По запросу «${$('#search').value.trim()}»` }));
}

function bindSearch() {
  const input = $('#search');
  let frame = 0;
  input.addEventListener('input', () => {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => { exitSelection(); render(); });
  });
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      e.stopPropagation();
      if (input.value) { input.value = ''; render(); } else input.blur();
    }
    if (e.key === 'Enter') input.blur();   // hides the phone keyboard
  });
}

// iOS bars: transparent until content scrolls under them; the small title
// appears in the bar once the large title has scrolled away.
let scrollQueued = false;
function onScroll() {
  if (scrollQueued) return;
  scrollQueued = true;
  requestAnimationFrame(() => {
    scrollQueued = false;
    const nav = $('#nav');
    nav.classList.toggle('scrolled', window.scrollY > 4);
    nav.classList.toggle('titled', $('#page-title').getBoundingClientRect().bottom < nav.getBoundingClientRect().bottom);
  });
}

// ---------- loading & rendering ----------

const FETCH = {
  photos: () => api.list('photos'),
  files: () => api.list('files'),
  notes: () => api.notes(),
  trash: () => api.trash(),
};

async function load(section) {
  const content = $('#content');
  if (!state.data[section] && state.section === section) {
    content.replaceChildren(el('div', { class: 'loading' }));
  }
  try {
    state.data[section] = await FETCH[section]();
  } catch (e) {
    if (e.status !== 401 && state.section === section) content.replaceChildren(el('div', { class: 'empty', text: e.message }));
    return;
  }
  if (state.section === section) render();
}

/** Reload whichever sections an operation may have touched. */
function reload(...sections) {
  return Promise.all(sections.map(load));
}

// Group items by their "YYYY/MM-Месяц" folder. Anything that doesn't sit in
// such a folder (e.g. put there by hand) goes into a trailing "Другое" group.
function groupByMonth(items) {
  const groups = new Map();
  for (const item of items) {
    const parts = item.path.split('/');
    const ok = parts.length >= 3 && /^\d{4}$/.test(parts[0]) && /^\d{2}-/.test(parts[1]);
    const key = ok ? `${parts[0]}/${parts[1]}` : '~';
    if (!groups.has(key)) groups.set(key, { year: ok ? parts[0] : null, month: ok ? parts[1].slice(3) : 'Другое', items: [] });
    groups.get(key).items.push(item);
  }
  return [...groups.entries()]
    .sort(([a], [b]) => (a === '~') - (b === '~') || b.localeCompare(a))
    .map(([, g]) => { g.items.sort((a, b) => b.date - a.date); return g; });
}

// Sort by when it was taken/created (folders on disk) or by when it was uploaded.
const SORTS = {
  photos: { taken: 'По дате съёмки', added: 'По дате загрузки' },
  files: { taken: 'По дате создания', added: 'По дате загрузки' },
};
const sortOrder = (() => {
  try { return { photos: 'taken', files: 'taken', ...JSON.parse(localStorage.getItem('lp-sort') || '{}') }; }
  catch { return { photos: 'taken', files: 'taken' }; }
})();

function chooseSort() {
  const section = state.section;
  return actionSheet('Сортировка', Object.entries(SORTS[section]).map(([key, label]) => ({
    label: key === sortOrder[section] ? `✓ ${label}` : label,
    run: () => {
      if (key === sortOrder[section]) return;
      sortOrder[section] = key;
      try { localStorage.setItem('lp-sort', JSON.stringify(sortOrder)); } catch { /* private mode */ }
      window.scrollTo(0, 0);
      render();
      animate($('#content'), [{ opacity: 0, transform: 'translateY(12px)' }, { opacity: 1, transform: 'none' }], 'smooth', { fill: 'none' });
    },
  })));
}

/** Months by upload time, newest first. */
function groupByAdded(items) {
  const groups = new Map();
  for (const item of [...items].sort((a, b) => b.added - a.added)) {
    const d = new Date(item.added);
    const key = `${d.getFullYear()}/${pad(d.getMonth() + 1)}`;
    if (!groups.has(key)) groups.set(key, { year: String(d.getFullYear()), month: MONTHS[d.getMonth()], items: [] });
    groups.get(key).items.push(item);
  }
  return [...groups.values()];
}

function render() {
  const section = state.section;
  const list = state.data[section];
  if (!list) return;
  if (section === 'notes') return renderNotes(list);
  if (section === 'trash') return renderTrash(list);
  const { total_size } = list;
  const all = list.items;
  const items = filterByQuery(all, (i) => i.name);
  const content = $('#content');

  const n = all.length;
  $('#search-box').hidden = !n;
  const byAdded = sortOrder[section] === 'added';
  const action = $('#head-action');
  action.hidden = !n;
  action.textContent = `${SORTS[section][sortOrder[section]]} ▾`;
  headAction = chooseSort;
  $('#summary').textContent = n
    ? `${n} ${section === 'photos' ? 'фото и видео' : plural(n, 'файл', 'файла', 'файлов')} · ${fmtSize(total_size)}`
    : '';

  if (!n) {
    content.replaceChildren(el('div', { class: 'empty' },
      icon(section === 'photos' ? 'image' : 'cloud'),
      el('div', { text: 'Здесь пока пусто' }),
      el('div', { class: 'hint', text: 'Нажмите «+» или перетащите файлы в окно' })));
    return;
  }
  if (items.length !== n) $('#summary').textContent = `Найдено ${items.length} из ${n}`;
  if (!items.length) {
    state.flat = [];
    content.replaceChildren(nothingFound());
    return;
  }

  // A re-render while the viewer is open (upload finished, file deleted) must keep it on the same photo.
  const openPath = viewer.open && section === 'photos' ? state.viewer[state.index]?.path : null;
  const groups = byAdded ? groupByAdded(items) : groupByMonth(items);
  state.flat = groups.flatMap((g) => g.items);   // display order, for range selection
  if (section === 'photos') state.viewer = state.flat;
  const frag = document.createDocumentFragment();
  for (const g of groups) {
    // Header and items share a block so the sticky header stops at the end of its month.
    frag.append(el('section', { class: 'month-block' },
      el('div', { class: 'month' },
        el('span', { class: 'month-chip glass' }, g.month + (g.year ? ` ${g.year}` : ''), el('span', { class: 'count', text: String(g.items.length) }))),
      section === 'photos' ? renderGrid(g.items) : renderFiles(g.items)));
  }
  content.replaceChildren(frag);
  reapplySelection();
  if (openPath) {
    const idx = state.viewer.findIndex((i) => i.path === openPath);
    if (idx >= 0) state.index = idx;
    liftTile(state.viewer[state.index]);
  }
}

function renderGrid(items) {
  const grid = el('div', { class: 'grid' });
  for (const item of items) {
    const tile = el('button', { class: 'tile', title: item.name, onclick: () => onItemTap('photos', item) });
    if (item.kind === 'image') {
      const src = api.thumbUrl('photos', item);
      const img = el('img', { src, alt: item.name, loading: 'lazy', decoding: 'async', draggable: 'false' });
      if (shownThumbs.has(src)) img.classList.add('loaded');
      img.onload = () => { shownThumbs.add(src); img.classList.add('loaded'); };
      img.onerror = () => img.replaceWith(el('div', { class: 'placeholder' }, icon('image'), item.name.split('.').pop().toUpperCase()));
      tile.append(img);
    } else if (item.kind === 'video' && item.blob) {
      tile.append(el('video', { src: item.blob, muted: true, preload: 'metadata' }));
    } else {
      tile.append(el('div', { class: 'placeholder' }, icon(item.kind === 'video' ? 'play' : 'image'), item.name.split('.').pop().toUpperCase()));
    }
    if (item.kind === 'video') tile.append(el('span', { class: 'badge' }, icon('play')));
    tile.append(el('span', { class: 'check' }, icon('check')));
    tiles.set(item, tile);
    nodeItem.set(tile, item);
    grid.append(tile);
  }
  return grid;
}

function renderFiles(items) {
  const box = el('div', { class: 'files' });
  for (const item of items) {
    const ext = splitName(item.name)[1];
    const row = el('div', { class: 'file-row', onclick: () => onItemTap('files', item) },
      el('span', { class: 'check' }, icon('check')),
      el('div', { class: 'file-icon', text: ext.slice(0, 4) || '—' }),
      el('div', { class: 'file-main' },
        el('div', { class: 'file-name', text: item.name }),
        // Two spans: one line on a phone, two columns on a wide screen.
        el('div', { class: 'file-meta' },
          el('span', { class: 'file-size', text: fmtSize(item.size) }),
          el('span', { class: 'file-date', text: sortOrder.files === 'added' ? `загружен ${fmtDate(item.added)}` : fmtDate(item.date) }))),
      el('button', { class: 'btn round', title: 'Действия', onclick: (e) => { e.stopPropagation(); itemActions('files', item); } }, icon('more')));
    rows.set(item, row);
    nodeItem.set(row, item);
    box.append(row);
  }
  return box;
}

/** Tap on a file: text opens in the editor, images in the photo editor, the rest downloads. */
function openFile(section, item) {
  if (isTextFile(item.name)) return openTextEditor(section, item);
  triggerDownload(api.fileUrl(section, item, true), item.name);
}

/** The "⋯" menu for a photo or file. */
function itemActions(section, item) {
  const canEditPhoto = item.kind === 'image' || item.kind === 'raw';
  const media = item.kind !== 'file';
  return actionSheet(item.name, [
    CAN_SHARE_FILES && { label: media ? 'Сохранить в галерею' : 'Поделиться', hint: media ? 'в меню выберите «Сохранить изображение/видео»' : '', run: () => shareItem(section, item) },
    isTextFile(item.name) && { label: 'Редактировать', run: () => openTextEditor(section, item) },
    canEditPhoto && { label: 'Редактировать фото', run: () => openEditor(section, item) },
    { label: 'Переименовать', run: () => renameItem(section, item) },
    ...conversionsFor(section, item),
    { label: 'Скачать', run: () => triggerDownload(api.fileUrl(section, item, true), item.name) },
    { label: 'Удалить', destructive: true, run: () => (section === 'photos' && viewer.open ? deleteFromViewer() : deleteItem(section, item)) },
  ]);
}

async function renameItem(section, item) {
  const name = await promptDialog('Переименовать', item.name, { selectStem: true });
  if (!name || name === item.name) return;
  try {
    const res = await api.rename(section, item, name);
    if (viewer.open) { item.path = res.path; item.name = res.path.split('/').pop(); $('#lb-title').textContent = item.name; }
    toast('Переименовано');
  } catch (e) {
    return toast(e.message);
  }
  await load(section);
}

async function deleteItem(section, item, { confirm = true } = {}) {
  if (confirm && !(await confirmDialog(`«${item.name}» будет перемещён в корзину. Его можно восстановить в течение 30 дней.`, { title: 'Удалить?' }))) return false;
  let res;
  try {
    res = await api.remove(section, item);
  } catch (e) {
    toast(e.message);
    return false;
  }
  // Let the item visibly go away before the list closes the gap.
  const node = section === 'photos' ? tiles.get(item) : rows.get(item);
  if (node?.isConnected && !viewer.open) {
    if (section === 'photos') {
      await animate(node, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'scale(.6)' }], 'snappy');
    } else {
      const h = node.offsetHeight;
      await animate(node, [{ opacity: 1, height: `${h}px` }, { opacity: 0, height: '0px', paddingTop: '0px', paddingBottom: '0px' }], 'snappy');
    }
  }
  state.data.trash = null;
  toast('Перемещено в корзину', { action: 'Отменить', onAction: () => undoDelete(res.trash_id, section) });
  await load(section);
  return true;
}

async function undoDelete(trashId, section) {
  try {
    await api.restore(trashId);
  } catch (e) {
    return toast(e.message);
  }
  state.data.trash = null;
  await load(section);
}

// ---------- viewer ----------
// Photos grow out of their tile and shrink back into it (like iOS Photos);
// swipe sideways to page, drag down to dismiss, tap to hide the controls.

const viewer = { open: false, aspect: null, rect: null };

function fitRect(aspect) {
  const W = window.innerWidth, H = window.innerHeight;
  if (!aspect) return { x: 0, y: 0, w: W, h: H };
  let w = W, h = W / aspect;
  if (h > H) { h = H; w = H * aspect; }
  return { x: (W - w) / 2, y: (H - h) / 2, w, h };
}

function placeFrame(aspect) {
  viewer.aspect = aspect;
  viewer.rect = fitRect(aspect);
  const f = $('#lb-frame').style;
  f.left = `${viewer.rect.x}px`;
  f.top = `${viewer.rect.y}px`;
  f.width = `${viewer.rect.w}px`;
  f.height = `${viewer.rect.h}px`;
}

function tileImage(item) {
  const img = tiles.get(item)?.querySelector('img');
  return img && img.complete && img.naturalWidth ? img : null;
}

function tileRectOnScreen(item) {
  const tile = tiles.get(item);
  if (!tile?.isConnected) return null;
  const r = tile.getBoundingClientRect();
  return r.width && r.bottom > 0 && r.top < window.innerHeight ? r : null;
}

// Keyframe that makes the fitted frame look exactly like its square, cover-cropped tile.
function asTile(tileRect) {
  const f = viewer.rect;
  const k = Math.max(tileRect.width / f.w, tileRect.height / f.h);
  const dx = tileRect.left + tileRect.width / 2 - (f.x + f.w / 2);
  const dy = tileRect.top + tileRect.height / 2 - (f.y + f.h / 2);
  const ix = Math.max(0, (f.w - tileRect.width / k) / 2);
  const iy = Math.max(0, (f.h - tileRect.height / k) / 2);
  return { transform: `translate(${dx}px, ${dy}px) scale(${k})`, clipPath: `inset(${iy}px ${ix}px round ${2 / k}px)` };
}
const FULL_CLIP = 'inset(0px 0px round 0px)';

function liftTile(item) {
  document.querySelectorAll('.tile.lifted').forEach((t) => t.classList.remove('lifted'));
  if (viewer.open && item) tiles.get(item)?.classList.add('lifted');
}

function fillFrame(item) {
  const img = $('#lb-img'), video = $('#lb-video'), fallback = $('#lb-fallback');
  $('#lb-title').textContent = item.name;
  // Upload date only when it says something new (another day than the photo's own date).
  const uploaded = item.added && new Date(item.added).toDateString() !== new Date(item.date).toDateString();
  $('#lb-meta').textContent = `${fmtDate(item.date)} · ${fmtSize(item.size)}${uploaded ? ` · загружено ${noteDateShort(item.added)}` : ''}`;
  $('#lb-edit').hidden = !(item.kind === 'image' || item.kind === 'raw');
  $('#lb-share').hidden = !CAN_SHARE_FILES;
  if (CAN_SHARE_FILES && item.kind !== 'video') prepareShare('photos', item);
  $('#lb-prev').disabled = state.index <= 0;
  $('#lb-next').disabled = state.index >= state.viewer.length - 1;
  fallback.hidden = true;
  video.pause();
  liftTile(item);

  const full = api.fileUrl('photos', item, false);
  if (item.kind === 'video') {
    img.hidden = true;
    img.removeAttribute('src');
    placeFrame(null);
    video.hidden = false;
    video.src = full;
    return;
  }
  video.hidden = true;
  video.removeAttribute('src');
  img.hidden = false;

  // Show the thumbnail we already have instantly, then swap in the full photo.
  const thumb = tileImage(item);
  placeFrame(thumb ? thumb.naturalWidth / thumb.naturalHeight : null);
  img.src = thumb ? thumb.currentSrc : full;
  const pre = new Image();
  pre.onload = () => {
    if (state.viewer[state.index] !== item) return;
    if (!viewer.aspect) placeFrame(pre.naturalWidth / pre.naturalHeight);
    img.src = full;
  };
  pre.onerror = () => {
    if (state.viewer[state.index] !== item) return;
    img.hidden = true;
    fallback.hidden = false;
  };
  pre.src = full;

  // Warm up the neighbours so swiping feels instant.
  for (const d of [-1, 1]) {
    const nb = state.viewer[state.index + d];
    if (nb && nb.kind === 'image') new Image().src = api.fileUrl('photos', nb, false);
  }
}

async function openViewer(item) {
  if (viewer.open) return;
  viewer.open = true;
  state.index = state.viewer.indexOf(item);
  const lb = $('#lightbox'), frame = $('#lb-frame');
  lb.classList.remove('bare');
  lb.hidden = false;
  const tileRect = tileRectOnScreen(item);   // measure before the tile is hidden
  fillFrame(item);

  animate($('#lb-bg'), [{ opacity: 0 }, { opacity: 1 }], 'smooth', { fill: 'none', duration: 320, easing: 'ease-out' });
  animate($('.lb-chrome'), [{ opacity: 0 }, { opacity: 1 }], 'smooth', { fill: 'none', duration: 280, delay: 120, easing: 'ease-out' });
  if (tileRect && viewer.aspect) {
    await animate(frame, [asTile(tileRect), { transform: 'none', clipPath: FULL_CLIP }], 'smooth', { fill: 'none' });
  } else {
    await animate(frame, [{ opacity: 0, transform: 'scale(.9)' }, { opacity: 1, transform: 'none' }], 'smooth', { fill: 'none' });
  }
}

async function closeViewer(drag) {
  if (!viewer.open) return;
  viewer.open = false;
  const lb = $('#lightbox'), frame = $('#lb-frame'), bg = $('#lb-bg'), chrome = $('.lb-chrome');
  const item = state.viewer[state.index];
  const video = $('#lb-video');
  video.pause();

  let tileRect = item && tileRectOnScreen(item);
  if (!tileRect && item && tiles.get(item)?.isConnected) {
    tiles.get(item).scrollIntoView({ block: 'center' });   // paged far away: bring its tile into view
    tileRect = tileRectOnScreen(item);
  }
  const from = { transform: drag?.transform || 'none', clipPath: FULL_CLIP };
  frame.style.transform = '';
  bg.style.opacity = '';

  animate(bg, [{ opacity: drag?.bg ?? 1 }, { opacity: 0 }], 'snappy', { easing: 'ease-out', duration: 300 });
  animate(chrome, [{ opacity: 1 }, { opacity: 0 }], 'snappy', { easing: 'ease-out', duration: 150 });
  if (tileRect && viewer.aspect && item.kind !== 'video') {
    await animate(frame, [from, asTile(tileRect)], 'smooth');
  } else {
    await animate(frame, [{ ...from, opacity: 1 }, { transform: `${from.transform} scale(.8)`, opacity: 0 }], 'snappy');
  }

  lb.hidden = true;
  liftTile(null);
  for (const n of [frame, bg, chrome]) n.getAnimations().forEach((a) => a.cancel());
  document.querySelectorAll('.lb-frame.ghost').forEach((g) => g.remove());
  video.removeAttribute('src');
  video.load();
  state.index = -1;
}

// Page to the previous/next photo. `fromX` is where a swipe left the photo.
async function stepViewer(dir, fromX = 0) {
  const frame = $('#lb-frame');
  const next = state.index + dir;
  frame.style.transform = '';
  if (next < 0 || next >= state.viewer.length) {
    if (fromX) await animate(frame, [{ transform: `translateX(${fromX}px)` }, { transform: 'none' }], 'bouncy', { fill: 'none' });
    return;
  }
  const W = window.innerWidth + 24;   // a small gap between photos, as in Photos
  const ghost = frame.cloneNode(true);
  ghost.removeAttribute('id');
  ghost.classList.add('ghost');
  ghost.querySelectorAll('[id]').forEach((n) => n.removeAttribute('id'));
  ghost.querySelector('video')?.removeAttribute('src');
  frame.before(ghost);

  state.index = next;
  fillFrame(state.viewer[next]);
  animate(ghost, [{ transform: `translateX(${fromX}px)` }, { transform: `translateX(${-dir * W}px)` }], 'smooth')
    .then(() => ghost.remove());
  await animate(frame, [{ transform: `translateX(${dir * W + fromX}px)` }, { transform: 'none' }], 'smooth', { fill: 'none' });
}

async function deleteFromViewer() {
  const item = state.viewer[state.index];
  if (!item) return;
  const idx = state.index;
  const frame = $('#lb-frame');
  if (!(await deleteItem('photos', item))) return;
  if (!state.viewer.length) return closeViewer();
  const out = await animate(frame, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'scale(.85)' }], 'snappy');
  state.index = Math.min(idx, state.viewer.length - 1);
  fillFrame(state.viewer[state.index]);
  out?.cancel?.();
  animate(frame, [{ opacity: 0, transform: 'scale(.92)' }, { opacity: 1, transform: 'none' }], 'smooth', { fill: 'none' });
}

function bindViewerGestures() {
  const lb = $('#lightbox'), stage = $('#lb-stage'), frame = $('#lb-frame'), bg = $('#lb-bg');
  let d = null;

  stage.addEventListener('pointerdown', (e) => {
    if (!viewer.open || e.button > 0) return;
    if (e.target.tagName === 'VIDEO' && e.pointerType === 'mouse') return;   // let the mouse use video controls
    d = { id: e.pointerId, x0: e.clientX, y0: e.clientY, dx: 0, dy: 0, vx: 0, vy: 0, lx: e.clientX, ly: e.clientY, lt: e.timeStamp, axis: null };
  });

  stage.addEventListener('pointermove', (e) => {
    if (!d || e.pointerId !== d.id) return;
    d.dx = e.clientX - d.x0;
    d.dy = e.clientY - d.y0;
    const dt = Math.max(1, e.timeStamp - d.lt);
    d.vx = 0.7 * ((e.clientX - d.lx) / dt) + 0.3 * d.vx;
    d.vy = 0.7 * ((e.clientY - d.ly) / dt) + 0.3 * d.vy;
    d.lx = e.clientX; d.ly = e.clientY; d.lt = e.timeStamp;

    if (!d.axis) {
      if (Math.hypot(d.dx, d.dy) < 10) return;
      d.axis = Math.abs(d.dx) > Math.abs(d.dy) ? 'x' : 'y';
      stage.setPointerCapture(d.id);
      frame.getAnimations().forEach((a) => a.finish());
      if (d.axis === 'y') lb.classList.add('bare');
    }
    if (d.axis === 'x') {
      const atEdge = (d.dx > 0 && state.index === 0) || (d.dx < 0 && state.index === state.viewer.length - 1);
      d.x = atEdge ? d.dx * 0.3 : d.dx;   // rubber band past the first/last photo
      frame.style.transform = `translateX(${d.x}px)`;
    } else {
      const y = d.dy < 0 ? d.dy * 0.25 : d.dy;
      const p = Math.min(Math.abs(y) / window.innerHeight, 1);
      d.transform = `translate(${d.dx * 0.6}px, ${y}px) scale(${1 - p * 0.45})`;
      d.bg = 1 - Math.min(Math.abs(y) / (window.innerHeight * 0.45), 1);
      frame.style.transform = d.transform;
      bg.style.opacity = String(d.bg);
    }
  });

  const end = (e) => {
    if (!d || e.pointerId !== d.id) return;
    const g = d;
    d = null;
    if (!g.axis) {
      if (e.type === 'pointerup' && e.target.tagName !== 'VIDEO') lb.classList.toggle('bare');
      return;
    }
    if (g.axis === 'x') {
      const dir = g.dx < 0 ? 1 : -1;
      const flung = Math.abs(g.vx) > 0.4 && Math.sign(g.vx) === -dir;
      if (Math.abs(g.dx) > window.innerWidth * 0.2 || flung) {
        stepViewer(dir, g.x);
      } else {
        frame.style.transform = '';
        animate(frame, [{ transform: `translateX(${g.x}px)` }, { transform: 'none' }], 'bouncy', { fill: 'none' });
      }
    } else if (g.dy > 110 || g.vy > 0.55) {
      closeViewer(g);
    } else {
      frame.style.transform = '';
      bg.style.opacity = '';
      lb.classList.remove('bare');
      animate(frame, [{ transform: g.transform }, { transform: 'none' }], 'bouncy', { fill: 'none' });
      animate(bg, [{ opacity: g.bg }, { opacity: 1 }], 'snappy', { fill: 'none' });
    }
  };
  stage.addEventListener('pointerup', end);
  stage.addEventListener('pointercancel', end);

  window.addEventListener('resize', () => { if (viewer.open) placeFrame(viewer.aspect); });
}

// ---------- uploads ----------

const uploadQueue = [];
let uploading = false;
let uploadStats = { done: 0, total: 0, failed: 0 };
let uploadsHideTimer;

const UPLOADS_IN = [{ opacity: 0, transform: 'translateY(30px) scale(.94)' }, { opacity: 1, transform: 'none' }];
const UPLOADS_OUT = [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'translateY(30px) scale(.94)' }];

function enqueueUploads(files) {
  if (!files.length) return;
  clearTimeout(uploadsHideTimer);
  const panel = $('#uploads');
  if (panel.hidden) enter(panel, UPLOADS_IN, 'bouncy');
  for (const file of files) {
    const bar = el('div');
    const status = el('span', { class: 'up-status', text: 'в очереди' });
    const row = el('div', { class: 'up-row' },
      el('div', { class: 'up-top' }, el('span', { class: 'up-name', text: file.name }), status),
      el('div', { class: 'up-bar' }, bar));
    $('#uploads-list').append(row);
    animate(row, [{ opacity: 0, transform: 'translateY(8px)' }, { opacity: 1, transform: 'none' }], 'smooth', { fill: 'none' });
    uploadQueue.push({ file, bar, status });
  }
  uploadStats.total += files.length;
  updateUploadTitle();
  if (!uploading) runUploads();
}

function updateUploadTitle() {
  $('#uploads-title').textContent = uploading || uploadQueue.length
    ? `Загрузка ${uploadStats.done} из ${uploadStats.total}`
    : `Загружено ${uploadStats.done} из ${uploadStats.total}`;
}

async function closeUploads() {
  if (uploading) return toast('Загрузка ещё идёт');
  await leave($('#uploads'), UPLOADS_OUT);
  $('#uploads-list').replaceChildren();
  uploadStats = { done: 0, total: 0, failed: 0 };
}

async function runUploads() {
  uploading = true;
  while (uploadQueue.length) {
    const job = uploadQueue.shift();
    job.status.textContent = '0%';
    try {
      const res = await api.upload(job.file, (p) => {
        job.bar.style.width = `${Math.round(p * 100)}%`;
        job.status.textContent = `${Math.round(p * 100)}%`;
      });
      job.status.textContent = res.section === 'photos' ? '→ Фото' : '→ Файлы';
      job.status.classList.add('ok');
      uploadStats.done++;
    } catch (e) {
      job.status.textContent = e.message;
      job.status.classList.add('err');
      uploadStats.failed++;
      if (e.status === 401) break;
    }
    updateUploadTitle();
  }
  uploading = false;
  updateUploadTitle();
  await reload('photos', 'files');
  // Everything went fine: tuck the panel away by itself.
  if (!uploadStats.failed) uploadsHideTimer = setTimeout(closeUploads, 2500);
}

// ---------- wiring ----------

function bindUi() {
  $('#login-form').addEventListener('submit', onLogin);

  document.querySelectorAll('.tab[data-section]').forEach((tab) => tab.addEventListener('click', () => setSection(tab.dataset.section)));
  window.addEventListener('scroll', onScroll, { passive: true });

  $('#upload-btn').addEventListener('click', onAdd);
  $('#upload-fab').addEventListener('click', onAdd);
  $('#trash-btn').addEventListener('click', () => setSection('trash'));
  $('#head-action').addEventListener('click', () => headAction?.());
  $('#file-input').addEventListener('change', (e) => {
    enqueueUploads([...e.target.files]);
    e.target.value = '';
  });

  $('#logout-btn').addEventListener('click', async () => {
    await api.logout().catch(() => {});
    await leave($('#app'), [{ opacity: 1 }, { opacity: 0 }], 'snappy', { duration: 200, easing: 'ease-in' });
    showLogin();
  });

  $('#uploads-close').addEventListener('click', closeUploads);

  $('#lb-close').addEventListener('click', () => closeViewer());
  $('#lb-prev').addEventListener('click', () => stepViewer(-1));
  $('#lb-next').addEventListener('click', () => stepViewer(1));
  $('#lb-delete').addEventListener('click', deleteFromViewer);
  $('#lb-share').addEventListener('click', () => {
    const item = state.viewer[state.index];
    if (item) shareItem('photos', item);
  });
  $('#lb-edit').addEventListener('click', () => {
    const item = state.viewer[state.index];
    if (item) openEditor('photos', item);
  });
  $('#lb-more').addEventListener('click', () => {
    const item = state.viewer[state.index];
    if (item) itemActions('photos', item);
  });
  bindViewerGestures();
  bindSearch();

  // Desktop shortcuts on the main screen: "/" searches, 1–3 switch sections.
  const typing = (t) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
  const overlayOpen = () => viewer.open || ['#modal', '#prompt', '#sheet', '#editor', '#doc'].some((s) => !$(s).hidden);
  document.addEventListener('keydown', (e) => {
    if ($('#app').hidden || overlayOpen() || typing(e.target) || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === '/' && !$('#search-box').hidden) { e.preventDefault(); $('#search').focus(); }
    else if (['1', '2', '3'].includes(e.key)) setSection(SECTIONS[Number(e.key) - 1]);
  });
  // Pasting a screenshot or copied files uploads them.
  window.addEventListener('paste', (e) => {
    const files = [...(e.clipboardData?.files || [])];
    if (!files.length || $('#app').hidden || overlayOpen() || typing(e.target)) return;
    if (state.section !== 'photos' && state.section !== 'files') return;
    e.preventDefault();
    enqueueUploads(files.map((f, i) => (f.name && f.name !== 'image.png' ? f
      : new File([f], `Вставка ${new Date().toISOString().slice(0, 19).replace('T', ' ').replace(/:/g, '-')}${i ? `-${i}` : ''}.${f.type.split('/')[1] || 'png'}`, { type: f.type }))));
  });

  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !$('#sheet').hidden) return $('#sheet-cancel').click();
    if (!viewer.open || !$('#modal').hidden || !$('#editor').hidden || !$('#sheet').hidden || !$('#prompt').hidden) return;
    if (e.key === 'Escape') closeViewer();
    else if (e.key === 'ArrowLeft') stepViewer(-1);
    else if (e.key === 'ArrowRight') stepViewer(1);
  });

  // Drag & drop anywhere in the window.
  let dragDepth = 0;
  const drop = $('#drop');
  const hasFiles = (e) => [...(e.dataTransfer?.types || [])].includes('Files');
  window.addEventListener('dragenter', (e) => {
    if ($('#app').hidden || !hasFiles(e)) return;
    if (dragDepth++ === 0) enter(drop, [{ opacity: 0 }, { opacity: 1 }], 'snappy', { duration: 180 });
  });
  window.addEventListener('dragleave', () => {
    if (dragDepth && --dragDepth === 0) leave(drop, [{ opacity: 1 }, { opacity: 0 }], 'snappy', { duration: 180 });
  });
  window.addEventListener('dragover', (e) => { if (hasFiles(e)) e.preventDefault(); });
  window.addEventListener('drop', (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    dragDepth = 0;
    leave(drop, [{ opacity: 1 }, { opacity: 0 }], 'snappy', { duration: 180 });
    if (!$('#app').hidden) enqueueUploads([...e.dataTransfer.files]);
  });
}

async function boot() {
  bindUi();
  try {
    const s = await api.session();
    if (s && s.authenticated) showApp();
    else showLogin();
  } catch {
    showLogin();
  }
}

