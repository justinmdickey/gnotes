-- Schema from docs/DESIGN.md. Ids are UUIDv7 text; timestamps are unix millis.

CREATE TABLE users (
    id            TEXT PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE,
    display_name  TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    is_admin      INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL
);

CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    user_agent TEXT
);

CREATE TABLE notebooks (
    id         TEXT PRIMARY KEY,
    owner_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    parent_id  TEXT REFERENCES notebooks(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE INDEX notebooks_owner ON notebooks(owner_id);
CREATE INDEX notebooks_parent ON notebooks(parent_id);

CREATE TABLE notes (
    id          TEXT PRIMARY KEY,
    owner_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    notebook_id TEXT REFERENCES notebooks(id) ON DELETE SET NULL,
    title       TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    deleted_at  INTEGER
);
CREATE INDEX notes_owner ON notes(owner_id);
CREATE INDEX notes_notebook ON notes(notebook_id);

CREATE TABLE note_snapshots (
    note_id    TEXT PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
    snapshot   BLOB NOT NULL,
    version    BLOB NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE note_updates (
    note_id    TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    data       BLOB NOT NULL,
    user_id    TEXT NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (note_id, seq)
);

CREATE TABLE shares (
    id            TEXT PRIMARY KEY,
    resource_type TEXT NOT NULL CHECK (resource_type IN ('note', 'notebook')),
    resource_id   TEXT NOT NULL,
    user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role          TEXT NOT NULL CHECK (role IN ('editor', 'viewer')),
    hidden        INTEGER NOT NULL DEFAULT 0,
    created_by    TEXT NOT NULL REFERENCES users(id),
    created_at    INTEGER NOT NULL,
    UNIQUE (resource_type, resource_id, user_id)
);
CREATE INDEX shares_user ON shares(user_id);

CREATE TABLE attachments (
    id          TEXT PRIMARY KEY,
    note_id     TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    uploader_id TEXT NOT NULL REFERENCES users(id),
    filename    TEXT NOT NULL,
    mime        TEXT NOT NULL,
    size        INTEGER NOT NULL,
    sha256      TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX attachments_note ON attachments(note_id);
