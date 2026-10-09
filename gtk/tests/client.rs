//! The client's non-GTK parts against a real in-process server: sign-in, the tree, live sync,
//! presence, offline edits and sharing.

use std::time::Duration;

use gnotes_gtk::{
    api::{Api, Role},
    note::NoteDoc,
    sync::{Event, Status, SyncClient},
};
use gnotes_server::{AppState, Config, auth, build, serve};
use tempfile::TempDir;

struct Server {
    base: String,
    state: AppState,
    _dir: TempDir,
}

async fn start() -> Server {
    start_with(None).await
}

async fn start_with(whisper: Option<gnotes_server::WhisperConfig>) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        vision: None,
        summary: None,
        embed: None,
        whisper,
        data_dir: dir.path().to_path_buf(),
        bind: "127.0.0.1:0".parse().unwrap(),
        web_dir: dir.path().join("web"),
        public_url: None,
    };
    let state = build(config).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(serve(state.clone(), listener));
    Server { base, state, _dir: dir }
}

async fn user(server: &Server, name: &str) -> Api {
    auth::create_user(&server.state.db, name, name, "password123", false).await.unwrap();
    Api::login(&server.base, name, "password123").await.unwrap().0
}

/// The next event that `want` picks out, skipping others.
async fn wait<T>(events: &async_channel::Receiver<Event>, mut want: impl FnMut(Event) -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(t) = want(events.recv().await.unwrap()) {
                return t;
            }
        }
    })
    .await
    .expect("timed out waiting for an event")
}

async fn online(events: &async_channel::Receiver<Event>) {
    wait(events, |e| matches!(e, Event::Status(Status::Online)).then_some(())).await;
}

/// Joins `note` and imports the catch-up the server sends.
async fn join(sync: &SyncClient, events: &async_channel::Receiver<Event>, note: &str, doc: &mut NoteDoc) -> Role {
    let version = doc.has_data.then(|| doc.version());
    sync.join(note, version.as_ref());
    let role = wait(events, |e| match e {
        Event::Joined { role, .. } => Some(role),
        _ => None,
    })
    .await;
    let data = wait(events, |e| match e {
        Event::Update { data, .. } => Some(data),
        _ => None,
    })
    .await;
    doc.import(&data).unwrap();
    role
}

fn send(sync: &SyncClient, note: &str, doc: &NoteDoc) {
    for u in doc.take_outbox() {
        assert!(sync.send_update(note, &u));
    }
}

#[tokio::test]
async fn signs_in_and_manages_the_tree() {
    let server = start().await;
    auth::create_user(&server.state.db, "ann", "Ann", "password123", false).await.unwrap();
    let err = Api::login(&server.base, "ann", "nope").await.err().unwrap();
    assert_eq!(err.to_string(), "Wrong username or password");
    let (api, me) = Api::login(&format!("{}/", server.base), "ann", "password123").await.unwrap();
    assert_eq!(me.display_name, "Ann");
    assert_eq!(api.me().await.unwrap().username, "ann");

    let home = api.create_notebook("Home", None).await.unwrap();
    let kitchen = api.create_notebook("Kitchen", Some(&home)).await.unwrap();
    let note = api.create_note(Some(&kitchen), Some(&uuid::Uuid::now_v7().to_string())).await.unwrap();
    api.rename_notebook(&kitchen, "Cooking").await.unwrap();
    let tree = api.tree().await.unwrap();
    assert!(tree.notebooks.iter().any(|n| n.name == "Cooking" && n.parent_id.as_deref() == Some(&home)));
    assert_eq!(tree.notes[0].notebook_id.as_deref(), Some(kitchen.as_str()));

    api.move_note(&note, None).await.unwrap();
    api.delete_notebook(&home).await.unwrap();
    assert!(api.tree().await.unwrap().notebooks.is_empty());
    let trash = api.trash().await.unwrap();
    assert_eq!(trash.len(), 1);
    api.restore("notebook", &home).await.unwrap();
    assert_eq!(api.tree().await.unwrap().notebooks.len(), 2);

    api.logout().await.unwrap();
    assert!(matches!(api.tree().await, Err(gnotes_gtk::api::Error::Unauthorized)));
}

#[tokio::test]
async fn two_clients_edit_one_note_live() {
    let server = start().await;
    let api = user(&server, "ann").await;
    let note = api.create_note(None, None).await.unwrap();

    let (a, a_events) = SyncClient::start(api.clone());
    let (b, b_events) = SyncClient::start(api.clone());
    online(&a_events).await;
    online(&b_events).await;

    let mut doc_a = NoteDoc::new();
    assert_eq!(join(&a, &a_events, &note, &mut doc_a).await, Role::Owner);
    doc_a.insert(0, "# Groceries\n");
    send(&a, &note, &doc_a);

    let mut doc_b = NoteDoc::new();
    join(&b, &b_events, &note, &mut doc_b).await;
    // The catch-up may come before A's edit lands; then it arrives as an update.
    while doc_b.text() != "# Groceries\n" {
        let data = wait(&b_events, |e| match e {
            Event::Update { data, .. } => Some(data),
            _ => None,
        })
        .await;
        doc_b.import(&data).unwrap();
    }

    doc_b.insert(12, "- milk 🥛");
    send(&b, &note, &doc_b);
    let data = wait(&a_events, |e| match e {
        Event::Update { data, .. } => Some(data),
        _ => None,
    })
    .await;
    let changes = doc_a.import(&data).unwrap();
    assert_eq!(changes, vec![vec![gnotes_gtk::note::Op::Retain(12), gnotes_gtk::note::Op::Insert("- milk 🥛".into())]]);
    assert_eq!(doc_a.text(), "# Groceries\n- milk 🥛");

    // Presence, in loro-codemirror's shape.
    doc_a.set_user("Ann", "user-red");
    doc_a.set_cursor(Some((3, 3)));
    for p in doc_a.take_presence_out() {
        a.send_presence(&note, &p);
    }
    while doc_b.peers().is_empty() || doc_b.peers()[0].cursor.is_none() {
        let data = wait(&b_events, |e| match e {
            Event::Presence { data, .. } => Some(data),
            _ => None,
        })
        .await;
        doc_b.apply_presence(&data);
    }
    assert_eq!(doc_b.peers()[0].name, "Ann");
    assert_eq!(doc_b.peers()[0].cursor, Some((3, 3)));

    // The server's title follows the text, and the tree says so.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let tree = api.tree().await.unwrap();
    assert_eq!(tree.notes[0].title, "Groceries");
    let found = api.search("milk").await.unwrap();
    assert_eq!(found[0].note, note);
}

#[tokio::test]
async fn offline_edits_merge_on_rejoin() {
    let server = start().await;
    let api = user(&server, "ann").await;
    let note = api.create_note(None, None).await.unwrap();

    let (a, events) = SyncClient::start(api.clone());
    online(&events).await;
    let mut doc = NoteDoc::new();
    join(&a, &events, &note, &mut doc).await;
    doc.insert(0, "online ");
    send(&a, &note, &doc);
    tokio::time::sleep(Duration::from_millis(200)).await;
    a.stop();

    // Typed with no connection, then saved and reopened, as the app does with its offline copy.
    doc.insert(7, "offline");
    let _ = doc.take_outbox();
    let mut reopened = NoteDoc::new();
    reopened.import(&doc.snapshot()).unwrap();

    let (a, events) = SyncClient::start(api.clone());
    online(&events).await;
    a.join(&note, Some(&reopened.version()));
    let server_vv = wait(&events, |e| match e {
        Event::Joined { version, .. } => Some(version),
        _ => None,
    })
    .await;
    assert!(reopened.version() > server_vv, "the server lacks the offline edit");
    assert!(a.send_update(&note, &reopened.export_from(&server_vv)));
    tokio::time::sleep(Duration::from_millis(200)).await;

    let (b, b_events) = SyncClient::start(api.clone());
    online(&b_events).await;
    let mut fresh = NoteDoc::new();
    join(&b, &b_events, &note, &mut fresh).await;
    assert_eq!(fresh.text(), "online offline");
}

#[tokio::test]
async fn notes_made_offline_keep_their_id() {
    let server = start().await;
    let api = user(&server, "ann").await;
    let id = uuid::Uuid::now_v7().to_string();
    assert_eq!(api.create_note(None, Some(&id)).await.unwrap(), id);
    assert_eq!(api.tree().await.unwrap().notes[0].id, id);
}

#[tokio::test]
async fn sharing_shows_up_for_the_other_account() {
    let server = start().await;
    let ann = user(&server, "ann").await;
    let bob = user(&server, "bob").await;
    let note = ann.create_note(None, None).await.unwrap();
    let users = ann.users().await.unwrap();
    assert!(users.iter().any(|u| u.username == "bob"));

    ann.share("note", &note, "bob", Role::Viewer).await.unwrap();
    let shares = ann.shares("note", &note).await.unwrap();
    assert_eq!((shares[0].username.as_str(), shares[0].role), ("bob", Role::Viewer));
    let tree = bob.tree().await.unwrap();
    assert_eq!(tree.shared[0].resource_id, note);
    assert_eq!(tree.notes[0].role, Role::Viewer);

    // A viewer can't edit over the websocket.
    let (b, events) = SyncClient::start(bob.clone());
    online(&events).await;
    let mut doc = NoteDoc::new();
    assert_eq!(join(&b, &events, &note, &mut doc).await, Role::Viewer);
    doc.insert(0, "nope");
    send(&b, &note, &doc);
    let code = wait(&events, |e| match e {
        Event::Error { code, .. } => Some(code),
        _ => None,
    })
    .await;
    assert_eq!(code, "forbidden");

    ann.set_share_role(&shares[0].id, Role::Editor).await.unwrap();
    let role = wait(&events, |e| match e {
        Event::Role { role, .. } => Some(role),
        _ => None,
    })
    .await;
    assert_eq!(role, Role::Editor);

    ann.unshare(&shares[0].id).await.unwrap();
    wait(&events, |e| matches!(e, Event::Revoked { .. }).then_some(())).await;
    assert!(bob.tree().await.unwrap().notes.is_empty());
}

#[tokio::test]
async fn photos_upload_and_download() {
    let server = start().await;
    let api = user(&server, "ann").await;
    let note = api.create_note(None, None).await.unwrap();
    // A 1×1 PNG.
    let png = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M8AAAMBAQAY3Y2wAAAAAElFTkSuQmCC",
    )
    .unwrap();
    let att = api.upload(&note, "dot.png", "image/png", png.clone()).await.unwrap();
    assert_eq!(att.mime, "image/png");
    assert_eq!(api.attachment(&att.id).await.unwrap(), png);
    assert_eq!(api.attachment_meta(&att.id).await.unwrap().note_id, note);
}

/// A realtime speech-to-text service, as OpenAI's events go: one delta once audio arrives, and the
/// finished text when the recording is committed.
async fn fake_realtime() -> String {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
                let mut heard = false;
                while let Some(Ok(Message::Text(text))) = ws.next().await {
                    let event: serde_json::Value = serde_json::from_str(&text).unwrap();
                    match event["type"].as_str() {
                        Some("input_audio_buffer.append") if !heard => {
                            heard = true;
                            let delta = serde_json::json!({ "type": "conversation.item.input_audio_transcription.delta", "delta": "Call the" });
                            ws.send(Message::text(delta.to_string())).await.unwrap();
                        }
                        Some("input_audio_buffer.commit") => {
                            let done = serde_json::json!({ "type": "conversation.item.input_audio_transcription.completed", "transcript": "Call the plumber." });
                            ws.send(Message::text(done.to_string())).await.unwrap();
                            let _ = ws.close(None).await;
                            return;
                        }
                        _ => {}
                    }
                }
            });
        }
    });
    url
}

#[tokio::test]
async fn live_transcripts_come_back_as_words_then_the_finished_text() {
    let realtime = fake_realtime().await;
    let server = start_with(Some(gnotes_server::WhisperConfig {
        url: "http://127.0.0.1:9/v1".into(),
        model: "stand-in".into(),
        key: None,
        realtime_url: Some(realtime),
    }))
    .await;
    let api = user(&server, "ann").await;
    assert!(api.features().await.unwrap().live_transcription);
    let (pcm_tx, pcm_rx) = tokio::sync::mpsc::unbounded_channel();
    let (ev_tx, ev_rx) = async_channel::unbounded();
    let session = tokio::spawn(gnotes_gtk::recorder::live(api, pcm_rx, ev_tx));
    // A tenth of a second of silence at a time, as the recorder sends it.
    for _ in 0..3 {
        pcm_tx.send(vec![0u8; 3200]).unwrap();
    }
    let first = tokio::time::timeout(Duration::from_secs(5), ev_rx.recv()).await.unwrap().unwrap();
    assert_eq!(first, gnotes_gtk::recorder::LiveEvent::Delta("Call the".into()));
    drop(pcm_tx);
    let last = tokio::time::timeout(Duration::from_secs(5), ev_rx.recv()).await.unwrap().unwrap();
    assert_eq!(last, gnotes_gtk::recorder::LiveEvent::Final("Call the plumber.".into()));
    tokio::time::timeout(Duration::from_secs(10), session).await.unwrap().unwrap();
}

#[test]
fn records_from_a_gstreamer_source() {
    // A tone stands in for the microphone.
    unsafe { std::env::set_var("GNOTES_AUDIO_SRC", "audiotestsrc is-live=true wave=sine") };
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let rec = gnotes_gtk::recorder::Recording::start(dir.path(), Some(tx)).unwrap();
    std::thread::sleep(Duration::from_millis(700));
    let bytes = gtk::glib::MainContext::default().block_on(rec.stop()).unwrap();
    // WebM starts with the EBML header.
    assert_eq!(&bytes[..4], &[0x1a, 0x45, 0xdf, 0xa3]);
    assert!(bytes.len() > 1000, "{} bytes", bytes.len());
    let mut pcm = 0;
    while let Ok(chunk) = rx.try_recv() {
        pcm += chunk.len();
    }
    // About 0.7 s of 16 kHz 16-bit mono.
    assert!(pcm > 10_000, "{pcm} bytes of PCM");
}

#[test]
fn a_recording_stopped_at_once_still_stops() {
    unsafe { std::env::set_var("GNOTES_AUDIO_SRC", "audiotestsrc is-live=true wave=sine") };
    let dir = tempfile::tempdir().unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let path = dir.path().to_path_buf();
    std::thread::spawn(move || {
        let rec = gnotes_gtk::recorder::Recording::start(&path, None).unwrap();
        let out = gtk::glib::MainContext::default().block_on(rec.stop());
        let _ = done_tx.send(out.map(|b| b.len()));
    });
    let out = done_rx.recv_timeout(Duration::from_secs(10)).expect("stopping hung");
    let _ = out;
}
