-- A note whose checkboxes are a plain list (groceries, packing) can keep them out of Tasks.
-- Set by its editors, for everyone who sees the note.
ALTER TABLE notes ADD COLUMN hide_tasks INTEGER NOT NULL DEFAULT 0;
