-- Second non-empty line of a note, shown under the title in lists.
ALTER TABLE notes ADD COLUMN preview TEXT NOT NULL DEFAULT '';
