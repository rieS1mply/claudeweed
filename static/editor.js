'use strict';
// Photo editor (crop, rotate, flip, light & colour), format conversion and the text editor.
// Everything is done in the browser; the server only stores the result.

// ---------- photo editor ----------

const ADJUSTMENTS = [
  { key: 'brightness', label: 'Яркость' },
  { key: 'contrast', label: 'Контраст' },
  { key: 'saturation', label: 'Насыщенность' },
  { key: 'warmth', label: 'Теплота' },
];
const ASPECTS = [
  { label: 'Свободно', value: null },
  { label: 'Оригинал', value: 'original' },
  { label: '1:1', value: 1 },
  { label: '4:3', value: 4 / 3 },
  { label: '3:4', value: 3 / 4 },
  { label: '16:9', value: 16 / 9 },
];
const ENCODABLE = { jpg: 'image/jpeg', jpeg: 'image/jpeg', png: 'image/png', webp: 'image/webp' };
const EXT_OF_MIME = { 'image/jpeg': 'jpg', 'image/png': 'png', 'image/webp': 'webp' };
const FULL_CROP = { x: 0, y: 0, w: 1, h: 1 };
const MIN_CROP = 0.05;
// iOS Safari cannot allocate canvases above ~16.7 megapixels.
const IS_IOS = /iP(hone|ad|od)/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
const MAX_PIXELS = IS_IOS ? 16_000_000 : 120_000_000;

const ed = {
  section: 'photos', item: null, img: null, preview: null,
  rot: 0, flip: false, crop: { ...FULL_CROP }, aspect: null,
  adj: { brightness: 0, contrast: 0, saturation: 0, warmth: 0 },
  param: 'brightness', mode: 'adjust', queued: false,
};

function loadImage(url) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.decoding = 'async';
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error('Браузер не может открыть этот формат'));
    img.src = url;
  });
}

function fitPixels(w, h) {
  const k = Math.min(1, Math.sqrt(MAX_PIXELS / (w * h)));
  return [Math.max(1, Math.floor(w * k)), Math.max(1, Math.floor(h * k))];
}

async function openEditor(section, item) {
  let img;
  try {
    img = await loadImage(api.fileUrl(section, item, false));
  } catch (e) {
    return toast(`${e.message} (${splitName(item.name)[1].toUpperCase()})`);
  }
  Object.assign(ed, {
    section, item, img, rot: 0, flip: false, crop: { ...FULL_CROP }, aspect: null,
    adj: { brightness: 0, contrast: 0, saturation: 0, warmth: 0 }, param: 'brightness', mode: 'adjust',
  });
  // Work on a screen-sized copy while editing; the full photo is only processed on save.
  const k = Math.min(1, 1600 / Math.max(img.naturalWidth, img.naturalHeight));
  const pv = document.createElement('canvas');
  pv.width = Math.round(img.naturalWidth * k);
  pv.height = Math.round(img.naturalHeight * k);
  pv.getContext('2d').drawImage(img, 0, 0, pv.width, pv.height);
  ed.preview = pv;

  buildEditorControls();
  const editor = $('#editor');
  editor.hidden = false;
  setEditorMode('adjust', true);
  await enter(editor, [{ opacity: 0, transform: 'scale(1.03)' }, { opacity: 1, transform: 'none' }], 'smooth');
}

async function closeEditor() {
  await leave($('#editor'), [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'scale(1.03)' }], 'snappy', { duration: 260 });
  ed.img = ed.preview = null;
}

function rotatedSize(w, h) {
  return ed.rot % 180 ? [h, w] : [w, h];
}

/** Draws `src` rotated/flipped and cropped to `crop` (fractions of the rotated image) into outW×outH. */
function drawTransformed(ctx, src, crop, outW, outH) {
  const sw = src.naturalWidth || src.width, sh = src.naturalHeight || src.height;
  const [RW, RH] = rotatedSize(sw, sh);
  ctx.save();
  ctx.scale(outW / (crop.w * RW), outH / (crop.h * RH));
  ctx.translate(-crop.x * RW, -crop.y * RH);
  ctx.translate(RW / 2, RH / 2);
  if (ed.flip) ctx.scale(-1, 1);
  ctx.rotate((ed.rot * Math.PI) / 180);
  ctx.drawImage(src, -sw / 2, -sh / 2, sw, sh);
  ctx.restore();
}

/** Light & colour in one pass over the pixels (same result in every browser). */
function applyAdjustments(ctx, w, h) {
  const { brightness: b, contrast: c, saturation: s, warmth: t } = ed.adj;
  if (!b && !c && !s && !t) return;
  const image = ctx.getImageData(0, 0, w, h);
  const p = image.data;
  const fb = 1 + b * 0.5;
  const fc = c >= 0 ? 1 + c * 1.2 : 1 + c * 0.8;
  const fs = 1 + s;
  const warm = t * 35;
  for (let i = 0; i < p.length; i += 4) {
    let r = (p[i] * fb - 128) * fc + 128;
    let g = (p[i + 1] * fb - 128) * fc + 128;
    let bl = (p[i + 2] * fb - 128) * fc + 128;
    const l = 0.2126 * r + 0.7152 * g + 0.0722 * bl;
    p[i] = l + (r - l) * fs + warm;          // Uint8ClampedArray clamps to 0..255
    p[i + 1] = l + (g - l) * fs + warm * 0.25;
    p[i + 2] = l + (bl - l) * fs - warm;
  }
  ctx.putImageData(image, 0, 0);
}

function queuePreview() {
  if (ed.queued) return;
  ed.queued = true;
  requestAnimationFrame(() => { ed.queued = false; renderPreview(); });
}

function renderPreview() {
  if (!ed.preview) return;
  const canvas = $('#ed-canvas'), stage = $('#ed-stage');
  const src = ed.preview;
  // While cropping, show the whole (rotated) photo with the crop frame on top.
  const crop = ed.mode === 'crop' ? FULL_CROP : ed.crop;
  const [RW, RH] = rotatedSize(src.width, src.height);
  const cw = crop.w * RW, ch = crop.h * RH;
  const box = stage.getBoundingClientRect();
  const pad = ed.mode === 'crop' ? 36 : 16;
  const scale = Math.min((box.width - pad * 2) / cw, (box.height - pad * 2) / ch);
  const dispW = Math.max(1, cw * scale), dispH = Math.max(1, ch * scale);
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  canvas.width = Math.round(dispW * dpr);
  canvas.height = Math.round(dispH * dpr);
  canvas.style.width = `${dispW}px`;
  canvas.style.height = `${dispH}px`;
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  drawTransformed(ctx, src, crop, canvas.width, canvas.height);
  applyAdjustments(ctx, canvas.width, canvas.height);
  if (ed.mode === 'crop') placeCropBox();
}

function placeCropBox() {
  const canvas = $('#ed-canvas'), stage = $('#ed-stage'), box = $('#ed-crop');
  const c = canvas.getBoundingClientRect(), s = stage.getBoundingClientRect();
  const st = box.style;
  st.left = `${c.left - s.left + ed.crop.x * c.width}px`;
  st.top = `${c.top - s.top + ed.crop.y * c.height}px`;
  st.width = `${ed.crop.w * c.width}px`;
  st.height = `${ed.crop.h * c.height}px`;
}

/** Aspect (width / height in pixels) for the current preset, or null when free. */
function currentAspect() {
  if (ed.aspect === null) return null;
  if (ed.aspect === 'original') {
    const [RW, RH] = rotatedSize(ed.img.naturalWidth, ed.img.naturalHeight);
    return RW / RH;
  }
  return ed.aspect;
}

/** Largest centred crop with the given aspect. */
function cropForAspect(aspect) {
  if (!aspect) return { ...FULL_CROP };
  const [RW, RH] = rotatedSize(ed.img.naturalWidth, ed.img.naturalHeight);
  let w = 1, h = (RW / aspect) / RH;
  if (h > 1) { h = 1; w = (RH * aspect) / RW; }
  return { x: (1 - w) / 2, y: (1 - h) / 2, w, h };
}

function animateCropTo(next) {
  const from = { ...ed.crop };
  if (REDUCED) { ed.crop = next; return placeCropBox(); }
  const start = performance.now(), dur = 380;
  const step = (t) => {
    const p = Math.min(1, (t - start) / dur);
    const e = 1 - Math.pow(1 - p, 3);
    ed.crop = { x: from.x + (next.x - from.x) * e, y: from.y + (next.y - from.y) * e, w: from.w + (next.w - from.w) * e, h: from.h + (next.h - from.h) * e };
    placeCropBox();
    if (p < 1) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

function setEditorMode(mode, instant = false) {
  ed.mode = mode;
  $('#ed-mode').dataset.active = mode;
  document.querySelectorAll('#ed-mode .tab').forEach((t) => t.classList.toggle('active', t.dataset.mode === mode));
  const thumb = $('#ed-mode .thumb');
  thumb.classList.remove('stretch');
  void thumb.offsetWidth;
  thumb.classList.add('stretch');
  $('#ed-crop').hidden = mode !== 'crop';
  const show = mode === 'crop' ? $('#ed-crop-tools') : $('#ed-adjust-tools');
  const hide = mode === 'crop' ? $('#ed-adjust-tools') : $('#ed-crop-tools');
  hide.hidden = true;
  show.hidden = false;
  if (!instant) animate(show, [{ opacity: 0, transform: 'translateY(8px)' }, { opacity: 1, transform: 'none' }], 'smooth', { fill: 'none' });
  renderPreview();
  if (!instant) animate($('#ed-canvas'), [{ opacity: 0.6, transform: 'scale(.98)' }, { opacity: 1, transform: 'none' }], 'smooth', { fill: 'none' });
}

function buildEditorControls() {
  $('#ed-title').textContent = ed.item.name;
  // Adjustments: one slider, chips choose what it controls (like Photos).
  const chips = $('#ed-chips');
  chips.replaceChildren(...ADJUSTMENTS.map((a) => el('button', {
    class: `chip${a.key === ed.param ? ' active' : ''}`, 'data-key': a.key,
    onclick: () => selectParam(a.key),
    ondblclick: () => { ed.adj[a.key] = 0; selectParam(a.key); queuePreview(); },
  }, el('span', { class: 'chip-value', text: '0' }), el('span', { text: a.label }))));
  selectParam('brightness');

  const aspects = $('#ed-aspects');
  aspects.replaceChildren(...ASPECTS.map((a) => el('button', {
    class: `chip${a.value === ed.aspect ? ' active' : ''}`, text: a.label,
    onclick: (e) => {
      ed.aspect = a.value;
      aspects.querySelectorAll('.chip').forEach((c) => c.classList.toggle('active', c === e.currentTarget));
      animateCropTo(cropForAspect(currentAspect()));
    },
  })));
}

function selectParam(key) {
  ed.param = key;
  const slider = $('#ed-slider');
  slider.value = Math.round(ed.adj[key] * 100);
  document.querySelectorAll('#ed-chips .chip').forEach((c) => {
    c.classList.toggle('active', c.dataset.key === key);
    const v = Math.round(ed.adj[c.dataset.key] * 100);
    c.querySelector('.chip-value').textContent = v > 0 ? `+${v}` : String(v);
    c.classList.toggle('changed', v !== 0);
  });
}

function bindEditor() {
  $('#ed-cancel').addEventListener('click', closeEditor);
  $('#ed-done').addEventListener('click', saveMenu);
  document.querySelectorAll('#ed-mode .tab').forEach((t) => t.addEventListener('click', () => setEditorMode(t.dataset.mode)));
  $('#ed-slider').addEventListener('input', (e) => {
    ed.adj[ed.param] = Number(e.target.value) / 100;
    selectParam(ed.param);
    queuePreview();
  });
  $('#ed-reset').addEventListener('click', () => {
    if (ed.mode === 'crop') {
      ed.rot = 0; ed.flip = false; ed.aspect = null;
      ed.crop = { ...FULL_CROP };
      buildEditorControls();
    } else {
      ed.adj = { brightness: 0, contrast: 0, saturation: 0, warmth: 0 };
      selectParam(ed.param);
    }
    renderPreview();
  });
  $('#ed-rotate').addEventListener('click', () => {
    ed.rot = (ed.rot + 270) % 360;   // 90° counter-clockwise, like Photos
    ed.crop = cropForAspect(currentAspect());
    renderPreview();
    animate($('#ed-canvas'), [{ transform: 'rotate(90deg) scale(.85)' }, { transform: 'none' }], 'smooth', { fill: 'none' });
  });
  $('#ed-flip').addEventListener('click', () => {
    ed.flip = !ed.flip;
    ed.crop = { ...ed.crop, x: 1 - ed.crop.x - ed.crop.w };
    renderPreview();
    animate($('#ed-canvas'), [{ transform: 'scaleX(-1)' }, { transform: 'none' }], 'smooth', { fill: 'none' });
  });
  window.addEventListener('resize', () => { if (!$('#editor').hidden) renderPreview(); });
  bindCropGestures();
}

function bindCropGestures() {
  const box = $('#ed-crop');
  let drag = null;
  box.addEventListener('pointerdown', (e) => {
    const c = $('#ed-canvas').getBoundingClientRect();
    drag = { h: e.target.dataset.h || 'move', x: e.clientX, y: e.clientY, start: { ...ed.crop }, cw: c.width, ch: c.height };
    box.setPointerCapture(e.pointerId);
    box.classList.add('dragging');
    e.preventDefault();
  });
  box.addEventListener('pointermove', (e) => {
    if (!drag) return;
    const dx = (e.clientX - drag.x) / drag.cw, dy = (e.clientY - drag.y) / drag.ch;
    let { x, y, w, h } = drag.start;
    const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
    if (drag.h === 'move') {
      x = clamp(x + dx, 0, 1 - w);
      y = clamp(y + dy, 0, 1 - h);
    } else {
      const right = x + w, bottom = y + h;
      if (drag.h.includes('w')) { x = clamp(x + dx, 0, right - MIN_CROP); w = right - x; }
      if (drag.h.includes('e')) w = clamp(w + dx, MIN_CROP, 1 - x);
      if (drag.h.includes('n')) { y = clamp(y + dy, 0, bottom - MIN_CROP); h = bottom - y; }
      if (drag.h.includes('s')) h = clamp(h + dy, MIN_CROP, 1 - y);
      const aspect = currentAspect();
      if (aspect) {
        // Keep the ratio: width leads, height follows; shrink both if that would leave the photo.
        const [RW, RH] = rotatedSize(ed.img.naturalWidth, ed.img.naturalHeight);
        const ratio = (aspect * RH) / RW;           // w / h in fractions
        h = w / ratio;
        const maxH = drag.h.includes('n') ? bottom : 1 - y;
        if (h > maxH) { h = maxH; w = h * ratio; }
        if (drag.h.includes('n')) y = bottom - h;
        if (drag.h.includes('w')) x = right - w;
      }
    }
    ed.crop = { x, y, w, h };
    placeCropBox();
  });
  const end = () => { drag = null; box.classList.remove('dragging'); };
  box.addEventListener('pointerup', end);
  box.addEventListener('pointercancel', end);
}

async function saveMenu() {
  const ext = splitName(ed.item.name)[1];
  const [RW, RH] = rotatedSize(ed.img.naturalWidth, ed.img.naturalHeight);
  const shrinks = ed.crop.w * RW * ed.crop.h * RH > MAX_PIXELS;
  // Overwriting would lose detail if the browser has to scale the photo down (iPhone limit).
  const canReplace = Boolean(ENCODABLE[ext]) && !shrinks;
  const note = shrinks ? 'Фото слишком большое для этого браузера — сохранится уменьшенная копия, оригинал не тронется'
    : canReplace ? '' : 'Этот формат браузер сохранить не может — будет создана копия в JPG';
  await actionSheet(note, [
    canReplace && { label: 'Сохранить', hint: 'оригинал уйдёт в корзину', run: () => saveEdit(false) },
    { label: 'Сохранить как копию', run: () => saveEdit(true) },
  ]);
}

/** Renders the full-resolution result. Returns the Blob and the extension it really got. */
async function renderFull(mime) {
  const img = ed.img;
  const [RW, RH] = rotatedSize(img.naturalWidth, img.naturalHeight);
  const [outW, outH] = fitPixels(Math.round(ed.crop.w * RW), Math.round(ed.crop.h * RH));
  const canvas = document.createElement('canvas');
  canvas.width = outW;
  canvas.height = outH;
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  if (mime === 'image/jpeg') { ctx.fillStyle = '#fff'; ctx.fillRect(0, 0, outW, outH); }
  drawTransformed(ctx, img, ed.crop, outW, outH);
  applyAdjustments(ctx, outW, outH);
  const blob = await new Promise((r) => canvas.toBlob(r, mime, 0.9));
  if (!blob) throw new Error('Не удалось подготовить изображение');
  // Safari silently writes PNG when asked for WebP: trust the blob, not the request.
  return { blob, ext: EXT_OF_MIME[blob.type] || 'png' };
}

async function saveEdit(asCopy) {
  const { section, item } = ed;
  const [stem, ext] = splitName(item.name);
  const mime = ENCODABLE[ext] || 'image/jpeg';
  const done = $('#ed-done');
  done.disabled = true;
  done.textContent = 'Сохранение…';
  try {
    const { blob, ext: outExt } = await renderFull(mime);
    const sameFormat = ENCODABLE[outExt] === ENCODABLE[ext];
    if (asCopy || !sameFormat) {
      await api.copy(section, item, `${stem} (ред.).${sameFormat ? ext : outExt}`, blob);
      toast('Копия сохранена');
    } else {
      await api.replace(section, item, blob, true);
      toast('Сохранено. Оригинал — в корзине', {});
    }
    await closeEditor();
    state.data.trash = null;
    await load(section);
    if (viewer.open && section === 'photos') fillFrame(state.viewer[state.index]);
  } catch (e) {
    toast(e.message);
  } finally {
    done.disabled = false;
    done.textContent = 'Готово';
  }
}

// ---------- conversion ----------

// Quality close to what phone cameras use: visually the same, without bloating the file.
const LOSSY_QUALITY = 0.85;
const IMAGE_TARGETS = [
  { ext: 'jpg', mime: 'image/jpeg', label: 'JPG' },
  { ext: 'png', mime: 'image/png', label: 'PNG' },
  { ext: 'webp', mime: 'image/webp', label: 'WebP' },
];
const LOSSY_SOURCES = ['jpg', 'jpeg', 'webp', 'heic', 'heif', 'avif'];
const CONVERTIBLE_IMAGES = ['jpg', 'jpeg', 'png', 'webp', 'gif', 'bmp', 'heic', 'heif', 'avif', 'tif', 'tiff'];

/** Menu entries "Конвертировать в …" for this item. */
function conversionsFor(section, item) {
  const ext = splitName(item.name)[1];
  if (CONVERTIBLE_IMAGES.includes(ext)) {
    const same = ext === 'jpeg' ? 'jpg' : ext;
    return IMAGE_TARGETS.filter((t) => t.ext !== same).map((t) => ({
      label: `Конвертировать в ${t.label}`,
      // PNG stores every pixel without loss: a camera photo becomes several times bigger.
      hint: t.ext === 'png' && LOSSY_SOURCES.includes(ext) ? 'без сжатия — файл станет в несколько раз больше' : '',
      run: () => convertImage(section, item, t),
    }));
  }
  const text = { csv: [['json', 'JSON']], json: [['csv', 'CSV']], txt: [['md', 'Markdown']], md: [['txt', 'TXT']] }[ext] || [];
  return text.map(([to, label]) => ({ label: `Конвертировать в ${label}`, run: () => convertText(section, item, to) }));
}

async function convertImage(section, item, target) {
  toast(`Конвертирую в ${target.label}…`);
  try {
    const img = await loadImage(api.fileUrl(section, item, false));
    const [w, h] = fitPixels(img.naturalWidth, img.naturalHeight);
    const canvas = document.createElement('canvas');
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext('2d');
    if (target.mime === 'image/jpeg') { ctx.fillStyle = '#fff'; ctx.fillRect(0, 0, w, h); }  // JPG has no transparency
    ctx.drawImage(img, 0, 0, w, h);
    const blob = await new Promise((r) => canvas.toBlob(r, target.mime, LOSSY_QUALITY));
    if (!blob || blob.type !== target.mime) throw new Error(`Этот браузер не умеет сохранять ${target.label}`);
    await api.copy(section, item, `${splitName(item.name)[0]}.${target.ext}`, blob);
    toast(`Готово: ${target.label} · ${fmtSize(blob.size)} (было ${fmtSize(item.size)})`);
    await load(section);
  } catch (e) {
    toast(e.message);
  }
}

/** CSV with quotes; the delimiter (',' or ';' as Excel writes in Russian locale) is guessed. */
function parseCsv(text) {
  const first = text.split('\n', 1)[0];
  const delim = (first.match(/;/g) || []).length > (first.match(/,/g) || []).length ? ';' : ',';
  const rowsOut = [];
  let row = [], field = '', quoted = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (quoted) {
      if (ch === '"' && text[i + 1] === '"') { field += '"'; i++; }
      else if (ch === '"') quoted = false;
      else field += ch;
    } else if (ch === '"') quoted = true;
    else if (ch === delim) { row.push(field); field = ''; }
    else if (ch === '\n' || ch === '\r') {
      if (ch === '\r' && text[i + 1] === '\n') i++;
      row.push(field); field = '';
      if (row.some((f) => f !== '')) rowsOut.push(row);
      row = [];
    } else field += ch;
  }
  row.push(field);
  if (row.some((f) => f !== '')) rowsOut.push(row);
  return rowsOut;
}

function csvToJson(text) {
  const [header = [], ...body] = parseCsv(text);
  const asValue = (v) => (v !== '' && !isNaN(Number(v)) ? Number(v) : v);
  return JSON.stringify(body.map((r) => Object.fromEntries(header.map((k, i) => [k, asValue(r[i] ?? '')]))), null, 2) + '\n';
}

function jsonToCsv(text) {
  let data = JSON.parse(text);
  if (!Array.isArray(data)) data = [data];
  const keys = [...new Set(data.flatMap((r) => (r && typeof r === 'object' ? Object.keys(r) : [])))];
  const cell = (v) => {
    const s = v === null || v === undefined ? '' : typeof v === 'object' ? JSON.stringify(v) : String(v);
    return /[",\n;]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  return [keys.map(cell).join(','), ...data.map((r) => keys.map((k) => cell(r?.[k])).join(','))].join('\n') + '\n';
}

async function convertText(section, item, to) {
  try {
    const text = await api.readText(section, item);
    const from = splitName(item.name)[1];
    const out = from === 'csv' && to === 'json' ? csvToJson(text) : from === 'json' && to === 'csv' ? jsonToCsv(text) : text;
    const type = { json: 'application/json', csv: 'text/csv', md: 'text/markdown', txt: 'text/plain' }[to];
    await api.copy(section, item, `${splitName(item.name)[0]}.${to}`, new Blob([out], { type }));
    toast(`Готово: ${to.toUpperCase()}`);
    await load(section);
  } catch (e) {
    toast(e instanceof SyntaxError ? 'Не получилось прочитать JSON' : e.message);
  }
}

// ---------- text editor ----------

// Extension -> highlight.js language. Text files without one still get line numbers.
const CODE_LANG = {
  rs: 'rust', py: 'python', pyw: 'python', js: 'javascript', mjs: 'javascript', cjs: 'javascript', jsx: 'javascript',
  ts: 'typescript', tsx: 'typescript', mts: 'typescript', c: 'c', h: 'cpp', cpp: 'cpp', cc: 'cpp', cxx: 'cpp', hpp: 'cpp',
  hh: 'cpp', ino: 'cpp', cs: 'csharp', java: 'java', kt: 'kotlin', kts: 'kotlin', go: 'go', php: 'php', rb: 'ruby',
  swift: 'swift', m: 'objectivec', lua: 'lua', pl: 'perl', r: 'r', vb: 'vbnet', sh: 'bash', bash: 'bash', zsh: 'bash',
  ps1: 'powershell', psm1: 'powershell', bat: 'dos', cmd: 'dos', sql: 'sql', html: 'xml', htm: 'xml', xml: 'xml',
  svg: 'xml', vue: 'xml', xaml: 'xml', csproj: 'xml', plist: 'xml', css: 'css', scss: 'scss', less: 'less',
  json: 'json', jsonc: 'json', yaml: 'yaml', yml: 'yaml', toml: 'ini', ini: 'ini', conf: 'ini', cfg: 'ini',
  env: 'ini', properties: 'ini', md: 'markdown', markdown: 'markdown', diff: 'diff', patch: 'diff',
  graphql: 'graphql', gql: 'graphql', mk: 'makefile', gradle: 'kotlin',
};
const TEXT_EXT = [...new Set([...Object.keys(CODE_LANG), 'txt', 'csv', 'tsv', 'log', 'gitignore', 'srt', 'vtt', 'nfo'])];
const MAX_TEXT_EDIT = 2 * 1024 * 1024;
// Beyond this, re-highlighting on every keystroke lags on a phone: plain text with line numbers.
const MAX_HIGHLIGHT = 300 * 1024;

function isTextFile(name) {
  return TEXT_EXT.includes(splitName(name)[1]);
}

let highlighter = null;
/** highlight.js (~130 KB) is loaded the first time a code file is opened. */
function loadHighlighter() {
  highlighter ||= new Promise((resolve, reject) => {
    const s = el('script', { src: 'highlight.js' });
    s.onload = () => resolve(window.hljs);
    s.onerror = () => { highlighter = null; reject(new Error('highlight.js')); };
    document.head.append(s);
  });
  return highlighter;
}

const CODE_PREFS_KEY = 'lp-code';
const codePrefs = (() => {
  // A phone is too narrow for long lines: wrap by default there.
  const defaults = { wrap: matchMedia('(max-width: 720px)').matches, size: 13.5 };
  try { return { ...defaults, ...JSON.parse(localStorage.getItem(CODE_PREFS_KEY) || '{}') }; } catch { return defaults; }
})();
function saveCodePrefs() {
  try { localStorage.setItem(CODE_PREFS_KEY, JSON.stringify(codePrefs)); } catch { /* private mode */ }
}

/** Full-screen document view, shared by the text editor and notes. */
const doc = { onClose: null, onInput: null, slide: 'up', code: false, lang: null, hljs: null, indent: '    ', lines: 0, frame: 0 };

async function openDocView({ title, text, mono, lang = null, file = null, left, right, status = '', slide = 'up', onInput, onClose }) {
  const view = $('#doc'), area = $('#doc-text');
  $('#doc-title').textContent = title;
  $('#doc-left').textContent = left;
  $('#doc-status').textContent = status;
  $('#doc-right').replaceChildren(...right);
  area.value = text;
  area.scrollTop = area.scrollLeft = 0;
  Object.assign(doc, { onClose, onInput, slide, code: mono, lang, file, hljs: null, lines: 0 });
  setCodeMode(mono, text);
  runnerOnOpen();   // runner.js: the Run button and output panel for runnable files
  const from = slide === 'left' ? 'translateX(100%)' : 'translateY(100%)';
  await enter(view, [{ transform: from }, { transform: 'none' }], 'smooth');
}

async function closeDocView() {
  const to = doc.slide === 'left' ? 'translateX(100%)' : 'translateY(100%)';
  runnerOnClose();
  await leave($('#doc'), [{ transform: 'none' }, { transform: to }], 'smooth', { duration: 380 });
  lockZoom(false);
}

/** iOS zooms the page into any field under 16px when it gets focus; code wants smaller text. */
function lockZoom(on) {
  const meta = document.querySelector('meta[name=viewport]');
  const base = 'width=device-width, initial-scale=1, viewport-fit=cover';
  meta.content = on && IS_IOS ? `${base}, maximum-scale=1` : base;
}

function setCodeMode(on, text) {
  const view = $('#doc'), area = $('#doc-text');
  view.classList.toggle('code', on);
  $('#code-bar').hidden = $('#code-hl').hidden = !on;
  // Autocorrect and capitals break code; notes keep them.
  for (const [k, v] of [['spellcheck', 'false'], ['autocapitalize', 'off'], ['autocorrect', 'off'], ['autocomplete', 'off']]) {
    if (on) area.setAttribute(k, v); else area.removeAttribute(k);
  }
  if (!on) { area.spellcheck = true; $('#code-gutter').hidden = true; return; }
  lockZoom(true);
  area.setSelectionRange(0, 0);   // code opens at the top, not with the caret at the end
  doc.indent = detectIndent(text);
  $('#code-lang').textContent = 'Текст';
  applyCodePrefs();
  renderCode();
  if (doc.lang && text.length <= MAX_HIGHLIGHT) {
    const lang = doc.lang;
    loadHighlighter().then((hljs) => {
      if (doc.lang !== lang || !hljs.getLanguage(lang)) return;
      doc.hljs = hljs;
      $('#code-lang').textContent = hljs.getLanguage(lang).name;
      renderCode();
    }).catch(() => {});
  }
  updateCursorPos();
}

function applyCodePrefs() {
  const view = $('#doc');
  view.classList.toggle('wrap', codePrefs.wrap);
  view.style.setProperty('--code-size', `${codePrefs.size}px`);
  $('#code-wrap').classList.toggle('on', codePrefs.wrap);
  // Line numbers can't follow wrapped lines.
  $('#code-gutter').hidden = codePrefs.wrap;
  doc.lines = 0;
}

/** Tabs if the file indents with tabs, otherwise the smallest space indent seen (2–8), 4 by default. */
function detectIndent(text) {
  let tabs = 0, spaces = 0, min = 0;
  for (const line of text.split('\n', 2000)) {
    if (line[0] === '\t') tabs++;
    else if (line[0] === ' ') {
      const n = line.length - line.trimStart().length;
      if (n >= 2 && line.trim()) { spaces++; min = min ? Math.min(min, n) : n; }
    }
  }
  if (tabs > spaces) return '\t';
  return ' '.repeat(min >= 2 && min <= 8 ? min : 4);
}

/** Redraws the highlighted copy and line numbers, at most once a frame. */
function renderCode() {
  if (!doc.code) return;
  cancelAnimationFrame(doc.frame);
  doc.frame = requestAnimationFrame(() => {
    const area = $('#doc-text'), pre = $('#code-hl'), text = area.value;
    // The trailing newline keeps the copy as tall as the textarea when the text ends with one.
    if (doc.hljs && text.length <= MAX_HIGHLIGHT) {
      pre.innerHTML = `${doc.hljs.highlight(text, { language: doc.lang, ignoreIllegals: true }).value}\n`;
    } else {
      pre.textContent = `${text}\n`;
    }
    if (!codePrefs.wrap) renderGutter(text);
    syncCodeScroll();
  });
}

function renderGutter(text) {
  let lines = 1;
  for (let i = text.indexOf('\n'); i !== -1; i = text.indexOf('\n', i + 1)) lines++;
  if (lines === doc.lines) return;
  doc.lines = lines;
  const nums = new Array(lines);
  for (let i = 0; i < lines; i++) nums[i] = i + 1;
  $('#code-gutter').textContent = nums.join('\n');
  $('#doc-body').style.setProperty('--gut', `${String(lines).length + 2}ch`);
}

function syncCodeScroll() {
  const area = $('#doc-text');
  $('#code-hl').style.transform = `translate(${-area.scrollLeft}px, ${-area.scrollTop}px)`;
  $('#code-gutter').style.transform = `translateY(${-area.scrollTop}px)`;
  // Wrapped lines must break at the same width as the textarea's (its scrollbar included).
  if (codePrefs.wrap) $('#code-hl').style.width = `${area.clientWidth}px`;
  else $('#code-hl').style.removeProperty('width');
}

function updateCursorPos() {
  if (!doc.code) return;
  const area = $('#doc-text');
  const before = area.value.slice(0, area.selectionStart);
  const line = before.split('\n').length;
  const col = before.length - before.lastIndexOf('\n');
  const sel = area.selectionEnd - area.selectionStart;
  $('#code-pos').textContent = `Стр ${line}, кол ${col}${sel ? ` · выделено ${sel}` : ''}`;
}

/** Inserts text keeping the browser's undo history. */
function insertText(area, text) {
  if (!document.execCommand('insertText', false, text)) {
    area.setRangeText(text, area.selectionStart, area.selectionEnd, 'end');
    area.dispatchEvent(new Event('input'));
  }
}

/** Tab / Shift+Tab: indents or outdents every selected line (or types an indent at the cursor). */
function indentSelection(area, outdent) {
  const v = area.value, s = area.selectionStart, e = area.selectionEnd;
  if (s === e && !outdent) return insertText(area, doc.indent);
  const start = v.lastIndexOf('\n', s - 1) + 1;
  let end = v.indexOf('\n', e > s && v[e - 1] === '\n' ? e - 1 : e);
  if (end === -1) end = v.length;
  const width = doc.indent === '\t' ? 4 : doc.indent.length;
  const lines = v.slice(start, end).split('\n');
  const out = lines.map((l) => {
    if (!outdent) return l ? doc.indent + l : l;
    if (l[0] === '\t') return l.slice(1);
    return l.slice(Math.min(width, l.length - l.replace(/^ +/, '').length));
  }).join('\n');
  if (out === v.slice(start, end)) return;
  area.setSelectionRange(start, end);
  insertText(area, out);
  if (s === e) {
    const moved = out.split('\n')[0].length - lines[0].length;
    area.setSelectionRange(Math.max(start, s + moved), Math.max(start, s + moved));
  } else {
    area.setSelectionRange(start, start + out.length);
  }
}

/** Enter keeps the line's indent, plus one level after an opening bracket or a colon. */
function newlineWithIndent(area) {
  const v = area.value, s = area.selectionStart;
  const line = v.slice(v.lastIndexOf('\n', s - 1) + 1, s);
  let indent = line.match(/^[ \t]*/)[0];
  if (/[{[(:]\s*$/.test(line)) indent += doc.indent;
  insertText(area, `\n${indent}`);
}

async function goToLine() {
  const area = $('#doc-text');
  const total = area.value.split('\n').length;
  const answer = await promptDialog(`Перейти к строке (1–${total})`, '', { ok: 'Перейти' });
  const n = parseInt(answer, 10);
  if (!n) return;
  const lines = area.value.split('\n');
  const line = Math.min(Math.max(n, 1), lines.length);
  let pos = 0;
  for (let i = 0; i < line - 1; i++) pos += lines[i].length + 1;
  area.focus();
  area.setSelectionRange(pos, pos + lines[line - 1].length);
  // Put the line a third of the way down, not at the very edge.
  const lh = parseFloat(getComputedStyle(area).lineHeight) || 20;
  if (!codePrefs.wrap) area.scrollTop = Math.max(0, (line - 1) * lh - area.clientHeight / 3);
  updateCursorPos();
}

function setCodeSize(delta) {
  codePrefs.size = Math.min(24, Math.max(10, codePrefs.size + delta));
  saveCodePrefs();
  applyCodePrefs();
  renderCode();
}

function bindDocView() {
  const area = $('#doc-text');
  $('#doc-left').addEventListener('click', () => doc.onClose?.());
  area.addEventListener('input', () => { doc.onInput?.(area.value); renderCode(); updateCursorPos(); });
  area.addEventListener('scroll', () => { if (doc.code) syncCodeScroll(); }, { passive: true });
  for (const ev of ['keyup', 'pointerup', 'select', 'focus']) area.addEventListener(ev, updateCursorPos);
  new ResizeObserver(() => { if (doc.code) syncCodeScroll(); }).observe(area);
  area.addEventListener('keydown', (e) => {
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key === 's') {
      e.preventDefault();
      $('#doc-right').querySelector('[data-save]')?.click();
    }
    if (e.key === 'Escape') { e.preventDefault(); doc.onClose?.(); }
    if (!doc.code || e.isComposing) return;
    if (e.key === 'Tab' && !mod && !e.altKey) { e.preventDefault(); indentSelection(area, e.shiftKey); }
    else if (e.key === 'Enter' && !mod && !e.shiftKey && !e.altKey) { e.preventDefault(); newlineWithIndent(area); }
    else if (mod && e.key === 'g') { e.preventDefault(); goToLine(); }
    else if (mod && (e.key === '=' || e.key === '+')) { e.preventDefault(); setCodeSize(1); }
    else if (mod && e.key === '-') { e.preventDefault(); setCodeSize(-1); }
    else if (e.altKey && e.code === 'KeyZ') { e.preventDefault(); $('#code-wrap').click(); }
  });
  $('#code-wrap').addEventListener('click', () => {
    codePrefs.wrap = !codePrefs.wrap;
    saveCodePrefs();
    applyCodePrefs();
    renderCode();
  });
  $('#code-bigger').addEventListener('click', () => setCodeSize(1));
  $('#code-smaller').addEventListener('click', () => setCodeSize(-1));
  $('#code-goto').addEventListener('click', goToLine);
  // A phone keyboard has no Tab key.
  $('#code-tab').addEventListener('pointerdown', (e) => e.preventDefault());   // keep the keyboard open
  $('#code-tab').addEventListener('click', () => { area.focus(); indentSelection(area, false); });
}

async function openTextEditor(section, item) {
  if (item.size > MAX_TEXT_EDIT) return toast('Файл слишком большой для редактора — скачайте его');
  let original;
  try {
    original = await api.readText(section, item);
  } catch (e) {
    return toast(e.message);
  }
  let current = original;
  const save = el('button', { class: 'doc-btn strong', text: 'Сохранить', 'data-save': '1', disabled: true });
  save.onclick = async () => {
    save.disabled = true;
    try {
      await api.replace(section, item, new Blob([current], { type: 'text/plain' }), false);
      original = current;
      $('#doc-status').textContent = 'Сохранено';
      load(section);
    } catch (e) {
      save.disabled = false;
      toast(e.message);
    }
  };
  await openDocView({
    title: item.name, text: original, mono: true, lang: CODE_LANG[splitName(item.name)[1]] || null, file: item.name,
    left: 'Закрыть', right: [save], slide: 'up',
    onInput: (v) => {
      current = v;
      save.disabled = v === original;
      $('#doc-status').textContent = v === original ? '' : 'Изменено';
    },
    onClose: async () => {
      if (current !== original && !(await confirmDialog('Изменения не сохранены.', { title: 'Закрыть без сохранения?', ok: 'Закрыть' }))) return;
      closeDocView();
    },
  });
}
