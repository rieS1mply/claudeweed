'use strict';
// Terminal of the home computer. One shell is shared by all open tabs and keeps running
// when the connection drops; on reconnect the server replays recent output.
// Output arrives raw-deflate compressed (one stream per connection) when the browser can inflate it.

const $ = (s) => document.querySelector(s);
const enc = new TextEncoder();
const touch = matchMedia('(pointer: coarse)').matches;

// Close codes from src/terminal.rs.
const CLOSE_RESYNC = 4000, CLOSE_EXITED = 4001, CLOSE_LOGGED_OUT = 4401, CLOSE_DISABLED = 4403;

const FONT_KEY = 'lp-term-font';
function savedFont() {
  try { return Number(localStorage.getItem(FONT_KEY)) || 0; } catch { return 0; }
}

const term = new Terminal({
  fontFamily: '"SF Mono", ui-monospace, Menlo, Consolas, "DejaVu Sans Mono", monospace',
  fontSize: savedFont() || (touch ? 12 : 14),
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
const fit = new FitAddon.FitAddon();
term.loadAddon(fit);
term.open($('#term'));
for (const [k, v] of [['autocorrect', 'off'], ['autocapitalize', 'off'], ['autocomplete', 'off'], ['spellcheck', 'false']]) {
  term.textarea?.setAttribute(k, v);
}

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
  if (n < 1024) return `${n} Б`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10240 ? 1 : 0)} КБ`;
  return `${(n / 1024 / 1024).toFixed(1)} МБ`;
}
setInterval(() => {
  if (!traffic.wire) return;
  const ratio = traffic.raw / traffic.wire;
  $('#term-traffic').textContent = `${fmtBytes(traffic.wire)}${ratio >= 1.2 ? ` · сжатие ×${ratio.toFixed(1)}` : ''}`;
  $('#term-traffic').title = `Получено ${fmtBytes(traffic.wire)}, без сжатия было бы ${fmtBytes(traffic.raw)}`;
}, 1000);

// ---------- connection ----------

let ws = null;
let retry = 0;
let retryTimer = 0;
let restarting = false;
let stopped = false;

function setState(s) { $('#term-dot').dataset.state = s; }

function connect() {
  clearTimeout(retryTimer);
  if (ws || stopped) return;
  setState('connecting');
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const sock = new WebSocket(`${proto}//${location.host}/api/terminal?z=${COMPRESS ? 1 : 0}`);
  sock.binaryType = 'arraybuffer';
  ws = sock;
  let opened = false;
  const write = (bytes) => { traffic.raw += bytes.length; term.write(bytes); };
  const decoder = COMPRESS ? inflater(write) : null;

  sock.onopen = () => {
    opened = true;
    retry = 0;
    setState('open');
    hideOverlay();
    term.reset();   // the server replays the screen
    sendResize();
    if (!touch) term.focus();
  };
  sock.onmessage = (e) => {
    const bytes = new Uint8Array(e.data);
    traffic.wire += bytes.length;
    if (decoder) decoder.push(bytes);
    else write(bytes);
  };
  sock.onclose = async (e) => {
    decoder?.close();
    if (ws !== sock) return;
    ws = null;
    setState('closed');
    if (e.code === CLOSE_RESYNC) return connect();
    if (e.code === CLOSE_LOGGED_OUT) return location.replace('/');
    if (e.code === CLOSE_DISABLED) return showDisabled();
    if (e.code === CLOSE_EXITED) {
      if (restarting) { restarting = false; return connect(); }
      return showOverlay('Сеанс завершён.', [['Новый сеанс', connect]]);
    }
    // Refused before opening: find out whether it's the login or the setting.
    if (!opened && !(await allowed())) return;
    scheduleRetry();
  };
}

function scheduleRetry() {
  if (document.hidden) return;   // reconnects when the tab is visible again
  const delay = Math.min(8000, 400 * 2 ** retry++);
  retryTimer = setTimeout(connect, delay);
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

// Whichever device the user is looking at sets the size (a laptop tab left open
// in the background must not keep the phone's narrow screen, and vice versa).
function comeBack() {
  if (document.hidden) return;
  if (!ws) connect();
  else sendResize();
}
document.addEventListener('visibilitychange', comeBack);
addEventListener('focus', comeBack);
addEventListener('online', () => { if (!ws) connect(); });

function send(bytes) {
  if (ws?.readyState === WebSocket.OPEN) ws.send(bytes);
}

function sendInput(str) {
  const b = enc.encode(str);
  const msg = new Uint8Array(b.length + 1);
  msg[0] = 0;
  msg.set(b, 1);
  send(msg);
}

function sendResize() {
  const msg = new Uint8Array(5);
  const v = new DataView(msg.buffer);
  msg[0] = 1;
  v.setUint16(1, term.cols);
  v.setUint16(3, term.rows);
  send(msg);
}

// ---------- overlay ----------

function showOverlay(text, actions = [], html = false) {
  const box = $('#term-msg-text');
  if (html) box.innerHTML = text; else box.textContent = text;
  const btn = $('#term-msg-btn');
  btn.hidden = !actions.length;
  for (const old of [...btn.parentNode.querySelectorAll('.extra')]) old.remove();
  actions.forEach(([label, fn], i) => {
    const b = i === 0 ? btn : btn.parentNode.appendChild(Object.assign(document.createElement('button'), { className: 'btn glass extra', type: 'button' }));
    b.textContent = label;
    b.onclick = () => { hideOverlay(); fn(); };
  });
  $('#term-overlay').hidden = false;
}

function hideOverlay() { $('#term-overlay').hidden = true; }

function showDisabled() {
  stopped = true;
  setState('closed');
  showOverlay('Терминал выключен. Включить его можно только на самом компьютере:<br><code>local-photo --terminal on</code>', [], true);
}

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

term.onData((data) => sendInput(withCtrl(data)));
term.onBinary((data) => {
  const msg = new Uint8Array(data.length + 1);
  for (let i = 0; i < data.length; i++) msg[i + 1] = data.charCodeAt(i) & 0xff;
  send(msg);
});

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
      sendInput((term.modes.applicationCursorKeysMode ? '\x1bO' : '\x1b[') + k);
    } else {
      sendInput(withCtrl(k ?? key));
    }
  });
}

// ---------- size ----------

let fitFrame = 0;
function refit() {
  cancelAnimationFrame(fitFrame);
  fitFrame = requestAnimationFrame(() => {
    try { fit.fit(); } catch { /* hidden */ }
  });
}
term.onResize(sendResize);
new ResizeObserver(refit).observe($('#term-wrap'));

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

function setFont(size) {
  term.options.fontSize = Math.max(9, Math.min(24, size));
  try { localStorage.setItem(FONT_KEY, String(term.options.fontSize)); } catch { /* private mode */ }
  refit();
}
$('#term-smaller').addEventListener('click', () => setFont(term.options.fontSize - 1));
$('#term-bigger').addEventListener('click', () => setFont(term.options.fontSize + 1));

$('#term-restart').addEventListener('click', () => {
  showOverlay('Закрыть оболочку и всё, что в ней запущено, и начать новый сеанс?', [
    ['Новый сеанс', () => {
      if (ws?.readyState === WebSocket.OPEN) { restarting = true; send(new Uint8Array([2])); } else connect();
    }],
    ['Отмена', () => {}],
  ]);
});

// ---------- start ----------

refit();
allowed().then((ok) => { if (ok) connect(); });
