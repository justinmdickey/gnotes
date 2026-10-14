//! Tasks: every checkbox line (`- [ ]` / `- [x]`) in the notes someone can see, kanban cards included.
//! The note's text is the only source; `note_tasks` (migration 0011) is an index of it, rewritten by
//! the note's room when its tasks change, so the Tasks list can show them all without opening notes.
//! Ticking one from the list edits the note's latest text on the server (`write_text`).

use std::sync::atomic::{AtomicBool, Ordering};

use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiResult, AppError},
    perms::{Role, note_role},
    rooms::{note_body, write_text},
    search::visible_notes,
};

/// Settings key set once every note that predates `note_tasks` has been indexed.
const BACKFILLED: &str = "tasks_backfilled";

/// One checkbox line in a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// 0-based line in the note's text.
    pub line: usize,
    /// The task without its marker or due date.
    pub text: String,
    pub done: bool,
    /// `YYYY-MM-DD`, from an `@YYYY-MM-DD` in the line.
    pub due: Option<String>,
    /// The `## Column` it sits under, for a card on a kanban board.
    pub column: Option<String>,
}

/// A fence line's marker (three or more backticks or tildes, indented at most three spaces) and
/// what follows it, e.g. `kanban`.
fn fence(line: &str) -> Option<&str> {
    let l = line.strip_prefix("   ").or(line.strip_prefix("  ")).or(line.strip_prefix(' ')).unwrap_or(line);
    let mark = l.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run = l.chars().take_while(|c| *c == mark).count();
    (run >= 3).then(|| l[run..].trim())
}

/// `- [ ] text`, `* [x] text`, `+ [X]`, indented or not: whether it's done and the rest of the line.
/// The same lines the editor draws as checkboxes and boards take as task cards (kanban.ts `CARD`).
fn checkbox(line: &str) -> Option<(bool, &str)> {
    let l = line.trim_start();
    let rest = l.strip_prefix(['-', '*', '+'])?;
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let rest = rest.trim_start().strip_prefix('[')?;
    let done = match rest.chars().next()? {
        ' ' => false,
        'x' | 'X' => true,
        _ => return None,
    };
    let rest = rest[1..].strip_prefix(']')?;
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some((done, rest.trim()))
}

/// The first `@YYYY-MM-DD` standing on its own in `text`, and the text without it.
fn due_date(text: &str) -> (Option<String>, String) {
    let bytes = text.as_bytes();
    for (i, _) in text.match_indices('@') {
        let Some(date) = text.get(i + 1..i + 11) else { continue };
        let before_ok = i == 0 || bytes[i - 1].is_ascii_whitespace();
        let after_ok = bytes.get(i + 11).is_none_or(|b| !b.is_ascii_alphanumeric());
        if before_ok && after_ok && is_date(date) {
            let rest = format!("{}{}", &text[..i], &text[i + 11..]);
            return (Some(date.to_owned()), rest.split_whitespace().collect::<Vec<_>>().join(" "));
        }
    }
    (None, text.to_owned())
}

/// `YYYY-MM-DD` with a real month and day.
pub fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |r: std::ops::Range<usize>| s.get(r).filter(|p| p.bytes().all(|c| c.is_ascii_digit())).and_then(|p| p.parse::<u32>().ok());
    matches!((num(0..4), num(5..7), num(8..10)), (Some(_), Some(1..=12), Some(1..=31)))
}

/// Every task in a note's Markdown. The first line is the title, never a task; lines in fenced
/// code are skipped, except a ```` ```kanban ```` block, whose `##` headings name the cards' column.
/// A checkbox with nothing after it is still being typed and isn't a task yet.
pub fn parse(body: &str) -> Vec<Task> {
    let mut out = Vec::new();
    let (mut in_fence, mut in_board) = (false, false);
    let mut column: Option<String> = None;
    for (i, line) in body.lines().enumerate().skip(1) {
        if let Some(info) = fence(line) {
            if in_board || in_fence {
                (in_board, in_fence, column) = (false, false, None);
            } else if info == "kanban" {
                in_board = true;
            } else {
                in_fence = true;
            }
            continue;
        }
        if in_fence {
            continue;
        }
        if in_board && let Some(h) = line.trim_start().strip_prefix("##") {
            if h.is_empty() || h.starts_with([' ', '\t']) {
                column = Some(h.trim().to_owned());
                continue;
            }
        }
        let Some((done, rest)) = checkbox(line) else { continue };
        let (due, text) = due_date(rest);
        if text.is_empty() {
            continue;
        }
        out.push(Task { line: i, text, done, due, column: if in_board { column.clone() } else { None } });
    }
    out
}

/// Replaces a note's rows in `note_tasks` with `tasks`, in one transaction.
pub async fn index(db: &sqlx::SqlitePool, note: &str, tasks: &[Task]) -> sqlx::Result<()> {
    let mut tx = db.begin().await?;
    index_in(&mut tx, note, tasks).await?;
    tx.commit().await
}

/// `index` inside a transaction the caller already has, e.g. an import's.
pub async fn index_in(tx: &mut sqlx::SqliteConnection, note: &str, tasks: &[Task]) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM note_tasks WHERE note_id = ?").bind(note).execute(&mut *tx).await?;
    for t in tasks {
        sqlx::query("INSERT INTO note_tasks (note_id, line, text, done, due, board_column) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(note)
            .bind(t.line as i64)
            .bind(&t.text)
            .bind(t.done)
            .bind(&t.due)
            .bind(&t.column)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

/// Indexes the tasks of every note written before `note_tasks` existed. Runs at startup, once.
pub async fn backfill(state: &AppState) -> anyhow::Result<()> {
    let done: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(BACKFILLED)
        .fetch_optional(&state.db)
        .await?;
    if done.is_some() {
        return Ok(());
    }
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM notes").fetch_all(&state.db).await?;
    for id in &ids {
        let body = match Uuid::parse_str(id) {
            Ok(uuid) => note_body(state, uuid).await,
            Err(e) => Err(e.into()),
        };
        match body {
            Ok(body) => index(&state.db, id, &parse(&body)).await?,
            Err(e) => tracing::error!("indexing tasks of note {id}: {e:#}"),
        }
    }
    sqlx::query("INSERT INTO settings (key, value) VALUES (?, 'true')").bind(BACKFILLED).execute(&state.db).await?;
    if !ids.is_empty() {
        tracing::info!("indexed tasks of {} notes", ids.len());
    }
    Ok(())
}

/// A task as the app and the agent API show it: where it is, and what it says.
#[derive(Serialize, Debug, Clone)]
pub struct TaskItem {
    pub note: String,
    /// The note's title.
    pub title: String,
    /// The note's notebook, `null` at the top level.
    pub notebook: Option<String>,
    pub line: i64,
    pub text: String,
    pub done: bool,
    pub due: Option<String>,
    pub column: Option<String>,
}

/// Tasks in notes `user` can see right now, the same set as /tree, by due date (undated last),
/// then note and line. `done` keeps only open (`false`) or only done (`true`) ones;
/// `due_before` only those due before that date.
pub async fn visible(state: &AppState, user: &str, done: Option<bool>, due_before: Option<&str>) -> sqlx::Result<Vec<TaskItem>> {
    let rows: Vec<(String, String, Option<String>, i64, String, bool, Option<String>, Option<String>)> = sqlx::query_as(concat!(
        visible_notes!(),
        "SELECT n.id, n.title, n.notebook_id, t.line, t.text, t.done, t.due, t.board_column
         FROM note_tasks t JOIN notes n ON n.id = t.note_id
         WHERE n.deleted_at IS NULL AND n.id IN (SELECT id FROM visible)
           AND (?2 IS NULL OR t.done = ?2) AND (?3 IS NULL OR t.due < ?3)
         ORDER BY t.due IS NULL, t.due, n.title, n.id, t.line"
    ))
    .bind(user)
    .bind(done)
    .bind(due_before)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(note, title, notebook, line, text, done, due, column)| TaskItem { note, title, notebook, line, text, done, due, column })
        .collect())
}

/// Sets the box of the task on `line` of `note` to `done`, as `user`, on the note's latest text.
/// If that line isn't a task reading `text` any more (someone edited it since the list was
/// loaded), nothing changes and it's a conflict.
pub async fn set_done(state: &AppState, user: &str, note: &str, line: usize, text: &str, done: bool) -> ApiResult<()> {
    match note_role(&state.db, user, note).await? {
        None => return Err(AppError::NotFound),
        Some(role) if role < Role::Editor => return Err(AppError::Forbidden),
        Some(_) => {}
    }
    let id: Uuid = note.parse().map_err(|_| AppError::NotFound)?;
    let found = AtomicBool::new(false);
    write_text(state, id, user, None, |body| {
        let Some(task) = parse(body).into_iter().find(|t| t.line == line && t.text == text) else {
            return body.to_owned();
        };
        found.store(true, Ordering::Relaxed);
        if task.done == done {
            return body.to_owned();
        }
        let mut lines: Vec<&str> = body.split('\n').collect();
        let l = lines[line];
        let open = l.find('[').expect("a checkbox line has a [");
        let ticked = format!("{}[{}]{}", &l[..open], if done { 'x' } else { ' ' }, &l[open + 3..]);
        lines[line] = &ticked;
        lines.join("\n")
    })
    .await?;
    if !found.load(Ordering::Relaxed) {
        return Err(AppError::Conflict("That task changed in the note".into()));
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct ListParams {
    pub done: Option<bool>,
}

#[derive(Serialize)]
pub struct TaskList {
    pub tasks: Vec<TaskItem>,
}

/// `GET /tasks?done=`: tasks in every note you can see.
pub async fn list(State(state): State<AppState>, CurrentUser(me): CurrentUser, Query(p): Query<ListParams>) -> ApiResult<Json<TaskList>> {
    Ok(Json(TaskList { tasks: visible(&state, &me.id, p.done, None).await? }))
}

#[derive(Deserialize)]
pub struct Toggle {
    pub note: String,
    pub line: usize,
    pub text: String,
    pub done: bool,
}

/// `POST /tasks/toggle {note, line, text, done}`: ticks or unticks a task from the list.
pub async fn toggle(State(state): State<AppState>, CurrentUser(me): CurrentUser, Json(t): Json<Toggle>) -> ApiResult<Json<serde_json::Value>> {
    set_done(&state, &me.id, &t.note, t.line, &t.text, t.done).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::{Task, parse};

    fn t(line: usize, text: &str, done: bool, due: Option<&str>, column: Option<&str>) -> Task {
        Task { line, text: text.into(), done, due: due.map(Into::into), column: column.map(Into::into) }
    }

    #[test]
    fn checkbox_lines_are_tasks() {
        let body = "- [ ] Title is never a task\n- [ ] milk\n  * [x] eggs\n+ [X] bread\n- [ ]\n- [] nope\n-[ ] nope\n- [ ]nope\n- plain item\nText";
        assert_eq!(parse(body), vec![t(1, "milk", false, None, None), t(2, "eggs", true, None, None), t(3, "bread", true, None, None)]);
    }

    #[test]
    fn due_dates_come_out_of_the_text() {
        let body = "Garage\n- [ ] Call about the door @2026-10-12\n- [ ] Filter @2026-13-01\n- [ ] Mail me@2026-10-12 x\n- [ ] @2026-10-20 first, @2026-10-21 second";
        assert_eq!(
            parse(body),
            vec![
                t(1, "Call about the door", false, Some("2026-10-12"), None),
                t(2, "Filter @2026-13-01", false, None, None),
                t(3, "Mail me@2026-10-12 x", false, None, None),
                t(4, "first, @2026-10-21 second", false, Some("2026-10-20"), None),
            ]
        );
    }

    #[test]
    fn boards_give_columns_and_code_is_skipped() {
        let body = "Remodel\n```kanban\n## To do\n- [ ] Pick hardware\n- plain card\n## Doing\n- [x] Order tiles\n```\n- [ ] after\n```md\n- [ ] in code\n```\n~~~\n- [ ] tilde code\n~~~\n- [ ] last";
        assert_eq!(
            parse(body),
            vec![
                t(3, "Pick hardware", false, None, Some("To do")),
                t(6, "Order tiles", true, None, Some("Doing")),
                t(8, "after", false, None, None),
                t(15, "last", false, None, None),
            ]
        );
    }

    #[test]
    fn headings_outside_boards_are_not_columns() {
        assert_eq!(parse("Notes\n## Action items\n- [ ] Send estimate"), vec![t(2, "Send estimate", false, None, None)]);
    }
}
