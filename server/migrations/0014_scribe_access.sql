-- Scribe's access level, chosen per person in Account: read, add (the default) or full.
ALTER TABLE users ADD COLUMN scribe_access TEXT NOT NULL DEFAULT 'add';
