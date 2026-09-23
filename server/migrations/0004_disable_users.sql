-- Disabled accounts can't log in; their notes and history stay intact and they can be re-enabled.
ALTER TABLE users ADD COLUMN disabled_at INTEGER;
