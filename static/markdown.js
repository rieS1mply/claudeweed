'use strict';
// Markdown → DOM for viewing .md files. Builds nodes only (never innerHTML): HTML in the
// file shows as text, links work only for http(s)/mailto, images are not loaded.

const MD_SAFE_URL = /^(https?:|mailto:)/i;

function mdText(s) { return document.createTextNode(s); }

function mdLink(url, children, title) {
  if (!MD_SAFE_URL.test(url)) return children;
  return [el('a', { href: url, target: '_blank', rel: 'noopener noreferrer', title: title || null }, ...children)];
}

/** Inline syntax of one paragraph/heading/cell. Returns an array of nodes. */
function mdInline(src) {
  const out = [];
  let buf = '';
  const flush = () => { if (buf) { out.push(mdText(buf)); buf = ''; } };
  let i = 0;
  while (i < src.length) {
    const ch = src[i];
    const rest = src.slice(i);
    let m;
    if (ch === '\\' && /[!-/:-@[-`{-~]/.test(src[i + 1] || '')) { buf += src[i + 1]; i += 2; continue; }
    if (ch === '\n') { flush(); out.push(el('br')); i++; continue; }
    if (ch === '`' && (m = /^(`+)([\s\S]*?[^`])\1(?!`)/.exec(rest))) {
      flush();
      out.push(el('code', { text: m[2].replace(/^ (.*) $/, '$1') }));
      i += m[0].length; continue;
    }
    if (ch === '!' && (m = /^!\[([^\]]*)\]\(\s*<?([^\s>)]+)>?(?:\s+"([^"]*)")?\s*\)/.exec(rest))) {
      flush();
      out.push(...mdLink(m[2], [mdText(`🖼 ${m[1] || m[2]}`)], m[3]));
      i += m[0].length; continue;
    }
    if (ch === '[' && (m = /^\[((?:\\.|[^\]\\])*)\]\(\s*<?([^\s>)]+)>?(?:\s+"([^"]*)")?\s*\)/.exec(rest))) {
      flush();
      out.push(...mdLink(m[2], mdInline(m[1]), m[3]));
      i += m[0].length; continue;
    }
    if (ch === '<' && (m = /^<((?:https?:\/\/|mailto:)[^\s>]+)>/i.exec(rest))) {
      flush();
      out.push(...mdLink(m[1], [mdText(m[1])]));
      i += m[0].length; continue;
    }
    if ((ch === 'h' || ch === 'H') && !/\w/.test(src[i - 1] || '') && (m = /^https?:\/\/[^\s<]+/i.exec(rest))) {
      const url = m[0].replace(/[.,;:!?)\]'"]+$/, '');
      flush();
      out.push(...mdLink(url, [mdText(url)]));
      i += url.length; continue;
    }
    if ((ch === '*' || ch === '_') && src[i + 1] === ch && (m = (ch === '*' ? /^\*\*(?=\S)([\s\S]*?\S)\*\*/ : /^__(?=\S)([\s\S]*?\S)__(?!\w)/).exec(rest))
        && (ch === '*' || !/\w/.test(src[i - 1] || ''))) {
      flush();
      out.push(el('strong', {}, ...mdInline(m[1])));
      i += m[0].length; continue;
    }
    if ((ch === '*' || ch === '_') && (m = (ch === '*' ? /^\*(?=[^\s*])([\s\S]*?[^\s*])\*(?!\*)/ : /^_(?=[^\s_])([\s\S]*?[^\s_])_(?!\w)/).exec(rest))
        && (ch === '*' || !/\w/.test(src[i - 1] || ''))) {
      flush();
      out.push(el('em', {}, ...mdInline(m[1])));
      i += m[0].length; continue;
    }
    if (ch === '~' && (m = /^~~(?=\S)([\s\S]*?\S)~~/.exec(rest))) {
      flush();
      out.push(el('del', {}, ...mdInline(m[1])));
      i += m[0].length; continue;
    }
    buf += ch;
    i++;
  }
  flush();
  return out;
}

/** Paragraph lines: a hard break where a line ends with two spaces or a backslash. */
function mdParagraph(lines) {
  const text = lines.map((l, k) => {
    const last = k === lines.length - 1;
    const t2 = l.trim();
    if (!last && (/ {2,}$/.test(l) || t2.endsWith('\\'))) return `${t2.replace(/\\$/, '')}\n`;
    return last ? t2 : `${t2} `;
  }).join('');
  return el('p', {}, ...mdInline(text));
}

function mdCells(line) {
  let s = line.trim();
  if (s.startsWith('|')) s = s.slice(1);
  if (s.endsWith('|') && !s.endsWith('\\|')) s = s.slice(0, -1);
  return s.split(/(?<!\\)\|/).map((c) => c.trim().replace(/\\\|/g, '|'));
}

const MD_TABLE_SEP = /^\s*\|?\s*:?-{1,}:?\s*(\|\s*:?-{1,}:?\s*)*\|?\s*$/;
const MD_LIST = /^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$/;
const MD_HR = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const MD_FENCE = /^\s{0,3}(`{3,}|~{3,})\s*([^`\s]*)/;

function mdStartsBlock(line) {
  return /^\s{0,3}#{1,6}(\s|$)/.test(line) || MD_FENCE.test(line) || /^\s{0,3}>/.test(line) || MD_HR.test(line) || MD_LIST.test(line);
}

/** Parses a list starting at `start`; returns [element, next line index]. */
function mdList(lines, start) {
  const first = MD_LIST.exec(lines[start]);
  const indent = first[1].length;
  const ordered = /\d/.test(first[2]);
  const list = el(ordered ? 'ol' : 'ul');
  if (ordered && parseInt(first[2], 10) !== 1) list.setAttribute('start', String(parseInt(first[2], 10)));
  let i = start;
  while (i < lines.length) {
    // Blank lines between items keep the list going ("loose" list).
    let j = i;
    while (j < lines.length && !lines[j].trim()) j++;
    const m = MD_LIST.exec(lines[j] || '');
    if (!m || m[1].length !== indent || /\d/.test(m[2]) !== ordered) break;
    i = j;
    // The item's own lines: continuation text and deeper-indented content.
    const body = [m[3]];
    i++;
    while (i < lines.length) {
      const line = lines[i];
      if (!line.trim()) {
        const next = lines[i + 1];
        if (next !== undefined && next.trim() && /^\s*/.exec(next)[0].length > indent) { body.push(''); i++; continue; }
        break;
      }
      const sub = MD_LIST.exec(line);
      if (sub && sub[1].length <= indent) break;
      if (!sub && /^\s*/.exec(line)[0].length <= indent && mdStartsBlock(line)) break;
      body.push(line.replace(new RegExp(`^\\s{0,${indent + 2}}`), ''));
      i++;
    }
    const li = el('li');
    const task = /^\[([ xX])\]\s+/.exec(body[0]);
    if (task) {
      li.className = 'task';
      li.append(el('input', { type: 'checkbox', disabled: true, checked: task[1] !== ' ' }));
      body[0] = body[0].slice(task[0].length);
    }
    // A simple item stays inline; one with nested blocks gets them parsed.
    const nested = body.slice(1).some((l) => l.trim());
    if (!nested) li.append(...mdInline(body[0]));
    else {
      const blocks = mdBlocks(body);
      // "Tight" item: unwrap the first paragraph so it doesn't get paragraph spacing.
      if (blocks.firstChild?.nodeName === 'P') li.append(...blocks.firstChild.childNodes, ...[...blocks.childNodes].slice(1));
      else li.append(blocks);
    }
    list.append(li);
  }
  return [list, i];
}

function mdBlocks(lines) {
  const frag = document.createDocumentFragment();
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    let m;
    if (!line.trim()) { i++; continue; }
    if ((m = MD_FENCE.exec(line))) {
      const fence = m[1];
      const code = [];
      i++;
      while (i < lines.length && !new RegExp(`^\\s{0,3}${fence[0] === '`' ? '`' : '~'}{${fence.length},}\\s*$`).test(lines[i])) code.push(lines[i++]);
      i++;
      const lang = m[2].toLowerCase().replace(/[^a-z0-9+#-]/g, '');
      frag.append(el('pre', {}, el('code', { class: lang ? `language-${lang}` : null, text: code.join('\n') })));
      continue;
    }
    if ((m = /^\s{0,3}(#{1,6})(?:\s+(.*?))?(?:\s+#+)?\s*$/.exec(line))) {
      frag.append(el(`h${m[1].length}`, {}, ...mdInline(m[2] || '')));
      i++; continue;
    }
    if (MD_HR.test(line)) { frag.append(el('hr')); i++; continue; }
    if (/^\s{0,3}>/.test(line)) {
      const quote = [];
      while (i < lines.length && lines[i].trim() && (/^\s{0,3}>/.test(lines[i]) || !mdStartsBlock(lines[i]))) {
        quote.push(lines[i].replace(/^\s{0,3}> ?/, ''));
        i++;
      }
      frag.append(el('blockquote', {}, mdBlocks(quote)));
      continue;
    }
    if (MD_LIST.test(line)) {
      const [list, next] = mdList(lines, i);
      frag.append(list);
      i = next;
      continue;
    }
    if (line.includes('|') && MD_TABLE_SEP.test(lines[i + 1] || '') && lines[i + 1].includes('-')) {
      const head = mdCells(line);
      const align = mdCells(lines[i + 1]).map((c) => (c.startsWith(':') && c.endsWith(':') ? 'align-center' : c.endsWith(':') ? 'align-right' : c.startsWith(':') ? 'align-left' : null));
      const cell = (tag, text, k) => el(tag, { class: align[k] || null }, ...mdInline(text));
      const table = el('table', {}, el('thead', {}, el('tr', {}, ...head.map((c, k) => cell('th', c, k)))));
      const body = el('tbody');
      i += 2;
      while (i < lines.length && lines[i].trim() && lines[i].includes('|')) {
        const cells = mdCells(lines[i]);
        body.append(el('tr', {}, ...head.map((_, k) => cell('td', cells[k] || '', k))));
        i++;
      }
      table.append(body);
      frag.append(table);
      continue;
    }
    const para = [];
    while (i < lines.length && lines[i].trim() && !(para.length && mdStartsBlock(lines[i]))) {
      if (para.length && lines[i].includes('|') && MD_TABLE_SEP.test(lines[i + 1] || '')) break;
      para.push(lines[i++]);
    }
    frag.append(mdParagraph(para));
  }
  return frag;
}

/** Markdown text → DocumentFragment. */
function renderMarkdown(text) {
  return mdBlocks(String(text).replace(/\r\n?/g, '\n').split('\n'));
}
