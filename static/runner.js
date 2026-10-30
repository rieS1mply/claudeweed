'use strict';
// Code runner: "▶ Запустить" in the code editor and a "new code" playground, like an online
// compiler. The server compiles and runs the file with what this computer has installed and
// streams the output back (NDJSON, see src/runner.rs). Off unless the owner ran --runner on.

// Extension -> runner language (src/runner.rs LANGS).
const RUN_LANG = {
  py: 'python', pyw: 'python', js: 'javascript', mjs: 'javascript', cjs: 'javascript', ts: 'typescript', mts: 'typescript',
  c: 'c', cpp: 'cpp', cc: 'cpp', cxx: 'cpp', cs: 'csharp', java: 'java', go: 'go', rs: 'rust', php: 'php', rb: 'ruby',
  lua: 'lua', ps1: 'powershell', sh: 'bash', bash: 'bash',
};
// Highlighting for each runner language (editor.js CODE_LANG keys are extensions).
const RUN_HL = {
  python: 'python', javascript: 'javascript', typescript: 'typescript', c: 'c', cpp: 'cpp', csharp: 'csharp', java: 'java',
  go: 'go', rust: 'rust', php: 'php', ruby: 'ruby', lua: 'lua', powershell: 'powershell', bash: 'bash',
};
const MAX_OUT_CHARS = 400_000;   // the page keeps the tail; the server already caps at 1 MB
const STDIN_KEY = 'lp-run-stdin';
const PANEL_KEY = 'lp-run-panel';

const run = {
  enabled: false,
  langs: null,         // [{ id, name, file, available, version, hint }] from the server
  lang: null,          // runner language of the open file, if it can run
  ctrl: null,          // AbortController of the run in progress
  outChars: 0,
};

function setRunnerEnabled(on) {
  run.enabled = on;
  $('#code-btn').hidden = !on;
  if (on && !run.langs) loadRunLangs().catch(() => {});
}

async function loadRunLangs(refresh = false) {
  const data = await request(`/api/run/langs${refresh ? '?refresh=1' : ''}`);
  run.langs = data.langs;
  return run.langs;
}

function runLangInfo(id) { return run.langs?.find((l) => l.id === id); }

// ---------- panel ----------

const panelPrefs = (() => {
  try { return { height: 0.4, ...JSON.parse(localStorage.getItem(PANEL_KEY) || '{}') }; } catch { return { height: 0.4 }; }
})();
function savePanelPrefs() {
  try { localStorage.setItem(PANEL_KEY, JSON.stringify(panelPrefs)); } catch { /* private mode */ }
}

function applyPanelHeight() {
  const h = Math.min(0.85, Math.max(0.15, panelPrefs.height));
  $('#doc').style.setProperty('--run-h', `${Math.round(h * 100)}%`);
}

/** Called by openDocView: shows "Запустить" for files the runner can run. */
function runnerOnOpen() {
  const ext = doc.file ? splitName(doc.file)[1] : '';
  run.lang = run.enabled && doc.code ? (doc.runLang || RUN_LANG[ext] || null) : null;
  doc.runLang = null;
  $('#code-run').hidden = !run.lang;
  $('#run-panel').hidden = true;
  $('#doc').classList.remove('run-open', 'run-full');
  resetExpand();
  setRunButton(false);
  updateSymBar();
}

function runnerOnClose() {
  stopRun();
  $('#run-panel').hidden = true;
  $('#sym-bar').hidden = true;
  $('#doc').classList.remove('run-open', 'run-full');
  fitDocToViewport(false);
}

function showPanel() {
  const panel = $('#run-panel');
  if (!panel.hidden) return;
  applyPanelHeight();
  panel.hidden = false;
  $('#doc').classList.add('run-open');
  enter(panel, [{ opacity: 0, transform: 'translateY(16px)' }, { opacity: 1, transform: 'none' }], 'smooth');
}

function hidePanel() {
  $('#run-panel').hidden = true;
  $('#doc').classList.remove('run-open', 'run-full');
  resetExpand();
}

function setRunButton(running) {
  const btn = $('#code-run');
  btn.classList.toggle('stop', running);
  btn.title = running ? t('Остановить (Ctrl+Enter)') : t('Запустить (Ctrl+Enter)');
  btn.querySelector('span').textContent = running ? t('Стоп') : t('Запустить');
  btn.querySelector('path').setAttribute('d', running ? 'M7 7h10v10H7z' : 'M7 4v16l13-8z');
}

function setRunStatus(text, state) {
  $('#run-status').textContent = text;
  $('#run-dot').dataset.state = state;
}

/** Appends output, keeping the panel scrolled to the end if it was there. */
function appendOut(text, kind) {
  if (!text) return;
  const out = $('#run-out');
  const atEnd = out.scrollHeight - out.scrollTop - out.clientHeight < 40;
  const last = out.lastChild;
  if (last && last.dataset.kind === kind) last.textContent += text;
  else out.append(el('span', { class: `run-${kind}`, 'data-kind': kind, text }));
  run.outChars += text.length;
  while (run.outChars > MAX_OUT_CHARS && out.firstChild) {
    run.outChars -= out.firstChild.textContent.length;
    out.firstChild.remove();
  }
  if (atEnd) out.scrollTop = out.scrollHeight;
}

const EXIT_TEXT = {
  timeout: t('Превышено время — программа остановлена'),
  limit: t('Слишком много вывода — программа остановлена'),
  compile: t('Ошибка компиляции'),
  stopped: t('Остановлено'),
  error: t('Не запущено'),
};

function fmtMs(ms) { return ms < 1000 ? t('{0} мс', { 0: ms }) : t('{0} с', { 0: (ms / 1000).toFixed(ms < 10_000 ? 2 : 1) }); }

// ---------- running ----------

async function runCode() {
  if (run.ctrl) return stopRun();
  if (!run.lang) return;
  const info = runLangInfo(run.lang) || (await loadRunLangs().catch(() => null), runLangInfo(run.lang));
  showPanel();
  $('#run-out').replaceChildren();
  run.outChars = 0;
  if (info && !info.available) {
    setRunStatus(t('{0} не установлен', { 0: info.name }), 'error');
    appendOut(t('{0} не найден на компьютере.\n{1}\n\nПосле установки обновите страницу.', { 0: info.name, 1: info.hint }), 'info');
    return;
  }
  const ctrl = new AbortController();
  run.ctrl = ctrl;
  setRunButton(true);
  setRunStatus(t('Запуск…'), 'busy');
  const started = performance.now();
  let finished = false;
  try {
    const res = await fetch('/api/run', {
      method: 'POST', signal: ctrl.signal, credentials: 'same-origin',
      headers: { 'X-Cloud': '1', 'Content-Type': 'application/json' },
      body: JSON.stringify({ lang: run.lang, code: $('#doc-text').value, stdin: stdinText() }),
    });
    if (res.status === 401) { showLogin(); throw new Error(t('Требуется вход')); }
    if (!res.ok) {
      const data = await res.json().catch(() => null);
      throw new Error(tServer(data?.error) || t('Ошибка {0}', { 0: res.status }));
    }
    const reader = res.body.getReader();
    const dec = new TextDecoder();
    let buf = '';
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      buf += dec.decode(value, { stream: true });
      let nl;
      while ((nl = buf.indexOf('\n')) !== -1) {
        const line = buf.slice(0, nl);
        buf = buf.slice(nl + 1);
        if (line) finished = onRunEvent(JSON.parse(line)) || finished;
      }
    }
    if (!finished) setRunStatus(t('Соединение прервалось'), 'error');
  } catch (e) {
    if (e.name === 'AbortError') {
      setRunStatus(t('Остановлено · {0}', { 0: fmtMs(Math.round(performance.now() - started)) }), 'error');
      appendOut(t('\n[остановлено]\n'), 'info');
    } else {
      setRunStatus(e.message, 'error');
    }
  } finally {
    if (run.ctrl === ctrl) run.ctrl = null;
    setRunButton(false);
  }
}

/** True for the final event. */
function onRunEvent(ev) {
  if (ev.t === 'phase') {
    setRunStatus(ev.v === 'compile' ? t('Компиляция…') : t('Работает…'), 'busy');
    if (ev.v === 'run' && $('#run-out').childNodes.length) appendOut('\n', 'info');
  } else if (ev.t === 'out') {
    appendOut(ev.v, 'out');
  } else if (ev.t === 'err') {
    appendOut(ev.v, 'err');
  } else if (ev.t === 'exit') {
    const time = fmtMs(ev.ms);
    if (ev.reason) setRunStatus(`${EXIT_TEXT[ev.reason] || ev.reason} · ${time}`, 'error');
    else setRunStatus(t('Готово · код {0} · {1}', { 0: ev.code ?? '?', 1: time }), ev.code === 0 ? 'ok' : 'error');
    if (!$('#run-out').childNodes.length) appendOut(t('(программа ничего не вывела)'), 'info');
    return true;
  }
  return false;
}

function stopRun() {
  run.ctrl?.abort();
  run.ctrl = null;
}

// ---------- stdin ----------

function stdinText() {
  const box = $('#run-stdin');
  return box.hidden ? '' : box.value;
}

function toggleStdin() {
  const box = $('#run-stdin');
  box.hidden = !box.hidden;
  $('#run-stdin-btn').classList.toggle('on', !box.hidden);
  if (!box.hidden) {
    try { box.value = box.value || sessionStorage.getItem(STDIN_KEY) || ''; } catch { /* private mode */ }
    box.focus();
  }
}

// ---------- resizing ----------

function bindPanelResize() {
  const grip = $('#run-grip');
  let drag = null;
  grip.addEventListener('pointerdown', (e) => {
    const body = $('#doc').getBoundingClientRect();
    drag = { top: body.top + $('.doc-bar').offsetHeight, bottom: body.bottom };
    grip.setPointerCapture(e.pointerId);
    $('#doc').classList.add('resizing');
  });
  grip.addEventListener('pointermove', (e) => {
    if (!drag) return;
    panelPrefs.height = (drag.bottom - e.clientY) / (drag.bottom - drag.top);
    applyPanelHeight();
  });
  const end = () => {
    if (!drag) return;
    drag = null;
    $('#doc').classList.remove('resizing');
    savePanelPrefs();
  };
  grip.addEventListener('pointerup', end);
  grip.addEventListener('pointercancel', end);
}

function resetExpand() {
  $('#run-expand').textContent = '⤢';
  $('#run-expand').title = t('Развернуть');
}

function toggleFull() {
  const full = $('#doc').classList.toggle('run-full');
  $('#run-expand').textContent = full ? '⤡' : '⤢';
  $('#run-expand').title = full ? t('Свернуть') : t('Развернуть');
}

// ---------- phone: symbols over the keyboard ----------

const SYMBOLS = ['⇥', '{', '}', '(', ')', '[', ']', ';', '=', '"', "'", '<', '>', '/', '\\', '|', '&', ':', '_', '+', '-', '*', '!', '#', '%', '.', ','];
const TOUCH = matchMedia('(pointer: coarse)');

function updateSymBar() {
  const bar = $('#sym-bar');
  const show = TOUCH.matches && doc.code && document.activeElement === $('#doc-text');
  bar.hidden = !show;
  fitDocToViewport(show);
}

function insertSymbol(sym) {
  const area = $('#doc-text');
  if (sym === '⇥') return indentSelection(area, false);
  // Pairs land with the caret between them, as in IDEs.
  const pair = { '(': ')', '[': ']', '{': '}', '"': '"', "'": "'" }[sym];
  const s = area.selectionStart, e = area.selectionEnd;
  const selected = area.value.slice(s, e);
  insertText(area, pair ? sym + selected + pair : sym);
  if (pair) area.setSelectionRange(s + 1, s + 1 + selected.length);
}

/**
 * iOS overlays the keyboard on the page instead of shrinking it, which would hide the symbol
 * bar and the end of the text. While typing, the editor takes just the visible part.
 */
function fitDocToViewport(on) {
  const view = $('#doc'), vv = window.visualViewport;
  if (!on || !vv) {
    view.style.removeProperty('height');
    view.style.removeProperty('top');
    return;
  }
  view.style.height = `${vv.height}px`;
  view.style.top = `${vv.offsetTop}px`;
}

// ---------- playground ----------

/** "Новый код": pick a language, get a starter program, run it, save it to Files if wanted. */
async function openPlayground() {
  let langs = run.langs;
  if (!langs) {
    try { langs = await loadRunLangs(); } catch (e) { return toast(e.message); }
  }
  const actions = langs.map((l) => ({
    label: l.available ? l.name : t('{0} — не установлен', { 0: l.name }),
    hint: l.available ? (l.version || '') : l.hint,
    run: () => (l.available ? startPlayground(l) : toast(l.hint)),
  }));
  actions.push({ label: t('Обновить список (после установки)'), run: async () => { await loadRunLangs(true).catch(() => {}); openPlayground(); } });
  actionSheet(t('Новый код — выберите язык'), actions);
}

async function startPlayground(lang) {
  let saved = null;   // { section, item } once saved to Files
  let original = null;
  const code = RUN_TEMPLATES[lang.id] || '';
  const save = el('button', { class: 'doc-btn strong', text: t('В Файлы'), 'data-save': '1' });
  const title = () => saved ? saved.item.name : t('{0} · черновик', { 0: lang.file });
  save.onclick = async () => {
    const text = $('#doc-text').value;
    save.disabled = true;
    try {
      if (saved) {
        await api.replace('files', saved.item, new Blob([text], { type: 'text/plain' }), false);
      } else {
        const name = await promptDialog(t('Сохранить в «Файлы» как'), lang.file.replace(/^Main\./, 'main.'), { ok: t('Сохранить'), selectStem: true });
        if (!name) { save.disabled = false; return; }
        const res = await api.upload(new File([text], name, { type: 'text/plain' }), () => {});
        saved = { item: { name: res.path.split('/').pop(), path: res.path, size: text.length } };
        save.textContent = t('Сохранить');
        $('#doc-title').textContent = title();
      }
      original = text;
      $('#doc-status').textContent = t('Сохранено');
      load('files');
    } catch (e) {
      save.disabled = false;
      toast(e.message);
    }
  };
  doc.runLang = lang.id;   // runnerOnOpen picks it up: a draft has no file name yet
  await openDocView({
    title: title(), text: code, mono: true, lang: RUN_HL[lang.id], file: lang.file,
    left: t('Закрыть'), right: [save], slide: 'up',
    onInput: (v) => {
      save.disabled = saved ? v === original : false;
      $('#doc-status').textContent = saved && v !== original ? t('Изменено') : '';
    },
    onClose: async () => {
      const text = $('#doc-text').value;
      const unsaved = saved ? text !== original : text !== code;
      if (unsaved && !(await confirmDialog(saved ? t('Изменения не сохранены.') : t('Черновик не сохранён в «Файлы».'), { title: t('Закрыть?'), ok: t('Закрыть') }))) return;
      closeDocView();
    },
  });
  if (!TOUCH.matches) $('#doc-text').focus();
}

// ---------- wiring ----------

function bindRunner() {
  const area = $('#doc-text');
  $('#code-run').addEventListener('click', runCode);
  $('#code-btn').addEventListener('click', openPlayground);
  $('#run-close').addEventListener('click', () => { stopRun(); hidePanel(); });
  $('#run-clear').addEventListener('click', () => { $('#run-out').replaceChildren(); run.outChars = 0; });
  $('#run-stdin-btn').addEventListener('click', toggleStdin);
  $('#run-expand').addEventListener('click', toggleFull);
  // Phone: tapping the header (not its buttons) also expands, the panel is small there.
  $('#run-head').addEventListener('click', (e) => { if (TOUCH.matches && !e.target.closest('button')) toggleFull(); });
  $('#run-stdin').addEventListener('input', (e) => {
    try { sessionStorage.setItem(STDIN_KEY, e.target.value); } catch { /* private mode */ }
  });
  area.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key === 'Enter' && run.lang) { e.preventDefault(); runCode(); }
  });
  // Keep the keyboard open: buttons must not take the focus away from the text.
  const bar = $('#sym-bar');
  for (const sym of SYMBOLS) {
    const b = el('button', { type: 'button', class: 'sym', text: sym, title: sym === '⇥' ? t('Отступ') : sym });
    b.addEventListener('pointerdown', (e) => e.preventDefault());
    b.addEventListener('click', () => insertSymbol(sym));
    bar.append(b);
  }
  area.addEventListener('focus', updateSymBar);
  area.addEventListener('blur', () => setTimeout(updateSymBar, 50));
  window.visualViewport?.addEventListener('resize', () => { if (!$('#sym-bar').hidden) fitDocToViewport(true); });
  window.visualViewport?.addEventListener('scroll', () => { if (!$('#sym-bar').hidden) fitDocToViewport(true); });
  bindPanelResize();
}
bindRunner();

// Starter programs: greet, echo one line of input (none is fine), compute something.
const RUN_TEMPLATES = {
  python: `def main():
    print("Привет из облака!")
    try:
        line = input()
    except EOFError:
        line = ""
    print("Ввод:", line)
    print("2 + 3 =", 2 + 3)


main()
`,
  javascript: `const lines = require('fs').readFileSync(0, 'utf8').split('\\n');

console.log('Привет из облака!');
console.log('Ввод: ' + (lines[0] || ''));
console.log('2 + 3 = ' + (2 + 3));
`,
  typescript: `import { readFileSync } from 'node:fs';

const line: string = readFileSync(0, 'utf8').split('\\n')[0] ?? '';
const sum = (a: number, b: number): number => a + b;

console.log('Привет из облака!');
console.log('Ввод: ' + line);
console.log('2 + 3 = ' + sum(2, 3));
`,
  c: `#include <stdio.h>
#include <string.h>

int main(void) {
  char line[256] = "";
  printf("Привет из облака!\\n");
  if (fgets(line, sizeof line, stdin)) line[strcspn(line, "\\r\\n")] = 0;
  printf("Ввод: %s\\n", line);
  printf("2 + 3 = %d\\n", 2 + 3);
  return 0;
}
`,
  cpp: `#include <iostream>
#include <string>

int main() {
  std::string line;
  std::cout << "Привет из облака!\\n";
  std::getline(std::cin, line);
  std::cout << "Ввод: " << line << "\\n";
  std::cout << "2 + 3 = " << 2 + 3 << "\\n";
}
`,
  csharp: `Console.WriteLine("Привет из облака!");
var line = Console.ReadLine() ?? "";
Console.WriteLine("Ввод: " + line);
Console.WriteLine("2 + 3 = " + (2 + 3));
`,
  java: `import java.util.Scanner;

public class Main {
  public static void main(String[] args) {
    System.out.println("Привет из облака!");
    Scanner in = new Scanner(System.in, "UTF-8");
    String line = in.hasNextLine() ? in.nextLine() : "";
    System.out.println("Ввод: " + line);
    System.out.println("2 + 3 = " + (2 + 3));
  }
}
`,
  go: `package main

import (
	"bufio"
	"fmt"
	"os"
	"strings"
)

func main() {
	fmt.Println("Привет из облака!")
	line, _ := bufio.NewReader(os.Stdin).ReadString('\\n')
	fmt.Println("Ввод:", strings.TrimRight(line, "\\r\\n"))
	fmt.Println("2 + 3 =", 2+3)
}
`,
  rust: `use std::io::stdin;

fn main() {
    println!("Привет из облака!");
    let mut line = String::new();
    stdin().read_line(&mut line).ok();
    println!("Ввод: {}", line.trim_end());
    println!("2 + 3 = {}", 2 + 3);
}
`,
  php: `<?php
echo "Привет из облака!\\n";
$line = fgets(STDIN);
echo "Ввод: " . trim($line === false ? "" : $line) . "\\n";
echo "2 + 3 = " . (2 + 3) . "\\n";
`,
  ruby: `puts "Привет из облака!"
line = ($stdin.gets || "").chomp
puts "Ввод: #{line}"
puts "2 + 3 = #{2 + 3}"
`,
  lua: `print("Привет из облака!")
local line = io.read("l") or ""
print("Ввод: " .. line)
print("2 + 3 = " .. (2 + 3))
`,
  powershell: `Write-Output "Привет из облака!"
$line = [Console]::In.ReadLine()
Write-Output "Ввод: $line"
Write-Output "2 + 3 = $(2 + 3)"
`,
  bash: `echo "Привет из облака!"
read -r line
echo "Ввод: $line"
echo "2 + 3 = $((2 + 3))"
`,
};
