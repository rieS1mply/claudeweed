'use strict';
// Interface language. The Russian text itself is the key: t('Удалить') is 'Удалить' in
// Russian and looks itself up in I18N.en for English. Dictionaries: lang-*.js.
// Placeholders: t('Найдено {n} из {m}', { n: 3, m: 10 }).

const I18N = {};
const LANGS = { ru: 'Русский', en: 'English' };

const LANG = (() => {
  try {
    const saved = localStorage.getItem('lp-lang');
    if (saved && LANGS[saved]) return saved;
  } catch { /* private mode */ }
  return (navigator.language || 'ru').toLowerCase().startsWith('ru') ? 'ru' : 'en';
})();
document.documentElement.lang = LANG;

function t(key, vars) {
  let s = (LANG !== 'ru' && I18N[LANG]?.[key]) || key;
  if (vars) s = s.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? String(vars[k]) : m));
  return s;
}

/** Plural form: Russian has three (1 файл, 2 файла, 5 файлов), English two (1 file, 2 files). */
function plural(n, one, few, many) {
  if (LANG !== 'ru') return t(n === 1 ? one : many);
  const m10 = n % 10, m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return one;
  if (m10 >= 2 && m10 <= 4 && (m100 < 10 || m100 >= 20)) return few;
  return many;
}

let serverPatterns = null;
/** Error text from the server (always Russian): exact match first, then keys with {placeholders}. */
function tServer(msg) {
  if (LANG === 'ru' || !msg) return msg;
  const dict = I18N[LANG] || {};
  if (dict[msg]) return dict[msg];
  serverPatterns ||= Object.keys(dict).filter((k) => k.includes('{')).map((k) => {
    const names = [];
    const src = k.replace(/[.*+?^$()|[\]\\]/g, '\\$&').replace(/\{(\w+)\}/g, (m, n) => { names.push(n); return '(.+?)'; });
    return { key: k, re: new RegExp(`^${src}$`, 's'), names };
  });
  for (const p of serverPatterns) {
    const m = p.re.exec(msg);
    if (m) return t(p.key, Object.fromEntries(p.names.map((n, i) => [n, m[i + 1]])));
  }
  return msg;
}

/** Translates the fixed texts of the page: text nodes and title/placeholder/aria-label. */
function translatePage(root = document.body) {
  if (LANG === 'ru') return;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const cyr = /[А-Яа-яЁё]/;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const text = n.nodeValue.trim();
    if (text && cyr.test(text) && !n.parentElement?.closest('script, style, textarea')) {
      n.nodeValue = n.nodeValue.replace(text, t(text));
    }
  }
  for (const node of root.querySelectorAll('[title], [placeholder], [aria-label]')) {
    for (const attr of ['title', 'placeholder', 'aria-label']) {
      const v = node.getAttribute(attr);
      if (v && cyr.test(v)) node.setAttribute(attr, t(v));
    }
  }
  if (document.title) document.title = t(document.title);
}

function setLang(lang) {
  try { localStorage.setItem('lp-lang', lang); } catch { /* private mode */ }
  location.reload();
}
