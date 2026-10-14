-- Each person's Inbox: the note quick-added tasks go to, made on their first one (tasks.rs).
-- Renaming or moving it keeps it their Inbox; once it's trashed, the next task makes a new one.
ALTER TABLE users ADD COLUMN inbox_note_id TEXT REFERENCES notes(id) ON DELETE SET NULL;
