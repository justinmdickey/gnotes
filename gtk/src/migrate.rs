//! Move to a Server: everything in the notes on this computer goes to a server account, as it is.
//! Notebooks are made there in the same shape; notes keep their ids, and their Loro documents go
//! over whole, history and all, so nothing is flattened into Markdown on the way; photos and
//! recordings are uploaded and their `att:` links pointed at the new copies. Trashed things stay.

use std::{collections::HashMap, time::Duration};

use loro::VersionVector;

use crate::{
    api::{Api, Role},
    note::NoteDoc,
    sync::{Event, Status, SyncClient},
};

/// How a move went.
#[derive(Debug, Default, PartialEq)]
pub struct Moved {
    pub notebooks: usize,
    pub notes: usize,
    pub attachments: usize,
}

/// A note's document, as a server has it: joins it and takes the catch-up.
async fn fetch(sync: &SyncClient, events: &async_channel::Receiver<Event>, note: &str) -> anyhow::Result<NoteDoc> {
    sync.join(note, None);
    let mut doc = NoteDoc::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match events.recv().await? {
                Event::Update { note: n, data } if n == note => {
                    doc.import(&data)?;
                    return anyhow::Ok(());
                }
                Event::Error { note: Some(n), code } if n == note => anyhow::bail!("reading a note: {code}"),
                _ => {}
            }
        }
    })
    .await??;
    sync.leave(note);
    Ok(doc)
}

/// Sends `doc` into an empty note on a server, and waits until the server has all of it.
async fn push(sync: &SyncClient, events: &async_channel::Receiver<Event>, note: &str, doc: &NoteDoc) -> anyhow::Result<()> {
    let ours = doc.version();
    for _ in 0..5 {
        sync.join(note, None);
        let server: VersionVector = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                match events.recv().await? {
                    Event::Joined { note: n, version, .. } if n == note => return anyhow::Ok(version),
                    Event::Error { note: Some(n), code } if n == note => anyhow::bail!("writing a note: {code}"),
                    _ => {}
                }
            }
        })
        .await??;
        if server >= ours {
            sync.leave(note);
            return Ok(());
        }
        sync.send_update(note, &doc.export_from(&server));
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    anyhow::bail!("the server didn't take a note")
}

async fn online(events: &async_channel::Receiver<Event>) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Event::Status(Status::Online) = events.recv().await? {
                return anyhow::Ok(());
            }
        }
    })
    .await?
}

/// The attachment ids a note's text links to, in order.
fn attachment_ids(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (i, _) in text.match_indices("att:") {
        let id: String = text[i + 4..].chars().take_while(|c| c.is_ascii_hexdigit() || *c == '-').collect();
        if id.len() == 36 && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// Moves everything from `from` (the notes on this computer) to `to` (a server account).
/// `progress` hears (done, total) notes. Must run inside the tokio runtime.
pub async fn move_all(from: &Api, to: &Api, progress: impl Fn(usize, usize)) -> anyhow::Result<Moved> {
    let tree = from.tree().await?;
    let mut moved = Moved::default();

    // Notebooks, parents first.
    let mut ids: HashMap<String, String> = HashMap::new();
    let mut left: Vec<_> = tree.notebooks.iter().filter(|n| n.role == Role::Owner).cloned().collect();
    while !left.is_empty() {
        let before = left.len();
        let mut next = Vec::new();
        for nb in left {
            let parent = match &nb.parent_id {
                None => None,
                Some(p) => match ids.get(p) {
                    Some(id) => Some(id.clone()),
                    // Its parent isn't made yet; next round.
                    None if tree.notebooks.iter().any(|n| &n.id == p) => {
                        next.push(nb);
                        continue;
                    }
                    None => None,
                },
            };
            let id = to.create_notebook(&nb.name, parent.as_deref()).await?;
            ids.insert(nb.id.clone(), id);
            moved.notebooks += 1;
        }
        if next.len() == before {
            break;
        }
        left = next;
    }

    let (from_sync, from_events) = SyncClient::start(from.clone());
    let (to_sync, to_events) = SyncClient::start(to.clone());
    online(&from_events).await?;
    online(&to_events).await?;

    let notes: Vec<_> = tree.notes.iter().filter(|n| n.role == Role::Owner).cloned().collect();
    for (i, note) in notes.iter().enumerate() {
        progress(i, notes.len());
        let notebook = note.notebook_id.as_ref().and_then(|n| ids.get(n)).cloned();
        // The same id where the server has no note by it; a new one where it does.
        let id = match to.create_note(notebook.as_deref(), Some(&note.id)).await {
            Ok(id) => id,
            Err(_) => to.create_note(notebook.as_deref(), None).await?,
        };
        let doc = fetch(&from_sync, &from_events, &note.id).await?;
        for att in attachment_ids(&doc.text()) {
            let Ok(meta) = from.attachment_meta(&att).await else { continue };
            let bytes = from.attachment(&att).await?;
            let copy = to.upload(&id, &meta.filename, &meta.mime, bytes).await?;
            moved.attachments += 1;
            // Every link to it, back to front so earlier places stay put.
            let text = doc.text();
            let mut at: Vec<usize> = text.match_indices(&format!("att:{att}")).map(|(b, _)| text[..b].chars().count() + 4).collect();
            at.reverse();
            for pos in at {
                doc.replace(pos, 36, &copy.id);
            }
        }
        let _ = doc.take_outbox();
        push(&to_sync, &to_events, &id, &doc).await?;
        moved.notes += 1;
    }
    progress(notes.len(), notes.len());
    from_sync.stop();
    to_sync.stop();
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_attachment_links() {
        let a = "0192f0c4-5b7e-7a3c-9d2e-112233445566";
        let text = format!("![p](att:{a}) and ![q](att:{a}) and att:short");
        assert_eq!(attachment_ids(&text), vec![a.to_owned()]);
    }
}
