//! Effective roles, as described under "Permissions" in docs/DESIGN.md.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Editor,
    Owner,
}

impl Role {
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "viewer" => Some(Role::Viewer),
            "editor" => Some(Role::Editor),
            "owner" => Some(Role::Owner),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Editor => "editor",
            Role::Owner => "owner",
        }
    }
}

fn best(roles: impl IntoIterator<Item = String>) -> Option<Role> {
    roles.into_iter().filter_map(|r| Role::parse(&r)).max()
}

/// Ancestor chain of a notebook (bound as ?1), starting with the notebook itself.
macro_rules! ancestors {
    () => {
        "WITH RECURSIVE anc(id, parent_id, depth) AS (
            SELECT id, parent_id, 0 FROM notebooks WHERE id = ?1
            UNION ALL
            SELECT n.id, n.parent_id, anc.depth + 1 FROM notebooks n JOIN anc ON n.id = anc.parent_id
            WHERE anc.depth < 64
        ) "
    };
}

/// The best role granted by shares on this notebook or any of its parents.
async fn notebook_share_role(db: &SqlitePool, user_id: &str, notebook_id: &str) -> sqlx::Result<Option<Role>> {
    let roles: Vec<String> = sqlx::query_scalar(concat!(
        ancestors!(),
        "SELECT role FROM shares
         WHERE user_id = ?2 AND resource_type = 'notebook' AND resource_id IN (SELECT id FROM anc)"
    ))
    .bind(notebook_id)
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok(best(roles))
}

/// `None` means the notebook doesn't exist for this user (missing, trashed, or no access).
pub async fn notebook_role(db: &SqlitePool, user_id: &str, notebook_id: &str) -> sqlx::Result<Option<Role>> {
    let row: Option<(String, Option<i64>)> =
        sqlx::query_as("SELECT owner_id, deleted_at FROM notebooks WHERE id = ?")
            .bind(notebook_id)
            .fetch_optional(db)
            .await?;
    match row {
        None | Some((_, Some(_))) => Ok(None),
        Some((owner, None)) if owner == user_id => Ok(Some(Role::Owner)),
        Some(_) => notebook_share_role(db, user_id, notebook_id).await,
    }
}

/// `None` means the note doesn't exist for this user (missing, trashed, or no access).
pub async fn note_role(db: &SqlitePool, user_id: &str, note_id: &str) -> sqlx::Result<Option<Role>> {
    let row: Option<(String, Option<String>, Option<i64>)> =
        sqlx::query_as("SELECT owner_id, notebook_id, deleted_at FROM notes WHERE id = ?")
            .bind(note_id)
            .fetch_optional(db)
            .await?;
    let (owner, notebook_id) = match row {
        None | Some((_, _, Some(_))) => return Ok(None),
        Some((owner, notebook_id, None)) => (owner, notebook_id),
    };
    if owner == user_id {
        return Ok(Some(Role::Owner));
    }
    let direct: Vec<String> = sqlx::query_scalar(
        "SELECT role FROM shares WHERE user_id = ? AND resource_type = 'note' AND resource_id = ?",
    )
    .bind(user_id)
    .bind(note_id)
    .fetch_all(db)
    .await?;
    let via_notebook = match notebook_id {
        Some(nb) => notebook_share_role(db, user_id, &nb).await?,
        None => None,
    };
    Ok(best(direct).max(via_notebook))
}

/// True if `candidate` is `notebook_id` or one of its descendants. Used to stop cycles when moving.
pub async fn is_self_or_descendant(db: &SqlitePool, notebook_id: &str, candidate: &str) -> sqlx::Result<bool> {
    let hit: Option<i64> = sqlx::query_scalar(concat!(ancestors!(), "SELECT 1 FROM anc WHERE id = ?2 LIMIT 1"))
    .bind(candidate)
    .bind(notebook_id)
    .fetch_optional(db)
    .await?;
    Ok(hit.is_some())
}
