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

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Features {
    #[serde(default)]
    pub transcription: bool,
    #[serde(default)]
    pub max_upload: u64,
    #[serde(default)]
    pub version: String,
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

/// `notes.example.com` becomes `https://notes.example.com`; a trailing slash or `/api` goes.
pub fn normalize_server(input: &str) -> String {
    let s = input.trim().trim_end_matches('/');
    let s = s.strip_suffix("/api").unwrap_or(s);
    if s.contains("://") { s.to_owned() } else { format!("https://{s}") }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("Gnotes GTK/", env!("CARGO_PKG_VERSION")))
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
        let base = normalize_server(server);
        let http = client();
        let res = http
            .post(format!("{base}/api/auth/login"))
            .json(&json!({ "username": username, "password": password }))
            .send()
            .await
            .map_err(|e| Error::Offline(e.to_string()))?;
        if res.status() == StatusCode::UNAUTHORIZED {
            return Err(Error::Server {
                status: 401,
                code: "unauthorized".into(),
                message: "Wrong username or password".into(),
            });
        }
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
