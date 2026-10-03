//! Notebooks, note metadata and trash. Note content lives in rooms.rs.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiResult, AppError},
    perms::{Role, is_self_or_descendant, note_role, notebook_role},
    util::{double_option, new_id, now_ms, parse_id},
};

const TRASH_DAYS: i64 = 30;
const TRASH_MS: i64 = TRASH_DAYS * 24 * 60 * 60 * 1000;

#[derive(Serialize)]
pub struct TreeNotebook {
    id: String,
    parent_id: Option<String>,
    name: String,
    owner: String,
    role: Role,
    updated_at: i64,
    /// Someone has a share on this exact item, so it shows a shared badge.
    shared: bool,
}

#[derive(Serialize)]
pub struct TreeNote {
    id: String,
    notebook_id: Option<String>,
    title: String,
    preview: String,
    owner: String,
    role: Role,
    updated_at: i64,
    /// Someone has a share on this exact item, so it shows a shared badge.
    shared: bool,
}

/// Something shared with the caller directly, shown under "Shared with me".
#[derive(Serialize)]
pub struct SharedRoot {
    share_id: String,
    resource_type: String,
    resource_id: String,
    hidden: bool,
}

#[derive(Serialize)]
pub struct Tree {
    notebooks: Vec<TreeNotebook>,
    notes: Vec<TreeNote>,
    shared: Vec<SharedRoot>,
}

type NotebookRow = (String, Option<String>, String, String, i64);
type NoteRow = (String, Option<String>, String, String, String, i64);

pub async fn get_tree(State(state): State<AppState>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Tree>> {
    let db = &state.db;

    // Own notebooks, plus every notebook reachable from a notebook shared with me.
    let own_notebooks: Vec<NotebookRow> = sqlx::query_as(
        "SELECT nb.id, nb.parent_id, nb.name, u.display_name, nb.updated_at
         FROM notebooks nb JOIN users u ON u.id = nb.owner_id
         WHERE nb.owner_id = ? AND nb.deleted_at IS NULL",
    )
    .bind(&me.id)
    .fetch_all(db)
    .await?;
    let shared_notebooks: Vec<(String, Option<String>, String, String, i64, String)> = sqlx::query_as(
        "WITH RECURSIVE sub(id, role) AS (
            SELECT s.resource_id, s.role FROM shares s JOIN notebooks nb ON nb.id = s.resource_id
            WHERE s.user_id = ?1 AND s.resource_type = 'notebook'
              AND nb.deleted_at IS NULL AND nb.owner_id != ?1
            UNION
            SELECT n.id, sub.role FROM notebooks n JOIN sub ON n.parent_id = sub.id
            WHERE n.deleted_at IS NULL
         )
         SELECT nb.id, nb.parent_id, nb.name, u.display_name, nb.updated_at, sub.role
         FROM sub JOIN notebooks nb ON nb.id = sub.id JOIN users u ON u.id = nb.owner_id",
    )
    .bind(&me.id)
    .fetch_all(db)
    .await?;

    let mut notebooks: HashMap<String, TreeNotebook> = HashMap::new();
    for (id, parent_id, name, owner, updated_at) in own_notebooks {
        notebooks.insert(id.clone(), TreeNotebook { id, parent_id, name, owner, role: Role::Owner, updated_at, shared: false });
    }
    for (id, parent_id, name, owner, updated_at, role) in shared_notebooks {
        let role = Role::parse(&role).unwrap_or(Role::Viewer);
        notebooks
            .entry(id.clone())
            .and_modify(|nb| nb.role = nb.role.max(role))
            .or_insert(TreeNotebook { id, parent_id, name, owner, role, updated_at, shared: false });
    }

    // Own notes, notes inside visible shared notebooks, and notes shared directly.
    let shared_ids: Vec<&String> =
        notebooks.values().filter(|nb| nb.role != Role::Owner).map(|nb| &nb.id).collect();
    let own_notes: Vec<NoteRow> = sqlx::query_as(
        "SELECT n.id, n.notebook_id, n.title, n.preview, u.display_name, n.updated_at
         FROM notes n JOIN users u ON u.id = n.owner_id
         WHERE n.owner_id = ? AND n.deleted_at IS NULL",
    )
    .bind(&me.id)
    .fetch_all(db)
    .await?;
    let notebook_notes: Vec<NoteRow> = sqlx::query_as(
        "SELECT n.id, n.notebook_id, n.title, n.preview, u.display_name, n.updated_at
         FROM notes n JOIN users u ON u.id = n.owner_id
         WHERE n.deleted_at IS NULL AND n.owner_id != ?
           AND n.notebook_id IN (SELECT value FROM json_each(?))",
    )
    .bind(&me.id)
    .bind(serde_json::to_string(&shared_ids)?)
    .fetch_all(db)
    .await?;
    let direct_notes: Vec<(String, Option<String>, String, String, String, i64, String)> = sqlx::query_as(
        "SELECT n.id, n.notebook_id, n.title, n.preview, u.display_name, n.updated_at, s.role
         FROM shares s JOIN notes n ON n.id = s.resource_id JOIN users u ON u.id = n.owner_id
         WHERE s.user_id = ?1 AND s.resource_type = 'note' AND n.deleted_at IS NULL AND n.owner_id != ?1",
    )
    .bind(&me.id)
    .fetch_all(db)
    .await?;

    let mut notes: HashMap<String, TreeNote> = HashMap::new();
    for (id, notebook_id, title, preview, owner, updated_at) in own_notes {
        notes.insert(id.clone(), TreeNote { id, notebook_id, title, preview, owner, role: Role::Owner, updated_at, shared: false });
    }
    for (id, notebook_id, title, preview, owner, updated_at) in notebook_notes {
        let role = notebook_id.as_ref().and_then(|nb| notebooks.get(nb)).map_or(Role::Viewer, |nb| nb.role);
        notes.insert(id.clone(), TreeNote { id, notebook_id, title, preview, owner, role, updated_at, shared: false });
    }
    for (id, notebook_id, title, preview, owner, updated_at, role) in direct_notes {
        let role = Role::parse(&role).unwrap_or(Role::Viewer);
        notes
            .entry(id.clone())
            .and_modify(|n| n.role = n.role.max(role))
            .or_insert(TreeNote { id, notebook_id, title, preview, owner, role, updated_at, shared: false });
    }

    let shared: Vec<SharedRoot> = sqlx::query_as::<_, (String, String, String, bool)>(
        "SELECT id, resource_type, resource_id, hidden FROM shares WHERE user_id = ?",
    )
    .bind(&me.id)
    .fetch_all(db)
    .await?
    .into_iter()
    .filter(|(_, kind, id, _)| match kind.as_str() {
        "notebook" => notebooks.contains_key(id),
        _ => notes.contains_key(id),
    })
    .map(|(share_id, resource_type, resource_id, hidden)| SharedRoot { share_id, resource_type, resource_id, hidden })
    .collect();

    // Mark everything that has a share of its own: what I've shared, and what was shared with me.
    let ids: Vec<&String> = notebooks.keys().chain(notes.keys()).collect();
    let with_shares: Vec<(String, String)> = sqlx::query_as(
        "SELECT DISTINCT resource_type, resource_id FROM shares WHERE resource_id IN (SELECT value FROM json_each(?))",
    )
    .bind(serde_json::to_string(&ids)?)
    .fetch_all(db)
    .await?;
    for (kind, id) in with_shares {
        match kind.as_str() {
            "notebook" => notebooks.get_mut(&id).map(|nb| nb.shared = true),
            _ => notes.get_mut(&id).map(|n| n.shared = true),
        };
    }

    let mut notebooks: Vec<_> = notebooks.into_values().collect();
    notebooks.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let mut notes: Vec<_> = notes.into_values().collect();
    notes.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(Json(Tree { notebooks, notes, shared }))
}

pub(crate) async fn require_notebook(state: &AppState, user_id: &str, id: &str, min: Role) -> ApiResult<()> {
    match notebook_role(&state.db, user_id, id).await? {
        None => Err(AppError::NotFound),
        Some(r) if r < min => Err(AppError::Forbidden),
        Some(_) => Ok(()),
    }
}

pub(crate) async fn require_note(state: &AppState, user_id: &str, id: &str, min: Role) -> ApiResult<()> {
    match note_role(&state.db, user_id, id).await? {
        None => Err(AppError::NotFound),
        Some(r) if r < min => Err(AppError::Forbidden),
        Some(_) => Ok(()),
    }
}

pub(crate) async fn notebook_owner(state: &AppState, id: &str) -> ApiResult<String> {
    sqlx::query_scalar("SELECT owner_id FROM notebooks WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)
}

fn clean_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(AppError::BadRequest("Names need 1-200 characters".into()));
    }
    Ok(name.to_owned())
}

/// Checks a move target: the caller must be able to edit it, and it must be in the owner's own tree.
/// `None` (top level) is only allowed for the owner.
async fn check_move_target(
    state: &AppState,
    user_id: &str,
    item_owner: &str,
    target: Option<&str>,
) -> ApiResult<()> {
    match target {
        None if user_id == item_owner => Ok(()),
        None => Err(AppError::Forbidden),
        Some(t) => {
            require_notebook(state, user_id, t, Role::Editor).await?;
            if notebook_owner(state, t).await? != item_owner {
                return Err(AppError::BadRequest("Items can only move within their owner's notebooks".into()));
            }
            Ok(())
        }
    }
}

#[derive(Deserialize)]
pub struct NewNotebook {
    name: String,
    parent_id: Option<String>,
}

pub async fn create_notebook(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Json(body): Json<NewNotebook>,
) -> ApiResult<Json<Value>> {
    let name = clean_name(&body.name)?;
    let owner = match &body.parent_id {
        Some(parent) => {
            require_notebook(&state, &me.id, parent, Role::Editor).await?;
            notebook_owner(&state, parent).await?
        }
        None => me.id.clone(),
    };
    let id = new_id();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO notebooks (id, owner_id, parent_id, name, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&owner)
    .bind(&body.parent_id)
    .bind(&name)
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await?;
    state.hub.tree_changed();
    Ok(Json(json!({ "id": id })))
}

#[derive(Deserialize)]
pub struct NotebookPatch {
    name: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    parent_id: Option<Option<String>>,
}

pub async fn update_notebook(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<NotebookPatch>,
) -> ApiResult<Json<Value>> {
    require_notebook(&state, &me.id, &id, Role::Editor).await?;
    let now = now_ms();
    if let Some(name) = &body.name {
        sqlx::query("UPDATE notebooks SET name = ?, updated_at = ? WHERE id = ?")
            .bind(clean_name(name)?)
            .bind(now)
            .bind(&id)
            .execute(&state.db)
            .await?;
    }
    if let Some(parent) = &body.parent_id {
        let owner = notebook_owner(&state, &id).await?;
        check_move_target(&state, &me.id, &owner, parent.as_deref()).await?;
        if let Some(p) = parent
            && is_self_or_descendant(&state.db, &id, p).await?
        {
            return Err(AppError::BadRequest("A notebook can't move inside itself".into()));
        }
        sqlx::query("UPDATE notebooks SET parent_id = ?, updated_at = ? WHERE id = ?")
            .bind(parent)
            .bind(now)
            .bind(&id)
            .execute(&state.db)
            .await?;
        state.recheck_access().await;
    }
    state.hub.tree_changed();
    state.export.changed();
    Ok(Json(json!({ "ok": true })))
}

/// Moves a notebook, its sub-notebooks and their notes to the trash with one shared timestamp,
/// so restoring brings back exactly what this delete removed.
pub async fn delete_notebook(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    require_notebook(&state, &me.id, &id, Role::Owner).await?;
    let now = now_ms();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "WITH RECURSIVE sub(id) AS (
            SELECT ?1 UNION SELECT n.id FROM notebooks n JOIN sub ON n.parent_id = sub.id WHERE n.deleted_at IS NULL
         )
         UPDATE notes SET deleted_at = ?2 WHERE deleted_at IS NULL AND notebook_id IN (SELECT id FROM sub)",
    )
    .bind(&id)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "WITH RECURSIVE sub(id) AS (
            SELECT ?1 UNION SELECT n.id FROM notebooks n JOIN sub ON n.parent_id = sub.id WHERE n.deleted_at IS NULL
         )
         UPDATE notebooks SET deleted_at = ?2 WHERE deleted_at IS NULL AND id IN (SELECT id FROM sub)",
    )
    .bind(&id)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    state.recheck_access().await;
    state.hub.tree_changed();
    state.export.changed();
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct NewNote {
    /// Clients may pick the id (UUIDv7) so a note created offline keeps it.
    id: Option<String>,
    notebook_id: Option<String>,
}

pub async fn create_note(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Json(body): Json<NewNote>,
) -> ApiResult<Json<Value>> {
    let id = match &body.id {
        Some(id) => parse_id(id).ok_or_else(|| AppError::BadRequest("Invalid note id".into()))?,
        None => new_id(),
    };
    let owner = match &body.notebook_id {
        Some(nb) => {
            require_notebook(&state, &me.id, nb, Role::Editor).await?;
            notebook_owner(&state, nb).await?
        }
        None => me.id.clone(),
    };
    let now = now_ms();
    let result = sqlx::query(
        "INSERT INTO notes (id, owner_id, notebook_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&owner)
    .bind(&body.notebook_id)
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await;
    match result {
        Ok(_) => {}
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(AppError::Conflict("A note with this id already exists".into()));
        }
        Err(e) => return Err(e.into()),
    }
    state.hub.tree_changed();
    Ok(Json(json!({ "id": id })))
}

#[derive(Deserialize)]
pub struct NotePatch {
    #[serde(default, deserialize_with = "double_option")]
    notebook_id: Option<Option<String>>,
}

pub async fn update_note(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<NotePatch>,
) -> ApiResult<Json<Value>> {
    require_note(&state, &me.id, &id, Role::Editor).await?;
    if let Some(target) = &body.notebook_id {
        let owner: String = sqlx::query_scalar("SELECT owner_id FROM notes WHERE id = ?")
            .bind(&id)
            .fetch_one(&state.db)
            .await?;
        check_move_target(&state, &me.id, &owner, target.as_deref()).await?;
        sqlx::query("UPDATE notes SET notebook_id = ?, updated_at = ? WHERE id = ?")
            .bind(target)
            .bind(now_ms())
            .bind(&id)
            .execute(&state.db)
            .await?;
        state.recheck_access().await;
        state.hub.tree_changed();
        state.export.changed();
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct DeleteNoteQuery {
    /// Permanently delete, but only if the note is blank. Used when leaving a new note untouched.
    #[serde(default)]
    discard: bool,
}

pub async fn delete_note(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<DeleteNoteQuery>,
) -> ApiResult<Json<Value>> {
    require_note(&state, &me.id, &id, Role::Owner).await?;
    if query.discard {
        let uuid = uuid::Uuid::parse_str(&id).map_err(|_| AppError::NotFound)?;
        if !crate::rooms::note_body(&state, uuid).await?.trim().is_empty() {
            return Err(AppError::Conflict("The note isn't empty".into()));
        }
        sqlx::query("DELETE FROM notes WHERE id = ?").bind(&id).execute(&state.db).await?;
    } else {
        sqlx::query("UPDATE notes SET deleted_at = ? WHERE id = ?")
            .bind(now_ms())
            .bind(&id)
            .execute(&state.db)
            .await?;
    }
    state.recheck_access().await;
    state.hub.tree_changed();
    state.export.changed();
    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct TrashItem {
    resource_type: String,
    id: String,
    name: String,
    deleted_at: i64,
    /// When `purge_trash` removes it for good.
    purge_at: i64,
}

/// Only the items the owner deleted directly, not everything swept along with a notebook.
pub async fn get_trash(State(state): State<AppState>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Vec<TrashItem>>> {
    let items = sqlx::query_as::<_, TrashItem>(
        "SELECT 'notebook' AS resource_type, nb.id, nb.name, nb.deleted_at, nb.deleted_at + ?2 AS purge_at FROM notebooks nb
         LEFT JOIN notebooks p ON p.id = nb.parent_id
         WHERE nb.owner_id = ?1 AND nb.deleted_at IS NOT NULL
           AND (p.id IS NULL OR p.deleted_at IS NOT nb.deleted_at)
         UNION ALL
         SELECT 'note', n.id, n.title, n.deleted_at, n.deleted_at + ?2 FROM notes n
         LEFT JOIN notebooks p ON p.id = n.notebook_id
         WHERE n.owner_id = ?1 AND n.deleted_at IS NOT NULL
           AND (p.id IS NULL OR p.deleted_at IS NOT n.deleted_at)
         ORDER BY 4 DESC",
    )
    .bind(&me.id)
    .bind(TRASH_MS)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(items))
}

pub async fn restore(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path((kind, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let lookup = match kind.as_str() {
        "notebook" => "SELECT deleted_at FROM notebooks WHERE id = ? AND owner_id = ?",
        "note" => "SELECT deleted_at FROM notes WHERE id = ? AND owner_id = ?",
        _ => return Err(AppError::NotFound),
    };
    let deleted_at: Option<Option<i64>> = sqlx::query_scalar(lookup)
    .bind(&id)
    .bind(&me.id)
    .fetch_optional(&state.db)
    .await?;
    let Some(Some(ts)) = deleted_at else {
        return Err(AppError::NotFound);
    };

    let mut tx = state.db.begin().await?;
    if kind == "notebook" {
        sqlx::query(
            "WITH RECURSIVE sub(id) AS (
                SELECT ?1 UNION SELECT n.id FROM notebooks n JOIN sub ON n.parent_id = sub.id WHERE n.deleted_at = ?2
             )
             UPDATE notes SET deleted_at = NULL WHERE deleted_at = ?2 AND notebook_id IN (SELECT id FROM sub)",
        )
        .bind(&id)
        .bind(ts)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "WITH RECURSIVE sub(id) AS (
                SELECT ?1 UNION SELECT n.id FROM notebooks n JOIN sub ON n.parent_id = sub.id WHERE n.deleted_at = ?2
             )
             UPDATE notebooks SET deleted_at = NULL WHERE deleted_at = ?2 AND id IN (SELECT id FROM sub)",
        )
        .bind(&id)
        .bind(ts)
        .execute(&mut *tx)
        .await?;
        // If the parent is still in the trash, come back at the top level instead.
        sqlx::query(
            "UPDATE notebooks SET parent_id = NULL WHERE id = ?
               AND parent_id IN (SELECT id FROM notebooks WHERE deleted_at IS NOT NULL)",
        )
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(
            "UPDATE notes SET deleted_at = NULL,
               notebook_id = CASE WHEN notebook_id IN (SELECT id FROM notebooks WHERE deleted_at IS NOT NULL)
                             THEN NULL ELSE notebook_id END
             WHERE id = ?",
        )
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    state.hub.tree_changed();
    state.export.changed();
    Ok(Json(json!({ "ok": true })))
}

/// Removes one trashed item for good: a note, or a notebook with everything trashed along with it.
async fn purge_item(tx: &mut sqlx::SqliteConnection, kind: &str, id: &str, ts: i64) -> sqlx::Result<()> {
    if kind == "notebook" {
        sqlx::query(
            "WITH RECURSIVE sub(id) AS (
                SELECT ?1 UNION SELECT n.id FROM notebooks n JOIN sub ON n.parent_id = sub.id WHERE n.deleted_at = ?2
             )
             DELETE FROM notes WHERE deleted_at = ?2 AND notebook_id IN (SELECT id FROM sub)",
        )
            .bind(id)
            .bind(ts)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "WITH RECURSIVE sub(id) AS (
                SELECT ?1 UNION SELECT n.id FROM notebooks n JOIN sub ON n.parent_id = sub.id WHERE n.deleted_at = ?2
             )
             DELETE FROM notebooks WHERE id IN (SELECT id FROM sub)",
        )
            .bind(id)
            .bind(ts)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("DELETE FROM notes WHERE id = ?").bind(id).execute(&mut *tx).await?;
    }
    Ok(())
}

async fn drop_dangling_shares(db: impl sqlx::SqliteExecutor<'_>) -> sqlx::Result<()> {
    sqlx::query(
        "DELETE FROM shares WHERE
           (resource_type = 'note' AND resource_id NOT IN (SELECT id FROM notes)) OR
           (resource_type = 'notebook' AND resource_id NOT IN (SELECT id FROM notebooks))",
    )
    .execute(db)
    .await?;
    Ok(())
}

/// `DELETE /trash/:kind/:id`: deletes one of your trashed items for good, without waiting 30 days.
pub async fn delete_forever(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    Path((kind, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let lookup = match kind.as_str() {
        "notebook" => "SELECT deleted_at FROM notebooks WHERE id = ? AND owner_id = ?",
        "note" => "SELECT deleted_at FROM notes WHERE id = ? AND owner_id = ?",
        _ => return Err(AppError::NotFound),
    };
    let deleted_at: Option<Option<i64>> = sqlx::query_scalar(lookup).bind(&id).bind(&me.id).fetch_optional(&state.db).await?;
    // Only what's already in the trash; live notes go there first.
    let Some(Some(ts)) = deleted_at else {
        return Err(AppError::NotFound);
    };
    let mut tx = state.db.begin().await?;
    purge_item(&mut tx, &kind, &id, ts).await?;
    drop_dangling_shares(&mut *tx).await?;
    tx.commit().await?;
    state.hub.tree_changed();
    state.export.changed();
    Ok(Json(json!({ "ok": true })))
}

/// `DELETE /trash`: empties your trash. Others' trash, and your live notes, are untouched.
pub async fn empty_trash(State(state): State<AppState>, CurrentUser(me): CurrentUser) -> ApiResult<Json<Value>> {
    let Json(items) = get_trash(State(state.clone()), CurrentUser(me)).await?;
    let mut tx = state.db.begin().await?;
    for item in &items {
        purge_item(&mut tx, &item.resource_type, &item.id, item.deleted_at).await?;
    }
    drop_dangling_shares(&mut *tx).await?;
    tx.commit().await?;
    state.hub.tree_changed();
    state.export.changed();
    Ok(Json(json!({ "deleted": items.len() })))
}

/// Permanently removes items that have been in the trash longer than the retention window.
pub async fn purge_trash(db: &sqlx::SqlitePool) -> sqlx::Result<()> {
    let cutoff = now_ms() - TRASH_MS;
    sqlx::query("DELETE FROM notes WHERE deleted_at < ?").bind(cutoff).execute(db).await?;
    sqlx::query("DELETE FROM notebooks WHERE deleted_at < ?").bind(cutoff).execute(db).await?;
    drop_dangling_shares(db).await
}
