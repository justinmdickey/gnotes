use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use gnotes_server::{AppState, Config, auth, build, serve};
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
    let config = Config {
        data_dir: dir.path().to_path_buf(),
        bind: "127.0.0.1:0".parse().unwrap(),
        web_dir: dir.path().join("web"),
        public_url: None,
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
