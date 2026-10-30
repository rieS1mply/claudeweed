//! Text for the Telegram chat with Claude: its Markdown turned into Telegram's HTML subset,
//! long answers cut into messages, one-line captions for the steps it takes.

/// Converts the Markdown Claude writes into Telegram's HTML (`parse_mode: HTML`). Tags are
/// only ever emitted in pairs around already converted text, so they always nest correctly:
/// Telegram refuses the whole message otherwise.
pub fn markdown_to_html(md: &str) -> String {
    let lines: Vec<&str> = md.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if let Some(lang) = trimmed.strip_prefix("```") {
            // An unclosed fence runs to the end, as in any Markdown viewer.
            let start = i + 1;
            let mut end = start;
            while end < lines.len() && !lines[end].trim_start().starts_with("```") {
                end += 1;
            }
            let lang = lang.trim();
            let class = if !lang.is_empty() && lang.chars().all(|c| c.is_ascii_alphanumeric() || "+#-_.".contains(c)) {
                format!(" class=\"language-{lang}\"")
            } else {
                String::new()
            };
            out.push(format!("<pre><code{class}>{}</code></pre>", escape(&lines[start..end].join("\n"))));
            i = end + 1;
        } else if trimmed.starts_with('|') {
            // A table: monospace keeps its columns readable.
            let start = i;
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                i += 1;
            }
            out.push(format!("<pre>{}</pre>", escape(&lines[start..i].join("\n"))));
        } else if quote_body(lines[i]).is_some() {
            let mut quoted = Vec::new();
            while let Some(body) = lines.get(i).and_then(|l| quote_body(l)) {
                quoted.push(inline(body));
                i += 1;
            }
            out.push(format!("<blockquote>{}</blockquote>", quoted.join("\n")));
        } else {
            out.push(block_line(lines[i]));
            i += 1;
        }
    }
    out.join("\n")
}

/// The text of a `> quote` line.
fn quote_body(line: &str) -> Option<&str> {
    let t = line.trim_start();
    if t == ">" {
        return Some("");
    }
    t.strip_prefix("> ")
}

fn block_line(line: &str) -> String {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) {
        if let Some(title) = trimmed[hashes..].strip_prefix(' ') {
            return format!("<b>{}</b>", inline(title.trim()));
        }
    }
    let t = trimmed.trim_end();
    if t.len() >= 3 && ["-", "*", "_"].iter().any(|m| t.chars().all(|c| c.to_string() == *m)) {
        return "———".into();
    }
    let indent = &line[..line.len() - trimmed.len()];
    if let Some(item) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
        return format!("{indent}• {}", inline(item));
    }
    inline(line)
}

/// Inline formatting of one line (or less). Markers without a closing pair stay as typed.
fn inline(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    'scan: while i < c.len() {
        if c[i] == '`' {
            if let Some(j) = find(&c, i + 1, &['`']) {
                out.push_str(&format!("<code>{}</code>", escape(&collect(&c[i + 1..j]))));
                i = j + 1;
                continue;
            }
        }
        if c[i] == '[' {
            if let Some((text, url, next)) = link_at(&c, i) {
                out.push_str(&format!("<a href=\"{}\">{}</a>", escape_attr(&url), inline(&text)));
                i = next;
                continue;
            }
        }
        for (marker, tag) in [(['*', '*'], "b"), (['_', '_'], "b"), (['~', '~'], "s")] {
            if c[i..].starts_with(&marker) {
                // In `***` closing bold and italic at once, bold takes the last two.
                let run_end = |mut j: usize| {
                    while c.get(j + 2) == Some(&marker[0]) {
                        j += 1;
                    }
                    j
                };
                if let Some(j) = find(&c, i + 3, &marker).map(run_end).filter(|&j| fits(&c[i + 2..j])) {
                    out.push_str(&format!("<{tag}>{}</{tag}>", inline(&collect(&c[i + 2..j]))));
                    i = j + 2;
                    continue 'scan;
                }
            }
        }
        if (c[i] == '*' || c[i] == '_') && (i == 0 || !c[i - 1].is_alphanumeric()) {
            let m = c[i];
            // The closing marker must end a word: `snake_case` and `2*3*4` stay as they are.
            let close = (i + 2..c.len()).find(|&j| c[j] == m && c.get(j + 1).is_none_or(|n| !n.is_alphanumeric() && *n != m));
            if let Some(j) = close.filter(|&j| fits(&c[i + 1..j])) {
                out.push_str(&format!("<i>{}</i>", inline(&collect(&c[i + 1..j]))));
                i = j + 1;
                continue;
            }
        }
        out.push_str(&escape(&c[i].to_string()));
        i += 1;
    }
    out
}

/// Text between markers: not empty, no space right inside the markers.
fn fits(inner: &[char]) -> bool {
    !inner.is_empty() && !inner[0].is_whitespace() && !inner[inner.len() - 1].is_whitespace()
}

/// Position of `pat` at or after `from`.
fn find(c: &[char], from: usize, pat: &[char]) -> Option<usize> {
    (from..c.len()).find(|&j| c[j..].starts_with(pat))
}

/// `[text](url)` at `i` with a web or Telegram url: (text, url, index after it).
fn link_at(c: &[char], i: usize) -> Option<(String, String, usize)> {
    let close = find(c, i + 1, &[']'])?;
    if c.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = find(c, close + 2, &[')'])?;
    let url = collect(&c[close + 2..end]);
    let lower = url.to_lowercase();
    if close == i + 1 || !["http://", "https://", "tg://"].iter().any(|s| lower.starts_with(s)) {
        return None;
    }
    Some((collect(&c[i + 1..close]), url, end + 1))
}

fn collect(c: &[char]) -> String {
    c.iter().collect()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    escape(s).replace('"', "&quot;")
}

/// Cuts a Markdown answer into messages of at most `limit` chars: at an empty line if possible,
/// else at a line break, a space, or anywhere. A code block cut in two is closed at the end of
/// one piece and opened again (same language) at the start of the next.
pub fn split_message(text: &str, limit: usize) -> Vec<String> {
    const CLOSE: &str = "\n```";
    let mut rest: Vec<char> = text.trim_matches('\n').chars().collect();
    let mut pieces = Vec::new();
    let mut reopen: Option<String> = None;
    while !rest.iter().all(|c| c.is_whitespace()) {
        let prefix = reopen.take().map(|fence| format!("{fence}\n")).unwrap_or_default();
        let room = limit.saturating_sub(prefix.chars().count()).max(CLOSE.len() + 1);
        if rest.len() <= room {
            push_piece(&mut pieces, format!("{prefix}{}", collect(&rest)));
            break;
        }
        // For each cut "before char k": inside a code block there, and the line that opened it.
        let mut inside = vec![false; room + 1];
        let mut opener: Vec<Option<String>> = vec![None; room + 1];
        let mut current = (!prefix.is_empty()).then(|| prefix.trim_end().to_string());
        for k in 0..=room {
            inside[k] = current.is_some();
            opener[k] = current.clone();
            if k == 0 || rest[k - 1] == '\n' {
                let line: String = rest[k..].iter().take_while(|&&c| c != '\n').collect();
                if line.trim_start().starts_with("```") {
                    current = if current.is_some() { None } else { Some(line.trim().to_string()) };
                }
            }
        }
        let pick = |max: usize, ok: &dyn Fn(usize) -> bool| (1..=max).rev().find(|&k| ok(k));
        let para = |k: usize| rest[k] == '\n' && rest.get(k + 1) == Some(&'\n');
        let line = |k: usize| rest[k] == '\n';
        let space = |k: usize| rest[k - 1] == ' ';
        let outside = pick(room, &|k| !inside[k] && para(k))
            .or_else(|| pick(room, &|k| !inside[k] && line(k)))
            .or_else(|| pick(room, &|k| !inside[k] && space(k)));
        let k = match outside {
            Some(k) => k,
            // Only inside a code block (or no break at all): leave room for the closing fence.
            None if inside[room] => {
                let max = room - CLOSE.len();
                pick(max, &line).or_else(|| pick(max, &space)).unwrap_or(max)
            }
            None => room,
        };
        let close = if inside[k] { opener[k].clone() } else { None };
        let piece = collect(&rest[..k]);
        let piece = piece.trim_end_matches([' ', '\n']);
        let at_line_break = rest[k] == '\n';
        let mut next = &rest[k..];
        while next.first() == Some(&'\n') || (!at_line_break && next.first() == Some(&' ')) {
            next = &next[1..];
        }
        let next = next.to_vec();
        push_piece(&mut pieces, format!("{prefix}{piece}{}", if close.is_some() { CLOSE } else { "" }));
        reopen = close;
        rest = next;
    }
    pieces
}

fn push_piece(pieces: &mut Vec<String>, piece: String) {
    let piece = piece.trim_start_matches('\n').trim_end();
    if !piece.is_empty() {
        pieces.push(piece.to_string());
    }
}

/// One short line about a tool call, for the progress message.
pub fn describe_tool(name: &str, input: &serde_json::Value) -> String {
    let field = |key: &str| input[key].as_str().unwrap_or_default().replace(['\r', '\n'], " ").trim().to_string();
    let file = || {
        let path = input["file_path"].as_str().or(input["notebook_path"].as_str()).unwrap_or_default();
        path.rsplit(['/', '\\']).next().unwrap_or_default().to_string()
    };
    let line = match name {
        "Bash" | "PowerShell" => {
            let what = match field("description") {
                d if d.is_empty() => input["command"].as_str().unwrap_or_default().lines().next().unwrap_or_default().trim().to_string(),
                d => d,
            };
            format!("💻 {}", cut(&what, 80))
        }
        "Read" => format!("📄 Читаю {}", file()),
        "Edit" | "MultiEdit" | "NotebookEdit" => format!("✏️ Правлю {}", file()),
        "Write" => format!("📝 Пишу {}", file()),
        "Glob" | "Grep" => format!("🔍 Ищу {}", cut(&field("pattern"), 60)),
        "WebSearch" => format!("🌐 Ищу в интернете: {}", cut(&field("query"), 60)),
        "WebFetch" => {
            let url = field("url");
            let host = url.split_once("://").map_or(url.as_str(), |(_, r)| r).split(['/', '?', '#']).next().unwrap_or_default();
            format!("🌐 Открываю {host}")
        }
        "Task" | "Agent" => format!("🤖 Помощник: {}", cut(&field("description"), 60)),
        "TodoWrite" => "📋 Обновляю план".into(),
        other => format!("🔧 {}", cut(other, 60)),
    };
    line.trim_end().to_string()
}

/// At most `max` chars, the last one `…` when something was cut.
fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every tag closed, in the right order.
    fn balanced(html: &str) -> bool {
        let mut stack: Vec<String> = Vec::new();
        let mut rest = html;
        while let Some(start) = rest.find('<') {
            let Some(len) = rest[start..].find('>') else { return false };
            let tag = &rest[start + 1..start + len];
            if let Some(name) = tag.strip_prefix('/') {
                if stack.pop().as_deref() != Some(name) {
                    return false;
                }
            } else {
                stack.push(tag.split(' ').next().unwrap_or("").to_string());
            }
            rest = &rest[start + len + 1..];
        }
        stack.is_empty()
    }

    #[test]
    fn text_is_escaped() {
        assert_eq!(markdown_to_html("a < b & c > d"), "a &lt; b &amp; c &gt; d");
    }

    #[test]
    fn inline_formatting() {
        assert_eq!(
            markdown_to_html("**жирный** и *курсив* и `код <x>`"),
            "<b>жирный</b> и <i>курсив</i> и <code>код &lt;x&gt;</code>"
        );
        assert_eq!(markdown_to_html("~~старое~~ __тоже__ _наклон_"), "<s>старое</s> <b>тоже</b> <i>наклон</i>");
        assert_eq!(markdown_to_html("**жирный *и курсив***"), "<b>жирный <i>и курсив</i></b>");
    }

    #[test]
    fn markers_inside_words_stay() {
        assert_eq!(markdown_to_html("snake_case_name и 2*3*4"), "snake_case_name и 2*3*4");
        assert_eq!(markdown_to_html("a * b * c"), "a * b * c");
        assert_eq!(markdown_to_html("**не закрыт"), "**не закрыт");
    }

    #[test]
    fn code_blocks() {
        assert_eq!(
            markdown_to_html("```python\nprint(\"<hi>\")\n```"),
            "<pre><code class=\"language-python\">print(\"&lt;hi&gt;\")</code></pre>"
        );
        assert_eq!(markdown_to_html("```\n**raw**\n```\nпосле"), "<pre><code>**raw**</code></pre>\nпосле");
        assert_eq!(markdown_to_html("```rs\nне закрыт"), "<pre><code class=\"language-rs\">не закрыт</code></pre>");
        assert_eq!(markdown_to_html("```\" onclick=x\nx\n```"), "<pre><code>x</code></pre>");
    }

    #[test]
    fn headings_lists_rules() {
        assert_eq!(markdown_to_html("# Заголовок\n- пункт\n- **два**"), "<b>Заголовок</b>\n• пункт\n• <b>два</b>");
        assert_eq!(markdown_to_html("  * вложенный\n1. номер\n---\n#не заголовок"), "  • вложенный\n1. номер\n———\n#не заголовок");
    }

    #[test]
    fn links() {
        assert_eq!(
            markdown_to_html("[сайт](https://ex.com/?a=1&b=\"2\")"),
            "<a href=\"https://ex.com/?a=1&amp;b=&quot;2&quot;\">сайт</a>"
        );
        assert!(!markdown_to_html("[x](javascript:alert(1))").contains("<a"));
        assert_eq!(markdown_to_html("[a] (b) и [](https://x)"), "[a] (b) и [](https://x)");
    }

    #[test]
    fn quotes_and_tables() {
        assert_eq!(markdown_to_html("> цитата\n> вторая"), "<blockquote>цитата\nвторая</blockquote>");
        let table = markdown_to_html("до\n| a | b |\n|---|---|\n| 1 | 2 |\nпосле");
        assert_eq!(table, "до\n<pre>| a | b |\n|---|---|\n| 1 | 2 |</pre>\nпосле");
    }

    #[test]
    fn short_and_empty_texts() {
        assert!(split_message("", 100).is_empty());
        assert!(split_message("  \n ", 100).is_empty());
        assert_eq!(split_message("\nкоротко\n\n", 100), vec!["коротко"]);
    }

    #[test]
    fn paragraphs_are_kept_together() {
        let p = |c: char| c.to_string().repeat(60);
        let text = format!("{}\n\n{}\n\n{}", p('а'), p('б'), p('в'));
        assert_eq!(split_message(&text, 130), vec![format!("{}\n\n{}", p('а'), p('б')), p('в')]);
    }

    #[test]
    fn long_lines_break_between_words() {
        let words: Vec<String> = (0..250).map(|i| format!("слово{i}")).collect();
        let pieces = split_message(&words.join(" "), 100);
        assert!(pieces.iter().all(|p| p.chars().count() <= 100 && !p.starts_with(' ') && !p.ends_with(' ')));
        assert_eq!(pieces.join(" "), words.join(" "));
    }

    #[test]
    fn text_without_breaks_is_cut_anywhere() {
        let text = "ж".repeat(350);
        let pieces = split_message(&text, 100);
        assert_eq!(pieces.len(), 4);
        assert_eq!(pieces.concat(), text);
    }

    #[test]
    fn code_blocks_are_reopened() {
        let code: Vec<String> = (1..=40).map(|i| format!("let x = 1; // строка {i}")).collect();
        let text = format!("Вот код:\n\n```rust\n{}\n```\nГотово.", code.join("\n"));
        let pieces = split_message(&text, 200);
        assert!(pieces.len() > 3);
        assert_eq!(pieces[0], "Вот код:", "the block starts in a new piece rather than being cut early");
        for (i, p) in pieces.iter().enumerate() {
            assert!(p.chars().count() <= 200, "{p}");
            assert_eq!(p.lines().filter(|l| l.starts_with("```")).count() % 2, 0, "{p}");
            if i > 1 {
                assert!(p.starts_with("```rust\n"), "{p}");
            }
        }
        // Nothing lost: every code line made it, in order.
        let all: Vec<&str> = pieces.iter().flat_map(|p| p.lines()).filter(|l| l.starts_with("let")).collect();
        assert_eq!(all, code.iter().map(String::as_str).collect::<Vec<_>>());
        assert!(pieces.last().unwrap().ends_with("Готово."));
    }

    #[test]
    fn tool_captions() {
        use serde_json::json;
        assert_eq!(describe_tool("Bash", &json!({ "command": "cargo test", "description": "Run tests" })), "💻 Run tests");
        assert_eq!(describe_tool("PowerShell", &json!({ "command": "Get-Date\nexit" })), "💻 Get-Date");
        let long = describe_tool("Bash", &json!({ "command": "x".repeat(200) }));
        assert_eq!(long.chars().count(), 2 + 80);
        assert!(long.ends_with('…'));
        assert_eq!(describe_tool("Read", &json!({ "file_path": "C:\\Users\\a\\file.rs" })), "📄 Читаю file.rs");
        assert_eq!(describe_tool("Edit", &json!({ "file_path": "/home/u/main.rs" })), "✏️ Правлю main.rs");
        assert_eq!(describe_tool("Write", &json!({ "file_path": "new.md" })), "📝 Пишу new.md");
        assert_eq!(describe_tool("Grep", &json!({ "pattern": "fn main" })), "🔍 Ищу fn main");
        assert_eq!(describe_tool("WebSearch", &json!({ "query": "погода" })), "🌐 Ищу в интернете: погода");
        assert_eq!(describe_tool("WebFetch", &json!({ "url": "https://docs.rs/tokio?x=1" })), "🌐 Открываю docs.rs");
        assert_eq!(describe_tool("Agent", &json!({ "description": "поиск" })), "🤖 Помощник: поиск");
        assert_eq!(describe_tool("TodoWrite", &json!({})), "📋 Обновляю план");
        assert_eq!(describe_tool("Read", &json!({})), "📄 Читаю");
        assert_eq!(describe_tool("mcp__x__y", &json!(null)), "🔧 mcp__x__y");
    }

    #[test]
    fn tags_always_balance() {
        for md in [
            "**a *b** c* `d",
            "*a **b* c**",
            "__a ~~b__ c~~",
            "[**x](https://a) y**",
            "**[a](https://b)** _c_ ~~**d**~~",
            "> **цитата\n> вторая**",
            "# *заголовок\n- __пункт",
            "***",
            "_*_*_",
        ] {
            let html = markdown_to_html(md);
            assert!(balanced(&html), "{md:?} → {html:?}");
        }
    }
}
