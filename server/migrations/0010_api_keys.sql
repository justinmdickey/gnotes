-- Keys an agent or script uses to reach /api/v1 as its user. Only the token's hash is kept, like
-- sessions; revoking a key deletes its row.
CREATE TABLE api_keys (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    scope        TEXT NOT NULL CHECK (scope IN ('read', 'write')),
    token_hash   TEXT NOT NULL UNIQUE,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER
);
CREATE INDEX api_keys_user ON api_keys (user_id);
