# Gnotes design

Gnotes is a self-hosted Markdown notes app. One server holds a household's accounts and notes. Phones and desktops use an installable web app (PWA) that works offline and edits live with other people. A native GTK client comes later and speaks the same protocol.

## Decisions

| Topic | Decision |
|---|---|
| Hosting | Self-hosted server. One binary or Docker image, SQLite, one data folder. |
| Clients | PWA first. GTK4/libadwaita client later. |
| Sync | Loro CRDT, one Loro document per note, over a websocket. |
| Live editing | Full live: edits, remote cursors and presence from day one. |
| Sharing | Single notes and whole notebooks, with other accounts on the same server only. |
| Roles | Owner, editor, viewer. |
| Encryption | TLS in transit. No end-to-end encryption, so the server admin can read notes. |
| Speech-to-text | The server forwards voice memos to an external OpenAI-compatible transcription API (e.g. faster-whisper). The note keeps the recording and the transcript. |
| AI summaries | On demand: a note's Summary tab sends the whole note to an OpenAI-compatible chat API (local or hosted) the first time it's opened, and shows the gist, key points and action items read-only. Kept in `note_summaries` beside the note, not in its text, with a hash of the text it covers so the tab can say when the note has changed. |
| Text from photos | On demand: Get Text on a photo (on hover, always on touch) sends it to an OpenAI-compatible chat API with a vision model (e.g. Ollama or llama.cpp on local hardware), and the text it reads goes under the photo. |
| AI search | Optional. Notes are embedded in chunks through an OpenAI-compatible embeddings API, in the background after edits settle. Search adds notes close in meaning; Ask sends the closest chunks of the user's visible notes to the summary chat model and shows its answer with links to those notes. See [AI search](#ai-search). |

**CRDT** (conflict-free replicated data type): a data structure where edits made on different devices, even offline, merge automatically without conflicts.

## Stack

- **Server:** Rust, axum, tokio, sqlx (SQLite), `loro`, argon2. It serves the built PWA itself.
- **PWA:** TypeScript, Svelte 5, Vite, CodeMirror 6, `loro-crdt`, `loro-codemirror` (sync, remote cursors, collaborative undo), vite-plugin-pwa.
- **Styling:** Adwaita-like CSS with light and dark themes that follow `prefers-color-scheme`.

Server and PWA use the same Loro binary update format, which is what lets a native client plug in later.

## Data model

Structure (notebooks, note placement, permissions) lives in SQL and is changed through REST. Only note content is a CRDT. Moving or renaming things goes through the server, which keeps permission checks simple.

```
users         id, username, display_name, password_hash, is_admin, created_at
sessions      token_hash, user_id, created_at, expires_at, user_agent
notebooks     id, owner_id, parent_id?, name, created_at, updated_at, deleted_at?
notes         id, owner_id, notebook_id?, title, created_at, updated_at, deleted_at?
note_snapshots note_id, snapshot BLOB, version BLOB, updated_at
note_updates  note_id, seq, data BLOB, user_id, created_at
shares        id, resource_type ('note'|'notebook'), resource_id, user_id,
              role ('editor'|'viewer'), created_by, created_at
attachments   id, note_id, uploader_id, filename, mime, size, sha256, created_at
```

- All ids are UUIDv7 (time-ordered), stored as text.
- `notes.title` is a cache taken from the first line of the note. The server updates it after each change.
- A note with no `notebook_id` sits at the owner's top level.
- `deleted_at` means the item is in the trash. The server permanently deletes items that have been in the trash for 30 days.

### Note content

Each note is a `LoroDoc` with one `LoroText` container named `body` holding the Markdown. The server stores:

- **`note_snapshots`:** a full snapshot and its version vector (a compact record of which edits a copy already has).
- **`note_updates`:** an append-only log of updates since that snapshot, tagged with the user who sent each one. This is the basis for "who changed what" history.

Compaction: when a note's room closes, or after 500 logged updates, the server writes a new snapshot and deletes the log rows it covers.

### Plain Markdown copies

The server keeps a readable copy of every note on disk, a few seconds after each edit:

```
data/export/<username>/<notebook path>/<title> (<short id>).md
data/export/<username>/<notebook path>/attachments/<filename> (<short id>).<ext>
```

These copies are for backup and leaving the app. The SQL database and Loro snapshots remain the source of truth.

- `<username>` is the note's owner, so a shared note is copied once, under its owner. The short id is the last 8 characters of the note's or attachment's id.
- Titles and notebook names lose characters that aren't safe in file names (`/ \ : * ? " < > |`, leading dots).
- One background task (`server/src/export.rs`) keeps the folder in line with the database, about 2 seconds after an edit or tree change, and once at startup so existing notes get copies. Each copy's modified time is its note's last edit; a copy whose time differs is rewritten.
- Renames, moves, trash and deletes remove the old copy and empty folders. Restoring brings it back. Blank notes have no copy. Files the server didn't write are left alone.
- Copies are written to a temporary file and renamed. A failed write is logged and never affects editing.

### Attachments

- Uploaded files are stored by content hash at `data/blobs/<sha256[0..2]>/<sha256>`.
- Notes reference them as `![alt](att:<attachment id>)`. The PWA resolves `att:` to `/api/attachments/<id>`.
- The plain Markdown copies rewrite `att:` links to relative `attachments/` paths.
- Audio recordings use the same syntax. The editor shows an audio player for audio MIME types.
- Every attachment request checks the requester's access to the note it belongs to.

### Import

`POST /import` brings existing Markdown in, for moving from Obsidian, Bear or a plain folder. The PWA sends files from Account › Import, or from a drop anywhere on the window into the open folder.

- A `.zip` becomes a notebook named after its one top folder, or after the zip. Folders inside become notebooks; folders with no notes (e.g. `attachments/`) don't.
- Loose `.md`/`.txt` files go straight into the target notebook.
- `![alt](path)` and Obsidian's `![[path]]` to an image or recording in the zip become attachments. Paths resolve next to the note first, then by file name anywhere in the zip.
- YAML front matter is dropped; its `title:` is kept. A note whose first line isn't a heading gets `# <file name>` on top, so the list shows a real title.
- Notes keep the zip's modified times, so an import doesn't flood Recent. Hidden files and folders (`.obsidian`) are ignored; anything else unused is listed as skipped.
- Content is written as a fresh Loro snapshot. Everything lands in one transaction, so a failed import leaves no half-made notebooks.

## Permissions

A user's effective role on a note is the highest of:

1. `owner`, if they own the note.
2. A share on the note itself.
3. A share on the note's notebook or any parent notebook.

`owner > editor > viewer`. No role means the note doesn't exist for that user, and the server returns 404, not 403.

| Action | Viewer | Editor | Owner |
|---|---|---|---|
| Read, see cursors, add own presence | yes | yes | yes |
| Edit content, upload attachments | | yes | yes |
| Move within a shared notebook | | yes | yes |
| Rename or delete a notebook, trash a note | | | yes |
| Share or unshare | | | yes |

- Removing someone's access closes their open websocket rooms for that note with `revoked`. A copy already saved offline on their device can't be pulled back.
- Anyone can hide a shared item from their own "Shared with me" list. This is a per-user flag and doesn't change the share.

## Auth

- Username and password, hashed with argon2id.
- Sessions use an HttpOnly, Secure, SameSite=Lax cookie holding a random token. The database stores only the token's hash.
- The first admin is created with `gnotes-server create-user --admin <username>`. Admins invite others with one-time links (7 days) that can also share a note or notebook.
- Browsers only allow service workers and microphone access over HTTPS. The server expects a reverse proxy that handles TLS, such as Caddy or Tailscale Serve. The docs should make that the default setup.

## REST API

All endpoints are under `/api` and need a session unless marked otherwise. They take and return JSON.

```
POST   /auth/login                 (public)
POST   /auth/logout
GET    /me
GET    /users                      everyone on the server, for the share picker

GET    /tree                       notebooks + note metadata visible to me
POST   /notebooks                  {name, parent_id?}
PATCH  /notebooks/:id              {name?, parent_id?}
DELETE /notebooks/:id              to trash
POST   /notes                      {id?, notebook_id?}  -> {id}
PATCH  /notes/:id                  {notebook_id?}
DELETE /notes/:id                  to trash
GET    /trash                      my directly deleted items
POST   /trash/:type/:id/restore

GET    /search?q=                  full-text search of notes I can see -> {results}
GET    /search/meaning?q=          semantic search of notes I can see -> {results}, empty when off
POST   /ask                        {question} -> {answer, sources}; 409 unless Ask is set up

GET    /notes/:id/shares           owner only (same for /notebooks/:id/shares)
POST   /shares                     {resource_type, resource_id, username, role}
PATCH  /shares/:id                 {role} (owner) or {hidden} (recipient)
DELETE /shares/:id

POST   /attachments                multipart {note_id, file}   (phase 2)
GET    /attachments/:id                                        (phase 2)
POST   /import                     multipart {notebook_id?, file...}: .md, .txt or .zip -> counts, skipped
GET    /features                   which optional services are on: transcription, photo_text, summaries,
                                   semantic_search, ask, ...

GET    /admin/settings             admin: every service's URL, model, has_key, from_env (never the key)
PUT    /admin/settings/:service    admin: {url, model, key?}; service is whisper, vision, summary or embed
POST   /admin/settings/:service/test  admin: checks the service's /models before saving

POST   /admin/users                admin only
GET    /invites                    admin: pending invite links
POST   /invites                    admin: {resource_type?, resource_id?, role} -> {token}; link is /join/<token>
DELETE /invites/:id                admin: revoke
GET    /join/:token                (public) who invited you and what's shared
POST   /join/:token                (public) {username, display_name, password}; logs in, applies the share
GET    /health                     (public)
```

Changes to `/tree` are pushed over the websocket as `tree_changed`, so clients only refetch when something changed.

`/search` matches every word typed as a prefix (`gro mil` finds "Groceries … milk"), anywhere in a note. Quotes and FTS5 operators in `q` are plain text. Results come best first, at most 200, never trashed notes:

```jsonc
{"results": [{"note": "<id>", "title": "Groceries", "source": "text",
              "snippet": [{"text": "…buy ", "hit": false}, {"text": "milk", "hit": true}]}]}
```

- `snippet` is a few words of the text after the title, around the match. A title-only match has no `hit` parts.
- `source` says which search found the note; full-text is `"text"`. Other kinds of search add their own value.
- The server keeps each note's text, with Markdown marks stripped, in an FTS5 index (`note_text`, `note_search`) updated on every edit and import. Notes from before the index existed are added at startup.
- The PWA folds results into the folder being searched and adds any title or preview matches. Offline it has only those.

### AI search

Both parts are off until an admin sets up Settings → Semantic Search (an OpenAI-compatible embeddings API). Ask also needs Settings → AI Summaries, whose chat model writes the answers. `/features` says which are on; the app shows neither when they're off.

**Embeddings.** Each note's search text (`note_text`) is cut into chunks of about 1,000 characters at line breaks. Each chunk is embedded as its note's title plus the chunk, and kept in `note_chunks` (`note_id, idx, text, hash, model, embedding`) with a hash of that text and the model's name.

- One background task (`server/src/semantic.rs`) does the embedding. An edit marks its note; once typing pauses for 3 seconds (at most 30 seconds of continuous typing), the task embeds the chunks whose hash changed and reuses the rest.
- At startup, when the service or model changes, after an import, and hourly, it checks every note. That backfills old notes and picks up anything that failed.
- Requests go out one at a time, up to 16 chunks each. If the service fails, the run stops, is logged, and retries after 1 minute, doubling up to an hour. Editing never waits on it.
- Trashed notes aren't embedded, and keep their chunks for a restore. Deleting a note deletes its chunks.

**`GET /search/meaning?q=`** embeds `q` (3+ letters or digits) and compares it, by cosine similarity, with every chunk of the caller's visible, untrashed notes, using the same visibility as `/search`. That's brute force in Rust, which is plenty for a household. A note's best chunk counts. It must score at least 0.3 and be within 0.15 of the best result; at most 10 come back, as `{"note", "title", "source": "meaning", "snippet": [{"text": "<start of that chunk>", "hit": false}]}`.

**`POST /ask {question}`** finds chunks the same way and sends the closest 6 (about 8,000 characters at most), grouped and numbered by note, to the chat model. The prompt says to answer only from those notes, cite them as `[n]`, keep it short, and reply `NO_ANSWER` when they don't contain the answer.

```jsonc
{"answer": "You still need milk and eggs [1].", "sources": [{"n": 1, "note": "<id>", "title": "Groceries"}]}
{"answer": null, "sources": []}   // nothing close enough, or the model said NO_ANSWER
```

- `sources` are the notes the answer cites, or every note sent when it cites none. Nothing close means the chat model isn't called.
- Only notes the caller can see right now are ever sent. Unsharing or trashing a note takes it out of both search and Ask at once.

**In the app.** With a search typed, a **By Meaning** group lists meaning matches the word search didn't already find, in the folder being searched, under the word matches. An **Ask Your Notes** row with the search text opens the answer in a dialog. The row comes first when the search reads like a question (ends in `?` or starts with a question word), otherwise last. With no matches at all it's the empty page's button. The answer's `[n]` become small numbered links, and the notes it used are listed under it.

## Websocket protocol

A client opens one websocket to `/api/ws` (authenticated by the session cookie) and joins the notes it has open on that one connection.

- **Control messages** are JSON text frames.
- **Document data** goes in binary frames: `[kind: u8][note id: 16 bytes][payload]`.

### Frame kinds

| Kind | Direction | Payload |
|---|---|---|
| `0x01` update | both | Loro update bytes |
| `0x02` presence | both | Encoded Loro `EphemeralStore` changes (cursors, name, color) |

### Control messages

```jsonc
// client -> server
{"t":"join",  "note":"<id>", "version":"<base64 version vector or null>"}
{"t":"leave", "note":"<id>"}

// server -> client
{"t":"joined", "note":"<id>", "role":"editor", "version":"<base64 version vector>"}
{"t":"error",  "note":"<id>", "code":"not_found|forbidden|bad_update|bad_version|out_of_sync"}
{"t":"role",   "note":"<id>", "role":"viewer"}   // a share changed while the note is open
{"t":"revoked","note":"<id>"}
{"t":"tree_changed"}                          // sent to every connection; carries no content
```

### Join and catch-up

1. The client sends `join` with the version vector of its local copy, or `null` if it has none.
2. The server loads the note's room if it isn't already in memory, checks the client's role, and replies `joined` with the server's version vector.
3. The server sends one `0x01` frame with everything the client is missing (`export({mode:"update", from: clientVersion})`, or a full snapshot if the client sent `null`).
4. The client sends one `0x01` frame with everything the server is missing, which covers edits made offline.
5. From then on, both sides stream `0x01` and `0x02` frames as changes happen.

### Server rules

- Each open note has one in-memory room holding the `LoroDoc`, the connected clients and the presence state. A room closes 60 seconds after its last client leaves.
- `0x01` from a viewer is rejected with `forbidden`. Otherwise the server imports the update, appends it to `note_updates`, and forwards it to the room's other clients.
- If an update depends on changes the server doesn't have, the server replies `out_of_sync` and doesn't forward it. The client rejoins with its version, which fills the gap.
- `0x02` presence is relayed to other clients and never saved. Presence entries expire after 30 seconds without a refresh.
- Loro peer ids are random for each session. The server tags every update with the sending user's id, so attribution never relies on peer ids.

## PWA

- **Views:** notebook tree, note list, editor, "Shared with me", trash, share dialog, settings. The layout adapts from a phone (one pane) to a desktop (sidebar, list and editor side by side), the same way libadwaita's split views do.
- **Editor:** CodeMirror 6 with Markdown highlighting and in-place styling instead of a separate preview pane. `LoroExtensions(doc, {ephemeral, user}, undoManager, doc => doc.getText("body"))` handles sync, cursors and undo.
- **Offline:** each opened note's Loro snapshot is saved in IndexedDB a moment after every change, along with a cached copy of `/tree`, in one database per account. With no server the app opens as the last signed-in account (kept in `localStorage`), shows the saved tree and notes, and edits them; phones and tablets show a quiet Offline strip, wide screens the sidebar footer. On reconnect the client rejoins open notes with its saved version, so offline edits merge on their own; notes edited offline and since closed are marked dirty and pushed the same way in the background. Creating a note offline gives it a client-made UUIDv7 id; it's queued and sent with `POST /notes {id}` on reconnect (at the top level if its notebook is gone). Log Out deletes the account's database; being signed out by the server keeps it, since it may hold unsent edits. Notes never opened on the device aren't available offline.
- **Voice notes:** recorded with MediaRecorder (Opus in WebM on Chrome and Firefox, AAC in MP4 on Safari), uploaded as an attachment, and linked into the note. When the server has speech-to-text, the transcript goes on the line under the player. Recording needs HTTPS. Recording shows a small floating bar, not a dialog, so the note stays readable.
- **Live transcripts:** with a live URL set, the app streams 16 kHz PCM from an AudioWorklet over `/api/transcribe/live`, and the server relays it to the realtime endpoint (OpenAI realtime transcription events, flat `session.update`). The browser can't reach that endpoint itself (it's usually plain `ws://` on the server's network). Words are written into the note as they arrive; each final transcript replaces its stretch, and the player goes above it on stop. If nothing comes through live, the whole recording is transcribed as before.
- **Photos:** picked from the camera or library, pasted, or dropped. Images over 2048px or 1.5 MB are shrunk to JPEG in the browser before upload.
- **Install:** a web app manifest plus a service worker that caches the app shell. The UI encourages installing to the home screen, because iOS can clear storage for tabs that aren't installed.

## Deployment

```
docker run -v gnotes-data:/data -p 8080:8080 gnotes
```

- Config comes from environment variables: `GNOTES_DATA_DIR` (default `./data`), `GNOTES_BIND` (default `0.0.0.0:8080`), `GNOTES_PUBLIC_URL` (used to validate the websocket Origin header).
- Speech-to-text is set by an admin under Settings → Speech-to-Text: an OpenAI-compatible API URL including the version (e.g. `http://whisper:8000/v1`), a model and an optional key. Once saved there, it overrides the env defaults `GNOTES_WHISPER_URL`, `GNOTES_WHISPER_MODEL` (default `whisper-1`) and `GNOTES_WHISPER_KEY`. An optional live URL (`ws://` or `wss://`, e.g. `ws://whisper:8000/v1/realtime`, env `GNOTES_WHISPER_REALTIME_URL`) turns on live transcripts. The key is never sent back to the app.
- Text from photos is set the same way under Settings → Text from Photos: an OpenAI-compatible URL including the version (e.g. `http://ollama:11434/v1`), a vision model (required, e.g. `qwen2.5vl`) and an optional key; env defaults `GNOTES_VISION_URL`, `GNOTES_VISION_MODEL`, `GNOTES_VISION_KEY`. `POST /attachments/:id/text` sends the photo as a data URL to `/chat/completions` and asks for the text only, or `NO_TEXT`.
- AI summaries are set the same way under Settings → AI Summaries; env defaults `GNOTES_SUMMARY_URL`, `GNOTES_SUMMARY_MODEL`, `GNOTES_SUMMARY_KEY`. Ask uses this chat model too.
- Semantic search is set under Settings → Semantic Search: an OpenAI-compatible URL including the version (e.g. `http://ollama:11434/v1`), an embedding model (required, e.g. `nomic-embed-text`) and an optional key; env defaults `GNOTES_EMBED_URL`, `GNOTES_EMBED_MODEL`, `GNOTES_EMBED_KEY`. The server calls `/embeddings`. Changing the model re-embeds every note.
- The data folder holds everything: `gnotes.db`, `blobs/` and `export/`. Backing up means copying that folder.

## Phases

1. **Foundation:** auth, schema, tree REST, websocket rooms with live edits and cursors, basic PWA editor. Two browsers editing one note live counts as done.
2. **Daily-use PWA:** offline storage, installable, sharing UI, images and voice recording, trash, Markdown copies on disk.
3. **Extras:** server-side whisper transcription, note history view, search (SQLite FTS5).
4. **GTK client:** Rust, gtk4-rs, libadwaita, the `loro` crate and the same websocket protocol.

## Decided

- **Search:** the server does full-text search (SQLite FTS5). Offline, the app falls back to matching titles and previews of the notes it has cached.
- **AI search:** optional, set up by an admin like Speech-to-Text. Semantic search finds notes by meaning through an OpenAI-compatible embeddings endpoint; Ask writes an answer from the matching notes and links them.
- **Accounts:** sign-up is by admin-made invite link only. Letting anyone with an email at an allowed domain join is planned after 1.0; it needs email addresses and a way to verify them.
