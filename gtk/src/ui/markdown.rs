//! Finds the Markdown structure the editor styles in place: headings, emphasis, code, quotes, lists,
//! checkboxes, links and photos. Offsets are in characters, like text iters.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub tag: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub start: usize,
    pub end: usize,
    pub url: String,
}

#[derive(Clone, Debug, Default)]
pub struct Styled {
    pub spans: Vec<Span>,
    /// Where each checkbox's `[` is, and whether it's ticked.
    pub tasks: Vec<(usize, bool)>,
    pub links: Vec<Link>,
    /// Photos: the whole `![alt](url)` and its url.
    pub images: Vec<Link>,
}

/// Every tag `analyze` uses, for the editor's tag table.
pub const TAGS: &[&str] =
    &["h1", "h2", "h3", "bold", "italic", "strike", "code", "codeblock", "quote", "mark", "link", "task", "done", "bullet", "table"];

/// Byte offset to character offset, for every byte position in `text`.
fn char_index(text: &str) -> Vec<usize> {
    let mut map = vec![0; text.len() + 1];
    let mut chars = 0;
    for (i, c) in text.char_indices() {
        for b in 0..c.len_utf8() {
            map[i + b] = chars;
        }
        chars += 1;
    }
    map[text.len()] = chars;
    map
}

pub fn analyze(text: &str) -> Styled {
    let ci = char_index(text);
    let mut out = Styled::default();
    let span = |out: &mut Styled, s: usize, e: usize, tag: &'static str| {
        if e > s {
            out.spans.push(Span { start: ci[s], end: ci[e], tag });
        }
    };
    let bytes = text.as_bytes();
    // Open links: where they start, their url, and where their text ends so far.
    let mut links: Vec<(usize, String, usize)> = Vec::new();
    // Open list items, for ticking a whole item when its box is.
    let mut items: Vec<std::ops::Range<usize>> = Vec::new();

    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES;
    for (event, r) in Parser::new_ext(text, opts).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let tag = match level {
                    HeadingLevel::H1 => "h1",
                    HeadingLevel::H2 => "h2",
                    _ => "h3",
                };
                let end = text[..r.end].trim_end_matches(['\n', '\r']).len().max(r.start);
                span(&mut out, r.start, end, tag);
                let hashes = bytes[r.start..r.end].iter().take_while(|b| **b == b'#').count();
                if hashes > 0 {
                    let mark_end = (r.start + hashes + 1).min(r.end);
                    span(&mut out, r.start, mark_end, "mark");
                }
            }
            Event::Start(Tag::Strong) => {
                span(&mut out, r.start, r.end, "bold");
                span(&mut out, r.start, r.start + 2, "mark");
                span(&mut out, r.end.saturating_sub(2), r.end, "mark");
            }
            Event::Start(Tag::Emphasis) => {
                span(&mut out, r.start, r.end, "italic");
                span(&mut out, r.start, r.start + 1, "mark");
                span(&mut out, r.end - 1, r.end, "mark");
            }
            Event::Start(Tag::Strikethrough) => {
                let n = bytes[r.start..r.end].iter().take_while(|b| **b == b'~').count().max(1);
                span(&mut out, r.start, r.end, "strike");
                span(&mut out, r.start, r.start + n, "mark");
                span(&mut out, r.end - n, r.end, "mark");
            }
            Event::Code(_) => {
                if let Some(l) = links.last_mut() {
                    l.2 = l.2.max(r.end);
                }
                let n = bytes[r.start..r.end].iter().take_while(|b| **b == b'`').count();
                span(&mut out, r.start, r.end, "code");
                span(&mut out, r.start, r.start + n, "mark");
                span(&mut out, r.end - n, r.end, "mark");
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                span(&mut out, r.start, r.end, "codeblock");
                if let CodeBlockKind::Fenced(_) = kind {
                    let slice = &text[r.start..r.end];
                    let first = slice.find('\n').map_or(r.end, |i| r.start + i);
                    span(&mut out, r.start, first, "mark");
                    let body = slice.trim_end_matches('\n');
                    if let Some(i) = body.rfind('\n') {
                        let last = &body[i + 1..];
                        if last.trim_start().starts_with("```") || last.trim_start().starts_with("~~~") {
                            span(&mut out, r.start + i + 1, r.start + body.len(), "mark");
                        }
                    }
                }
            }
            Event::Start(Tag::BlockQuote(_)) => {
                span(&mut out, r.start, r.end, "quote");
                let mut at = r.start;
                for line in text[r.start..r.end].split_inclusive('\n') {
                    let indent = line.len() - line.trim_start().len();
                    if line.trim_start().starts_with('>') {
                        span(&mut out, at + indent, at + indent + 1, "mark");
                    }
                    at += line.len();
                }
            }
            Event::Start(Tag::Item) => {
                items.push(r.clone());
                let line = &text[r.start..r.end];
                let n = line.find([' ', '\n']).unwrap_or(line.len());
                span(&mut out, r.start, r.start + n, "bullet");
            }
            Event::End(TagEnd::Item) => {
                items.pop();
            }
            Event::TaskListMarker(checked) => {
                out.tasks.push((ci[r.start], checked));
                span(&mut out, r.start, r.end, "task");
                if checked && let Some(item) = items.last() {
                    let start = r.end + (text[r.end..].len() - text[r.end..].trim_start_matches(' ').len());
                    let end = text[start..item.end].find('\n').map_or(item.end, |i| start + i);
                    span(&mut out, start, end, "done");
                }
            }
            Event::Start(Tag::Link { dest_url, .. }) => links.push((r.start, dest_url.to_string(), r.start + 1)),
            Event::Text(_) if !links.is_empty() => {
                if let Some(l) = links.last_mut() {
                    l.2 = l.2.max(r.end);
                }
            }
            Event::End(TagEnd::Link) => {
                if let Some((start, url, text_end)) = links.pop() {
                    // `<https://…>` autolinks have no brackets to dim.
                    if bytes[start] == b'[' {
                        span(&mut out, start, start + 1, "mark");
                        span(&mut out, text_end, r.end, "mark");
                    }
                    span(&mut out, start, r.end, "link");
                    out.links.push(Link { start: ci[start], end: ci[r.end], url });
                }
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                span(&mut out, r.start, r.end, "mark");
                out.images.push(Link { start: ci[r.start], end: ci[r.end], url: dest_url.to_string() });
            }
            Event::Start(Tag::Table(_)) => span(&mut out, r.start, r.end, "table"),
            Event::Rule => span(&mut out, r.start, r.end, "mark"),
            _ => {}
        }
    }
    // Bare web addresses, which CommonMark leaves as text.
    for (i, _) in text.match_indices("http") {
        let rest = &text[i..];
        if !(rest.starts_with("https://") || rest.starts_with("http://")) {
            continue;
        }
        if i > 0 && matches!(bytes[i - 1], b'(' | b'<' | b'[') {
            continue;
        }
        let len = rest.find(|c: char| c.is_whitespace() || c == ')' || c == ']' || c == '>').unwrap_or(rest.len());
        let url = rest[..len].trim_end_matches(['.', ',', ';', ':', '!', '?']);
        let (s, e) = (ci[i], ci[i + url.len()]);
        if out.links.iter().any(|l| l.start <= s && s < l.end) {
            continue;
        }
        out.spans.push(Span { start: s, end: e, tag: "link" });
        out.links.push(Link { start: s, end: e, url: url.to_owned() });
    }
    // [[Wiki links]] to other notes, by title.
    let mut from = 0;
    while let Some(open) = text[from..].find("[[").map(|i| from + i) {
        let Some(close) = text[open + 2..].find("]]").map(|i| open + 2 + i) else { break };
        let title = &text[open + 2..close];
        if title.is_empty() || title.contains('\n') || title.contains("[[") {
            from = open + 2;
            continue;
        }
        out.spans.push(Span { start: ci[open], end: ci[open + 2], tag: "mark" });
        out.spans.push(Span { start: ci[open + 2], end: ci[close], tag: "link" });
        out.spans.push(Span { start: ci[close], end: ci[close + 2], tag: "mark" });
        out.links.push(Link { start: ci[open], end: ci[close + 2], url: format!("wiki:{title}") });
        from = close + 2;
    }
    out
}

/// A Markdown block marker at the start of a line: heading, checkbox, bullet, number or quote.
pub fn line_prefix(line: &str) -> &str {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let rest = &line[indent..];
    let hashes = rest.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&hashes) && rest[hashes..].starts_with(' ') {
        return &line[..indent + hashes + 1];
    }
    for p in ["- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "* [x] ", "- ", "* ", "+ ", "> "] {
        if rest.starts_with(p) {
            return &line[..indent + p.len()];
        }
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && (rest[digits..].starts_with(". ") || rest[digits..].starts_with(") ")) {
        return &line[..indent + digits + 2];
    }
    &line[..0]
}

/// What Enter starts the next line with after `line`: the same kind of list item, unticked, and the
/// next number. None when the line isn't a list item or quote.
pub fn continue_prefix(line: &str) -> Option<String> {
    let prefix = line_prefix(line);
    let trimmed = prefix.trim_start();
    if prefix.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let indent = &prefix[..prefix.len() - trimmed.len()];
    if trimmed.contains("[x]") || trimmed.contains("[X]") || trimmed.contains("[ ]") {
        return Some(format!("{indent}{}", &trimmed[..2]) + "[ ] ");
    }
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 {
        let n: u64 = trimmed[..digits].parse().unwrap_or(0);
        return Some(format!("{indent}{}{}", n + 1, &trimmed[digits..]));
    }
    Some(prefix.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(text: &str) -> Vec<(String, &'static str)> {
        let chars: Vec<char> = text.chars().collect();
        analyze(text).spans.iter().map(|s| (chars[s.start..s.end].iter().collect(), s.tag)).collect()
    }

    #[test]
    fn styles_headings_and_emphasis() {
        let t = tags("# Title\n\nSome **bold** and *it* `code` 🍎 ~~gone~~");
        assert!(t.contains(&("# Title".into(), "h1")));
        assert!(t.contains(&("# ".into(), "mark")));
        assert!(t.contains(&("**bold**".into(), "bold")));
        assert!(t.contains(&("*it*".into(), "italic")));
        assert!(t.contains(&("`code`".into(), "code")));
        assert!(t.contains(&("~~gone~~".into(), "strike")));
    }

    #[test]
    fn finds_tasks_links_and_photos() {
        let text = "- [x] milk\n- [ ] eggs\n\nSee [docs](https://example.com) and https://gnome.org.\n![p](att:123) [[Pantry]]";
        let s = analyze(text);
        assert_eq!(s.tasks, vec![(2, true), (13, false)]);
        let urls: Vec<&str> = s.links.iter().map(|l| l.url.as_str()).collect();
        assert_eq!(urls, vec!["https://example.com", "https://gnome.org", "wiki:Pantry"]);
        assert_eq!(s.images[0].url, "att:123");
        assert!(tags(text).contains(&("milk".into(), "done")));
    }

    #[test]
    fn continues_lists() {
        assert_eq!(continue_prefix("- [x] milk").as_deref(), Some("- [ ] "));
        assert_eq!(continue_prefix("  9. nine").as_deref(), Some("  10. "));
        assert_eq!(continue_prefix("> said").as_deref(), Some("> "));
        assert_eq!(continue_prefix("## Heading"), None);
        assert_eq!(continue_prefix("plain"), None);
        assert_eq!(line_prefix("### Sub"), "### ");
    }
}
