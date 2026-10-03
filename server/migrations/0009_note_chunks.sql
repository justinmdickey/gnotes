-- Semantic search: each note's search text (note_text, 0008) cut into chunks of a few paragraphs,
-- each with its embedding from the admin's embeddings service (semantic.rs).
-- `hash` covers the exact text that was embedded, so a chunk that didn't change keeps its vector;
-- `model` is the model that made it, so changing models re-embeds everything.
CREATE TABLE note_chunks (
    note_id   TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    idx       INTEGER NOT NULL,
    text      TEXT NOT NULL,
    hash      TEXT NOT NULL,
    model     TEXT NOT NULL,
    -- Little-endian f32s.
    embedding BLOB NOT NULL,
    PRIMARY KEY (note_id, idx)
);
