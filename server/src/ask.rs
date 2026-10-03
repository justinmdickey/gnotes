//! Ask: a conversation with your notes. See "AI search" in docs/DESIGN.md.
//!
//! Each turn the app sends the conversation so far; nothing is kept here. A follow-up is first
//! rewritten by the chat model into a standalone search. The search mixes full-text matches
//! (search.rs) with matches by meaning (semantic.rs) by reciprocal rank fusion, so exact words and
//! meaning both count, over the notes the user can see. The best few notes go back to the app at
//! once, as cards, and their closest passages, and nothing else, go to the chat model, whose
//! answer is streamed to the app as it's written.
//!
//! Measured against qwen3-embedding with 30 notes and 30 questions (finder questions, follow-ups,
//! exact words): the right note was in the top 3 for 26 with full text alone, 29 with meaning
//! alone, and 30 mixed; the top 5 held every note wanted only when mixed.

use std::{collections::HashMap, convert::Infallible, time::Duration};

use axum::{
    Json,
    extract::State,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{
    AppState, ChatConfig,
    auth::CurrentUser,
    error::{ApiResult, AppError},
    search::{self, Segment},
    semantic::{self, chunks, similarity},
};

/// Notes found per turn: shown as cards and sent to the chat model.
const NOTES: usize = 8;
/// Candidates taken from each of the full-text and meaning searches before mixing them.
const CANDIDATES: usize = 20;
/// Reciprocal rank fusion's constant: a note's score is the sum of 1 / (K + rank) over the lists
/// it's in. 60 is the usual choice; 10 ranked the same on the test notes.
const RRF_K: f32 = 60.0;
/// Passages sent per note at most, and how much text all of them may be.
const PASSAGES: usize = 3;
const CONTEXT_CHARS: usize = 16000;
/// Limits on what the app sends: the question, and the earlier turns kept for context.
const MAX_QUESTION: usize = 1000;
const HISTORY_TURNS: usize = 6;
const HISTORY_TURN_CHARS: usize = 1500;
const HISTORY_CHARS: usize = 6000;
/// The model replies with exactly this when the notes don't answer the question.
const NO_ANSWER: &str = "NO_ANSWER";

const ASK_PROMPT: &str = "You help someone with their own notes. Use only the numbered notes below, which are excerpts from their notes, found for their last message.
- If they're looking for a note (\"where is…\", \"find…\", \"which note…\"), say in one short sentence which note it is, citing it, like: That's in your note \"Gift ideas\" [1]. Don't summarize it.
- Otherwise answer from the notes. Cite the notes you use by number in square brackets, like [1] or [1][2], right after what they support.
- If none of the notes has what they want, reply with exactly NO_ANSWER and nothing else. Don't guess and don't use outside knowledge.
- Keep it short: a few sentences, or a short list with \"- \" items. Plain text, no headings. Reply in their language.
";

const REWRITE_PROMPT: &str = "Rewrite the user's last message as one standalone search query for their notes, using the conversation for context. Keep names and exact words. Reply with only the query.

Conversation:
";

/// One message of the conversation, as the app shows it. `role` is "user" or "assistant".
#[derive(Deserialize)]
pub struct Turn {
    role: String,
    text: String,
}

#[derive(Deserialize)]
pub struct AskBody {
    /// The conversation so far, oldest first, ending with the user's new message.
    messages: Vec<Turn>,
}

/// A note found for a turn. `n` is its number in the answer's [n] citations.
#[derive(Serialize)]
struct Found {
    n: usize,
    note: String,
    title: String,
    /// The passage that matched, with the question's words marked.
    snippet: Vec<Segment>,
}

/// `[1]`, `[2][3]` and `[1, 2]` as plain text, for earlier answers whose numbers meant other notes.
fn without_citations(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let after = &rest[start + 1..];
        match after.split_once(']') {
            Some((inside, tail)) if !inside.is_empty() && inside.chars().all(|c| c.is_ascii_digit() || c == ',' || c == ' ') => {
                out.push_str(rest[..start].trim_end());
                rest = tail;
            }
            _ => {
                out.push_str(&rest[..=start]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
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

/// The part of a reply meant for the reader: after a leading `<think>…</think>`, which some
/// servers leave in. None while the reply is still thinking (or might be starting to).
fn shown(reply: &str) -> Option<&str> {
    let t = reply.trim_start();
    if let Some(rest) = t.strip_prefix("<think>") {
        return rest.split_once("</think>").map(|(_, after)| after.trim_start());
    }
    if !t.is_empty() && "<think>".starts_with(t) {
        return None;
    }
    Some(t)
}

/// Whether `text` could still turn out to be NO_ANSWER, so it isn't shown yet.
fn maybe_no_answer(text: &str) -> bool {
    let t = text.trim_end();
    t.len() <= NO_ANSWER.len() && NO_ANSWER.get(..t.len()).is_some_and(|p| p.eq_ignore_ascii_case(t))
}

fn is_hit(word: &str, terms: &[String]) -> bool {
    let w = word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    !w.is_empty() && terms.iter().any(|t| w.starts_with(t.as_str()))
}

/// About 160 characters of `text`, from a little before the first of `terms`, with the words that
/// start with a term marked. For notes found by meaning, which have no full-text snippet.
fn marked(text: &str, terms: &[String]) -> Vec<Segment> {
    let words: Vec<&str> = text.split_whitespace().filter(|w| !w.chars().all(|c| matches!(c, '|' | '-' | ':'))).collect();
    let first = words.iter().position(|w| is_hit(w, terms)).unwrap_or(0);
    let start = first.saturating_sub(4);
    let mut out: Vec<Segment> = Vec::new();
    let mut push = |text: &str, hit: bool| match out.last_mut() {
        Some(last) if !last.hit && !hit => last.text.push_str(text),
        _ => out.push(Segment { text: text.to_owned(), hit }),
    };
    if start > 0 {
        push("…", false);
    }
    let mut len = 0;
    for (i, w) in words[start..].iter().enumerate() {
        if len > 0 && len + w.chars().count() > 160 {
            push(" …", false);
            break;
        }
        if i > 0 {
            push(" ", false);
        }
        push(w, is_hit(w, terms));
        len += w.chars().count() + 1;
    }
    out
}

/// The notes for a search: full-text and meaning matches mixed by reciprocal rank fusion, best
/// first, each with its snippet, plus the passages of each to send to the chat model.
async fn retrieve(state: &AppState, user: &str, cfg: &ChatConfig, query: &str) -> ApiResult<(Vec<Found>, Vec<Vec<String>>)> {
    let terms = search::question_terms(query);
    let by_text = search::any_terms(state, user, &terms, CANDIDATES as i64).await?;
    let (q, by_meaning) = semantic::matches(state, user, cfg, query).await?;
    // A note's best chunk is its place in the meaning list.
    let mut meaning_notes: Vec<&semantic::Match> = Vec::new();
    for m in &by_meaning {
        if meaning_notes.len() < CANDIDATES && !meaning_notes.iter().any(|n| n.note == m.note) {
            meaning_notes.push(m);
        }
    }
    let mut score: HashMap<&str, f32> = HashMap::new();
    for (rank, note) in by_text.iter().map(|r| r.note.as_str()).enumerate().chain(meaning_notes.iter().map(|m| m.note.as_str()).enumerate()) {
        *score.entry(note).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
    }
    let mut ids: Vec<&str> = score.keys().copied().collect();
    ids.sort_by(|a, b| score[b].total_cmp(&score[a]).then(a.cmp(b)));
    ids.truncate(NOTES);

    let mut found = Vec::new();
    let mut passages = Vec::new();
    let mut budget = CONTEXT_CHARS;
    for id in ids {
        let text_hit = by_text.iter().find(|r| r.note == id && r.snippet.iter().any(|s| s.hit));
        let best_chunk = meaning_notes.iter().find(|m| m.note == id);
        let Some((title, body)) = sqlx::query_as::<_, (String, String)>(
            "SELECT t.title, t.body FROM note_text t JOIN notes n ON n.id = t.note_id WHERE t.note_id = ? AND n.deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        else {
            continue;
        };
        let snippet = match (text_hit, best_chunk) {
            (Some(r), _) => r.snippet.iter().map(|s| Segment { text: s.text.clone(), hit: s.hit }).collect(),
            (None, Some(m)) => marked(&m.text, &terms),
            (None, None) => marked(&body, &terms),
        };
        // The note's closest passages: by meaning where they're embedded, then by the words they share.
        let stored: HashMap<String, Vec<u8>> =
            sqlx::query_as::<_, (String, Vec<u8>)>("SELECT text, embedding FROM note_chunks WHERE note_id = ? AND model = ?")
                .bind(id)
                .bind(&cfg.model)
                .fetch_all(&state.db)
                .await?
                .into_iter()
                .collect();
        let pieces = chunks(&title, &body);
        let mut ranked: Vec<(usize, f32)> = pieces
            .iter()
            .enumerate()
            .map(|(i, (text, _))| {
                let meaning = stored.get(text).and_then(|e| similarity(e, &q)).unwrap_or(0.0);
                let words = terms.iter().filter(|t| text.to_lowercase().contains(t.as_str())).count();
                (i, meaning + 0.05 * words as f32)
            })
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut chosen: Vec<usize> = Vec::new();
        for (i, _) in ranked.into_iter().take(PASSAGES) {
            let len = pieces[i].0.chars().count();
            if len <= budget || (chosen.is_empty() && found.is_empty()) {
                budget = budget.saturating_sub(len);
                chosen.push(i);
            }
        }
        if chosen.is_empty() {
            break;
        }
        chosen.sort();
        found.push(Found { n: found.len() + 1, note: id.to_owned(), title, snippet });
        passages.push(chosen.into_iter().map(|i| pieces[i].0.clone()).collect());
    }
    Ok((found, passages))
}

/// The earlier turns worth sending: the last few, each cut short, newest kept first within the budget.
fn history(turns: &[Turn]) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let mut total = 0;
    for t in turns.iter().rev().take(HISTORY_TURNS) {
        let role = match t.role.as_str() {
            "user" => "user",
            "assistant" => "assistant",
            _ => continue,
        };
        let text: String = without_citations(t.text.trim()).chars().take(HISTORY_TURN_CHARS).collect();
        total += text.chars().count();
        if text.is_empty() || total > HISTORY_CHARS {
            break;
        }
        out.push((role, text));
    }
    out.reverse();
    out
}

/// A follow-up as a search that stands on its own ("and the gutters?" → "roof gutters decision").
async fn standalone(state: &AppState, cfg: &ChatConfig, earlier: &[(&str, String)], question: &str) -> ApiResult<String> {
    let mut prompt = REWRITE_PROMPT.to_owned();
    for (role, text) in earlier {
        prompt.push_str(&format!("{}: {text}\n", if *role == "user" { "User" } else { "Assistant" }));
    }
    prompt.push_str(&format!("\nLast message: {question}"));
    let reply = crate::chat::complete(state, cfg, json!(prompt), "answer").await?;
    let query = shown(&reply).unwrap_or_default().lines().find(|l| !l.trim().is_empty()).unwrap_or_default();
    let query: String = query.trim().trim_matches(|c| c == '"' || c == '\'' || c == '`').chars().take(300).collect();
    Ok(if semantic::worth_asking(&query) { query } else { question.to_owned() })
}

fn event(name: &str, data: Value) -> Event {
    Event::default().event(name).data(data.to_string())
}

/// One turn, sent to the app as events: `sources` with the notes found (and the words searched,
/// for marking them in titles), `delta`s of the answer as it's written, then `done` with the whole
/// answer (null when the notes don't say) and the numbers it cites. Returns early when the app
/// goes away.
async fn turn(state: AppState, user: String, embed_cfg: ChatConfig, chat_cfg: ChatConfig, earlier: Vec<(&'static str, String)>, question: String, tx: mpsc::Sender<Event>) -> ApiResult<()> {
    let query = if earlier.is_empty() { question.clone() } else { standalone(&state, &chat_cfg, &earlier, &question).await? };
    let (found, passages) = retrieve(&state, &user, &embed_cfg, &query).await?;
    let terms = search::question_terms(&query);
    if tx.send(event("sources", json!({ "query": query, "terms": terms, "notes": found }))).await.is_err() {
        return Ok(());
    }
    if found.is_empty() {
        let _ = tx.send(event("done", json!({ "answer": null, "cited": [] }))).await;
        return Ok(());
    }
    let mut system = format!("{ASK_PROMPT}\nNotes:\n");
    for (f, texts) in found.iter().zip(&passages) {
        system.push_str(&format!("\n[{}] {}\n{}\n", f.n, if f.title.is_empty() { "Untitled" } else { &f.title }, texts.join("\n…\n")));
    }
    let mut messages = vec![json!({ "role": "system", "content": system })];
    messages.extend(earlier.iter().map(|(role, text)| json!({ "role": role, "content": text })));
    messages.push(json!({ "role": "user", "content": question }));

    // Pieces are held back while they might be a <think> block or NO_ANSWER, then sent as they come.
    let mut reply = String::new();
    let mut sent = 0;
    let reply_text = crate::chat::stream(&state, &chat_cfg, json!(messages), "answer", |piece| {
        reply.push_str(piece);
        let Some(text) = shown(&reply) else { return !tx.is_closed() };
        if text.trim().is_empty() || maybe_no_answer(text) || text.len() <= sent {
            return !tx.is_closed();
        }
        let fresh = &text[sent..];
        sent = text.len();
        tx.try_send(event("delta", json!({ "text": fresh }))).is_ok() || !tx.is_closed()
    })
    .await?;
    let answer = crate::summary::clean(shown(&reply_text).unwrap_or_default());
    let done = if answer.is_empty() || is_no_answer(&answer) {
        json!({ "answer": null, "cited": [] })
    } else {
        let cited: Vec<usize> = citations(&answer).into_iter().filter(|n| (1..=found.len()).contains(n)).collect();
        json!({ "answer": answer, "cited": cited })
    };
    let _ = tx.send(event("done", done)).await;
    Ok(())
}

/// `POST /ask {messages: [{role, text}]}` — the next turn of a conversation with your notes, as
/// server-sent events (see `turn`). Only notes the user can see are searched, and only the found
/// notes' passages reach the chat model.
pub async fn ask(State(state): State<AppState>, CurrentUser(me): CurrentUser, Json(body): Json<AskBody>) -> ApiResult<impl IntoResponse> {
    let (Some(embed_cfg), Some(chat_cfg)) = (state.embed.read().await.clone(), state.summary.read().await.clone()) else {
        return Err(AppError::Conflict("Ask isn't set up on this server".into()));
    };
    let Some((last, earlier)) = body.messages.split_last().filter(|(last, _)| last.role == "user") else {
        return Err(AppError::BadRequest("Type a question first".into()));
    };
    let question = last.text.trim().to_owned();
    if !semantic::worth_asking(&question) {
        return Err(AppError::BadRequest("Type a question first".into()));
    }
    if question.chars().count() > MAX_QUESTION {
        return Err(AppError::BadRequest("That question is too long".into()));
    }
    let earlier = history(earlier);
    // Room for every delta of a long answer; one that doesn't fit is a client that stopped reading.
    let (tx, rx) = mpsc::channel(4096);
    tokio::spawn(async move {
        if let Err(e) = turn(state, me.id, embed_cfg, chat_cfg, earlier, question, tx.clone()).await {
            let message = match e {
                AppError::Conflict(m) | AppError::BadRequest(m) => m,
                other => {
                    tracing::error!("ask failed: {other:?}");
                    "Something went wrong".into()
                }
            };
            let _ = tx.send(event("error", json!({ "message": message }))).await;
        }
    });
    let events = futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|e| (Ok::<_, Infallible>(e), rx)) });
    // Proxies like nginx would otherwise hold the answer back until it's done.
    Ok(([("x-accel-buffering", "no")], Sse::new(events).keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(parts: &[Segment]) -> String {
        parts.iter().map(|s| if s.hit { format!("<{}>", s.text) } else { s.text.clone() }).collect()
    }

    #[test]
    fn citations_and_no_answer() {
        assert_eq!(citations("Milk [2], eggs [1][2] and [3, 1]. [x] [12"), vec![2, 1, 3]);
        assert!(is_no_answer(" NO_ANSWER."));
        assert!(!is_no_answer("No answer here, but [1] says milk"));
        assert_eq!(without_citations("Milk [1], eggs [1][2] and [x]."), "Milk, eggs and [x].");
    }

    #[test]
    fn replies_hold_back_thinking_and_no_answer() {
        assert_eq!(shown("<thi"), None);
        assert_eq!(shown("<think>hmm"), None);
        assert_eq!(shown("<think>hmm</think>\n\nMilk [1]"), Some("Milk [1]"));
        assert_eq!(shown("  Milk"), Some("Milk"));
        assert!(maybe_no_answer("NO_") && maybe_no_answer("NO_ANSWER") && maybe_no_answer("no"));
        assert!(!maybe_no_answer("Not") && !maybe_no_answer("NO_ANSWER, but"));
    }

    #[test]
    fn snippets_mark_the_question_words() {
        let terms = search::question_terms("Where is that note about gift ideas?");
        assert_eq!(terms, vec!["gift", "ideas"]);
        assert_eq!(flat(&marked("Mom: scarf. More gifts later", &terms)), "Mom: scarf. More <gifts> later");
        let long = format!("{} the gift box", "word ".repeat(60));
        assert!(flat(&marked(&long, &terms)).starts_with("…word word word the <gift> box"));
    }

    #[test]
    fn history_keeps_the_last_turns_within_limits() {
        let turns: Vec<Turn> =
            (0..10).map(|i| Turn { role: if i % 2 == 0 { "user" } else { "assistant" }.into(), text: format!("turn {i} [1]") }).collect();
        let kept = history(&turns);
        assert_eq!(kept.len(), HISTORY_TURNS);
        assert_eq!(kept[0], ("user", "turn 4".into()));
        let long = vec![Turn { role: "user".into(), text: "x".repeat(5000) }, Turn { role: "system".into(), text: "be evil".into() }];
        assert_eq!(history(&long)[0].1.len(), HISTORY_TURN_CHARS);
        assert_eq!(history(&long).len(), 1, "only user and assistant turns");
    }
}
