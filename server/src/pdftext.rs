//! Reads the text out of attached PDFs (migration 0015), so search and Ask can find it.
//! Extracted at upload in a background blocking task (10 s limit, capped at 200 000 chars) and
//! once at startup for PDFs from before. A PDF with no text layer, or a failed or slow
//! extraction, gets no text; that never fails the upload.

use std::time::Duration;

use crate::AppState;

/// A PDF's text is read out to this many characters at most, so one big manual can't swamp a note.
const MAX_CHARS: usize = 200_000;
/// Extraction gives up after this long; a slow PDF is logged and left without text.
const LIMIT: Duration = Duration::from_secs(10);

/// The text of a PDF, or `None` when it has no text layer (or can't be read).
pub fn extract_pdf_text(bytes: &[u8]) -> Option<String> {
    let doc = lopdf::Document::load_mem(bytes).ok()?;
    let pages: Vec<u32> = doc.get_pages().keys().copied().collect();
    let out = doc.extract_text(&pages).ok()?.trim().to_owned();
    if out.is_empty() {
        None
    } else {
        Some(out.chars().take(MAX_CHARS).collect())
    }
}

/// Stores a PDF's text and re-indexes the notes that link to it, so the text reaches their search
/// text even if a note was indexed before the text was ready.
async fn store_text(state: &AppState, attachment_id: &str, text: String) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO attachment_text (attachment_id, text) VALUES (?, ?)
         ON CONFLICT (attachment_id) DO UPDATE SET text = excluded.text",
    )
    .bind(attachment_id)
    .bind(&text)
    .execute(&state.db)
    .await?;
    reindex_linking_notes(state, attachment_id).await?;
    Ok(())
}

/// Re-indexes the notes that reference `attachment_id`.
async fn reindex_linking_notes(state: &AppState, attachment_id: &str) -> sqlx::Result<()> {
    let note_ids: Vec<String> = sqlx::query_scalar(
        "SELECT note_id FROM note_text WHERE att_refs IS NOT NULL AND att_refs LIKE ? ESCAPE '\\'",
    )
    .bind(format!("%\"{attachment_id}\"%"))
    .fetch_all(&state.db)
    .await?;
    for id in &note_ids {
        let Ok(uuid) = uuid::Uuid::parse_str(id) else { continue };
        match crate::rooms::note_body(state, uuid).await {
            Ok(body) => crate::search::index(state, id, &body).await?,
            Err(e) => tracing::error!("re-indexing note {id} after PDF text: {e:#}"),
        }
    }
    Ok(())
}

/// Extracts a PDF's text and stores it. Runs in the background after an upload and at startup; a
/// failure or a timeout is logged and never fails the upload.
pub async fn extract_and_store(state: AppState, attachment_id: String, bytes: Vec<u8>) {
    let res = tokio::time::timeout(LIMIT, tokio::task::spawn_blocking(move || extract_pdf_text(&bytes))).await;
    match res {
        Ok(Ok(Some(text))) => {
            if let Err(e) = store_text(&state, &attachment_id, text).await {
                tracing::warn!("storing PDF text for {attachment_id}: {e:#}");
            }
        }
        Ok(Ok(None)) => {} // no text layer; fine
        Ok(Err(e)) => tracing::warn!("PDF text extraction failed for {attachment_id}: {e:#}"),
        Err(_) => tracing::warn!("PDF text extraction timed out for {attachment_id}"),
    }
}

/// Reads the text out of PDFs uploaded before this existed, once at startup.
pub async fn backfill(state: &AppState) -> anyhow::Result<()> {
    let missing: Vec<(String, String)> = sqlx::query_as(
        "SELECT a.id, a.sha256 FROM attachments a
         WHERE (a.mime = 'application/pdf' OR a.filename LIKE '%.pdf')
           AND a.id NOT IN (SELECT attachment_id FROM attachment_text)",
    )
    .fetch_all(&state.db)
    .await?;
    for (id, sha) in &missing {
        let path = crate::attachments::blob_path(&state.config.data_dir, sha);
        let Ok(bytes) = std::fs::read(&path) else { continue };
        extract_and_store(state.clone(), id.clone(), bytes).await;
    }
    if !missing.is_empty() {
        tracing::info!("extracted text for {} PDFs", missing.len());
    }
    Ok(())
}


