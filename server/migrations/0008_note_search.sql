-- Full-text search over notes. `note_text` holds each note's text with Markdown marks stripped,
-- split into its title (first line) and the rest, refreshed on every edit (search.rs);
-- `note_search` is its FTS5 index.
-- Notes that predate this table are filled in at startup, since their text lives in Loro snapshots.
CREATE TABLE note_text (
    id      INTEGER PRIMARY KEY,
    note_id TEXT NOT NULL UNIQUE REFERENCES notes(id) ON DELETE CASCADE,
    title   TEXT NOT NULL,
    body    TEXT NOT NULL
);

CREATE VIRTUAL TABLE note_search USING fts5(
    title,
    body,
    content = 'note_text',
    content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER note_text_insert AFTER INSERT ON note_text BEGIN
    INSERT INTO note_search (rowid, title, body) VALUES (new.id, new.title, new.body);
END;

CREATE TRIGGER note_text_delete AFTER DELETE ON note_text BEGIN
    INSERT INTO note_search (note_search, rowid, title, body) VALUES ('delete', old.id, old.title, old.body);
END;

CREATE TRIGGER note_text_update AFTER UPDATE ON note_text BEGIN
    INSERT INTO note_search (note_search, rowid, title, body) VALUES ('delete', old.id, old.title, old.body);
    INSERT INTO note_search (rowid, title, body) VALUES (new.id, new.title, new.body);
END;
