//! Tidy Up: the summary chat model improves a note's structure and formatting, and the app shows
//! the result for the user to apply. The rule is that nothing is ever removed. The prompt asks for
//! that, but it's enforced here: `lost` compares the result with the note and the result is refused
//! when any word, line of code, attachment, link or checkbox went missing or changed.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiResult, AppError},
    perms::Role,
    rooms::note_body,
    tree::require_note,
    util::parse_id,
};

/// The whole note goes to the model and comes back rewritten, so this bounds both ways.
pub const MAX_CHARS: usize = 20_000;

const PROMPT: &str = "Improve the structure and formatting of the Markdown note below, and reply with the whole improved note.

Never delete, shorten, reword, translate or correct anything the note says, not even typos: every word, number, name, link and emoji must still be there. Don't summarize, and don't add facts or opinions.

You may:
- add a title line with \"# \" at the top if the note has none, and add \"## \" headings or short labels to group the text into sections;
- move lines so related ones sit together under their heading;
- turn lists into \"- \" bullets, \"1. \" numbered items or \"- [ ] \" / \"- [x] \" checklist items, keeping every box ticked or unticked as it is;
- make tables of tabular text, and keep tables tidy;
- put pasted code, commands and config in fenced code blocks, naming the language after the opening ``` (one of: bash, c, cpp, csharp, css, diff, dockerfile, go, html, java, javascript, json, kotlin, lua, markdown, php, powershell, python, ruby, rust, sql, swift, toml, typescript, xml, yaml), with each line of code exactly as it was;
- bold a few key terms, and fix spacing: one blank line between blocks, no trailing spaces.

Leave existing code blocks, attachment lines like ![Photo](att:…), and links exactly as they are.

Reply with only the note, starting with its first line: no explanation, and no ``` fence around the whole note.

The note:

";

#[derive(Deserialize, Default)]
pub struct TidyBody {
    /// The text to tidy, as the app shows it; by default the note's text on the server.
    text: Option<String>,
}

/// `POST /notes/:id/tidy`: the note's text improved by the chat model, for the app to show and
/// apply. Nothing is saved here; the app writes it through the note's Loro doc. Editors only.
pub async fn tidy(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
    body: Option<Json<TidyBody>>,
) -> ApiResult<Json<Value>> {
    let id = parse_id(&id).ok_or(AppError::NotFound)?;
    require_note(&state, &me.id, &id, Role::Editor).await?;
    let Some(cfg) = state.summary.read().await.clone() else {
        return Err(AppError::Conflict("AI Summaries aren't set up on this server".into()));
    };
    let original = match body.and_then(|b| b.0.text) {
        Some(text) => text,
        None => note_body(&state, id.parse().map_err(|_| AppError::NotFound)?).await?,
    };
    if original.trim().is_empty() {
        return Err(AppError::BadRequest("There's nothing in this note to tidy yet".into()));
    }
    if original.chars().count() > MAX_CHARS {
        return Err(AppError::BadRequest(format!("This note is too long to tidy (over {} characters)", MAX_CHARS)));
    }
    let reply = crate::chat::complete(&state, &cfg, json!(format!("{PROMPT}{original}")), "AI").await?;
    let text = reply_note(&reply, &original);
    if text.trim().is_empty() {
        return Err(AppError::Conflict("The AI service sent back nothing".into()));
    }
    if let Some(what) = lost(&original, &text) {
        tracing::info!("tidy refused for note {id}: {what}");
        return Err(AppError::Conflict("Couldn't tidy this note without losing some of it".into()));
    }
    Ok(Json(json!({ "original": original, "text": text })))
}

/// The note in a model's reply: without a leading `<think>…</think>`, a ``` fence wrapped around
/// all of it, or blank lines around it; ending the way the original ends.
pub fn reply_note(reply: &str, original: &str) -> String {
    let mut text = reply.trim_start();
    if let Some(rest) = text.strip_prefix("<think>") {
        text = rest.split_once("</think>").map_or("", |(_, after)| after);
    }
    let mut text = text.trim_matches('\n').trim_end();
    // A fence around the whole reply, unless the note itself starts with code.
    if text.starts_with("```") && text.ends_with("```") && !original.trim_start().starts_with("```") {
        if let Some((_, inner)) = text.split_once('\n') {
            text = inner.strip_suffix("```").unwrap_or(inner).trim_matches('\n').trim_end();
        }
    }
    let tail = &original[original.trim_end().len()..];
    format!("{text}{tail}")
}

/// What of `before` is missing from `after`, or None when everything is still there. Reordering
/// and additions are fine; these must all survive:
/// - every word: runs of letters or digits, case-insensitive, at least as many times as before
///   (Markdown marks like `#`, `-`, `*`, `>`, `|`, `[ ]` and backticks aren't words; emoji and
///   other symbols count, as do `$ % & @ ?`);
/// - every line of code in a fenced block, as a line of code, apart from its indentation;
///   and every line of code in the result must be in the original, as a line or part of one
///   (e.g. a command taken from the end of a sentence);
/// - every attachment (`att:`) and URL;
/// - every checklist item, with its box ticked or not as before.
pub fn lost(before: &str, after: &str) -> Option<String> {
    let (b, a) = (Parsed::new(before), Parsed::new(after));
    if let Some(w) = missing(&b.words, &a.words) {
        return Some(format!("the word {w:?}"));
    }
    if let Some(l) = missing(&b.code, &a.code) {
        return Some(format!("the line of code {l:?}"));
    }
    if let Some(l) = a.code.iter().find(|l| !before.lines().any(|o| o.contains(l.as_str()))) {
        return Some(format!("new code {l:?}"));
    }
    if let Some(l) = missing(&b.links, &a.links) {
        return Some(format!("the link {l:?}"));
    }
    if let Some((done, words)) = missing(&b.tasks, &a.tasks) {
        return Some(format!("the {} checklist item {words:?}", if done { "ticked" } else { "unticked" }));
    }
    None
}

/// The first item of `need` that `have` doesn't hold as many of.
fn missing<T: std::hash::Hash + Eq + Clone>(need: &[T], have: &[T]) -> Option<T> {
    let mut count: HashMap<&T, i64> = HashMap::new();
    for h in have {
        *count.entry(h).or_default() += 1;
    }
    need.iter().find(|n| {
        let c = count.entry(n).or_default();
        *c -= 1;
        *c < 0
    }).cloned()
}

/// The parts of a note that Tidy Up must keep.
#[derive(Default)]
struct Parsed {
    words: Vec<String>,
    /// Lines inside fenced code blocks, trimmed, blank ones left out.
    code: Vec<String>,
    /// `att:` links and URLs.
    links: Vec<String>,
    /// Checklist items: ticked or not, and their words.
    tasks: Vec<(bool, String)>,
}

impl Parsed {
    fn new(text: &str) -> Parsed {
        let mut p = Parsed::default();
        let mut fence: Option<(char, usize)> = None;
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(open) = fence_of(trimmed) {
                match fence {
                    // The closing fence: the same mark, at least as long, and nothing after it.
                    Some((c, n)) if open.0 == c && open.1 >= n && trimmed.trim_start_matches(c).trim().is_empty() => {
                        fence = None;
                        continue;
                    }
                    Some(_) => {}
                    // An opening fence; its language isn't part of the text.
                    None => {
                        fence = Some(open);
                        continue;
                    }
                }
            }
            p.links.extend(links(line));
            if fence.is_some() {
                p.words.extend(words(line));
                if !trimmed.is_empty() {
                    p.code.push(trimmed.to_owned());
                }
                continue;
            }
            match task(line) {
                Some((done, rest)) => {
                    let w = words(rest);
                    p.tasks.push((done, w.join(" ")));
                    p.words.extend(w);
                }
                None => p.words.extend(words(line)),
            }
        }
        p
    }
}

/// A line that opens or closes a fenced code block: its mark (` or ~) and how many.
fn fence_of(trimmed: &str) -> Option<(char, usize)> {
    let c = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = trimmed.chars().take_while(|x| *x == c).count();
    (n >= 3).then_some((c, n))
}

/// A checklist item: `- [ ] text`, `* [x] text`, `1. [X] text`, or a bare `[ ] text`.
/// Whether it's ticked, and the text after the box.
fn task(line: &str) -> Option<(bool, &str)> {
    let mut rest = line.trim_start();
    if let Some(r) = rest.strip_prefix(['-', '*', '+']) {
        rest = r.trim_start();
    } else {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 {
            if let Some(r) = rest[digits..].strip_prefix(['.', ')']) {
                rest = r.trim_start();
            }
        }
    }
    let done = match rest.get(..3)? {
        "[ ]" => false,
        "[x]" | "[X]" => true,
        _ => return None,
    };
    let after = &rest[3..];
    (after.is_empty() || after.starts_with(char::is_whitespace)).then_some((done, after))
}

/// Symbols that are part of what the text says rather than Markdown.
fn is_symbol(c: char) -> bool {
    matches!(c, '$' | '%' | '&' | '@' | '?')
        // Arrows, math, technical and other symbols, dingbats and emoji; not the variation
        // selectors and joiners that only change how an emoji looks.
        || (c as u32 >= 0x2190 && !c.is_whitespace() && !matches!(c as u32, 0xFE00..=0xFE0F | 0x200D))
}

/// The words of a line: runs of letters and digits, lowercased, and symbols one by one.
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    for c in line.chars() {
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
            continue;
        }
        if !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
        if is_symbol(c) {
            out.push(c.to_string());
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// `att:` links and http(s) URLs in a line, without the punctuation that ends a sentence or a
/// Markdown link around them.
fn links(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, _) in line.match_indices(['a', 'h']) {
        let rest = &line[i..];
        if !(rest.starts_with("att:") || rest.starts_with("http://") || rest.starts_with("https://")) {
            continue;
        }
        // Only at the start of a link, not inside another word.
        if line[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric()) {
            continue;
        }
        let end = rest.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`' | ')' | ']')).unwrap_or(rest.len());
        let link = rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', '*', '_']);
        if link.len() > 4 {
            out.push(link.to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{lost, reply_note, task, words};

    const NOTE: &str = "Groceries\nmilk, eggs & 2 loaves\n[x] call mom\n[ ] pay rent $1200\nsee https://example.com/a?b=1.\n![Photo](att:01a0d056-667e-733b-a280-1d118b647773)\n```python\n  def f(x):\n      return x\n```\nfun 🎉";

    #[test]
    fn words_are_letter_and_digit_runs_and_symbols() {
        assert_eq!(words("## Buy *Milk*, 2 eggs!"), ["buy", "milk", "2", "eggs"]);
        assert_eq!(words("| a | b |\n> - [ ] `c`"), ["a", "b", "c"]);
        assert_eq!(words("Café 5% → 🎉\u{FE0F}"), ["café", "5", "%", "→", "🎉"]);
        assert_eq!(task("- [x] Done it"), Some((true, " Done it")));
        assert_eq!(task("1. [ ] next"), Some((false, " next")));
        assert_eq!(task("[ ]"), Some((false, "")));
        assert_eq!(task("- [link](x)"), None);
    }

    #[test]
    fn formatting_reordering_and_additions_pass() {
        let tidy = "# Groceries\n\n## Shopping\n\n- **Milk**, eggs & 2 loaves\n\n## To do\n\n- [ ] Pay rent $1200\n- [X] Call Mom\n\nSee https://example.com/a?b=1.\n\n![Photo](att:01a0d056-667e-733b-a280-1d118b647773)\n\n```py\ndef f(x):\n    return x\n```\n\nFun 🎉";
        assert_eq!(lost(NOTE, tidy), None);
        assert_eq!(lost(NOTE, NOTE), None);
        // Plain text that's code may go into a new fence, line for line.
        assert_eq!(lost("Run this:\n  cargo test -p x\nthen", "Run this:\n\n```bash\ncargo test -p x\n```\n\nthen"), None);
        // Or part of a line, when the rest of it stays as text.
        assert_eq!(lost("undo it: git reset --soft HEAD~1", "Undo it:\n\n```bash\ngit reset --soft HEAD~1\n```"), None);
        // Numbers count as words even as list markers.
        assert_eq!(lost("1. a\n2. b", "## Steps\n\n1. a\n2. b"), None);
    }

    #[test]
    fn anything_lost_or_changed_is_refused() {
        let refused = |after: &str| assert!(lost(NOTE, after).is_some(), "should refuse:\n{after}");
        // A word dropped, changed or corrected, a number dropped, a symbol or emoji dropped.
        refused(&NOTE.replace("eggs ", ""));
        refused(&NOTE.replace("mom", "Mum"));
        refused(&NOTE.replace("2 loaves", "two loaves"));
        refused(&NOTE.replace("$1200", "1200"));
        refused(&NOTE.replace(" 🎉", ""));
        // Code changed inside a fence, or a fenced line moved out of code.
        refused(&NOTE.replace("return x", "return x + 1"));
        refused(&NOTE.replace("```python\n", "").replace("\n```", ""));
        // A link or attachment changed.
        refused(&NOTE.replace("a?b=1", "a?b=2"));
        refused(&NOTE.replace("att:01a0d056", "att:01a0d057"));
        // A box ticked or unticked.
        refused(&NOTE.replace("[x] call", "[ ] call"));
        refused(&NOTE.replace("[ ] pay", "[x] pay"));
        // New code that wasn't in the note.
        assert!(lost("hello", "hello\n\n```\nprint('hello')\n```").is_some());
        assert!(lost("x = 1 + 2", "```python\nx = 1 +  2\n```").is_some());
    }

    #[test]
    fn replies_lose_thinking_and_wrapping_fences() {
        assert_eq!(reply_note("<think>hmm</think>\n\n# A\n\nb\n", "A\nb"), "# A\n\nb");
        assert_eq!(reply_note("```markdown\n# A\n- b\n```", "A\nb\n"), "# A\n- b\n");
        // A note that starts with code keeps it.
        assert_eq!(reply_note("```sh\nls\n```", "```sh\nls\n```"), "```sh\nls\n```");
    }
}
