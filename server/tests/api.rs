use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use gnotes_server::{AppState, Config, WhisperConfig, auth, build, serve};
use loro::{ExportMode, LoroDoc, VersionVector, awareness::EphemeralStore};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use uuid::Uuid;

struct Server {
    base: String,
    state: AppState,
    task: JoinHandle<anyhow::Result<()>>,
}

async fn start(dir: &TempDir) -> Server {
    start_with(dir, None).await
}

async fn start_with(dir: &TempDir, whisper: Option<WhisperConfig>) -> Server {
    let config = Config {
        vision: None,
        summary: None,
        data_dir: dir.path().to_path_buf(),
        bind: "127.0.0.1:0".parse().unwrap(),
        web_dir: dir.path().join("web"),
        public_url: None,
        whisper,
    };
    let state = build(config).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
    let task = tokio::spawn(serve(state.clone(), listener));
    Server { base, state, task }
}

struct User {
    http: reqwest::Client,
    cookie: String,
    base: String,
}

impl User {
    fn url(&self, path: &str) -> String {
        format!("http://{}/api{path}", self.base)
    }

    async fn get(&self, path: &str) -> Value {
        let res = self.http.get(self.url(path)).send().await.unwrap();
        assert!(res.status().is_success(), "GET {path}: {}", res.status());
        res.json().await.unwrap()
    }

    async fn send(&self, method: reqwest::Method, path: &str, body: Value) -> reqwest::Response {
        self.http.request(method, self.url(path)).json(&body).send().await.unwrap()
    }

    async fn post(&self, path: &str, body: Value) -> Value {
        let res = self.send(reqwest::Method::POST, path, body).await;
        assert!(res.status().is_success(), "POST {path}: {}", res.status());
        res.json().await.unwrap()
    }

    async fn ws(&self) -> Ws {
        let mut req = format!("ws://{}/api/ws", self.base).into_client_request().unwrap();
        req.headers_mut().insert("cookie", self.cookie.parse().unwrap());
        let (stream, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        Ws { stream }
    }
}

async fn login(base: &str, username: &str) -> User {
    let http = reqwest::Client::builder().cookie_store(true).build().unwrap();
    let res = http
        .post(format!("http://{base}/api/auth/login"))
        .json(&json!({ "username": username, "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success(), "login {username}: {}", res.status());
    let cookie = res.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_owned();
    User { http, cookie, base: base.to_owned() }
}

async fn user(server: &Server, username: &str) -> User {
    auth::create_user(&server.state.db, username, "", "password123", false).await.unwrap();
    login(&server.base, username).await
}

enum Msg {
    Control(Value),
    Frame(u8, Vec<u8>),
}

type Stream = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Ws {
    stream: Stream,
}

impl Ws {
    async fn join(&mut self, note: &str, version: Option<&VersionVector>) {
        let version = version.map(|v| STANDARD.encode(v.encode()));
        self.send_json(json!({ "t": "join", "note": note, "version": version })).await;
    }

    async fn send_json(&mut self, v: Value) {
        self.stream.send(Message::Text(v.to_string().into())).await.unwrap();
    }

    async fn send_frame(&mut self, kind: u8, note: &str, payload: &[u8]) {
        let mut buf = vec![kind];
        buf.extend_from_slice(Uuid::parse_str(note).unwrap().as_bytes());
        buf.extend_from_slice(payload);
        self.stream.send(Message::Binary(buf.into())).await.unwrap();
    }

    async fn next(&mut self) -> Msg {
        loop {
            let msg = tokio::time::timeout(Duration::from_secs(5), self.stream.next())
                .await
                .expect("timed out waiting for a websocket message")
                .unwrap()
                .unwrap();
            match msg {
                Message::Text(t) => return Msg::Control(serde_json::from_str(&t).unwrap()),
                Message::Binary(b) => {
                    return Msg::Frame(b[0], b[17..].to_vec());
                }
                _ => continue,
            }
        }
    }

    /// Next control message of type `t`, skipping `tree_changed` noise and frames.
    async fn control(&mut self, t: &str) -> Value {
        loop {
            if let Msg::Control(v) = self.next().await
                && v["t"] == t
            {
                return v;
            }
        }
    }

    async fn frame(&mut self, kind: u8) -> Vec<u8> {
        loop {
            match self.next().await {
                Msg::Frame(k, data) if k == kind => return data,
                Msg::Control(v) if v["t"] == "error" => panic!("unexpected error: {v}"),
                _ => {}
            }
        }
    }

    /// Joins and returns (role, server version, initial data).
    async fn join_and_sync(&mut self, note: &str, version: Option<&VersionVector>) -> (String, VersionVector, Vec<u8>) {
        self.join(note, version).await;
        let joined = self.control("joined").await;
        let vv = VersionVector::decode(&STANDARD.decode(joined["version"].as_str().unwrap()).unwrap()).unwrap();
        let data = self.frame(0x01).await;
        (joined["role"].as_str().unwrap().to_owned(), vv, data)
    }
}

fn edit(doc: &LoroDoc, pos: usize, text: &str) -> Vec<u8> {
    let before = doc.oplog_vv();
    doc.get_text("body").insert(pos, text).unwrap();
    doc.commit();
    doc.export(ExportMode::updates(&before)).unwrap()
}

#[tokio::test]
async fn first_account_from_the_browser() {
    let dir = TempDir::new().unwrap();
    let server = start(&dir).await;
    let http = reqwest::Client::builder().cookie_store(true).build().unwrap();
    let setup = format!("http://{}/api/auth/setup", server.base);
    let needed = |v: Value| v["needed"].as_bool().unwrap();
    assert!(needed(http.get(&setup).send().await.unwrap().json().await.unwrap()));

    let short = http.post(&setup).json(&json!({ "username": "ann", "password": "short" })).send().await.unwrap();
    assert_eq!(short.status(), 400);
    let res = http
        .post(&setup)
        .json(&json!({ "username": "Ann", "display_name": "Ann", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());
    let me: Value = http.get(format!("http://{}/api/me", server.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!((me["username"].as_str(), me["is_admin"].as_bool()), (Some("ann"), Some(true)));

    // Once there's an account, nobody else can set the server up.
    assert!(!needed(http.get(&setup).send().await.unwrap().json().await.unwrap()));
    let again = reqwest::Client::new().post(&setup).json(&json!({ "username": "eve", "password": "password123" })).send().await.unwrap();
    assert_eq!(again.status(), 409);
}

#[tokio::test]
async fn login_me_logout() {
    let dir = TempDir::new().unwrap();
    let server = start(&dir).await;
    let http = reqwest::Client::new();
    let me = format!("http://{}/api/me", server.base);
    assert_eq!(http.get(&me).send().await.unwrap().status(), 401);

    auth::create_user(&server.state.db, "alice", "Alice", "password123", true).await.unwrap();
    let bad = http
        .post(format!("http://{}/api/auth/login", server.base))
        .json(&json!({ "username": "alice", "password": "wrong-password" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 401);

    let alice = login(&server.base, "alice").await;
    let body = alice.get("/me").await;
    assert_eq!(body["username"], "alice");
    assert_eq!(body["display_name"], "Alice");
    assert_eq!(body["is_admin"], true);

    alice.post("/auth/logout", json!({})).await;
    let res = alice.http.get(alice.url("/me")).header("cookie", &alice.cookie).send().await.unwrap();
    assert_eq!(res.status(), 401, "the session must be gone server-side, not just the cookie");
}

#[tokio::test]
async fn sharing_controls_access() {
    let dir = TempDir::new().unwrap();
    let server = start(&dir).await;
    let alice = user(&server, "alice").await;
    let bob = user(&server, "bob").await;

    let notebook = alice.post("/notebooks", json!({ "name": "Groceries" })).await["id"].as_str().unwrap().to_owned();
    let note = alice.post("/notes", json!({ "notebook_id": notebook })).await["id"].as_str().unwrap().to_owned();

    assert_eq!(bob.get("/tree").await["notes"].as_array().unwrap().len(), 0);
    let mut bob_ws = bob.ws().await;
    bob_ws.join(&note, None).await;
    assert_eq!(bob_ws.control("error").await["code"], "not_found");

    // Viewer via the notebook.
    let share = alice
        .post("/shares", json!({ "resource_type": "notebook", "resource_id": notebook, "username": "bob", "role": "viewer" }))
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let tree = bob.get("/tree").await;
    assert_eq!(tree["notes"][0]["id"], note);
    assert_eq!(tree["notes"][0]["role"], "viewer");
    assert_eq!(tree["shared"][0]["resource_id"], notebook);
    // Both sides see the notebook marked shared; the note inside isn't shared on its own.
    assert_eq!(tree["notebooks"][0]["shared"], true);
    assert_eq!(tree["notes"][0]["shared"], false);
    let own = alice.get("/tree").await;
    assert_eq!(own["notebooks"][0]["shared"], true);

    let (role, _, _) = bob_ws.join_and_sync(&note, None).await;
    assert_eq!(role, "viewer");
    bob_ws.send_frame(0x01, &note, &edit(&LoroDoc::new(), 0, "nope")).await;
    assert_eq!(bob_ws.control("error").await["code"], "forbidden");

    // An update built on changes the server never accepted is flagged, not silently dropped.
    let rejected = LoroDoc::new();
    let _ = edit(&rejected, 0, "a");
    let dependent = edit(&rejected, 1, "b");

    // Upgrading to editor reaches the open connection, and edits flow to alice.
    let res = alice.send(reqwest::Method::PATCH, &format!("/shares/{share}"), json!({ "role": "editor" })).await;
    assert!(res.status().is_success());
    assert_eq!(bob_ws.control("role").await["role"], "editor");

    bob_ws.send_frame(0x01, &note, &dependent).await;
    assert_eq!(bob_ws.control("error").await["code"], "out_of_sync");

    let mut alice_ws = alice.ws().await;
    alice_ws.join_and_sync(&note, None).await;
    let bob_doc = LoroDoc::new();
    bob_ws.send_frame(0x01, &note, &edit(&bob_doc, 0, "milk ")).await;
    let alice_doc = LoroDoc::new();
    alice_doc.import(&alice_ws.frame(0x01).await).unwrap();
    assert!(alice_doc.get_text("body").to_string().contains("milk"));

    // Bob can't share alice's notebook onward.
    let res = bob
        .send(reqwest::Method::POST, "/shares", json!({ "resource_type": "notebook", "resource_id": notebook, "username": "alice", "role": "editor" }))
        .await;
    assert_eq!(res.status(), 403);

    // Revoking closes bob's room.
    let res = alice.send(reqwest::Method::DELETE, &format!("/shares/{share}"), json!({})).await;
    assert!(res.status().is_success());
    assert_eq!(bob_ws.control("revoked").await["note"], note);
    assert_eq!(bob.get("/tree").await["notes"].as_array().unwrap().len(), 0);
    assert_eq!(alice.get("/tree").await["notebooks"][0]["shared"], false);
}

#[tokio::test]
async fn live_sync_presence_and_persistence() {
    let dir = TempDir::new().unwrap();
    let server = start(&dir).await;
    let alice = user(&server, "alice").await;
    let bob = user(&server, "bob").await;
    let note = alice.post("/notes", json!({})).await["id"].as_str().unwrap().to_owned();
    alice
        .post("/shares", json!({ "resource_type": "note", "resource_id": note, "username": "bob", "role": "editor" }))
        .await;

    let mut alice_ws = alice.ws().await;
    let mut bob_ws = bob.ws().await;
    let alice_doc = LoroDoc::new();
    let bob_doc = LoroDoc::new();
    alice_ws.join_and_sync(&note, None).await;
    let (_, _, data) = bob_ws.join_and_sync(&note, None).await;
    bob_doc.import(&data).unwrap();

    // Live edit.
    alice_ws.send_frame(0x01, &note, &edit(&alice_doc, 0, "# Shopping\n")).await;
    bob_doc.import(&bob_ws.frame(0x01).await).unwrap();
    assert_eq!(bob_doc.get_text("body").to_string(), "# Shopping\n");

    // Presence is relayed, and cleared when the sender leaves.
    let presence = EphemeralStore::new(30_000);
    presence.set("1-cm-user", "Alice");
    alice_ws.send_frame(0x02, &note, &presence.encode_all()).await;
    let seen = EphemeralStore::new(30_000);
    seen.apply(&bob_ws.frame(0x02).await).unwrap();
    assert_eq!(seen.get("1-cm-user").unwrap().into_string().unwrap().as_str(), "Alice");
    alice_ws.send_json(json!({ "t": "leave", "note": note })).await;
    seen.apply(&bob_ws.frame(0x02).await).unwrap();
    assert!(seen.get("1-cm-user").is_none());

    // The title comes from the first line.
    let tree = alice.get("/tree").await;
    assert_eq!(tree["notes"][0]["title"], "Shopping");

    // Bob edits while disconnected, then catches up with his version.
    drop(bob_ws);
    let _ = edit(&bob_doc, bob_doc.get_text("body").len_unicode(), "eggs\n");
    alice_ws.join_and_sync(&note, None).await;
    alice_ws.send_frame(0x01, &note, &edit(&alice_doc, 11, "milk\n")).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut bob_ws = bob.ws().await;
    let (_, server_vv, missing) = bob_ws.join_and_sync(&note, Some(&bob_doc.oplog_vv())).await;
    bob_doc.import(&missing).unwrap();
    bob_ws.send_frame(0x01, &note, &bob_doc.export(ExportMode::updates(&server_vv)).unwrap()).await;
    alice_doc.import(&alice_ws.frame(0x01).await).unwrap();
    let merged = alice_doc.get_text("body").to_string();
    assert!(merged.contains("milk") && merged.contains("eggs"), "merged: {merged:?}");
    assert_eq!(bob_doc.get_text("body").to_string(), merged);

    // Each editing session is put down to the person whose connection sent it.
    let authors = bob.get(&format!("/notes/{note}/authors")).await;
    let name_of = |doc: &LoroDoc| authors["peers"][doc.peer_id().to_string()]["name"].clone();
    assert_eq!((name_of(&alice_doc), name_of(&bob_doc)), (json!("alice"), json!("bob")), "authors: {authors}");

    // Everything survives a restart.
    tokio::time::sleep(Duration::from_millis(100)).await;
    server.task.abort();
    drop(server);
    let server = start(&dir).await;
    let alice_again = login(&server.base, "alice").await;
    let mut ws = alice_again.ws().await;
    let (_, _, snapshot) = ws.join_and_sync(&note, None).await;
    let reloaded = LoroDoc::new();
    reloaded.import(&snapshot).unwrap();
    assert_eq!(reloaded.get_text("body").to_string(), merged);
}

#[tokio::test]
async fn trash_and_restore_notebook() {
    let dir = TempDir::new().unwrap();
    let server = start(&dir).await;
    let alice = user(&server, "alice").await;
    let parent = alice.post("/notebooks", json!({ "name": "Home" })).await["id"].as_str().unwrap().to_owned();
    let child = alice.post("/notebooks", json!({ "name": "Recipes", "parent_id": parent })).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    alice.post("/notes", json!({ "notebook_id": child })).await;

    // Can't move a notebook inside itself.
    let res = alice.send(reqwest::Method::PATCH, &format!("/notebooks/{parent}"), json!({ "parent_id": child })).await;
    assert_eq!(res.status(), 400);

    let res = alice.send(reqwest::Method::DELETE, &format!("/notebooks/{parent}"), json!({})).await;
    assert!(res.status().is_success());
    let tree = alice.get("/tree").await;
    assert_eq!(tree["notebooks"].as_array().unwrap().len(), 0);
    assert_eq!(tree["notes"].as_array().unwrap().len(), 0);
    let trash = alice.get("/trash").await;
    assert_eq!(trash.as_array().unwrap().len(), 1, "only the directly deleted notebook: {trash}");
    let gone = &trash[0];
    assert_eq!(gone["name"], "Home");
    let kept = gone["purge_at"].as_i64().unwrap() - gone["deleted_at"].as_i64().unwrap();
    assert_eq!(kept, 30 * 24 * 60 * 60 * 1000, "trash keeps items for 30 days: {gone}");

    alice.post(&format!("/trash/notebook/{parent}/restore"), json!({})).await;
    let tree = alice.get("/tree").await;
    assert_eq!(tree["notebooks"].as_array().unwrap().len(), 2);
    assert_eq!(tree["notes"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn discard_only_removes_blank_notes() {
    let dir = TempDir::new().unwrap();
    let server = start(&dir).await;
    let alice = user(&server, "alice").await;

    let blank = alice.post("/notes", json!({})).await["id"].as_str().unwrap().to_owned();
    let res = alice.send(reqwest::Method::DELETE, &format!("/notes/{blank}?discard=true"), json!({})).await;
    assert!(res.status().is_success());
    assert_eq!(alice.get("/trash").await.as_array().unwrap().len(), 0, "discarded notes skip the trash");

    let note = alice.post("/notes", json!({})).await["id"].as_str().unwrap().to_owned();
    let mut ws = alice.ws().await;
    ws.join_and_sync(&note, None).await;
    ws.send_frame(0x01, &note, &edit(&LoroDoc::new(), 0, "Keep me\n- [ ] **second** line")).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let res = alice.send(reqwest::Method::DELETE, &format!("/notes/{note}?discard=true"), json!({})).await;
    assert_eq!(res.status(), 409);

    let tree = alice.get("/tree").await;
    assert_eq!(tree["notes"][0]["title"], "Keep me");
    assert_eq!(tree["notes"][0]["preview"], "second line");
}

#[tokio::test]
async fn invite_link_creates_account_and_shares() {
    let dir = TempDir::new().unwrap();
    let server = start(&dir).await;
    auth::create_user(&server.state.db, "justin", "Justin", "password123", true).await.unwrap();
    let justin = login(&server.base, "justin").await;
    let bob = user(&server, "bob").await;

    let groceries = justin.post("/notebooks", json!({ "name": "Groceries" })).await["id"].as_str().unwrap().to_owned();
    let note = justin.post("/notes", json!({ "notebook_id": groceries })).await["id"].as_str().unwrap().to_owned();

    // Only admins invite, and only for things they own.
    let res = bob.send(reqwest::Method::POST, "/invites", json!({})).await;
    assert_eq!(res.status(), 403);

    let invite = justin
        .post("/invites", json!({ "resource_type": "notebook", "resource_id": groceries }))
        .await;
    let token = invite["token"].as_str().unwrap().to_owned();
    assert_eq!(justin.get("/invites").await.as_array().unwrap().len(), 1);

    let anon = reqwest::Client::builder().cookie_store(true).build().unwrap();
    let join = format!("http://{}/api/join/{token}", server.base);
    let preview: Value = anon.get(&join).send().await.unwrap().json().await.unwrap();
    assert_eq!(preview["inviter"], "Justin");
    assert_eq!(preview["shared"], "Groceries");

    // A taken username fails without burning the link.
    let res = anon.post(&join).json(&json!({ "username": "bob", "password": "password123" })).send().await.unwrap();
    assert_eq!(res.status(), 409);

    let res = anon
        .post(&join)
        .json(&json!({ "username": "sam", "display_name": "Sam", "password": "password123" }))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());

    // Logged in straight away, with the notebook shared.
    let tree: Value = anon.get(format!("http://{}/api/tree", server.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(tree["notebooks"][0]["name"], "Groceries");
    assert_eq!(tree["notebooks"][0]["role"], "editor");
    assert_eq!(tree["notes"][0]["id"], note);

    // Single use.
    assert_eq!(anon.get(&join).send().await.unwrap().status(), 404);
    let res = anon.post(&join).json(&json!({ "username": "sam2", "password": "password123" })).send().await.unwrap();
    assert_eq!(res.status(), 404);
    assert_eq!(justin.get("/invites").await.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn settings_password_and_admin_controls() {
    let dir = TempDir::new().unwrap();
    let server = start(&dir).await;
    auth::create_user(&server.state.db, "justin", "Justin", "password123", true).await.unwrap();
    let justin = login(&server.base, "justin").await;
    let phone = login(&server.base, "justin").await;
    let bob = user(&server, "bob").await;
    let bob_id = bob.get("/me").await["id"].as_str().unwrap().to_owned();

    // Rename.
    let res = justin.send(reqwest::Method::PATCH, "/me", json!({ "display_name": "Dad" })).await;
    assert!(res.status().is_success());
    assert_eq!(justin.get("/me").await["display_name"], "Dad");

    // Password change needs the current one, and signs out other devices but not this one.
    let res = justin.send(reqwest::Method::POST, "/me/password", json!({ "current": "nope-nope", "new": "newpassword1" })).await;
    assert_eq!(res.status(), 400);
    justin.post("/me/password", json!({ "current": "password123", "new": "newpassword1" })).await;
    assert!(justin.http.get(justin.url("/me")).send().await.unwrap().status().is_success());
    assert_eq!(phone.http.get(phone.url("/me")).send().await.unwrap().status(), 401);

    // Admin-only listing; bob can't see it.
    assert_eq!(bob.http.get(bob.url("/admin/users")).send().await.unwrap().status(), 403);
    assert_eq!(justin.get("/admin/users").await.as_array().unwrap().len(), 2);

    // Can't lock yourself out.
    let justin_id = justin.get("/me").await["id"].as_str().unwrap().to_owned();
    let res = justin.send(reqwest::Method::PATCH, &format!("/admin/users/{justin_id}"), json!({ "is_admin": false })).await;
    assert_eq!(res.status(), 400);

    // Reset bob's password: his session ends, his open connection closes, and the new password works.
    let mut bob_ws = bob.ws().await;
    justin.post(&format!("/admin/users/{bob_id}/password"), json!({ "password": "bobsnewpass" })).await;
    assert_eq!(bob.http.get(bob.url("/me")).send().await.unwrap().status(), 401);
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match bob_ws.stream.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                _ => {}
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "bob's websocket should be closed");
    let http = reqwest::Client::new();
    let login_url = format!("http://{}/api/auth/login", server.base);
    let ok = http.post(&login_url).json(&json!({ "username": "bob", "password": "bobsnewpass" })).send().await.unwrap();
    assert!(ok.status().is_success());

    // Disabled accounts can't log in and drop out of the share picker; re-enabling restores them.
    let res = justin.send(reqwest::Method::PATCH, &format!("/admin/users/{bob_id}"), json!({ "disabled": true })).await;
    assert!(res.status().is_success());
    let denied = http.post(&login_url).json(&json!({ "username": "bob", "password": "bobsnewpass" })).send().await.unwrap();
    assert_eq!(denied.status(), 401);
    assert_eq!(justin.get("/users").await.as_array().unwrap().len(), 1);
    justin.send(reqwest::Method::PATCH, &format!("/admin/users/{bob_id}"), json!({ "disabled": false })).await;
    let back = http.post(&login_url).json(&json!({ "username": "bob", "password": "bobsnewpass" })).send().await.unwrap();
    assert!(back.status().is_success());
}

async fn upload(user: &User, note: &str, name: &str, mime: &str, bytes: Vec<u8>) -> reqwest::Response {
    let file = reqwest::multipart::Part::bytes(bytes).file_name(name.to_owned()).mime_str(mime).unwrap();
    let form = reqwest::multipart::Form::new().text("note_id", note.to_owned()).part("file", file);
    user.http.post(user.url("/attachments")).multipart(form).send().await.unwrap()
}

#[tokio::test]
async fn attachments_follow_note_access() {
    let dir = TempDir::new().unwrap();
    let server = start(&dir).await;
    let alice = user(&server, "alice").await;
    let bob = user(&server, "bob").await;
    let note = alice.post("/notes", json!({})).await["id"].as_str().unwrap().to_owned();

    let png = b"\x89PNG fake image bytes".to_vec();
    let res = upload(&alice, &note, "photo.png", "image/png", png.clone()).await;
    assert!(res.status().is_success(), "{}", res.status());
    let att: Value = res.json().await.unwrap();
    let id = att["id"].as_str().unwrap();
    assert_eq!(att["mime"], "image/png");

    // Stored once by hash, served with its type.
    let res = alice.http.get(alice.url(&format!("/attachments/{id}"))).send().await.unwrap();
    assert_eq!(res.headers()["content-type"], "image/png");
    assert_eq!(res.bytes().await.unwrap().to_vec(), png);
    assert_eq!(alice.get(&format!("/attachments/{id}/meta")).await["filename"], "photo.png");

    // Ranges work, for audio seeking on Safari.
    let res = alice
        .http
        .get(alice.url(&format!("/attachments/{id}")))
        .header("range", "bytes=0-3")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 206);
    assert_eq!(res.bytes().await.unwrap().len(), 4);

    // Bob sees nothing until the note is shared, and a viewer can't upload.
    let res = bob.http.get(bob.url(&format!("/attachments/{id}"))).send().await.unwrap();
    assert_eq!(res.status(), 404);
    assert_eq!(upload(&bob, &note, "x.png", "image/png", png.clone()).await.status(), 404);
    alice
        .post("/shares", json!({ "resource_type": "note", "resource_id": note, "username": "bob", "role": "viewer" }))
        .await;
    let res = bob.http.get(bob.url(&format!("/attachments/{id}"))).send().await.unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(upload(&bob, &note, "x.png", "image/png", png).await.status(), 403);

    // Only images and audio; SVG could carry script.
    let res = upload(&alice, &note, "x.svg", "image/svg+xml", b"<svg/>".to_vec()).await;
    assert_eq!(res.status(), 400);
    let res = upload(&alice, &note, "x.html", "text/html", b"<p>".to_vec()).await;
    assert_eq!(res.status(), 400);
}

async fn import(user: &User, notebook: Option<&str>, files: Vec<(&str, Vec<u8>)>) -> reqwest::Response {
    let mut form = reqwest::multipart::Form::new();
    if let Some(nb) = notebook {
        form = form.text("notebook_id", nb.to_owned());
    }
    for (name, bytes) in files {
        form = form.part("file", reqwest::multipart::Part::bytes(bytes).file_name(name.to_owned()));
    }
    user.http.post(user.url("/import")).multipart(form).send().await.unwrap()
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let when = zip::DateTime::from_date_and_time(2021, 3, 4, 5, 6, 8).unwrap();
    for (name, bytes) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default().last_modified_time(when)).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[tokio::test]
async fn import_zip_of_markdown_folders() {
    let dir = TempDir::new().unwrap();
    let server = start(&dir).await;
    let alice = user(&server, "alice").await;
    let bob = user(&server, "bob").await;
    let png = b"\x89PNG fake image bytes".to_vec();
    let zip = zip_of(&[
        ("Vault/Recipes/Pancakes.md", b"flour\r\n![batter](../attachments/my%20photo.png)\n"),
        ("Vault/Ideas.md", b"---\ntitle: Big ideas\ntags: [x]\n---\nfly ![[my photo.png]]"),
        ("Vault/attachments/my photo.png", &png),
        ("Vault/.obsidian/app.json", b"{}"),
        ("Vault/budget.xlsx", b"PK"),
    ]);
    let res = import(&alice, None, vec![("export.zip", zip)]).await;
    assert!(res.status().is_success(), "{}", res.status());
    let result: Value = res.json().await.unwrap();
    assert_eq!((result["notes"].as_i64(), result["notebooks"].as_i64(), result["attachments"].as_i64()), (Some(2), Some(2), Some(2)));
    assert_eq!(result["skipped"], json!(["Vault/budget.xlsx"]));

    // The wrapping folder becomes the notebook; folders without notes (attachments, .obsidian) don't.
    let tree = alice.get("/tree").await;
    let notebooks = tree["notebooks"].as_array().unwrap();
    let vault = notebooks.iter().find(|n| n["name"] == "Vault").unwrap();
    assert_eq!(vault["id"], result["notebook_id"]);
    assert!(vault["parent_id"].is_null());
    let recipes = notebooks.iter().find(|n| n["name"] == "Recipes").unwrap();
    assert_eq!(recipes["parent_id"], vault["id"]);
    assert_eq!(notebooks.len(), 2);

    // Titles come from the file name or front matter, and dates from the zip.
    let notes = tree["notes"].as_array().unwrap();
    let pancakes = notes.iter().find(|n| n["title"] == "Pancakes").unwrap();
    assert_eq!(pancakes["notebook_id"], recipes["id"]);
    assert_eq!(pancakes["preview"], "flour");
    assert_eq!(pancakes["updated_at"], 1614834368000i64);
    let ideas = notes.iter().find(|n| n["title"] == "Big ideas").unwrap();
    assert_eq!(ideas["notebook_id"], vault["id"]);

    // Image links point at stored attachments, readable through the note.
    let id: Uuid = pancakes["id"].as_str().unwrap().parse().unwrap();
    let body = gnotes_server::rooms::note_body(&server.state, id).await.unwrap();
    let att = body.split("(att:").nth(1).unwrap().split(')').next().unwrap().to_owned();
    assert_eq!(body, format!("# Pancakes\n\nflour\n![batter](att:{att})"));
    let res = alice.http.get(alice.url(&format!("/attachments/{att}"))).send().await.unwrap();
    assert_eq!(res.headers()["content-type"], "image/png");
    assert_eq!(res.bytes().await.unwrap().to_vec(), png);
    let id: Uuid = ideas["id"].as_str().unwrap().parse().unwrap();
    let body = gnotes_server::rooms::note_body(&server.state, id).await.unwrap();
    assert!(body.starts_with("# Big ideas\n\nfly ![](att:"), "{body}");

    // Loose files go straight into the chosen notebook; a viewer can't import there.
    let res = import(&alice, vault["id"].as_str(), vec![("Todo.md", b"- milk".to_vec()), ("x.pdf", b"%PDF".to_vec())]).await;
    let result: Value = res.json().await.unwrap();
    assert_eq!((result["notes"].as_i64(), result["notebooks"].as_i64()), (Some(1), Some(0)));
    assert_eq!(result["skipped"], json!(["x.pdf"]));
    let tree = alice.get("/tree").await;
    let todo = tree["notes"].as_array().unwrap().iter().find(|n| n["title"] == "Todo").unwrap().clone();
    assert_eq!(todo["notebook_id"], vault["id"]);
    alice
        .post("/shares", json!({ "resource_type": "notebook", "resource_id": vault["id"], "username": "bob", "role": "viewer" }))
        .await;
    let res = import(&bob, vault["id"].as_str(), vec![("Todo.md", b"x".to_vec())]).await;
    assert_eq!(res.status(), 403);
    let res = import(&alice, None, vec![("x.pdf", b"%PDF".to_vec())]).await;
    assert_eq!(res.status(), 400);
}

/// A stand-in for an OpenAI-compatible speech-to-text server. Returns its base URL.
async fn fake_whisper() -> String {
    let fake = axum::Router::new()
        .route(
            "/v1/audio/transcriptions",
            axum::routing::post(|headers: axum::http::HeaderMap, mut form: axum::extract::Multipart| async move {
                let mut model = String::new();
                let mut size = 0;
                while let Some(field) = form.next_field().await.unwrap() {
                    match field.name() {
                        Some("model") => model = field.text().await.unwrap(),
                        Some("file") => size = field.bytes().await.unwrap().len(),
                        _ => {}
                    }
                }
                let auth = headers.get("authorization").map(|v| v.to_str().unwrap().to_owned()).unwrap_or_default();
                axum::Json(json!({ "text": format!(" heard {size} bytes with {model} ({auth}) ") }))
            }),
        )
        .route(
            "/v1/models",
            axum::routing::get(|| async { axum::Json(json!({ "data": [{ "id": "small" }, { "id": "large" }] })) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, fake).await });
    url
}

#[tokio::test]
async fn transcription_uses_the_configured_service() {
    let fake_url = fake_whisper().await;
    let dir = TempDir::new().unwrap();
    let whisper = WhisperConfig { url: fake_url, model: "small".into(), key: Some("sekrit".into()), realtime_url: None };
    let server = start_with(&dir, Some(whisper)).await;
    let alice = user(&server, "alice").await;
    assert_eq!(alice.get("/features").await["transcription"], true);
    let note = alice.post("/notes", json!({})).await["id"].as_str().unwrap().to_owned();

    let att: Value = upload(&alice, &note, "memo.webm", "audio/webm;codecs=opus", vec![7; 1000]).await.json().await.unwrap();
    assert_eq!(att["mime"], "audio/webm");
    let out = alice.post(&format!("/attachments/{}/transcribe", att["id"].as_str().unwrap()), json!({})).await;
    assert_eq!(out["text"], "heard 1000 bytes with small (Bearer sekrit)");

    // Images aren't sent.
    let img: Value = upload(&alice, &note, "a.png", "image/png", vec![1; 10]).await.json().await.unwrap();
    let res = alice.send(reqwest::Method::POST, &format!("/attachments/{}/transcribe", img["id"].as_str().unwrap()), json!({})).await;
    assert_eq!(res.status(), 400);
}

#[tokio::test]
async fn transcription_off_without_config() {
    let dir = TempDir::new().unwrap();
    let server = start(&dir).await;
    let alice = user(&server, "alice").await;
    assert_eq!(alice.get("/features").await["transcription"], false);
    let note = alice.post("/notes", json!({})).await["id"].as_str().unwrap().to_owned();
    let att: Value = upload(&alice, &note, "memo.webm", "audio/webm", vec![7; 10]).await.json().await.unwrap();
    let res = alice.send(reqwest::Method::POST, &format!("/attachments/{}/transcribe", att["id"].as_str().unwrap()), json!({})).await;
    assert_eq!(res.status(), 409);
}

#[tokio::test]
async fn admin_changes_speech_to_text_in_the_app() {
    let fake_url = fake_whisper().await;
    let dir = TempDir::new().unwrap();
    let server = start(&dir).await;
    auth::create_user(&server.state.db, "admin", "Admin", "password123", true).await.unwrap();
    let admin = login(&server.base, "admin").await;
    let bob = user(&server, "bob").await;

    assert_eq!(admin.get("/admin/settings").await["whisper"]["enabled"], false);
    assert_eq!(bob.get("/features").await["transcription"], false);
    let res = bob.send(reqwest::Method::PUT, "/admin/settings/whisper", json!({ "url": fake_url })).await;
    assert_eq!(res.status(), 403);

    // Test before saving: it connects and warns about a model the service doesn't list.
    let test = admin.post("/admin/settings/whisper/test", json!({ "url": fake_url, "model": "tiny" })).await;
    assert_eq!(test["ok"], true);
    assert!(test["message"].as_str().unwrap().contains("small, large"), "{test}");
    let test = admin.post("/admin/settings/whisper/test", json!({ "url": "http://127.0.0.1:1/v1" })).await;
    assert_eq!(test["ok"], false);

    let saved = admin
        .send(reqwest::Method::PUT, "/admin/settings/whisper", json!({ "url": format!("{fake_url}/"), "model": "large", "key": "k1" }))
        .await;
    let saved: Value = saved.json().await.unwrap();
    assert_eq!(saved["whisper"]["has_key"], true);
    assert!(saved.to_string().find("k1").is_none(), "key must not be sent back");
    assert_eq!(bob.get("/features").await["transcription"], true);

    // Bob's memo goes to the service the admin chose, with the saved key.
    let note = bob.post("/notes", json!({})).await["id"].as_str().unwrap().to_owned();
    let att: Value = upload(&bob, &note, "m.webm", "audio/webm", vec![1; 5]).await.json().await.unwrap();
    let out = bob.post(&format!("/attachments/{}/transcribe", att["id"].as_str().unwrap()), json!({})).await;
    assert_eq!(out["text"], "heard 5 bytes with large (Bearer k1)");

    // Saving without a key keeps it; it survives a restart.
    admin.send(reqwest::Method::PUT, "/admin/settings/whisper", json!({ "url": fake_url, "model": "small" })).await;
    server.task.abort();
    let server = start(&dir).await;
    let admin = login(&server.base, "admin").await;
    let w = &admin.get("/admin/settings").await["whisper"];
    assert_eq!((w["model"].as_str(), w["has_key"].as_bool(), w["from_env"].as_bool()), (Some("small"), Some(true), Some(false)));

    // An empty URL turns it off.
    admin.send(reqwest::Method::PUT, "/admin/settings/whisper", json!({ "url": "" })).await;
    assert_eq!(admin.get("/features").await["transcription"], false);
}

/// A stand-in realtime speech-to-text service: echoes each audio chunk's size as a delta and, on
/// commit, finishes with the total. Returns its ws:// URL.
async fn fake_realtime() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/v1/realtime", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
                let mut total = 0;
                while let Some(Ok(Message::Text(text))) = ws.next().await {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    let reply = match event["type"].as_str().unwrap() {
                        "session.update" => {
                            assert_eq!(event["session"]["sample_rate"], 16000);
                            json!({ "type": "session.updated" })
                        }
                        "input_audio_buffer.append" => {
                            let n = STANDARD.decode(event["audio"].as_str().unwrap()).unwrap().len();
                            total += n;
                            json!({ "type": "conversation.item.input_audio_transcription.delta", "delta": format!("{n} ") })
                        }
                        "input_audio_buffer.commit" => {
                            json!({ "type": "conversation.item.input_audio_transcription.completed", "transcript": format!("{total} bytes") })
                        }
                        _ => continue,
                    };
                    ws.send(Message::Text(reply.to_string().into())).await.unwrap();
                }
            });
        }
    });
    url
}

#[tokio::test]
async fn live_transcription_relays_audio_and_text() {
    let dir = TempDir::new().unwrap();
    let whisper = WhisperConfig {
        url: "http://127.0.0.1:9/v1".into(),
        model: "small".into(),
        key: None,
        realtime_url: Some(fake_realtime().await),
    };
    let server = start_with(&dir, Some(whisper)).await;
    let alice = user(&server, "alice").await;
    assert_eq!(alice.get("/features").await["live_transcription"], true);

    let mut req = format!("ws://{}/api/transcribe/live", alice.base).into_client_request().unwrap();
    req.headers_mut().insert("cookie", alice.cookie.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let mut next = async || -> Value {
        loop {
            if let Message::Text(t) = ws.next().await.unwrap().unwrap() {
                return serde_json::from_str(&t).unwrap();
            }
        }
    };
    assert_eq!(next().await["t"], "ready");
    drop(next);
    ws.send(Message::Binary(vec![0u8; 3200].into())).await.unwrap();
    ws.send(Message::Binary(vec![0u8; 1600].into())).await.unwrap();
    ws.send(Message::Text(r#"{"t":"commit"}"#.into())).await.unwrap();
    let mut got = Vec::new();
    while got.len() < 3 {
        if let Message::Text(t) = ws.next().await.unwrap().unwrap() {
            got.push(serde_json::from_str::<Value>(&t).unwrap());
        }
    }
    assert_eq!(got[0], json!({ "t": "delta", "text": "3200 " }));
    assert_eq!(got[1], json!({ "t": "delta", "text": "1600 " }));
    assert_eq!(got[2], json!({ "t": "final", "text": "4800 bytes" }));

    // The live URL has to be a websocket address.
    auth::create_user(&server.state.db, "root", "", "password123", true).await.unwrap();
    let root = login(&server.base, "root").await;
    let body = json!({ "url": "http://127.0.0.1:9/v1", "realtime_url": "http://nope" });
    assert_eq!(root.send(reqwest::Method::PUT, "/admin/settings/whisper", body).await.status(), 400);
}

/// A stand-in vision model: "reads" a photo as its byte count, or reports no text for a 1-byte image.
async fn fake_vision() -> String {
    let fake = axum::Router::new()
        .route(
            "/v1/chat/completions",
            axum::routing::post(|axum::Json(body): axum::Json<Value>| async move {
                assert_eq!(body["model"], "qwen2.5vl");
                let url = body["messages"][0]["content"][1]["image_url"]["url"].as_str().unwrap().to_owned();
                let (head, data) = url.split_once(";base64,").unwrap();
                assert_eq!(head, "data:image/png");
                let n = STANDARD.decode(data).unwrap().len();
                let content = if n == 1 { "NO_TEXT".to_owned() } else { format!("```\nMilk\n{n} eggs\n```") };
                axum::Json(json!({ "choices": [{ "message": { "role": "assistant", "content": content } }] }))
            }),
        )
        .route("/v1/models", axum::routing::get(|| async { axum::Json(json!({ "data": [{ "id": "qwen2.5vl" }] })) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, fake).await });
    url
}

#[tokio::test]
async fn photos_are_read_by_the_configured_vision_model() {
    let fake_url = fake_vision().await;
    let dir = TempDir::new().unwrap();
    let server = start(&dir).await;
    auth::create_user(&server.state.db, "root", "", "password123", true).await.unwrap();
    let root = login(&server.base, "root").await;
    let alice = user(&server, "alice").await;
    assert_eq!(alice.get("/features").await["photo_text"], false);

    // An admin sets it up from Settings; the model has to be named.
    let res = root.send(reqwest::Method::PUT, "/admin/settings/vision", json!({ "url": fake_url })).await;
    assert_eq!(res.status(), 400);
    let tested = root.post("/admin/settings/vision/test", json!({ "url": fake_url, "model": "qwen2.5vl" })).await;
    assert_eq!(tested["message"], "Connected");
    let saved = root.send(reqwest::Method::PUT, "/admin/settings/vision", json!({ "url": fake_url, "model": "qwen2.5vl" })).await;
    assert!(saved.status().is_success());
    assert_eq!(root.get("/admin/settings").await["vision"]["model"], "qwen2.5vl");
    assert_eq!(alice.get("/features").await["photo_text"], true);

    let note = alice.post("/notes", json!({})).await["id"].as_str().unwrap().to_owned();
    let photo: Value = upload(&alice, &note, "board.png", "image/png", vec![1; 42]).await.json().await.unwrap();
    let out = alice.post(&format!("/attachments/{}/text", photo["id"].as_str().unwrap()), json!({})).await;
    assert_eq!(out["text"], "Milk\n42 eggs");

    // A photo without words reads as nothing, and recordings aren't sent.
    let blank: Value = upload(&alice, &note, "dot.png", "image/png", vec![1]).await.json().await.unwrap();
    assert_eq!(alice.post(&format!("/attachments/{}/text", blank["id"].as_str().unwrap()), json!({})).await["text"], "");
    let memo: Value = upload(&alice, &note, "m.webm", "audio/webm", vec![7; 10]).await.json().await.unwrap();
    let res = alice.send(reqwest::Method::POST, &format!("/attachments/{}/text", memo["id"].as_str().unwrap()), json!({})).await;
    assert_eq!(res.status(), 400);
}

/// A stand-in chat model for summaries: checks it was sent the note and answers with a fenced summary.
async fn fake_summarizer() -> String {
    let fake = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(|axum::Json(body): axum::Json<Value>| async move {
            assert_eq!(body["model"], "llama3.1");
            let prompt = body["messages"][0]["content"].as_str().unwrap().to_owned();
            assert!(prompt.contains("Call Bob about the roof") && prompt.contains("## Key points"), "{prompt}");
            let content = "```markdown\nA plan for the roof.\n\n## Key points\n- Bob does roofs\n\n## Action items\n- [ ] Call Bob\n```";
            axum::Json(json!({ "choices": [{ "message": { "role": "assistant", "content": content } }] }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, fake).await });
    url
}

#[tokio::test]
async fn notes_are_summarized_on_request() {
    let fake_url = fake_summarizer().await;
    let dir = TempDir::new().unwrap();
    let server = start(&dir).await;
    auth::create_user(&server.state.db, "root", "", "password123", true).await.unwrap();
    let root = login(&server.base, "root").await;
    let alice = user(&server, "alice").await;
    let bob = user(&server, "bob").await;
    assert_eq!(alice.get("/features").await["summaries"], false);
    let saved = root.send(reqwest::Method::PUT, "/admin/settings/summary", json!({ "url": fake_url, "model": "llama3.1" })).await;
    assert!(saved.status().is_success());
    assert_eq!(root.get("/admin/settings").await["summary"]["model"], "llama3.1");
    assert_eq!(alice.get("/features").await["summaries"], true);

    let res = import(&alice, None, vec![("Plan.md", b"Call Bob about the roof\nBuy nails".to_vec()), ("Short.md", b"# Short".to_vec())]).await;
    assert!(res.status().is_success());
    let tree = alice.get("/tree").await;
    let note = |title: &str| tree["notes"].as_array().unwrap().iter().find(|n| n["title"] == title).unwrap()["id"].as_str().unwrap().to_owned();
    let plan = note("Plan");

    // Nothing until someone asks; then it's made, cleaned of fences, and kept.
    assert!(alice.get(&format!("/notes/{plan}/summary")).await["summary"].is_null());
    let made = alice.post(&format!("/notes/{plan}/summary"), json!({})).await;
    assert_eq!(made["summary"], "A plan for the roof.\n\n## Key points\n- Bob does roofs\n\n## Action items\n- [ ] Call Bob");
    let got = alice.get(&format!("/notes/{plan}/summary")).await;
    assert_eq!((got["summary"].clone(), got["stale"].clone()), (made["summary"].clone(), json!(false)));

    // It says when the note no longer matches what was summarized.
    sqlx::query("UPDATE note_summaries SET body_hash = 'older'").execute(&server.state.db).await.unwrap();
    assert_eq!(alice.get(&format!("/notes/{plan}/summary")).await["stale"], true);

    // Readers of the note can see and ask for it; others can't; a one-line note has nothing to summarize.
    let res = bob.send(reqwest::Method::GET, &format!("/notes/{plan}/summary"), json!({})).await;
    assert_eq!(res.status(), 404);
    alice.post("/shares", json!({ "resource_type": "note", "resource_id": plan, "username": "bob", "role": "viewer" })).await;
    assert_eq!(bob.get(&format!("/notes/{plan}/summary")).await["stale"], true);
    let res = alice.send(reqwest::Method::POST, &format!("/notes/{}/summary", note("Short")), json!({})).await;
    assert_eq!(res.status(), 400);
}
