//! Search inside notes and text files (the page itself only matches names).

use std::path::Path;

use serde::Serialize;

/// Files bigger than this are not read: logs and dumps would make every search slow.
const MAX_FILE: u64 = 2 * 1024 * 1024;
const MAX_HITS: usize = 300;
/// Characters shown around the match.
const CONTEXT: usize = 50;

/// Extensions read as text. Mirrors the editor's list in editor.js, minus binary-ish ones.
const TEXT_EXT: &[&str] = &[
    "txt", "md", "markdown", "csv", "tsv", "log", "json", "jsonc", "yaml", "yml", "toml", "ini", "conf", "cfg", "env",
    "xml", "html", "htm", "svg", "css", "scss", "less", "js", "mjs", "cjs", "jsx", "ts", "tsx", "rs", "py", "c", "h",
    "cpp", "hpp", "cc", "cs", "java", "kt", "go", "php", "rb", "swift", "lua", "pl", "r", "sh", "bash", "ps1", "bat",
    "cmd", "sql", "srt", "vtt", "nfo", "properties", "gitignore",
];

#[derive(Serialize, Debug)]
pub struct Hit {
    /// "notes" or "files".
    pub section: &'static str,
    /// Note id, or the path inside the section.
    pub path: String,
    /// Text around the first match.
    pub snippet: String,
}

/// Lowercase with ё → е, so "ёлка" finds "Елка" (as the page's own search does).
pub fn normalize(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).map(|c| if c == 'ё' { 'е' } else { c }).collect()
}

fn words(query: &str) -> Vec<String> {
    normalize(query).split_whitespace().map(str::to_string).collect()
}

/// Snippet around the first word, if the text contains every word.
pub fn matches(text: &str, words: &[String]) -> Option<String> {
    if words.is_empty() {
        return None;
    }
    let norm = normalize(text);
    if !words.iter().all(|w| norm.contains(w.as_str())) {
        return None;
    }
    // normalize() keeps one char per char for everything but rare letters whose lowercase
    // is longer; count in chars, and fall back to the start if that shifted anything.
    let byte = norm.find(words[0].as_str()).unwrap_or(0);
    let at = norm[..byte].chars().count();
    let chars: Vec<char> = text.chars().collect();
    let at = at.min(chars.len());
    let start = at.saturating_sub(CONTEXT);
    let end = (at + words[0].chars().count() + CONTEXT).min(chars.len());
    let mut snippet: String = chars[start..end].iter().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    if start > 0 {
        snippet.insert(0, '…');
    }
    if end < chars.len() {
        snippet.push('…');
    }
    Some(snippet)
}

fn is_text(name: &str) -> bool {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    TEXT_EXT.contains(&ext.as_str())
}

pub fn search(notes_dir: &Path, files_root: &Path, query: &str) -> Vec<Hit> {
    let words = words(query);
    let mut hits = Vec::new();
    if words.is_empty() {
        return hits;
    }
    for entry in std::fs::read_dir(notes_dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".md") else { continue };
        if name.starts_with('.') {
            continue;
        }
        let Some((content, _)) = crate::notes::read(notes_dir, id) else { continue };
        if let Some(snippet) = matches(&content, &words) {
            hits.push(Hit { section: "notes", path: id.to_string(), snippet });
        }
    }
    for item in crate::storage::list(files_root) {
        if hits.len() >= MAX_HITS {
            break;
        }
        if item.size > MAX_FILE || !is_text(&item.name) {
            continue;
        }
        let Some(path) = crate::storage::resolve(files_root, &item.path) else { continue };
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        if let Some(snippet) = matches(&text, &words) {
            hits.push(Hit { section: "files", path: item.path, snippet });
        }
    }
    hits.truncate(MAX_HITS);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_all_words_with_context() {
        let w = words("ЁЛКУ игрушки");
        assert_eq!(matches("Купить игрушки на елку", &w).as_deref(), Some("Купить игрушки на елку"));
        assert!(matches("Купить игрушки", &w).is_none());
        let long = format!("{}нужное слово{}", "а ".repeat(100), " б".repeat(100));
        let s = matches(&long, &words("нужное")).unwrap();
        assert!(s.starts_with('…') && s.ends_with('…') && s.contains("нужное слово"));
        assert!(matches("x", &[]).is_none());
    }

    #[test]
    fn searches_notes_and_text_files() {
        let dir = std::env::temp_dir().join(format!("lp-search-{}", std::process::id()));
        let (notes, files) = (dir.join("notes"), dir.join("files"));
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::create_dir_all(files.join("2026/10-Октябрь")).unwrap();
        std::fs::write(notes.join("abc1.md"), "Покупки\nмолоко и хлеб").unwrap();
        std::fs::write(files.join("2026/10-Октябрь/list.txt"), "хлеб, сыр").unwrap();
        std::fs::write(files.join("2026/10-Октябрь/photo.jpg"), "хлеб").unwrap();
        let hits = search(&notes, &files, "хлеб");
        let found: Vec<_> = hits.iter().map(|h| (h.section, h.path.as_str())).collect();
        assert_eq!(found, vec![("notes", "abc1"), ("files", "2026/10-Октябрь/list.txt")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
