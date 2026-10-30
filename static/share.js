'use strict';
// Page behind a share link: lists the shared files, shows photos and videos, downloads the rest.

const $ = (sel) => document.querySelector(sel);
const token = location.pathname.split('/').pop();
const base = `/api/s/${encodeURIComponent(token)}`;

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

function fmtSize(bytes) {
  const units = [t('Б'), t('КБ'), t('МБ'), t('ГБ'), t('ТБ')];
  let v = bytes, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${i === 0 ? v : v.toFixed(v < 10 ? 1 : 0)} ${units[i]}`;
}

const fmtDay = (ms) => new Date(ms).toLocaleString(LANG, { day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit' });
const fileUrl = (item, download) => `${base}/file/${item.index}${download ? '?download=1' : ''}`;

function view(item) {
  const stage = $('#share-stage');
  stage.replaceChildren(item.kind === 'video'
    ? el('video', { src: fileUrl(item), controls: true, autoplay: true, playsinline: true })
    : el('img', { src: item.kind === 'raw' ? `${base}/preview/${item.index}` : fileUrl(item), alt: item.name }));
  $('#share-dl').href = fileUrl(item, true);
  $('#share-view').hidden = false;
}

function closeView() {
  $('#share-view').hidden = true;
  $('#share-stage').replaceChildren();
}

function render(info) {
  document.title = info.name;
  $('#share-title').textContent = info.name;
  const total = info.items.reduce((s, i) => s + i.size, 0);
  $('#share-meta').textContent = `${info.items.length} · ${fmtSize(total)} · ${t('ссылка действует до {d}', { d: fmtDay(info.expires) })}`;
  const media = info.items.filter((i) => info.section === 'photos' && i.kind !== 'file');
  const files = info.items.filter((i) => !media.includes(i));
  const out = [];
  if (media.length) {
    out.push(el('div', { class: 'grid' }, ...media.map((item) => {
      const tile = el('button', { class: 'tile', title: item.name, onclick: () => view(item) });
      if (item.kind === 'video') {
        tile.append(el('div', { class: 'placeholder', text: '▶' }));
      } else {
        const img = el('img', { src: `${base}/thumb/${item.index}`, alt: item.name, loading: 'lazy' });
        img.onload = () => img.classList.add('loaded');
        img.onerror = () => img.replaceWith(el('div', { class: 'placeholder', text: item.name.split('.').pop().toUpperCase() }));
        tile.append(img);
      }
      return tile;
    })));
  }
  if (files.length) {
    out.push(el('div', { class: 'files' }, ...files.map((item) => {
      const ext = item.name.includes('.') ? item.name.split('.').pop().toLowerCase() : '';
      const open = ext === 'pdf' ? el('a', { class: 'btn link', href: `${fileUrl(item)}?view=1`, target: '_blank', rel: 'noopener', text: t('Открыть') }) : null;
      return el('div', { class: 'file-row' },
        el('div', { class: 'file-icon', text: ext.slice(0, 4) || '—' }),
        el('div', { class: 'file-main' },
          el('div', { class: 'file-name', text: item.name }),
          el('div', { class: 'file-meta', text: fmtSize(item.size) })),
        open,
        el('a', { class: 'btn link', href: fileUrl(item, true), text: t('Скачать') }));
    })));
  }
  if (!out.length) out.push(el('div', { class: 'empty', text: t('Файлы по этой ссылке больше не лежат там, где были.') }));
  $('#share-content').replaceChildren(...out);
}

async function boot() {
  translatePage();
  $('#share-close').addEventListener('click', closeView);
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') closeView(); });
  try {
    const res = await fetch(base);
    const data = await res.json().catch(() => null);
    if (!res.ok) throw new Error((data && tServer(data.error)) || `${t('Ошибка')} ${res.status}`);
    render(data);
  } catch (e) {
    $('#share-title').textContent = t('Ссылка не открывается');
    $('#share-content').replaceChildren(el('div', { class: 'empty', text: tServer(e.message) }));
  }
}

boot();
