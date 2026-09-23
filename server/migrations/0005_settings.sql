-- Server-wide settings an admin can change in the app, stored as JSON per key.
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
