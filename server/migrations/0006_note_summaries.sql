-- AI summaries, one per note, made on demand. body_hash says which version of the note it covers.
CREATE TABLE note_summaries (
    note_id    TEXT PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
    summary    TEXT NOT NULL,
    body_hash  TEXT NOT NULL,
    model      TEXT NOT NULL,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL
);
