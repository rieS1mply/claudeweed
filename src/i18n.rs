//! Language of the console and of Telegram messages (`--lang ru|en`, Russian by default).
//!
//! Texts stay written in Russian in the code; [`tr`] looks the finished text up in the
//! English table. Keys with `{}` match any text in that place, so formatted messages
//! ("Облако запущено:\n{url}") are found too. The web page has its own table (lang-en.js).

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn set(lang: Option<&str>) {
    ENGLISH.store(lang == Some("en"), Ordering::Relaxed);
}

pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// Prints a line in the chosen language.
#[macro_export]
macro_rules! say {
    () => { println!() };
    ($($arg:tt)*) => {
        println!("{}", $crate::i18n::tr(&format!($($arg)*)))
    };
}

/// `text` in the chosen language: as is in Russian, from the table in English.
/// Unknown texts (file names, Claude's answers) come back unchanged.
pub fn tr(text: &str) -> String {
    if !english() {
        return text.to_string();
    }
    // Spaces around are layout (indented lines, "Prompt: "), not part of the key.
    let start = text.trim_start_matches(' ');
    let body = start.trim_end_matches(' ');
    let (indent, tail) = (&text[..text.len() - start.len()], &start[body.len()..]);
    if let Some(&(_, en)) = crate::lang_en::EN.iter().find(|(ru, _)| *ru == body) {
        return format!("{indent}{en}{tail}");
    }
    for &(ru, en) in crate::lang_en::EN.iter().filter(|(ru, _)| ru.contains("{}")) {
        if let Some(parts) = match_pattern(ru, body) {
            let mut out = String::from(indent);
            let mut pieces = en.split("{}");
            out.push_str(pieces.next().unwrap_or(""));
            for (i, piece) in pieces.enumerate() {
                out.push_str(parts.get(i).map_or("", String::as_str));
                out.push_str(piece);
            }
            out.push_str(tail);
            return out;
        }
    }
    text.to_string()
}

/// Matches `text` against a key with `{}` holes; returns what stands in the holes.
fn match_pattern(key: &str, text: &str) -> Option<Vec<String>> {
    let lits: Vec<&str> = key.split("{}").collect();
    let mut rest = text.strip_prefix(lits[0])?;
    let mut parts = Vec::new();
    for (i, lit) in lits.iter().enumerate().skip(1) {
        let last = i == lits.len() - 1;
        let at = if last {
            if !rest.ends_with(lit) {
                return None;
            }
            rest.len() - lit.len()
        } else if lit.is_empty() {
            return None; // two holes in a row: ambiguous
        } else {
            rest.find(lit)?
        };
        parts.push(rest[..at].to_string());
        rest = &rest[at + lit.len()..];
    }
    Some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert_eq!(match_pattern("Облако запущено:\n{}", "Облако запущено:\nhttps://x"), Some(vec!["https://x".to_string()]));
        assert_eq!(match_pattern("a {} b {} c", "a 1 b 22 c"), Some(vec!["1".into(), "22".into()]));
        assert_eq!(match_pattern("a {} b", "a 1 c"), None);
        assert_eq!(match_pattern("x", "y"), None);
        set(Some("en"));
        assert_eq!(tr("  Хранилище:   D:\\Cloud"), "  Storage:     D:\\Cloud");
        assert_eq!(tr("какой-то текст"), "какой-то текст");
        set(None);
        assert_eq!(tr("  Хранилище:   D:\\Cloud"), "  Хранилище:   D:\\Cloud");
    }
}
