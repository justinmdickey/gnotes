//! Bringing existing Markdown notes in: a zip of folders, or loose .md files. See "Import" in docs/DESIGN.md.

use std::{
    collections::HashMap,
    io::{Cursor, Read},
};

use axum::{
    Json,
    extract::{Multipart, State},
};
use serde::Serialize;

use crate::{
    AppState,
    attachments::{allowed, clean_filename, mime_for, store_blob},
    auth::CurrentUser,
    error::{ApiResult, AppError},
    perms::Role,
    rooms::{seed, summarize},
    tree::{notebook_owner, require_notebook},
    util::{new_id, now_ms, parse_id},
};

/// Big enough for a few years of notes with photos.
pub const MAX_IMPORT: usize = 200 * 1024 * 1024;
/// Stops a small zip from unpacking into something huge.
const MAX_UNPACKED: u64 = 1024 * 1024 * 1024;

/// A Markdown file found in the upload.
struct Doc {
    /// Folders above the file, which become notebooks.
    dir: Vec<String>,
    /// File name without its extension.
    stem: String,
    text: String,
    modified: Option<i64>,
}

#[derive(Default)]
struct Unpacked {
    docs: Vec<Doc>,
    /// Everything else, by full path, for resolving image links.
    files: HashMap<String, Vec<u8>>,
    skipped: Vec<String>,
}

impl Unpacked {
    /// Finds a linked file next to the note, or anywhere by name the way Obsidian does.
    fn resolve(&self, dir: &[String], target: &str) -> Option<String> {
        let mut parts: Vec<&str> = dir.iter().map(String::as_str).collect();
        for part in target.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                p => parts.push(p),
            }
        }
        let path = parts.join("/");
        if self.files.contains_key(&path) {
            return Some(path);
        }
        let name = target.rsplit('/').next()?.to_lowercase();
        let mut found: Vec<&String> =
            self.files.keys().filter(|k| k.rsplit('/').next().is_some_and(|n| n.to_lowercase() == name)).collect();
        found.sort();
        found.first().map(|k| (*k).clone())
    }
}

#[derive(Serialize, Default)]
pub struct ImportResult {
    notes: usize,
    notebooks: usize,
    attachments: usize,
    /// Files that weren't Markdown or an image or recording a note links to.
    skipped: Vec<String>,
    /// The notebook the first zip became, so the app can open it.
    notebook_id: Option<String>,
}

fn is_note(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".txt")
}

fn stem(name: &str) -> String {
    name.rsplit_once('.').map_or(name, |(s, _)| s).trim().to_owned()
}

fn text(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    s.trim_start_matches('\u{feff}').replace("\r\n", "\n")
}

/// A zip entry's modified time in unix millis. Zips don't record a time zone, so this reads it as UTC.
fn zip_time(t: zip::DateTime) -> Option<i64> {
    let month = time::Month::try_from(t.month()).ok()?;
    let date = time::Date::from_calendar_date(t.year().into(), month, t.day()).ok()?;
    let at = date.with_hms(t.hour(), t.minute(), t.second()).ok()?.assume_utc();
    Some(at.unix_timestamp() * 1000)
}

/// Reads a zip into notes and files. Folders and files starting with "." (e.g. .obsidian) are app data, not notes.
fn unzip(bytes: Vec<u8>, zip_name: &str) -> Result<Unpacked, AppError> {
    let bad = || AppError::BadRequest(format!("{zip_name} isn't a zip file that can be read"));
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| bad())?;
    let mut out = Unpacked::default();
    let mut unpacked = 0u64;
    let mut entries = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|_| bad())?;
        if file.is_dir() {
            continue;
        }
        let Some(path) = file.enclosed_name() else { continue };
        let parts: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        if parts.iter().any(|p| p.starts_with('.') || p == "__MACOSX") {
            continue;
        }
        unpacked += file.size();
        if unpacked > MAX_UNPACKED {
            return Err(AppError::BadRequest(format!("{zip_name} unpacks to more than 1 GB")));
        }
        let size = file.size();
        let mut data = Vec::with_capacity(size as usize);
        file.by_ref().take(size).read_to_end(&mut data).map_err(|_| bad())?;
        let modified = file.last_modified().and_then(zip_time);
        entries.push((parts, data, modified));
    }
    // An export usually wraps everything in one folder; that folder is the notebook. Otherwise the zip is.
    let wrapped = entries.first().is_some_and(|(first, _, _)| {
        entries.iter().all(|(p, _, _)| p.len() > 1 && p[0] == first[0])
    });
    for (mut parts, data, modified) in entries {
        if !wrapped {
            parts.insert(0, stem(zip_name));
        }
        let name = parts.pop().unwrap_or_default();
        if is_note(&name) {
            out.docs.push(Doc { dir: parts, stem: stem(&name), text: text(&data), modified });
        } else {
            parts.push(name);
            out.files.insert(parts.join("/"), data);
        }
    }
    Ok(out)
}

/// Splits off YAML front matter, returning its `title:` if it has one.
fn front_matter(text: &str) -> (Option<String>, &str) {
    let Some(rest) = text.strip_prefix("---\n") else { return (None, text) };
    let Some(end) = rest.find("\n---\n").map(|i| (i, i + 5)).or_else(|| rest.strip_suffix("\n---").map(|r| (r.len(), rest.len())))
    else {
        return (None, text);
    };
    let title = rest[..end.0].lines().find_map(|l| l.strip_prefix("title:")).map(|t| t.trim().trim_matches(['"', '\'']).to_owned());
    (title.filter(|t| !t.is_empty()), rest[end.1..].trim_start_matches('\n'))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Rewrites `![alt](path)` and Obsidian's `![[path|alt]]` to `![alt](att:<id>)` when the file is in the upload.
/// `link` returns the attachment id for a linked file, or None to leave the link alone.
fn rewrite_links(body: &str, link: &mut impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("![") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let replaced = if let Some(inner) = after.strip_prefix('[') {
            inner.find("]]").and_then(|end| {
                let (target, alt) = inner[..end].split_once('|').unwrap_or((&inner[..end], ""));
                // Obsidian's "|300" is a width, not a caption.
                let alt = if alt.chars().all(|c| c.is_ascii_digit() || c == 'x') { "" } else { alt };
                let id = link(target.trim())?;
                Some((format!("![{alt}](att:{id})"), 1 + end + 2))
            })
        } else {
            after.find("](").and_then(|mid| {
                let alt = &after[..mid];
                let close = after[mid + 2..].find(')')?;
                let raw = after[mid + 2..mid + 2 + close].trim();
                let target = match raw.strip_prefix('<') {
                    Some(r) => r.split_once('>').map_or(r, |(t, _)| t),
                    None => raw.split_once(" \"").map_or(raw, |(t, _)| t),
                };
                if alt.contains('\n') || target.contains("://") || target.starts_with("att:") || target.starts_with("data:") {
                    return None;
                }
                let id = link(&percent_decode(target))?;
                Some((format!("![{alt}](att:{id})"), mid + 2 + close + 1))
            })
        };
        match replaced {
            Some((text, used)) => {
                out.push_str(&text);
                rest = &after[used..];
            }
            None => {
                out.push_str("![");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The note's Markdown: front matter dropped, a title line added when the file has none.
fn note_body(doc: &Doc, link: &mut impl FnMut(&str) -> Option<String>) -> String {
    let (fm_title, body) = front_matter(&doc.text);
    let body = rewrite_links(body.trim_end(), link);
    let first = body.lines().find(|l| !l.trim().is_empty()).unwrap_or_default();
    let title = fm_title.unwrap_or_else(|| doc.stem.clone());
    if first.trim_start().starts_with('#') || summarize(&body).0 == summarize(&title).0 || title.is_empty() {
        body
    } else if body.is_empty() {
        format!("# {title}")
    } else {
        format!("# {title}\n\n{body}")
    }
}

/// `POST /import`, multipart with an optional `notebook_id` then one or more `file`s (.zip, .md, .txt).
/// A zip becomes a notebook with its folders as notebooks inside; loose files land in the target notebook.
pub async fn import(
    State(state): State<AppState>,
    CurrentUser(me): CurrentUser,
    mut form: Multipart,
) -> ApiResult<Json<ImportResult>> {
    let bad = |m: &str| AppError::BadRequest(m.into());
    let mut target: Option<String> = None;
    let mut loose = Unpacked::default();
    let mut zips = Vec::new();
    while let Some(field) = form.next_field().await.map_err(|e| bad(&e.body_text()))? {
        match field.name() {
            Some("notebook_id") => {
                let text = field.text().await.map_err(|e| bad(&e.body_text()))?;
                let id = parse_id(&text).ok_or_else(|| bad("Unknown notebook"))?;
                require_notebook(&state, &me.id, &id, Role::Editor).await?;
                target = Some(id);
            }
            Some("file") => {
                let name = clean_filename(field.file_name().unwrap_or(""));
                let bytes = field.bytes().await.map_err(|e| bad(&e.body_text()))?.to_vec();
                if name.to_lowercase().ends_with(".zip") || bytes.starts_with(b"PK\x03\x04") {
                    zips.push((name, bytes));
                } else if is_note(&name) {
                    loose.docs.push(Doc { dir: Vec::new(), stem: stem(&name), text: text(&bytes), modified: None });
                } else {
                    loose.skipped.push(name);
                }
            }
            _ => {}
        }
    }
    let mut batches = vec![loose];
    for (name, bytes) in zips {
        let unpacked = tokio::task::spawn_blocking(move || unzip(bytes, &name)).await.map_err(anyhow::Error::from)??;
        batches.push(unpacked);
    }
    if batches.iter().all(|b| b.docs.is_empty()) {
        return Err(bad("No Markdown notes found. Import .md files, or a .zip of folders with .md files."));
    }

    let owner = match &target {
        Some(nb) => notebook_owner(&state, nb).await?,
        None => me.id.clone(),
    };
    let now = now_ms();
    let mut result = ImportResult::default();
    let mut tx = state.db.begin().await?;
    for mut batch in batches {
        batch.docs.sort_by(|a, b| (&a.dir, &a.stem).cmp(&(&b.dir, &b.stem)));
        let mut notebooks: HashMap<Vec<String>, String> = HashMap::new();
        let mut used = std::collections::HashSet::new();
        for doc in &batch.docs {
            // Each folder on the way down becomes a notebook, once.
            let mut parent = target.clone();
            for depth in 1..=doc.dir.len() {
                let path = doc.dir[..depth].to_vec();
                if let Some(id) = notebooks.get(&path) {
                    parent = Some(id.clone());
                    continue;
                }
                let id = new_id();
                let name: String = path[depth - 1].trim().chars().take(200).collect();
                sqlx::query(
                    "INSERT INTO notebooks (id, owner_id, parent_id, name, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(&owner)
                .bind(&parent)
                .bind(if name.is_empty() { "Imported".into() } else { name })
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                if depth == 1 && result.notebook_id.is_none() {
                    result.notebook_id = Some(id.clone());
                }
                result.notebooks += 1;
                notebooks.insert(path, id.clone());
                parent = Some(id);
            }

            let note_id = new_id();
            let mut linked: HashMap<String, String> = HashMap::new();
            let body = note_body(doc, &mut |target| {
                let path = batch.resolve(&doc.dir, target)?;
                let mime = mime_for(&path)?;
                if !allowed(mime) {
                    return None;
                }
                Some(linked.entry(path).or_insert_with(new_id).clone())
            });
            let (title, preview) = summarize(&body);
            let when = doc.modified.unwrap_or(now);
            sqlx::query(
                "INSERT INTO notes (id, owner_id, notebook_id, title, preview, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&note_id)
            .bind(&owner)
            .bind(&parent)
            .bind(&title)
            .bind(&preview)
            .bind(when)
            .bind(when)
            .execute(&mut *tx)
            .await?;
            seed(&mut *tx, &note_id, &body).await?;
            for (path, att_id) in linked {
                used.insert(path.clone());
                let bytes = &batch.files[&path];
                let sha = store_blob(&state, bytes).await?;
                let filename = clean_filename(&path);
                sqlx::query(
                    "INSERT INTO attachments (id, note_id, uploader_id, filename, mime, size, sha256, created_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&att_id)
                .bind(&note_id)
                .bind(&me.id)
                .bind(&filename)
                .bind(mime_for(&path))
                .bind(bytes.len() as i64)
                .bind(&sha)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                result.attachments += 1;
            }
            result.notes += 1;
        }
        let mut unused: Vec<String> = batch.files.into_keys().filter(|p| !used.contains(p)).collect();
        unused.sort();
        result.skipped.append(&mut batch.skipped);
        result.skipped.append(&mut unused);
    }
    tx.commit().await?;
    state.hub.tree_changed();
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(stem: &str, text: &str) -> String {
        let doc = Doc { dir: Vec::new(), stem: stem.into(), text: text.into(), modified: None };
        note_body(&doc, &mut |t| (t == "img.png").then(|| "ID".to_owned()))
    }

    #[test]
    fn adds_a_title_only_when_missing() {
        assert_eq!(body("Groceries", "milk\neggs"), "# Groceries\n\nmilk\neggs");
        assert_eq!(body("Groceries", "# Shopping\nmilk"), "# Shopping\nmilk");
        assert_eq!(body("Groceries", "Groceries\nmilk"), "Groceries\nmilk");
        assert_eq!(body("Empty", ""), "# Empty");
        assert_eq!(body("x", "---\ntitle: \"Real title\"\ntags: [a]\n---\nbody"), "# Real title\n\nbody");
    }

    #[test]
    fn links_to_files_in_the_upload_become_attachments() {
        assert_eq!(body("a", "# a\n![cat](img.png) ![[img.png|300]] ![x](missing.png)"), "# a\n![cat](att:ID) ![](att:ID) ![x](missing.png)");
        assert_eq!(body("a", "# a\n![](<img.png>) ![](https://x/img.png)"), "# a\n![](att:ID) ![](https://x/img.png)");
    }
}
