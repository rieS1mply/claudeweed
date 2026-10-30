'use strict';
// Terminal of the home computer: several named sessions (tabs), shown in 1, 2 or 4 windows.
// Each session is a shell on the server that keeps running when the connection drops;
// on reconnect the server replays recent output. Every window has its own WebSocket.
// Output arrives raw-deflate compressed (one stream per connection) when the browser can inflate it.

const $ = (s, root = document) => root.querySelector(s);
const enc = new TextEncoder();
const touch = matchMedia('(pointer: coarse)').matches;

// Close codes from src/terminal.rs.
const CLOSE_RESYNC = 4000, CLOSE_EXITED = 4001, CLOSE_LOGGED_OUT = 4401, CLOSE_DISABLED = 4403, CLOSE_GONE = 4404;

const FONT_KEY = 'lp-term-font';
const LAYOUT_KEY = 'lp-term-layout';
const LAYOUTS = [1, 2, 4];

function load(key) {
  try { return JSON.parse(localStorage.getItem(key)); } catch { return null; }
}
function save(key, value) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* private mode */ }
}

let fontSize = Number(load(FONT_KEY)) || (touch ? 12 : 14);

// ---------- inflate ----------

function canInflate() {
  try { new DecompressionStream('deflate-raw'); return true; } catch { return false; }
}
const COMPRESS = canInflate();

/** Streaming raw-deflate decoder: the server sync-flushes each frame, so output shows at once. */
function inflater(onData) {
  const ds = new DecompressionStream('deflate-raw');
  const writer = ds.writable.getWriter();
  const reader = ds.readable.getReader();
  (async () => {
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        onData(value);
      }
    } catch { /* connection replaced */ }
  })();
  return {
    push: (bytes) => { writer.write(bytes).catch(() => {}); },
    close: () => { writer.abort().catch(() => {}); },
  };
}

// ---------- traffic counter ----------

const traffic = { wire: 0, raw: 0 };
function fmtBytes(n) {
  if (n < 1024) return `${n} ${t('Б')}`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10240 ? 1 : 0)} ${t('КБ')}`;
  return `${(n / 1024 / 1024).toFixed(1)} ${t('МБ')}`;
}
setInterval(() => {
  if (!traffic.wire) return;
  const ratio = traffic.raw / traffic.wire;
  $('#term-traffic').textContent = `${fmtBytes(traffic.wire)}${ratio >= 1.2 ? ` · ${t('сжатие')} ×${ratio.toFixed(1)}` : ''}`;
  $('#term-traffic').title = t('Получено {0}, без сжатия было бы {1}', { 0: fmtBytes(traffic.wire), 1: fmtBytes(traffic.raw) });
}, 1000);

// ---------- sessions API ----------

let sessions = [];
let maxSessions = 8;
let stopped = false;

async function api(method, path, body) {
  const res = await fetch(path, {
    method,
    credentials: 'same-origin',
    headers: { 'X-Cloud': '1', ...(body ? { 'Content-Type': 'application/json' } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => ({}));
  if (res.status === 401) { location.replace('/'); throw new Error('logged out'); }
  if (res.status === 404 && path === '/api/terminal/sessions') { showDisabled(); throw new Error('disabled'); }
  if (!res.ok) throw new Error(tServer(data.error) || t('Ошибка {0}', { 0: res.status }));
  return data;
}

async function refreshSessions() {
  if (stopped) return;
  try {
    const data = await api('GET', '/api/terminal/sessions');
    sessions = data.sessions;
    maxSessions = data.max;
  } catch { return; }
  renderTabs();
}

async function createSession() {
  if (sessions.length >= maxSessions) {
    showMessage(t('Открыто максимум сеансов ({0}). Закройте ненужный.', { 0: maxSessions }), [[t('Понятно'), () => {}]]);
    return null;
  }
  try {
    const s = await api('POST', '/api/terminal/sessions', {});
    sessions.push(s);
    renderTabs();
    return s;
  } catch (e) {
    if (!stopped) showMessage(e.message, [[t('Понятно'), () => {}]]);
    return null;
  }
}

function sessionById(id) { return sessions.find((s) => s.id === id); }

// ---------- windows (panes) ----------

const wrap = $('#term-wrap');
const panes = [];
let layout = 1;
let active = null;

/** xterm 6 doesn't scroll by finger. A vertical swipe scrolls the history line by line; in
 *  full-screen programs (less, vim) it becomes wheel events, which xterm passes on to them. */
function swipeScroll(term) {
  const screen = term.element.querySelector('.xterm-screen') || term.element;
  let lastY = null;
  term.element.addEventListener('touchstart', (e) => {
    lastY = e.touches.length === 1 ? e.touches[0].clientY : null;
  }, { passive: true });
  term.element.addEventListener('touchmove', (e) => {
    if (lastY === null || e.touches.length !== 1) return;
    e.preventDefault();   // keep the page from bouncing under the finger
    const { clientX, clientY } = e.touches[0];
    const lineHeight = screen.clientHeight / term.rows || 16;
    const lines = Math.trunc((lastY - clientY) / lineHeight);
    if (!lines) return;
    lastY -= lines * lineHeight;
    if (term.buffer.active.type === 'normal') {
      term.scrollLines(lines);
      return;
    }
    for (let i = Math.abs(lines); i > 0; i--) {
      screen.dispatchEvent(new WheelEvent('wheel', {
        deltaY: Math.sign(lines), deltaMode: WheelEvent.DOM_DELTA_LINE, clientX, clientY, bubbles: true, cancelable: true,
      }));
    }
  }, { passive: false });
  term.element.addEventListener('touchend', () => { lastY = null; });
}

class Pane {
  constructor() {
    this.el = $('#pane-tpl').content.firstElementChild.cloneNode(true);
    wrap.append(this.el);
    this.id = null;
    this.ws = null;
    this.retry = 0;
    this.retryTimer = 0;
    this.restarting = false;
    this.term = new Terminal({
      fontFamily: '"SF Mono", ui-monospace, Menlo, Consolas, "DejaVu Sans Mono", monospace',
      fontSize,
      cursorBlink: true,
      scrollback: 5000,
      macOptionIsMeta: true,
      theme: {
        background: '#000000',
        foreground: '#f5f5f7',
        cursor: '#f5f5f7',
        selectionBackground: 'rgba(10, 132, 255, .4)',
      },
    });
    this.fit = new FitAddon.FitAddon();
    this.term.loadAddon(this.fit);
    this.term.open($('.term', this.el));
    for (const [k, v] of [['autocorrect', 'off'], ['autocapitalize', 'off'], ['autocomplete', 'off'], ['spellcheck', 'false']]) {
      this.term.textarea?.setAttribute(k, v);
    }
    if (touch) swipeScroll(this.term);
    this.term.textarea?.addEventListener('focus', () => setActive(this));
    this.el.addEventListener('pointerdown', () => setActive(this));
    this.term.onData((data) => this.sendInput(withCtrl(data)));
    this.term.onBinary((data) => {
      const msg = new Uint8Array(data.length + 1);
      for (let i = 0; i < data.length; i++) msg[i + 1] = data.charCodeAt(i) & 0xff;
      this.send(msg);
    });
    this.term.onResize(() => this.sendResize());
    this.fitFrame = 0;
    new ResizeObserver(() => this.refit()).observe(this.el);
  }

  get visible() { return panes.indexOf(this) < layout; }

  /** Shows session `id` in this window (null: an empty window). */
  show(id) {
    if (this.id === id && (this.ws || id === null)) return;
    this.disconnect();
    this.id = id;
    this.term.reset();
    this.hideOverlay();
    if (id === null) this.showEmpty();
    else this.connect();
    updateTitle();
    saveLayout();
  }

  connect() {
    clearTimeout(this.retryTimer);
    if (this.ws || stopped || this.id === null || !this.visible) return;
    this.setState('connecting');
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const sock = new WebSocket(`${proto}//${location.host}/api/terminal?z=${COMPRESS ? 1 : 0}&id=${this.id}`);
    sock.binaryType = 'arraybuffer';
    this.ws = sock;
    let opened = false;
    const write = (bytes) => { traffic.raw += bytes.length; this.term.write(bytes); };
    const decoder = COMPRESS ? inflater(write) : null;

    sock.onopen = () => {
      opened = true;
      this.retry = 0;
      this.setState('open');
      this.hideOverlay();
      this.term.reset();   // the server replays the screen
      this.sendResize();
      if (!touch && this === active) this.term.focus();
    };
    sock.onmessage = (e) => {
      const bytes = new Uint8Array(e.data);
      traffic.wire += bytes.length;
      if (decoder) decoder.push(bytes);
      else write(bytes);
    };
    sock.onclose = async (e) => {
      decoder?.close();
      if (this.ws !== sock) return;
      this.ws = null;
      this.setState('closed');
      if (e.code === CLOSE_RESYNC) return this.connect();
      if (e.code === CLOSE_LOGGED_OUT) return location.replace('/');
      if (e.code === CLOSE_DISABLED) return showDisabled();
      if (e.code === CLOSE_GONE) return sessionGone(this.id);
      if (e.code === CLOSE_EXITED) {
        if (this.restarting) { this.restarting = false; return this.connect(); }
        refreshSessions();
        return this.showOverlay(t('Сеанс завершён.'), [
          [t('Начать заново'), () => this.connect()],
          [t('Закрыть сеанс'), () => closeSession(this.id, false)],
        ]);
      }
      // Refused before opening: find out whether it's the login or the setting.
      if (!opened && !(await allowed())) return;
      this.scheduleRetry();
    };
  }

  disconnect() {
    clearTimeout(this.retryTimer);
    const sock = this.ws;
    this.ws = null;
    sock?.close(1000);
    this.setState('closed');
  }

  scheduleRetry() {
    if (document.hidden) return;   // reconnects when the tab is visible again
    const delay = Math.min(8000, 400 * 2 ** this.retry++);
    this.retryTimer = setTimeout(() => this.connect(), delay);
  }

  send(bytes) {
    if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(bytes);
  }

  sendInput(str) {
    const b = enc.encode(str);
    const msg = new Uint8Array(b.length + 1);
    msg[0] = 0;
    msg.set(b, 1);
    this.send(msg);
  }

  sendResize() {
    const msg = new Uint8Array(5);
    const v = new DataView(msg.buffer);
    msg[0] = 1;
    v.setUint16(1, this.term.cols);
    v.setUint16(3, this.term.rows);
    this.send(msg);
  }

  restart() {
    if (this.ws?.readyState === WebSocket.OPEN) { this.restarting = true; this.send(new Uint8Array([2])); } else this.connect();
  }

  refit() {
    cancelAnimationFrame(this.fitFrame);
    this.fitFrame = requestAnimationFrame(() => {
      try { this.fit.fit(); } catch { /* hidden */ }
    });
  }

  setState(s) {
    this.state = s;
    if (this === active) $('#term-dot').dataset.state = s;
  }

  showOverlay(text, actions = [], html = false) {
    fillMessage($('.term-overlay', this.el), text, actions, html);
  }

  hideOverlay() { $('.term-overlay', this.el).hidden = true; }

  showEmpty() {
    const free = sessions.filter((s) => !panes.some((p) => p.visible && p.id === s.id));
    const actions = [[t('Новый сеанс'), async () => { const s = await createSession(); if (s) this.show(s.id); }]];
    for (const s of free.slice(0, 3)) actions.push([s.name, () => this.show(s.id)]);
    this.showOverlay(t('Пустое окно. Откройте в нём сеанс:'), actions);
  }
}

function setActive(pane) {
  if (active === pane) return;
  active = pane;
  for (const p of panes) p.el.classList.toggle('active', p === pane);
  $('#term-dot').dataset.state = pane.state || 'closed';
  updateTitle();
  renderTabs();
  saveLayout();
}

function updateTitle() {
  const name = active && sessionById(active.id)?.name;
  $('#term-name').textContent = name || t('Терминал');
  document.title = name ? `${name} — ${t('Терминал')}` : t('Терминал');
}

/** Phones get one window: four 20-column terminals help nobody. */
function maxLayout() { return innerWidth < 700 ? 1 : 4; }

function setLayout(n, fill = true) {
  layout = Math.min(n, maxLayout());
  wrap.dataset.layout = String(layout);
  while (panes.length < layout) panes.push(new Pane());
  panes.forEach((p, i) => {
    const vis = i < layout;
    p.el.hidden = !vis;
    if (!vis) p.disconnect();
    else if (p.id !== null && !p.ws) p.connect();
  });
  if (!panes[layout - 1] || !active?.visible) setActive(panes[0]);
  if (fill) fillEmpty(true);
  for (const p of panes) if (p.visible) p.refit();
  saveLayout();
}

/** Puts sessions no window shows into empty windows; `create` opens new ones if there are none. */
async function fillEmpty(create) {
  for (const p of panes.filter((x) => x.visible && x.id === null)) {
    const free = sessions.find((s) => !panes.some((x) => x.visible && x.id === s.id));
    const s = free || (create ? await createSession() : null);
    if (s) p.show(s.id); else p.showEmpty();
  }
}

function saveLayout() {
  if (!panes.length) return;
  save(LAYOUT_KEY, { layout, ids: panes.map((p) => p.id), active: Math.max(0, panes.indexOf(active)) });
}

// ---------- tabs ----------

const tabs = $('#term-tabs');

function renderTabs() {
  for (const old of [...tabs.querySelectorAll('.term-tab')]) old.remove();
  const shown = new Set(panes.filter((p) => p.visible).map((p) => p.id));
  for (const s of sessions) {
    const tab = document.createElement('div');
    tab.className = 'term-tab';
    tab.classList.toggle('current', active?.id === s.id);
    tab.classList.toggle('shown', shown.has(s.id));
    tab.classList.toggle('dead', !s.alive);
    tab.title = `${s.name}${s.clients > 1 ? ` · ${t('подключено окон: {0}', { 0: s.clients })}` : ''}\n${t('Двойное нажатие — переименовать')}`;
    const name = Object.assign(document.createElement('button'), { type: 'button', className: 'term-tab-name', textContent: s.name });
    name.addEventListener('click', () => openTab(s.id));
    name.addEventListener('dblclick', () => renameSession(s.id));
    longPress(name, () => renameSession(s.id));
    const x = Object.assign(document.createElement('button'), { type: 'button', className: 'term-tab-close', textContent: '×', title: t('Закрыть сеанс') });
    x.addEventListener('click', () => closeSession(s.id, true));
    tab.append(name, x);
    tabs.insertBefore(tab, $('#term-new'));
  }
  $('#term-new').disabled = sessions.length >= maxSessions;
  updateTitle();
}

function longPress(el, fn) {
  let timer = 0;
  el.addEventListener('pointerdown', () => { timer = setTimeout(fn, 600); });
  for (const ev of ['pointerup', 'pointerleave', 'pointercancel']) el.addEventListener(ev, () => clearTimeout(timer));
}

/** A session already on screen gets the focus; otherwise it replaces the active window's. */
function openTab(id) {
  const shown = panes.find((p) => p.visible && p.id === id);
  if (shown) setActive(shown);
  else active.show(id);
  renderTabs();
  if (!touch) active.term.focus();
}

$('#term-new').addEventListener('click', async () => {
  const s = await createSession();
  if (!s) return;
  const empty = panes.find((p) => p.visible && p.id === null);
  const target = empty || active;
  target.show(s.id);
  setActive(target);
  renderTabs();
  if (!touch) target.term.focus();
});

function renameSession(id) {
  const s = sessionById(id);
  if (!s) return;
  const name = prompt(t('Название сеанса'), s.name);
  if (!name || name.trim() === s.name) return;
  api('PUT', `/api/terminal/sessions/${id}`, { name })
    .then((info) => { Object.assign(s, info); renderTabs(); })
    .catch((e) => showMessage(e.message, [[t('Понятно'), () => {}]]));
}

function closeSession(id, ask) {
  const s = sessionById(id);
  const doClose = () => api('DELETE', `/api/terminal/sessions/${id}`).catch(() => {}).then(() => sessionGone(id));
  if (!ask || !s?.alive) return doClose();
  showMessage(t('Закрыть «{0}» и всё, что в нём запущено?', { 0: s.name }), [[t('Закрыть'), doClose], [t('Отмена'), () => {}]]);
}

/** The session was closed here or elsewhere: its windows take another one or go empty. */
async function sessionGone(id) {
  sessions = sessions.filter((s) => s.id !== id);
  await refreshSessions();
  for (const p of panes.filter((x) => x.id === id)) p.show(null);
  await fillEmpty(false);
  renderTabs();
}

// ---------- messages ----------

const overlay = $('#term-overlay');

function fillMessage(box, text, actions, html) {
  const msg = $('.term-msg-text', box);
  if (html) msg.innerHTML = text; else msg.textContent = text;
  const row = $('.term-msg-actions', box);
  row.replaceChildren();
  actions.forEach(([label, fn], i) => {
    const b = Object.assign(document.createElement('button'), { type: 'button', className: i === 0 ? 'btn primary' : 'btn glass', textContent: label });
    b.addEventListener('click', () => { box.hidden = true; fn(); });
    row.append(b);
  });
  box.hidden = false;
}

function showMessage(text, actions, html = false) { fillMessage(overlay, text, actions, html); }

function showDisabled() {
  stopped = true;
  for (const p of panes) p.disconnect();
  $('#term-dot').dataset.state = 'closed';
  showMessage(t('Терминал выключен. Включить его можно только на самом компьютере:<br><code>local-photo --terminal on</code>'), [], true);
}

/** False (and handles it) if logged out or the terminal is off. */
async function allowed() {
  try {
    const res = await fetch('/api/session', { credentials: 'same-origin' });
    const s = await res.json();
    if (!s.authenticated) { location.replace('/'); return false; }
    if (!s.terminal) { showDisabled(); return false; }
  } catch { /* offline: keep retrying */ }
  return true;
}

// ---------- coming back ----------

// Whichever device the user is looking at sets the size (a laptop tab left open
// in the background must not keep the phone's narrow screen, and vice versa).
function comeBack() {
  if (document.hidden || stopped) return;
  refreshSessions();
  for (const p of panes) {
    if (!p.visible) continue;
    if (!p.ws) p.connect();
    else p.sendResize();
  }
}
document.addEventListener('visibilitychange', comeBack);
addEventListener('focus', comeBack);
addEventListener('online', () => { for (const p of panes) if (p.visible && !p.ws) p.connect(); });
// Tabs opened or closed on another device.
setInterval(() => { if (!document.hidden) refreshSessions(); }, 10000);

// ---------- input ----------

let ctrl = false;
function setCtrl(on) {
  ctrl = on;
  $('#key-ctrl').classList.toggle('on', on);
}

/** Ctrl from the on-screen key applies to the next typed character. */
function withCtrl(data) {
  if (!ctrl || data.length !== 1) return data;
  setCtrl(false);
  const c = data.toUpperCase().charCodeAt(0);
  if (c >= 64 && c <= 95) return String.fromCharCode(c - 64);   // @ A-Z [ \ ] ^ _
  if (data === ' ') return '\x00';
  if (data === '?') return '\x7f';
  return data;
}

const KEYS = {
  esc: '\x1b', tab: '\t', '^c': '\x03',
  up: 'A', down: 'B', right: 'C', left: 'D',
};
for (const b of document.querySelectorAll('#term-keys button')) {
  b.tabIndex = -1;
  // Keep focus (and the phone keyboard) on the terminal.
  b.addEventListener('pointerdown', (e) => e.preventDefault());
  b.addEventListener('click', () => {
    const key = b.dataset.key;
    if (key === 'ctrl') return setCtrl(!ctrl);
    const k = KEYS[key];
    if (k && k.length === 1 && 'ABCD'.includes(k)) {
      active.sendInput((active.term.modes.applicationCursorKeysMode ? '\x1bO' : '\x1b[') + k);
    } else {
      active.sendInput(withCtrl(k ?? key));
    }
  });
}

// Ctrl+Shift+←/→ moves between windows (plain Ctrl+arrows belong to the shell).
addEventListener('keydown', (e) => {
  if (!(e.ctrlKey && e.shiftKey) || layout === 1) return;
  const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
  if (!step) return;
  e.preventDefault();
  e.stopPropagation();
  const next = panes[(panes.indexOf(active) + step + layout) % layout];
  setActive(next);
  next.term.focus();
}, true);

// ---------- size ----------

// Phone keyboard: shrink the page to the visible area so the prompt stays above it.
if (window.visualViewport) {
  const vv = window.visualViewport;
  const follow = () => {
    document.body.style.setProperty('--vvh', `${vv.height}px`);
    window.scrollTo(0, 0);
  };
  vv.addEventListener('resize', follow);
  follow();
}

let wanted = 1;   // the layout picked, kept while the screen is too narrow for it
addEventListener('resize', () => { if (Math.min(wanted, maxLayout()) !== layout) setLayout(wanted, false); });

function setFont(size) {
  fontSize = Math.max(9, Math.min(24, size));
  save(FONT_KEY, fontSize);
  for (const p of panes) { p.term.options.fontSize = fontSize; p.refit(); }
}
$('#term-smaller').addEventListener('click', () => setFont(fontSize - 1));
$('#term-bigger').addEventListener('click', () => setFont(fontSize + 1));

$('#term-layout').addEventListener('click', () => {
  wanted = LAYOUTS[(LAYOUTS.indexOf(layout) + 1) % LAYOUTS.length];
  setLayout(wanted);
});

$('#term-restart').addEventListener('click', () => {
  if (active?.id == null) return;
  showMessage(t('Закрыть оболочку в этом окне и всё, что в ней запущено, и начать заново?'), [
    [t('Перезапустить'), () => active.restart()],
    [t('Отмена'), () => {}],
  ]);
});

// ---------- cheat sheet: the same list as `cmds` in the shell ----------

const cheat = $('#cheat');
const cheatList = $('#cheat-list');
const cheatQuery = $('#cheat-q');
let commands = null;
/** Keys (Ctrl+C, Tab, ↑ ↓, Esc) are shown, not typed in. */
const isKey = (cmd) => /^(Ctrl\+|Shift\+|Alt\+|Tab$|Esc$|[↑↓←→])/.test(cmd);
const fold = (s) => s.toLowerCase().replaceAll('ё', 'е');

async function openCheat() {
  cheat.hidden = false;
  if (!commands) {
    try {
      const res = await fetch('commands.json');
      commands = res.ok ? await res.json() : [];
    } catch { commands = []; }
  }
  renderCheat();
  if (!touch) cheatQuery.focus();
}

function closeCheat() {
  cheat.hidden = true;
  if (!touch) active?.term.focus();
}

function renderCheat() {
  const words = fold(cheatQuery.value).split(/\s+/).filter(Boolean);
  const en = LANG !== 'ru';
  const rows = [];
  let group = null;
  for (const c of commands || []) {
    if (!words.every((w) => fold([c.g, c.ge, c.cmd, c.ru, c.en].join(' ')).includes(w))) continue;
    const name = en ? c.ge : c.g;
    if (name !== group) {
      group = name;
      const h = document.createElement('h3');
      h.textContent = name;
      rows.push(h);
    }
    const key = isKey(c.cmd);
    const row = document.createElement(key ? 'div' : 'button');
    row.className = 'cheat-row';
    const code = document.createElement('code');
    code.textContent = c.cmd;
    const about = document.createElement('span');
    about.textContent = en ? c.en : c.ru;
    row.append(code, about);
    if (!key) {
      row.type = 'button';
      row.addEventListener('click', () => insertCommand(c.cmd));
    }
    rows.push(row);
  }
  if (!rows.length) {
    const p = document.createElement('p');
    p.className = 'cheat-empty';
    p.textContent = t('Ничего не найдено');
    rows.push(p);
  }
  cheatList.replaceChildren(...rows);
  cheatList.scrollTop = 0;
}

/** Types the command without Enter: placeholders like <папка> need filling in first. */
function insertCommand(cmd) {
  closeCheat();
  if (active?.id == null) return;
  active.sendInput(cmd);
  active.term.focus();
}

$('#term-help').addEventListener('click', openCheat);
$('#cheat-close').addEventListener('click', closeCheat);
cheatQuery.addEventListener('input', renderCheat);
cheat.addEventListener('click', (e) => { if (e.target === cheat) closeCheat(); });
cheat.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') closeCheat();
  // Enter in the search box types the only (or first) match.
  if (e.key === 'Enter' && e.target === cheatQuery) $('button.cheat-row', cheatList)?.click();
});

// ---------- start ----------

(async () => {
  translatePage();
  if (!(await allowed())) return;
  await refreshSessions();
  if (stopped) return;
  if (!sessions.length) await createSession();
  const saved = load(LAYOUT_KEY) || {};
  wanted = LAYOUTS.includes(saved.layout) ? saved.layout : 1;
  setLayout(wanted, false);
  // Restore which session each window showed, if it still exists.
  const ids = Array.isArray(saved.ids) ? saved.ids : [];
  panes.forEach((p, i) => {
    if (!p.visible) return;
    const id = ids[i];
    if (sessionById(id) && !panes.some((x) => x !== p && x.id === id)) p.show(id);
  });
  await fillEmpty(false);
  setActive(panes[Math.min(saved.active || 0, layout - 1)] || panes[0]);
  renderTabs();
})();
