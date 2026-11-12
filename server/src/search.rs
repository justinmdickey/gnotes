//! Full-text search over note bodies with SQLite FTS5 (migration 0008).
//! Each note's text, with Markdown marks stripped, is kept in `note_text` on every edit;
//! results only include notes the caller can see right now, the same set /tree shows.

use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AppState, auth::CurrentUser, error::ApiResult, rooms::note_body};

/// Enough for any list someone scrolls; the app narrows these further to the folder being searched.
const MAX_RESULTS: i64 = 200;
/// Words past this many are ignored, so a pasted paragraph can't make a huge query.
const MAX_TERMS: usize = 16;
/// Snippet markers around matched words. Control characters, so they never occur in indexed text.
const HIT_START: char = '\u{2}';
const HIT_END: char = '\u{3}';

/// `![label](att:…)` becomes its label and `[name](att:…)` its name (a file link's text is its
/// file name), e.g. "manual.pdf", wherever it sits in the line, so a file's name is searchable.
fn embeds_as_labels(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(at) = rest.find("](att:") {
        let before = &rest[..at];
        let Some(bracket) = before.rfind('[') else { break };
        let label = &before[bracket + 1..];
        let tail = &rest[at + 6..];
        let Some(close) = tail.find(')') else { break };
        // Drop a leading `!` (an embed) so `![label](att:…)` and `[name](att:…)` both become the text.
        let up_to = bracket.checked_sub(1).filter(|&i| before.get(i..i + 1) == Some("!")).unwrap_or(bracket);
        out.push_str(&rest[..up_to]);
        out.push_str(label);
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    out
}

/// The note as search reads it: no block marks, emphasis, code fences or attachment links,
/// so snippets read like the text. Words are unchanged, so everything typed is still found.
pub fn plain_text(body: &str) -> String {
    let mut lines = Vec::new();
    for line in body.lines() {
        let mut l = line.trim();
        if l.starts_with("```") {
            continue;
        }
        l = l.trim_start_matches('#').trim_start();
        l = l.trim_start_matches('>').trim_start();
        for marker in ["- [ ] ", "- [x] ", "- [X] ", "* ", "- ", "+ "] {
            if let Some(rest) = l.strip_prefix(marker) {
                l = rest;
                break;
            }
        }
        let l: String = crate::tasks::without_dates(&embeds_as_labels(l))
            .replace("**", "")
            .replace("~~", "")
            .replace('`', "")
            .replace("[[", "")
            .replace("]]", "")
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        if !l.trim().is_empty() {
            lines.push(l.trim().to_owned());
        }
    }
    lines.join("\n")
}

/// Turns what someone typed into an FTS5 query that can't fail: every word quoted, as a prefix,
/// all required. Quotes, `*`, `-`, `OR` and the like are just text. `None` when there are no words.
pub fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .take(MAX_TERMS)
        .map(|t| format!("\"{t}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// Words that say nothing about which note is meant: question words, fillers, and talk about notes
/// themselves ("where is that note about…"). Left out of `question_terms`.
const STOP_WORDS: &[&str] = &[
    "a", "about", "again", "all", "also", "am", "an", "and", "any", "are", "as", "at", "be", "been", "but", "by", "can",
    "could", "did", "do", "does", "doing", "find", "for", "from", "get", "go", "going", "got", "had", "has", "have", "how",
    "i", "if", "in", "into", "is", "it", "its", "just", "know", "let", "me", "much", "my", "note", "notes", "of", "on",
    "or", "our", "please", "put", "remember", "s", "said", "say", "show", "should", "so", "some", "stuff", "tell",
    "than", "that", "the", "their", "them", "then", "there", "these", "they", "thing", "things", "this", "those", "to",
    "too", "was", "we", "were", "what", "when", "where", "which", "who", "whom", "why", "will", "with", "would",
    "wrote", "you", "your",
];

/// The words of a question worth looking for, lowercased, without stop words or single letters.
pub fn question_terms(input: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in input.split(|c: char| !c.is_alphanumeric()).map(str::to_lowercase) {
        if t.chars().count() > 1 && !STOP_WORDS.contains(&t.as_str()) && !out.contains(&t) {
            out.push(t);
        }
    }
    out.truncate(MAX_TERMS);
    out
}

/// An FTS5 query matching notes with any of `terms` (as prefixes), for questions, where requiring
/// every word would find nothing. `None` when there are no terms.
pub fn fts_any(terms: &[String]) -> Option<String> {
    (!terms.is_empty()).then(|| terms.iter().map(|t| format!("\"{t}\"*")).collect::<Vec<_>>().join(" OR "))
}

/// Stores a note's current text for search, including the text of the PDFs it links (migration
/// 0015). Called on every edit and for imported notes; the title is kept apart so snippets show
/// the rest of the note, not the title again.
pub async fn index(state: &AppState, note: &str, body: &str) -> sqlx::Result<()> {
    let own = plain_text(body);
    let (title, own_rest) = own.split_once('\n').unwrap_or((&own, ""));
    let refs = att_refs(body);
    // The text of the linked PDFs, appended after the note's own text, so FTS, embeddings and
    // Ask read it with no changes of their own.
    let texts: Vec<(String, String)> = if refs.is_empty() {
        vec![]
    } else {
        sqlx::query_as::<_, (String, String)>(
            "SELECT a.filename, t.text FROM attachment_text t
             JOIN attachments a ON a.id = t.attachment_id
             WHERE a.id IN (SELECT value FROM json_each(?))",
        )
        .bind(serde_json::to_string(&refs).unwrap_or_default())
        .fetch_all(&state.db)
        .await?
    };
    let mut rest = own_rest.to_owned();
    let mut spans: Vec<(String, i64, i64)> = Vec::new();
    for (filename, text) in &texts {
        rest.push('\n');
        let start = rest.len() as i64;
        rest.push_str(text);
        let end = rest.len() as i64;
        spans.push((filename.clone(), start, end));
    }
    sqlx::query(
        "INSERT INTO note_text (note_id, title, body, pdf_spans, att_refs) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (note_id) DO UPDATE SET title = excluded.title, body = excluded.body,
             pdf_spans = excluded.pdf_spans, att_refs = excluded.att_refs
         WHERE note_text.title != excluded.title OR note_text.body != excluded.body
            OR note_text.pdf_spans IS NOT excluded.pdf_spans OR note_text.att_refs IS NOT excluded.att_refs",
    )
    .bind(note)
    .bind(title)
    .bind(&rest)
    .bind(serde_json::to_string(&spans).unwrap_or_default())
    .bind(serde_json::to_string(&refs).unwrap_or_default())
    .execute(&state.db)
    .await?;
    Ok(())
}

/// The attachment ids a note's body links to (`att:…`), for PDF text and re-indexing.
fn att_refs(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for m in body.match_indices("att:") {
        let rest = &body[m.0 + 4..];
        let id: String = rest.chars().take_while(|c| c.is_ascii_hexdigit() || *c == '-').collect();
        if id.len() == 36 && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// Indexes notes that have no search text yet: everything written before search existed,
/// and notes made but never edited. Runs once at startup.
pub async fn backfill(state: &AppState) -> anyhow::Result<()> {
    let missing: Vec<String> = sqlx::query_scalar("SELECT id FROM notes WHERE id NOT IN (SELECT note_id FROM note_text)")
        .fetch_all(&state.db)
        .await?;
    for id in &missing {
        let body = match Uuid::parse_str(id) {
            Ok(uuid) => note_body(state, uuid).await,
            Err(e) => Err(e.into()),
        };
        match body {
            Ok(body) => index(state, id, &body).await?,
            Err(e) => tracing::error!("indexing note {id} for search: {e:#}"),
        }
    }
    if !missing.is_empty() {
        tracing::info!("indexed {} notes for search", missing.len());
    }
    Ok(())
}

/// Notes the user bound as `?1` can see, as in /tree: their own, shared with them directly, or inside
/// a notebook shared with them. A `WITH` clause defining `visible(id)`; callers still drop trashed notes.
macro_rules! visible_notes {
    () => {
        "WITH RECURSIVE shared_nb(id) AS (
            SELECT s.resource_id FROM shares s JOIN notebooks nb ON nb.id = s.resource_id
            WHERE s.user_id = ?1 AND s.resource_type = 'notebook' AND nb.deleted_at IS NULL
            UNION
            SELECT nb.id FROM notebooks nb JOIN shared_nb ON nb.parent_id = shared_nb.id
            WHERE nb.deleted_at IS NULL
         ),
         visible(id) AS (
            SELECT id FROM notes WHERE owner_id = ?1
            UNION SELECT resource_id FROM shares WHERE user_id = ?1 AND resource_type = 'note'
            UNION SELECT id FROM notes WHERE notebook_id IN (SELECT id FROM shared_nb)
         ) "
    };
}
pub(crate) use visible_notes;

#[derive(Deserialize)]
pub struct SearchParams {
    pub q: String,
}

/// A piece of a snippet; `hit` pieces are the words that matched.
#[derive(Serialize)]
pub struct Segment {
    pub text: String,
    pub hit: bool,
}

#[derive(Serialize)]
pub struct SearchResult {
    pub note: String,
    pub title: String,
    pub snippet: Vec<Segment>,
    /// Which search found it: "text" here, "meaning" from semantic search (semantic.rs).
    pub source: &'static str,
    /// The attached PDF the snippet came from, when the match is in its text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

#[derive(Serialize)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
}

fn segments(snippet: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    for (i, part) in snippet.split(HIT_START).enumerate() {
        let (hit, rest) = if i == 0 { ("", part) } else { part.split_once(HIT_END).unwrap_or((part, "")) };
        if !hit.is_empty() {
            out.push(Segment { text: hit.to_owned(), hit: true });
        }
        if !rest.is_empty() {
            out.push(Segment { text: rest.replace('\n', " "), hit: false });
        }
    }
    out
}

/// `GET /search?q=` — notes whose text contains every word typed (as prefixes, so it works
/// while typing), best match first with title matches weighed up, each with a snippet of the
/// text after the title around the match.
pub async fn search(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<SearchResults>> {
    Ok(Json(SearchResults { results: by_words(&state, &me.id, &params.q, MAX_RESULTS).await? }))
}

/// Notes the user can see with every word of `q`, best first, at most `limit`.
pub(crate) async fn by_words(state: &AppState, user: &str, q: &str, limit: i64) -> sqlx::Result<Vec<SearchResult>> {
    let Some(query) = fts_query(q) else { return Ok(vec![]) };
    let terms = query_terms(q);
    let rows: Vec<(String, String, String, String, Option<String>)> = sqlx::query_as(concat!(
        visible_notes!(),
        "SELECT n.id, n.title, snippet(note_search, 1, char(2), char(3), '…', 12), t.body, t.pdf_spans
         FROM note_search
         JOIN note_text t ON t.id = note_search.rowid
         JOIN notes n ON n.id = t.note_id
         WHERE note_search MATCH ?2 AND n.deleted_at IS NULL AND n.id IN (SELECT id FROM visible)
         ORDER BY bm25(note_search, 4.0, 1.0)
         LIMIT ?3"
    ))
    .bind(user)
    .bind(&query)
    .bind(limit)
    .fetch_all(&state.db)
    .await?;
    Ok(rows.into_iter().map(|(note, title, snippet, body, pdf_spans)| {
        let spans = parse_spans(&pdf_spans);
        let file = attribute_file(&body, &spans, &terms);
        SearchResult { note, title, snippet: segments(&snippet), source: "text", file }
    }).collect())
}

/// Notes the user can see whose text has any of `terms`, best first, at most `limit`, each with
/// a snippet around the words found. For Ask (semantic.rs), which mixes these with meaning matches.
pub async fn any_terms(state: &AppState, user: &str, terms: &[String], limit: i64) -> sqlx::Result<Vec<SearchResult>> {
    let Some(query) = fts_any(terms) else { return Ok(vec![]) };
    let rows: Vec<(String, String, String, String, Option<String>)> = sqlx::query_as(concat!(
        visible_notes!(),
        "SELECT n.id, n.title, snippet(note_search, 1, char(2), char(3), '…', 16), t.body, t.pdf_spans
         FROM note_search
         JOIN note_text t ON t.id = note_search.rowid
         JOIN notes n ON n.id = t.note_id
         WHERE note_search MATCH ?2 AND n.deleted_at IS NULL AND n.id IN (SELECT id FROM visible)
         ORDER BY bm25(note_search, 4.0, 1.0)
         LIMIT ?3"
    ))
    .bind(user)
    .bind(&query)
    .bind(limit)
    .fetch_all(&state.db)
    .await?;
    let lower: Vec<String> = terms.iter().map(|t| t.to_lowercase()).collect();
    Ok(rows.into_iter().map(|(note, title, snippet, body, pdf_spans)| {
        let spans = parse_spans(&pdf_spans);
        let file = attribute_file(&body, &spans, &lower);
        SearchResult { note, title, snippet: segments(&snippet), source: "text", file }
    }).collect())
}

/// The words of a search, lowercased, for attributing a snippet to a linked PDF.
fn query_terms(q: &str) -> Vec<String> {
    q.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).take(MAX_TERMS).map(|t| t.to_lowercase()).collect()
}

/// The (filename, start, end) spans a note's search text carries for its linked PDFs.
fn parse_spans(json: &Option<String>) -> Vec<(String, i64, i64)> {
    json.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default()
}

/// The attached PDF a match came from: a word that's in a linked PDF's text but not in the note's
/// own text. `None` when the match is in the note's own text, or there are no linked PDFs.
fn attribute_file(rest: &str, spans: &[(String, i64, i64)], terms: &[String]) -> Option<String> {
    if spans.is_empty() {
        return None;
    }
    let own_end = spans.first().map(|s| s.1).unwrap_or(rest.len() as i64) as usize;
    let own = rest.get(..own_end.min(rest.len())).unwrap_or("");
    let own_lower = own.to_lowercase();
    for t in terms {
        if t.is_empty() || own_lower.contains(t) {
            continue; // in the note's own text, so not PDF-specific
        }
        for (name, start, end) in spans {
            let (s, e) = (*start as usize, *end as usize);
            if s <= e && e <= rest.len() && rest[s..e].to_lowercase().contains(t) {
                return Some(name.clone());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{fts_query, plain_text, segments};

    #[test]
    fn queries_are_quoted_prefixes() {
        assert_eq!(fts_query("milk"), Some("\"milk\"*".into()));
        assert_eq!(fts_query("  \"milk\" OR -eggs*  "), Some("\"milk\"* \"OR\"* \"eggs\"*".into()));
        assert_eq!(fts_query("café au-lait"), Some("\"café\"* \"au\"* \"lait\"*".into()));
        assert_eq!(fts_query(" \"*()-: "), None);
    }

    #[test]
    fn plain_text_drops_marks() {
        let body = "# Groceries\n\n- [ ] **milk**\n```bash\nnpm run build\n```\nsee ![Voice memo](att:01a0d056-667e-733b-a280-1d118b647773) here";
        assert_eq!(plain_text(body), "Groceries\nmilk\nnpm run build\nsee Voice memo here");
        // Wiki links keep their words, lose the brackets.
        assert_eq!(plain_text("See [[Groceries]] for the list"), "See Groceries for the list");
        // A task's due date reads as nothing; Tasks shows it.
        assert_eq!(plain_text("- [ ] Call the plumber @2026-10-20"), "Call the plumber");
    }

    #[test]
    fn snippets_split_into_hits() {
        let parts = segments("…buy \u{2}milk\u{3} and\neggs");
        let flat: Vec<(&str, bool)> = parts.iter().map(|s| (s.text.as_str(), s.hit)).collect();
        assert_eq!(flat, vec![("…buy ", false), ("milk", true), (" and eggs", false)]);
    }
}
