-- One-time signup links. Optionally share one note or notebook with the new account.
CREATE TABLE invites (
    id            TEXT PRIMARY KEY,
    token_hash    TEXT NOT NULL UNIQUE,
    created_by    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    resource_type TEXT CHECK (resource_type IN ('note', 'notebook')),
    resource_id   TEXT,
    role          TEXT NOT NULL DEFAULT 'editor' CHECK (role IN ('editor', 'viewer')),
    created_at    INTEGER NOT NULL,
    expires_at    INTEGER NOT NULL,
    used_at       INTEGER,
    used_by       TEXT REFERENCES users(id) ON DELETE SET NULL
);
