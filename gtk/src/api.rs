//! The server's REST API, as `web/src/lib/api.ts` uses it. Runs on the tokio runtime; no GTK here.

use reqwest::{Method, StatusCode, header};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

pub const SESSION_COOKIE: &str = "gnotes_session";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Editor,
    Owner,
}

impl Role {
    pub fn can_edit(self) -> bool {
        self >= Role::Editor
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub display_name: String,
    #[serde(default)]
    pub is_admin: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeNotebook {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub owner: String,
    pub role: Role,
    pub updated_at: i64,
    /// Has a share of its own, either given by you or to you.
    pub shared: bool,
    #[serde(default)]
    pub shared_with: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeNote {
    pub id: String,
    pub notebook_id: Option<String>,
    pub title: String,
    pub preview: String,
    pub owner: String,
    pub role: Role,
    pub updated_at: i64,
    pub shared: bool,
    #[serde(default)]
    pub shared_with: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SharedRoot {
    pub share_id: String,
    /// "note" or "notebook".
    pub resource_type: String,
    pub resource_id: String,
    pub hidden: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Tree {
    pub notebooks: Vec<TreeNotebook>,
    pub notes: Vec<TreeNote>,
    #[serde(default)]
    pub shared: Vec<SharedRoot>,
}

/// Something you deleted directly, not what was swept along inside a deleted notebook.
#[derive(Clone, Debug, Deserialize)]
pub struct TrashItem {
    pub resource_type: String,
    pub id: String,
    pub name: String,
    pub deleted_at: i64,
    pub purge_at: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ShareInfo {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub role: Role,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UserSummary {
    pub id: String,
    pub username: String,
    pub display_name: String,
}

/// A piece of a search snippet; `hit` pieces are the words that matched.
#[derive(Clone, Debug, Deserialize)]
pub struct SnippetPart {
    pub text: String,
    pub hit: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SearchResult {
    pub note: String,
    pub title: String,
    pub snippet: Vec<SnippetPart>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AttachmentMeta {
    pub id: String,
    pub note_id: String,
    pub filename: String,
    pub mime: String,
    pub size: u64,
}

/// Which optional services the server has on.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Features {
    pub transcription: bool,
    /// Text appears in the note while a voice memo is being recorded.
    pub live_transcription: bool,
    pub photo_text: bool,
    pub summaries: bool,
    pub semantic_search: bool,
    pub ask: bool,
    pub max_upload: u64,
    pub version: String,
}

/// A key an agent or script uses to reach /api/v1 as you. The key itself is only shown when made.
#[derive(Clone, Debug, Deserialize)]
pub struct ApiKey {
    pub id: String,
    pub name: String,
    /// "read" or "write".
    pub scope: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AdminUser {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub is_admin: bool,
    pub disabled: bool,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PendingInvite {
    pub id: String,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub role: String,
    pub created_at: i64,
    pub expires_at: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct InvitePreview {
    pub inviter: String,
    /// The shared note's or notebook's name, if the invite shares one.
    pub shared: Option<String>,
    pub expires_at: i64,
}

/// An outside service the server calls, as Settings shows it. Never the key.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ServiceSettings {
    pub enabled: bool,
    pub url: String,
    pub model: String,
    pub has_key: bool,
    pub from_env: bool,
    /// Speech-to-text only: the live (websocket) endpoint.
    pub realtime_url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct AllServices {
    pub whisper: ServiceSettings,
    pub vision: ServiceSettings,
    pub summary: ServiceSettings,
    pub embed: ServiceSettings,
}

/// What Settings sends for a service. `key`: None keeps the saved one, Some("") clears it.
#[derive(Clone, Debug, Serialize)]
pub struct ServiceInput {
    pub url: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub realtime_url: Option<String>,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ImportResult {
    pub notes: u64,
    pub notebooks: u64,
    pub attachments: u64,
    #[serde(default)]
    pub skipped: Vec<String>,
    pub notebook_id: Option<String>,
}

/// A note's AI summary; `summary` is None until someone asks for one.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct NoteSummary {
    pub summary: Option<String>,
    #[serde(default)]
    pub stale: bool,
    pub created_at: Option<i64>,
}

/// Whose editing session a Loro peer was.
#[derive(Clone, Debug, Deserialize)]
pub struct Author {
    pub user_id: Option<String>,
    pub name: Option<String>,
}

/// One message of an Ask conversation: role "user" or "assistant".
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AskMessage {
    pub role: String,
    pub text: String,
}

/// A note Ask found; `n` is its number in the answer's [n] citations.
#[derive(Clone, Debug, Deserialize)]
pub struct AskNote {
    pub n: u32,
    pub note: String,
    pub title: String,
    pub snippet: Vec<SnippetPart>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AskSources {
    pub query: String,
    #[serde(default)]
    pub terms: Vec<String>,
    pub notes: Vec<AskNote>,
}

/// Ask's finished answer, with [n] citations; None when the notes don't say.
#[derive(Clone, Debug, Deserialize)]
pub struct AskDone {
    pub answer: Option<String>,
    #[serde(default)]
    pub cited: Vec<u32>,
}

/// What an Ask answer streams.
#[derive(Clone, Debug)]
pub enum AskEvent {
    Sources(AskSources),
    Text(String),
    Done(AskDone),
    Error(String),
}

#[derive(Debug)]
pub enum Error {
    /// The server couldn't be reached.
    Offline(String),
    /// The session is gone (logged out elsewhere, expired).
    Unauthorized,
    /// The server said no, with its message.
    Server { status: u16, code: String, message: String },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Offline(e) => write!(f, "Couldn't reach the server ({e})"),
            Error::Unauthorized => write!(f, "Signed out"),
            Error::Server { message, .. } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    pub fn is_offline(&self) -> bool {
        matches!(self, Error::Offline(_))
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Server { status, .. } => Some(*status),
            Error::Unauthorized => Some(401),
            Error::Offline(_) => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A signed-in connection to one server. Cheap to clone.
#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    /// e.g. `http://localhost:8080`, without a trailing slash.
    pub base: String,
    pub token: String,
}

/// The token from an invite link (`https://notes.example.com/join/<token>`), or the token itself.
pub fn invite_token(link: &str) -> &str {
    let link = link.trim().trim_end_matches('/');
    link.rsplit_once("/join/").map_or(link, |(_, t)| t)
}

/// `notes.example.com` becomes `https://notes.example.com`; a trailing slash or `/api` goes.
pub fn normalize_server(input: &str) -> String {
    let s = input.trim().trim_end_matches('/');
    let s = s.strip_suffix("/api").unwrap_or(s);
    if s.contains("://") { s.to_owned() } else { format!("https://{s}") }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(format!("Gnotes GTK/{}", crate::VERSION))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("HTTP client")
}

async fn read<T: DeserializeOwned>(res: reqwest::Response) -> Result<T> {
    let status = res.status();
    if status == StatusCode::UNAUTHORIZED {
        return Err(Error::Unauthorized);
    }
    if !status.is_success() {
        let body: Value = res.json().await.unwrap_or_default();
        let text = |k: &str| body.get(k).and_then(Value::as_str).map(str::to_owned);
        return Err(Error::Server {
            status: status.as_u16(),
            code: text("error").unwrap_or_else(|| "unknown".into()),
            message: text("message").unwrap_or_else(|| status.canonical_reason().unwrap_or("Error").into()),
        });
    }
    res.json().await.map_err(|e| Error::Offline(e.to_string()))
}

impl Api {
    /// Signs in with a password and keeps the session cookie it gets.
    pub async fn login(server: &str, username: &str, password: &str) -> Result<(Api, User)> {
        let body = json!({ "username": username, "password": password });
        match Api::start_session(server, "/auth/login", body).await {
            Err(Error::Unauthorized) => Err(Error::Server {
                status: 401,
                code: "unauthorized".into(),
                message: "Wrong username or password".into(),
            }),
            r => r,
        }
    }

    /// Makes a fresh server's first account, an admin, and signs in.
    pub async fn setup(server: &str, username: &str, display_name: &str, password: &str) -> Result<(Api, User)> {
        let body = json!({ "username": username, "display_name": display_name, "password": password });
        Api::start_session(server, "/auth/setup", body).await
    }

    /// Whether a server still needs its first account.
    pub async fn setup_needed(server: &str) -> Result<bool> {
        let res = client()
            .get(format!("{}/api/auth/setup", normalize_server(server)))
            .send()
            .await
            .map_err(|e| Error::Offline(e.to_string()))?;
        let v: Value = read(res).await?;
        Ok(v["needed"].as_bool().unwrap_or(false))
    }

    /// Who an invite link is from and what it shares. `link` is the whole link or just its token.
    pub async fn preview_invite(server: &str, link: &str) -> Result<InvitePreview> {
        let res = client()
            .get(format!("{}/api/join/{}", normalize_server(server), invite_token(link)))
            .send()
            .await
            .map_err(|e| Error::Offline(e.to_string()))?;
        read(res).await
    }

    /// Makes an account with an invite link and signs in.
    pub async fn join(server: &str, link: &str, username: &str, display_name: &str, password: &str) -> Result<(Api, User)> {
        let body = json!({ "username": username, "display_name": display_name, "password": password });
        Api::start_session(server, &format!("/join/{}", invite_token(link)), body).await
    }

    async fn start_session(server: &str, path: &str, body: Value) -> Result<(Api, User)> {
        let base = normalize_server(server);
        let http = client();
        let res = http
            .post(format!("{base}/api{path}"))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Offline(e.to_string()))?;
        let token = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|c| c.strip_prefix(&format!("{SESSION_COOKIE}=")))
            .map(|c| c.split(';').next().unwrap_or_default().to_owned());
        let user: User = read(res).await?;
        let token = token.ok_or_else(|| Error::Offline("the server sent no session".into()))?;
        Ok((Api { http, base, token }, user))
    }

    /// A saved session, not yet checked.
    pub fn resume(base: String, token: String) -> Api {
        Api { http: client(), base, token }
    }

    /// `ws://` or `wss://` URL of the sync socket.
    pub fn ws_url(&self) -> String {
        let rest = self.base.split_once("://").map_or(self.base.as_str(), |(_, r)| r);
        let scheme = if self.base.starts_with("https://") { "wss" } else { "ws" };
        format!("{scheme}://{rest}/api/ws")
    }

    pub fn cookie(&self) -> String {
        format!("{SESSION_COOKIE}={}", self.token)
    }

    fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}/api{path}", self.base))
            .header(header::COOKIE, self.cookie())
    }

    async fn send<T: DeserializeOwned>(&self, method: Method, path: &str, body: Option<Value>) -> Result<T> {
        let mut req = self.request(method, path);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await.map_err(|e| Error::Offline(e.to_string()))?;
        read(res).await
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.send(Method::GET, path, None).await
    }

    async fn post<T: DeserializeOwned>(&self, path: &str, body: Value) -> Result<T> {
        self.send(Method::POST, path, Some(body)).await
    }

    async fn patch(&self, path: &str, body: Value) -> Result<Value> {
        self.send(Method::PATCH, path, Some(body)).await
    }

    async fn delete(&self, path: &str) -> Result<Value> {
        self.send(Method::DELETE, path, None).await
    }

    pub async fn me(&self) -> Result<User> {
        self.get("/me").await
    }

    pub async fn logout(&self) -> Result<()> {
        self.post::<Value>("/auth/logout", json!({})).await.map(drop)
    }

    pub async fn tree(&self) -> Result<Tree> {
        self.get("/tree").await
    }

    pub async fn features(&self) -> Result<Features> {
        self.get("/features").await
    }

    pub async fn users(&self) -> Result<Vec<UserSummary>> {
        self.get("/users").await
    }

    pub async fn create_notebook(&self, name: &str, parent_id: Option<&str>) -> Result<String> {
        let v: Value = self.post("/notebooks", json!({ "name": name, "parent_id": parent_id })).await?;
        Ok(v["id"].as_str().unwrap_or_default().to_owned())
    }

    pub async fn rename_notebook(&self, id: &str, name: &str) -> Result<()> {
        self.patch(&format!("/notebooks/{id}"), json!({ "name": name })).await.map(drop)
    }

    pub async fn move_notebook(&self, id: &str, parent_id: Option<&str>) -> Result<()> {
        self.patch(&format!("/notebooks/{id}"), json!({ "parent_id": parent_id })).await.map(drop)
    }

    pub async fn delete_notebook(&self, id: &str) -> Result<()> {
        self.delete(&format!("/notebooks/{id}")).await.map(drop)
    }

    /// `id` is a client-made UUIDv7, so a note made offline keeps the same id.
    pub async fn create_note(&self, notebook_id: Option<&str>, id: Option<&str>) -> Result<String> {
        let v: Value = self.post("/notes", json!({ "id": id, "notebook_id": notebook_id })).await?;
        Ok(v["id"].as_str().unwrap_or_default().to_owned())
    }

    pub async fn move_note(&self, id: &str, notebook_id: Option<&str>) -> Result<()> {
        self.patch(&format!("/notes/{id}"), json!({ "notebook_id": notebook_id })).await.map(drop)
    }

    pub async fn delete_note(&self, id: &str) -> Result<()> {
        self.delete(&format!("/notes/{id}")).await.map(drop)
    }

    /// Permanently removes the note only if it's blank; the server refuses otherwise.
    pub async fn discard_note(&self, id: &str) -> Result<()> {
        self.delete(&format!("/notes/{id}?discard=true")).await.map(drop)
    }

    pub async fn trash(&self) -> Result<Vec<TrashItem>> {
        self.get("/trash").await
    }

    pub async fn restore(&self, kind: &str, id: &str) -> Result<()> {
        self.post::<Value>(&format!("/trash/{kind}/{id}/restore"), json!({})).await.map(drop)
    }

    pub async fn delete_forever(&self, kind: &str, id: &str) -> Result<()> {
        self.delete(&format!("/trash/{kind}/{id}")).await.map(drop)
    }

    pub async fn empty_trash(&self) -> Result<()> {
        self.delete("/trash").await.map(drop)
    }

    pub async fn search(&self, q: &str) -> Result<Vec<SearchResult>> {
        #[derive(Deserialize)]
        struct Results {
            results: Vec<SearchResult>,
        }
        let mut url = reqwest::Url::parse("http://x/search").expect("url");
        url.query_pairs_mut().append_pair("q", q);
        let path = format!("/search?{}", url.query().unwrap_or_default());
        Ok(self.get::<Results>(&path).await?.results)
    }

    /// `kind` is "note" or "notebook".
    pub async fn shares(&self, kind: &str, id: &str) -> Result<Vec<ShareInfo>> {
        self.get(&format!("/{kind}s/{id}/shares")).await
    }

    pub async fn share(&self, kind: &str, id: &str, username: &str, role: Role) -> Result<()> {
        let body = json!({ "resource_type": kind, "resource_id": id, "username": username, "role": role });
        self.post::<Value>("/shares", body).await.map(drop)
    }

    pub async fn set_share_role(&self, share_id: &str, role: Role) -> Result<()> {
        self.patch(&format!("/shares/{share_id}"), json!({ "role": role })).await.map(drop)
    }

    pub async fn unshare(&self, share_id: &str) -> Result<()> {
        self.delete(&format!("/shares/{share_id}")).await.map(drop)
    }

    /// Hides or shows something shared with you in your own Shared list.
    pub async fn hide_share(&self, share_id: &str, hidden: bool) -> Result<()> {
        self.patch(&format!("/shares/{share_id}"), json!({ "hidden": hidden })).await.map(drop)
    }

    pub async fn update_me(&self, display_name: &str) -> Result<User> {
        self.send(Method::PATCH, "/me", Some(json!({ "display_name": display_name }))).await
    }

    pub async fn change_password(&self, current: &str, new: &str) -> Result<()> {
        self.post::<Value>("/me/password", json!({ "current": current, "new": new })).await.map(drop)
    }

    pub async fn logout_others(&self) -> Result<()> {
        self.post::<Value>("/me/logout-others", json!({})).await.map(drop)
    }

    pub async fn api_keys(&self) -> Result<Vec<ApiKey>> {
        self.get("/me/keys").await
    }

    /// The new key's token, shown only this once.
    pub async fn create_api_key(&self, name: &str, scope: &str) -> Result<String> {
        let v: Value = self.post("/me/keys", json!({ "name": name, "scope": scope })).await?;
        Ok(v["token"].as_str().unwrap_or_default().to_owned())
    }

    pub async fn revoke_api_key(&self, id: &str) -> Result<()> {
        self.delete(&format!("/me/keys/{id}")).await.map(drop)
    }

    pub async fn admin_users(&self) -> Result<Vec<AdminUser>> {
        self.get("/admin/users").await
    }

    pub async fn admin_update_user(&self, id: &str, patch: Value) -> Result<()> {
        self.patch(&format!("/admin/users/{id}"), patch).await.map(drop)
    }

    pub async fn admin_reset_password(&self, id: &str, password: &str) -> Result<()> {
        self.post::<Value>(&format!("/admin/users/{id}/password"), json!({ "password": password })).await.map(drop)
    }

    /// An invite link's token; with `share`, joining also shares that ("note" or "notebook", id).
    pub async fn invite(&self, share: Option<(&str, &str)>, role: Role) -> Result<String> {
        let body = json!({ "resource_type": share.map(|s| s.0), "resource_id": share.map(|s| s.1), "role": role });
        let v: Value = self.post("/invites", body).await?;
        Ok(v["token"].as_str().unwrap_or_default().to_owned())
    }

    pub async fn invites(&self) -> Result<Vec<PendingInvite>> {
        self.get("/invites").await
    }

    pub async fn revoke_invite(&self, id: &str) -> Result<()> {
        self.delete(&format!("/invites/{id}")).await.map(drop)
    }

    pub async fn services(&self) -> Result<AllServices> {
        self.get("/admin/settings").await
    }

    /// `kind` is whisper, vision, summary or embed. Returns what the server kept.
    pub async fn save_service(&self, kind: &str, body: &ServiceInput) -> Result<ServiceSettings> {
        let mut v: Value = self.send(Method::PUT, &format!("/admin/settings/{kind}"), Some(json!(body))).await?;
        serde_json::from_value(v[kind].take()).map_err(|e| Error::Offline(e.to_string()))
    }

    pub async fn test_service(&self, kind: &str, body: &ServiceInput) -> Result<TestResult> {
        self.post(&format!("/admin/settings/{kind}/test"), json!(body)).await
    }

    /// Embeds every note again; how many.
    pub async fn reindex(&self) -> Result<u64> {
        let v: Value = self.post("/admin/semantic/reindex", json!({})).await?;
        Ok(v["notes"].as_u64().unwrap_or(0))
    }

    /// Markdown files or zips of them, as (file name, bytes); a zip becomes a notebook.
    pub async fn import(&self, notebook_id: Option<&str>, files: Vec<(String, Vec<u8>)>) -> Result<ImportResult> {
        let mut form = reqwest::multipart::Form::new();
        if let Some(nb) = notebook_id {
            form = form.text("notebook_id", nb.to_owned());
        }
        for (name, bytes) in files {
            form = form.part("file", reqwest::multipart::Part::bytes(bytes).file_name(name));
        }
        let res = self
            .request(Method::POST, "/import")
            .multipart(form)
            .send()
            .await
            .map_err(|e| Error::Offline(e.to_string()))?;
        if res.status() == StatusCode::PAYLOAD_TOO_LARGE {
            return Err(Error::Server { status: 413, code: "too_large".into(), message: "Imports can be up to 200 MB at a time".into() });
        }
        read(res).await
    }

    pub async fn summary(&self, note: &str) -> Result<NoteSummary> {
        self.get(&format!("/notes/{note}/summary")).await
    }

    pub async fn summarize(&self, note: &str) -> Result<NoteSummary> {
        self.post(&format!("/notes/{note}/summary"), json!({})).await
    }

    /// `text` improved by the summary chat model, with nothing of it lost; nothing saved.
    pub async fn tidy(&self, note: &str, text: &str) -> Result<String> {
        let v: Value = self.post(&format!("/notes/{note}/tidy"), json!({ "text": text })).await?;
        Ok(v["text"].as_str().unwrap_or_default().to_owned())
    }

    pub async fn edit_with_ai(&self, note: &str, instruction: &str, text: &str) -> Result<String> {
        let v: Value = self.post(&format!("/notes/{note}/edit"), json!({ "instruction": instruction, "text": text })).await?;
        Ok(v["text"].as_str().unwrap_or_default().to_owned())
    }

    /// Markdown for position `at` (UTF-16 code units, as the PWA counts) of `text`.
    pub async fn write_with_ai(&self, note: &str, prompt: &str, text: &str, at: usize) -> Result<String> {
        let v: Value = self.post(&format!("/notes/{note}/write"), json!({ "prompt": prompt, "text": text, "at": at })).await?;
        Ok(v["text"].as_str().unwrap_or_default().to_owned())
    }

    /// Whose editing session each Loro peer (by its number) was.
    pub async fn authors(&self, note: &str) -> Result<std::collections::HashMap<String, Author>> {
        let mut v: Value = self.get(&format!("/notes/{note}/authors")).await?;
        serde_json::from_value(v["peers"].take()).map_err(|e| Error::Offline(e.to_string()))
    }

    pub async fn photo_text(&self, attachment: &str) -> Result<String> {
        let v: Value = self.post(&format!("/attachments/{attachment}/text"), json!({})).await?;
        Ok(v["text"].as_str().unwrap_or_default().to_owned())
    }

    pub async fn transcribe(&self, attachment: &str) -> Result<String> {
        let v: Value = self.post(&format!("/attachments/{attachment}/transcribe"), json!({})).await?;
        Ok(v["text"].as_str().unwrap_or_default().to_owned())
    }

    /// Notes about what `q` means, even without its words. Empty when semantic search is off.
    pub async fn search_meaning(&self, q: &str) -> Result<Vec<SearchResult>> {
        #[derive(Deserialize)]
        struct Results {
            results: Vec<SearchResult>,
        }
        let mut url = reqwest::Url::parse("http://x/search/meaning").expect("url");
        url.query_pairs_mut().append_pair("q", q);
        let path = format!("/search/meaning?{}", url.query().unwrap_or_default());
        Ok(self.get::<Results>(&path).await?.results)
    }

    /// The next turn of an Ask conversation (`messages` ends with the new question). Each piece of
    /// the answer goes to `on` as it streams in.
    pub async fn ask(&self, messages: &[AskMessage], on: impl Fn(AskEvent)) -> Result<()> {
        let res = self
            .request(Method::POST, "/ask")
            .json(&json!({ "messages": messages }))
            .send()
            .await
            .map_err(|e| Error::Offline(e.to_string()))?;
        if !res.status().is_success() {
            return read::<Value>(res).await.map(drop);
        }
        let mut res = res;
        // Server-sent events: blocks of "event: <name>" and "data: <json>" lines, a blank line apart.
        let mut buffer = String::new();
        while let Some(chunk) = res.chunk().await.map_err(|e| Error::Offline(e.to_string()))? {
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(end) = buffer.find("\n\n") {
                let block: String = buffer.drain(..end + 2).collect();
                let field = |name: &str| {
                    block.lines().find_map(|l| l.strip_prefix(name)).map(|v| v.strip_prefix(' ').unwrap_or(v).to_owned())
                };
                let (Some(event), Some(data)) = (field("event:"), field("data:")) else { continue };
                let Ok(body) = serde_json::from_str::<Value>(&data) else { continue };
                let ev = match event.as_str() {
                    "sources" => serde_json::from_value(body).ok().map(AskEvent::Sources),
                    "delta" => body["text"].as_str().map(|t| AskEvent::Text(t.to_owned())),
                    "done" => serde_json::from_value(body).ok().map(AskEvent::Done),
                    "error" => Some(AskEvent::Error(body["message"].as_str().unwrap_or("Couldn't answer").to_owned())),
                    _ => None,
                };
                if let Some(ev) = ev {
                    let last = matches!(ev, AskEvent::Done(_) | AskEvent::Error(_));
                    on(ev);
                    if last {
                        return Ok(());
                    }
                }
            }
        }
        on(AskEvent::Error("The answer stopped partway".into()));
        Ok(())
    }

    pub async fn attachment(&self, id: &str) -> Result<Vec<u8>> {
        let res = self
            .request(Method::GET, &format!("/attachments/{id}"))
            .send()
            .await
            .map_err(|e| Error::Offline(e.to_string()))?;
        if !res.status().is_success() {
            return read::<Value>(res).await.map(|_| Vec::new());
        }
        res.bytes().await.map(|b| b.to_vec()).map_err(|e| Error::Offline(e.to_string()))
    }

    pub async fn attachment_meta(&self, id: &str) -> Result<AttachmentMeta> {
        self.get(&format!("/attachments/{id}/meta")).await
    }

    pub async fn upload(&self, note_id: &str, filename: &str, mime: &str, bytes: Vec<u8>) -> Result<AttachmentMeta> {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename.to_owned())
            .mime_str(mime)
            .map_err(|e| Error::Offline(e.to_string()))?;
        let form = reqwest::multipart::Form::new().text("note_id", note_id.to_owned()).part("file", part);
        let res = self
            .request(Method::POST, "/attachments")
            .multipart(form)
            .send()
            .await
            .map_err(|e| Error::Offline(e.to_string()))?;
        read(res).await
    }
}
