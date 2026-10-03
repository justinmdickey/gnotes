-- Who each Loro peer (one editing session) belongs to, recorded when its edits reach the server,
-- so the app can show who wrote which line. Peers are stored as decimal text, as Loro's JS API gives them.
CREATE TABLE note_peers (
    note_id    TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    peer       TEXT NOT NULL,
    user_id    TEXT REFERENCES users(id) ON DELETE SET NULL,
    first_seen INTEGER NOT NULL,
    PRIMARY KEY (note_id, peer)
);
