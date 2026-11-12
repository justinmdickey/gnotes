-- The text of a PDF attachment, extracted at upload, so search and Ask can read it.
-- A PDF with no text layer, or a failed or slow extraction, has no row.
CREATE TABLE attachment_text (
    attachment_id TEXT PRIMARY KEY REFERENCES attachments(id) ON DELETE CASCADE,
    text          TEXT NOT NULL
);

-- A note's search text also carries its linked PDFs: where each PDF's text sits (so a snippet
-- can name its file) and which attachments it references (so a freshly extracted PDF can
-- re-index it). Both are JSON, kept in note_text on every edit.
ALTER TABLE note_text ADD COLUMN pdf_spans TEXT;
ALTER TABLE note_text ADD COLUMN att_refs  TEXT;
