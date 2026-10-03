//! Semantic search and Ask, through an admin's OpenAI-compatible embeddings service (`/embeddings`).
//! See "AI search" in docs/DESIGN.md.
//!
//! Each note's search text (`note_text`, kept by search.rs) is cut into chunks of a few paragraphs,
//! and each chunk's embedding is kept in `note_chunks` (migration 0009) with a hash of the text it
//! covers. One background task keeps them current: edits mark their note, and once typing has settled
//! the task embeds only the chunks whose text changed. At startup, when the service is set up, and
//! hourly it checks every note, which fills in notes from before and retries anything that failed.
//! Editing never waits on it.
//!
//! Searching embeds the query and compares it with every chunk of the notes the user can see
//! (brute force, which is plenty for a household). Ask sends the best few of those chunks, and
//! nothing else, to the chat model that writes summaries.

use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Notify;

use crate::{
    AppState, ChatConfig,
    auth::CurrentUser,
    error::{ApiResult, AppError},
    search::{SearchParams, SearchResult, SearchResults, Segment, visible_notes},
};

/// How long typing has to pause before a note is re-embedded.
const SETTLE: Duration = Duration::from_secs(3);
/// Someone typing for minutes on end still gets their note embedded this often.
const MAX_WAIT: Duration = Duration::from_secs(30);
/// After the service fails, the first retry waits this long, doubling up to an hour.
const RETRY: Duration = Duration::from_secs(60);
/// Roughly a few paragraphs: short enough that a chunk is about one thing, long enough to carry context.
const CHUNK_CHARS: usize = 1000;
/// Chunks per `/embeddings` request. One request at a time, so a small local server isn't swamped.
const BATCH: usize = 16;
/// Cosine similarity a chunk needs to count as related at all.
const MIN_SCORE: f32 = 0.3;
/// Results also have to be about as close as the best one; embedding models differ in how far apart
/// unrelated text lands, so a fixed cutoff alone lets in noise from some models.
const SPREAD: f32 = 0.15;
/// Notes "By meaning" shows at most.
const MAX_RESULTS: usize = 10;
/// Chunks Ask sends to the chat model at most, and how much text that may be.
const ASK_CHUNKS: usize = 6;
const ASK_CHARS: usize = 8000;
/// The model replies with exactly this when the notes don't answer the question.
const NO_ANSWER: &str = "NO_ANSWER";

const ASK_PROMPT: &str = "Answer the question using only the notes below. They are excerpts from the user's own notes, numbered.
- Cite the notes you use by number in square brackets, like [1] or [1][2], right after what they support.
- If the notes don't contain the answer, reply with exactly NO_ANSWER and nothing else. Don't guess and don't use outside knowledge.
- Keep it short: a few sentences, or a short list with \"- \" items. Plain text, no headings. Reply in the question's language.
";

/// What the background task still has to look at.
struct Dirty {
    /// Check every note, not just the ones listed.
    all: bool,
    notes: HashSet<String>,
}

pub struct Embedder {
    wake: Notify,
    dirty: Mutex<Dirty>,
}

impl Default for Embedder {
    /// Starts by checking every note, which backfills anything not embedded yet.
    fn default() -> Self {
        Self { wake: Notify::new(), dirty: Mutex::new(Dirty { all: true, notes: HashSet::new() }) }
    }
}

impl Embedder {
    /// A note's text changed; re-embed it once typing settles. Cheap; call it on every edit.
    pub fn changed(&self, note: &str) {
        self.dirty.lock().unwrap().notes.insert(note.to_owned());
        self.wake.notify_one();
    }

    /// Check every note soon: after imports, when the service changes, and hourly.
    pub fn refresh_all(&self) {
        self.dirty.lock().unwrap().all = true;
        self.wake.notify_one();
    }

    fn take(&self) -> Dirty {
        let mut dirty = self.dirty.lock().unwrap();
        Dirty { all: std::mem::take(&mut dirty.all), notes: std::mem::take(&mut dirty.notes) }
    }

    /// Waits for a change, then for typing to pause. `retry` also ends the wait, after a failure.
    async fn settled(&self, retry: Option<Duration>) {
        match retry {
            Some(after) => {
                if tokio::time::timeout(after, self.wake.notified()).await.is_err() {
                    return;
                }
            }
            None => self.wake.notified().await,
        }
        let start = Instant::now();
        while let Some(left) = MAX_WAIT.checked_sub(start.elapsed()) {
            if tokio::time::timeout(SETTLE.min(left), self.wake.notified()).await.is_err() {
                break;
            }
        }
    }
}

/// The embeddings service failed (down, refused, or answered nonsense), as opposed to a problem
/// with one note. The whole run stops and is retried later.
#[derive(Debug)]
struct ServiceError(String);

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ServiceError {}

/// Runs the embedding task: once at startup, then whenever notes change.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut retry: Option<Duration> = None;
        loop {
            let work = state.embedder.take();
            let cfg = state.embed.read().await.clone();
            // Off: nothing to do. Turning it on checks every note.
            if let Some(cfg) = cfg {
                match update(&state, &cfg, &work).await {
                    Ok(0) => retry = None,
                    Ok(n) => {
                        retry = None;
                        tracing::info!("embedded {n} notes for semantic search");
                    }
                    Err(e) => {
                        let after = retry.map_or(RETRY, |r| (r * 2).min(Duration::from_secs(60 * 60)));
                        tracing::warn!("semantic search embeddings failed, trying again in {}s: {e:#}", after.as_secs());
                        // Whatever was left over is picked up by checking every note next time.
                        state.embedder.dirty.lock().unwrap().all = true;
                        retry = Some(after);
                    }
                }
            }
            state.embedder.settled(retry).await;
        }
    });
}

/// Brings the chunks of the notes in `work` up to date. Returns how many notes were re-embedded.
async fn update(state: &AppState, cfg: &ChatConfig, work: &Dirty) -> anyhow::Result<usize> {
    let ids: Vec<String> = if work.all {
        sqlx::query_scalar("SELECT t.note_id FROM note_text t JOIN notes n ON n.id = t.note_id WHERE n.deleted_at IS NULL")
            .fetch_all(&state.db)
            .await?
    } else {
        work.notes.iter().cloned().collect()
    };
    let mut done = 0;
    for id in &ids {
        match embed_note(state, cfg, id).await {
            Ok(true) => done += 1,
            Ok(false) => {}
            Err(e) if e.is::<ServiceError>() => return Err(e),
            Err(e) => tracing::error!("embedding note {id}: {e:#}"),
        }
    }
    Ok(done)
}

fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// A note's search text in chunks of about `CHUNK_CHARS`, split between lines where possible.
/// Each is (the chunk's text, what's embedded: the title, then the chunk), so every chunk knows its note.
fn chunks(title: &str, body: &str) -> Vec<(String, String)> {
    if body.trim().is_empty() {
        return if title.trim().is_empty() { vec![] } else { vec![(title.to_owned(), title.to_owned())] };
    }
    let mut pieces: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in body.lines() {
        for part in split_long(line) {
            if !current.is_empty() && current.chars().count() + part.chars().count() + 1 > CHUNK_CHARS {
                pieces.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(part);
        }
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces.into_iter().map(|p| (p.clone(), format!("{title}\n{p}"))).collect()
}

/// A line longer than a chunk, cut at spaces (or anywhere, if it has none).
fn split_long(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = line;
    while rest.chars().count() > CHUNK_CHARS {
        let limit = rest.char_indices().nth(CHUNK_CHARS).map_or(rest.len(), |(i, _)| i);
        let cut = rest[..limit].rfind(' ').filter(|&i| i > 0).unwrap_or(limit);
        out.push(&rest[..cut]);
        rest = rest[cut..].trim_start();
    }
    out.push(rest);
    out
}

/// Re-embeds a note if its chunks changed, reusing the vectors of chunks that didn't.
/// Trashed notes are skipped; their chunks stay for when they're restored.
async fn embed_note(state: &AppState, cfg: &ChatConfig, id: &str) -> anyhow::Result<bool> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT t.title, t.body FROM note_text t JOIN notes n ON n.id = t.note_id WHERE t.note_id = ? AND n.deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let Some((title, body)) = row else { return Ok(false) };
    let pieces = chunks(&title, &body);
    let hashes: Vec<String> = pieces.iter().map(|(_, input)| hash(input)).collect();
    let have: Vec<(String, String)> = sqlx::query_as("SELECT hash, model FROM note_chunks WHERE note_id = ? ORDER BY idx")
        .bind(id)
        .fetch_all(&state.db)
        .await?;
    if have.len() == hashes.len() && have.iter().zip(&hashes).all(|((h, m), want)| h == want && *m == cfg.model) {
        return Ok(false);
    }
    let mut known: HashMap<String, Vec<u8>> =
        sqlx::query_as::<_, (String, Vec<u8>)>("SELECT hash, embedding FROM note_chunks WHERE note_id = ? AND model = ?")
            .bind(id)
            .bind(&cfg.model)
            .fetch_all(&state.db)
            .await?
            .into_iter()
            .collect();
    let mut missing: Vec<(String, String)> = Vec::new();
    for ((_, input), h) in pieces.iter().zip(&hashes) {
        if !known.contains_key(h) && !missing.iter().any(|(m, _)| m == h) {
            missing.push((h.clone(), input.clone()));
        }
    }
    for batch in missing.chunks(BATCH) {
        let inputs: Vec<String> = batch.iter().map(|(_, input)| input.clone()).collect();
        let vectors = embed(state, cfg, &inputs).await?;
        for ((h, _), v) in batch.iter().zip(vectors) {
            known.insert(h.clone(), to_bytes(&v));
        }
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM note_chunks WHERE note_id = ?").bind(id).execute(&mut *tx).await?;
    for (idx, ((text, _), h)) in pieces.iter().zip(&hashes).enumerate() {
        sqlx::query("INSERT INTO note_chunks (note_id, idx, text, hash, model, embedding) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(id)
            .bind(idx as i64)
            .bind(text)
            .bind(h)
            .bind(&cfg.model)
            .bind(&known[h])
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// Asks the service for one embedding per input, scaled to length 1 so similarity is a dot product.
async fn embed(state: &AppState, cfg: &ChatConfig, inputs: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
    let fail = |why: String| anyhow::Error::new(ServiceError(why));
    let mut req = state
        .http
        .post(format!("{}/embeddings", cfg.url))
        .timeout(Duration::from_secs(120))
        .json(&json!({ "model": cfg.model, "input": inputs }));
    if let Some(key) = &cfg.key {
        req = req.bearer_auth(key);
    }
    let res = req.send().await.map_err(|e| fail(format!("request failed: {e}")))?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(fail(format!("answered {status}: {body}")));
    }
    let body: Value = res.json().await.map_err(|e| fail(format!("unreadable answer: {e}")))?;
    let mut data: Vec<(u64, Vec<f32>)> = body["data"]
        .as_array()
        .ok_or_else(|| fail("no data in the answer".into()))?
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let v = d["embedding"].as_array().map(|v| v.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect());
            (d["index"].as_u64().unwrap_or(i as u64), v.unwrap_or_default())
        })
        .collect();
    data.sort_by_key(|(i, _)| *i);
    if data.len() != inputs.len() || data.iter().any(|(_, v)| v.is_empty()) {
        return Err(fail(format!("sent {} embeddings for {} inputs", data.len(), inputs.len())));
    }
    Ok(data.into_iter().map(|(_, v)| normalized(v)).collect())
}

fn normalized(mut v: Vec<f32>) -> Vec<f32> {
    let len = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if len > 0.0 {
        v.iter_mut().for_each(|x| *x /= len);
    }
    v
}

fn to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Dot product of a stored vector with the query, or None when their sizes differ.
fn similarity(stored: &[u8], query: &[f32]) -> Option<f32> {
    if stored.len() != query.len() * 4 {
        return None;
    }
    Some(stored.chunks_exact(4).zip(query).map(|(b, q)| f32::from_le_bytes([b[0], b[1], b[2], b[3]]) * q).sum())
}

/// A chunk that matched, from a note the user can see.
struct Match {
    note: String,
    title: String,
    text: String,
    score: f32,
}

/// The chunks of the user's visible, untrashed notes that are close to `query`, best first.
async fn matches(state: &AppState, user: &str, cfg: &ChatConfig, query: &str) -> ApiResult<Vec<Match>> {
    let q = embed(state, cfg, &[query.to_owned()]).await.map_err(|e| {
        tracing::warn!("semantic search query failed: {e:#}");
        AppError::Conflict("Couldn't reach the semantic search service".into())
    })?;
    let rows: Vec<(String, String, String, Vec<u8>)> = sqlx::query_as(concat!(
        visible_notes!(),
        "SELECT c.note_id, n.title, c.text, c.embedding FROM note_chunks c JOIN notes n ON n.id = c.note_id
         WHERE n.deleted_at IS NULL AND c.model = ?2 AND c.note_id IN (SELECT id FROM visible)"
    ))
    .bind(user)
    .bind(&cfg.model)
    .fetch_all(&state.db)
    .await?;
    let mut found: Vec<Match> = rows
        .into_iter()
        .filter_map(|(note, title, text, emb)| {
            let score = similarity(&emb, &q[0])?;
            (score >= MIN_SCORE).then_some(Match { note, title, text, score })
        })
        .collect();
    found.sort_by(|a, b| b.score.total_cmp(&a.score));
    if let Some(best) = found.first().map(|m| m.score) {
        found.retain(|m| m.score >= best - SPREAD);
    }
    Ok(found)
}

/// The start of a chunk, for a result's snippet: about 160 characters, ending at a word.
/// Table borders (`|`, `---`) are left out, so a table reads as its cells.
fn excerpt(text: &str) -> String {
    let words = text.split_whitespace().filter(|w| !w.chars().all(|c| matches!(c, '|' | '-' | ':')));
    let flat = words.collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 160 {
        return flat;
    }
    let cut: String = flat.chars().take(160).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{cut}…")
}

/// Something worth embedding: a word or two, not a stray letter.
fn worth_asking(q: &str) -> bool {
    q.chars().filter(|c| c.is_alphanumeric()).count() >= 3
}

/// `GET /search/meaning?q=` — notes about what `q` means, even without its words, best first.
/// Empty when semantic search isn't set up. Each result's snippet is the start of its closest chunk.
pub async fn search(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<SearchResults>> {
    let Some(cfg) = state.embed.read().await.clone() else {
        return Ok(Json(SearchResults { results: vec![] }));
    };
    let q = params.q.trim();
    if !worth_asking(q) {
        return Ok(Json(SearchResults { results: vec![] }));
    }
    let mut seen = HashSet::new();
    let results = matches(&state, &me.id, &cfg, q)
        .await?
        .into_iter()
        .filter(|m| seen.insert(m.note.clone()))
        .take(MAX_RESULTS)
        .map(|m| SearchResult { note: m.note, title: m.title, snippet: vec![Segment { text: excerpt(&m.text), hit: false }], source: "meaning" })
        .collect();
    Ok(Json(SearchResults { results }))
}

#[derive(Deserialize)]
pub struct AskBody {
    question: String,
}

/// A note an answer was written from. `n` is its number in the answer's [n] citations.
#[derive(Serialize)]
pub struct Source {
    n: usize,
    note: String,
    title: String,
}

/// The numbers cited as `[1]`, `[2][3]` or `[1, 2]`, in order, once each.
fn citations(answer: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for part in answer.split('[').skip(1) {
        let Some((inside, _)) = part.split_once(']') else { continue };
        let nums: Vec<Option<usize>> = inside.split(',').map(|n| n.trim().parse().ok()).collect();
        for n in nums.into_iter().flatten() {
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

fn is_no_answer(answer: &str) -> bool {
    answer.trim_matches(|c: char| !c.is_alphanumeric() && c != '_').eq_ignore_ascii_case(NO_ANSWER)
}

/// `POST /ask {question}` — a short answer written from the user's own notes, with the notes it
/// came from. Only the closest chunks of notes the user can see go to the chat model.
/// `answer` is null when the notes don't say.
pub async fn ask(State(state): State<AppState>, CurrentUser(me): CurrentUser, Json(body): Json<AskBody>) -> ApiResult<Json<Value>> {
    let (Some(embed_cfg), Some(chat_cfg)) = (state.embed.read().await.clone(), state.summary.read().await.clone()) else {
        return Err(AppError::Conflict("Ask isn't set up on this server".into()));
    };
    let question = body.question.trim();
    if !worth_asking(question) {
        return Err(AppError::BadRequest("Type a question first".into()));
    }
    if question.chars().count() > 1000 {
        return Err(AppError::BadRequest("That question is too long".into()));
    }
    let nothing = || Ok(Json(json!({ "answer": null, "sources": [] })));
    // The closest chunks, grouped by note in order of their best chunk.
    let mut notes: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut chars = 0;
    for m in matches(&state, &me.id, &embed_cfg, question).await?.into_iter().take(ASK_CHUNKS) {
        chars += m.text.chars().count();
        if chars > ASK_CHARS && !notes.is_empty() {
            break;
        }
        match notes.iter_mut().find(|(id, _, _)| *id == m.note) {
            Some((_, _, texts)) => texts.push(m.text),
            None => notes.push((m.note, m.title, vec![m.text])),
        }
    }
    if notes.is_empty() {
        return nothing();
    }
    let mut prompt = format!("{ASK_PROMPT}\nNotes:\n");
    for (i, (_, title, texts)) in notes.iter().enumerate() {
        prompt.push_str(&format!("\n[{}] {}\n{}\n", i + 1, if title.is_empty() { "Untitled" } else { title }, texts.join("\n…\n")));
    }
    prompt.push_str(&format!("\nQuestion: {question}"));
    let answer = crate::summary::clean(&crate::chat::complete(&state, &chat_cfg, json!(prompt), "answer").await?);
    if answer.is_empty() || is_no_answer(&answer) {
        return nothing();
    }
    let cited: Vec<usize> = citations(&answer).into_iter().filter(|n| (1..=notes.len()).contains(n)).collect();
    // A model that forgot to cite still answered from these notes, so list them all.
    let used: Vec<usize> = if cited.is_empty() { (1..=notes.len()).collect() } else { cited };
    let sources: Vec<Source> = used
        .into_iter()
        .map(|n| Source { n, note: notes[n - 1].0.clone(), title: notes[n - 1].1.clone() })
        .collect();
    Ok(Json(json!({ "answer": answer, "sources": sources })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_split_into_chunks_between_lines() {
        assert!(chunks("", "").is_empty());
        assert_eq!(chunks("Groceries", ""), vec![("Groceries".into(), "Groceries".into())]);
        let line = "word ".repeat(150);
        let body = format!("{line}\n{line}\n{line}");
        let parts = chunks("Long", &body);
        assert_eq!(parts.len(), 3, "each 750-char line is its own chunk");
        assert!(parts.iter().all(|(text, input)| input.starts_with("Long\n") && input.ends_with(text.as_str())));
        let huge = "x".repeat(2500);
        assert_eq!(chunks("T", &huge).iter().map(|(t, _)| t.len()).collect::<Vec<_>>(), vec![1000, 1000, 500]);
    }

    #[test]
    fn citations_and_no_answer() {
        assert_eq!(citations("Milk [2], eggs [1][2] and [3, 1]. [x] [12"), vec![2, 1, 3]);
        assert!(is_no_answer(" NO_ANSWER."));
        assert!(!is_no_answer("No answer here, but [1] says milk"));
    }

    #[test]
    fn similarity_is_a_dot_product_of_unit_vectors() {
        let a = normalized(vec![3.0, 4.0]);
        assert!((similarity(&to_bytes(&a), &a).unwrap() - 1.0).abs() < 1e-6);
        assert_eq!(similarity(&to_bytes(&a), &[1.0]), None);
        assert_eq!(excerpt(&"word ".repeat(50)).chars().count(), 160);
        assert_eq!(excerpt("| Item | Qty |\n| --- | :-: |\n| Milk | 2 |"), "Item Qty Milk 2");
    }
}
