-- Tasks: every checkbox line in a note, derived from its text on each edit that changes them
-- (tasks.rs). The note's text stays the only source; this is an index for the Tasks list.
-- Notes that predate this table are filled in once at startup.
CREATE TABLE note_tasks (
    note_id      TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    -- 0-based line in the note's text.
    line         INTEGER NOT NULL,
    -- The task without its "- [ ]" marker or due date.
    text         TEXT NOT NULL,
    done         INTEGER NOT NULL,
    -- "YYYY-MM-DD" from an "@YYYY-MM-DD" in the line.
    due          TEXT,
    -- The "## Column" heading, for a card on a kanban board.
    board_column TEXT,
    PRIMARY KEY (note_id, line)
);
CREATE INDEX note_tasks_due ON note_tasks(due);
