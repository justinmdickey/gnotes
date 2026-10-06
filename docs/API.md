# Gnotes API

Let an agent or script find, read and write notes as you. Everything is JSON under `/api/v1`.

## Get a key

In the app, open **Account → API Keys → New API Key**. Pick:

- **Read only**: search and read notes. Any request other than `GET` is refused.
- **Read and write**: also make, change, move and trash notes.

The key (`gnk_…`) is shown once. Send it with every request:

```sh
export GNOTES=https://notes.example.com
export KEY=gnk_...
curl -H "Authorization: Bearer $KEY" $GNOTES/api/v1/me
```

A key acts as you: it sees what you see, including notes and notebooks shared with you, with your role on each. Revoke it in the same place; it stops working at once. Keys can't reach your account, sharing or admin settings.

## Endpoints

| Method and path | Does |
| --- | --- |
| `GET /me` | Who the key acts as, and its scope |
| `GET /notebooks` | Notebooks you can see, A–Z |
| `POST /notebooks` | Make a notebook: `{ "name", "parent_id"? }` |
| `GET /notes` | Notes you can see, newest change first. Query: `notebook_id`, `limit` (100, max 1000) |
| `POST /notes` | Make a note: `{ "notebook_id"?, "text"? }` |
| `GET /notes/{id}` | A note with its text |
| `PATCH /notes/{id}` | Change its text, move it, or both: `{ "text"?, "version"?, "notebook_id"? }` |
| `POST /notes/{id}/append` | Add `{ "text" }` at the end, after a blank line |
| `DELETE /notes/{id}` | Move a note you own to the trash (restorable for 30 days) |
| `GET /search?q=` | Notes that match, best first. Query: `mode` (`words` or `meaning`), `limit` (20, max 50) |

Leave out `notebook_id` to work at the top level. On `PATCH`, `"notebook_id": null` moves a note to the top level.

## Notes are Markdown

A note is its text. Its title is the first line, so to rename a note, change its first line.

```sh
curl -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" \
  -d '{"text": "# Standup\n\n- Shipped the API"}' $GNOTES/api/v1/notes
```

```json
{
  "id": "019a0b6e-…",
  "notebook_id": null,
  "title": "Standup",
  "owner": "Justin",
  "role": "owner",
  "updated_at": "2026-10-06T14:03:51.123Z",
  "version": "AQHx…",
  "text": "# Standup\n\n- Shipped the API"
}
```

Writes go through the note's live session, so anyone with it open sees the change as it happens.

## Editing without overwriting people

Send back the `version` you read along with the new text. Your change is merged with anything written since, the way two people typing at once are merged.

```sh
curl -s -H "Authorization: Bearer $KEY" $GNOTES/api/v1/notes/$ID > note.json
version=$(jq -r .version note.json)
# …work out NEW_TEXT from the note's .text…
jq -n --arg text "$NEW_TEXT" --arg version "$version" '{text: $text, version: $version}' |
  curl -X PATCH -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" -d @- $GNOTES/api/v1/notes/$ID
```

Without `version`, your text replaces whatever the note holds now. To only add, use `append`, which never needs a version:

```sh
curl -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" \
  -d '{"text": "- 14:05 deploy done"}' $GNOTES/api/v1/notes/$ID/append
```

## Search

`mode=words` (the default) finds notes with every word, as prefixes; matched words are wrapped in `**` in the snippet. `mode=meaning` finds notes about what you asked, even without its words. It needs Semantic Search set up in the admin's AI Services, and answers `409` otherwise.

```sh
curl -H "Authorization: Bearer $KEY" "$GNOTES/api/v1/search?q=deploy+checklist"
```

```json
[{ "id": "019a…", "title": "Release", "snippet": "…the **deploy** **checklist** before tagging…" }]
```

## Errors

Errors have a status and a JSON body saying what went wrong:

```json
{ "error": "forbidden", "message": "This key can only read; make a Read and write key to change notes" }
```

| Status | Means |
| --- | --- |
| 400 | The request is wrong: a missing or unknown field, or a `version` this note never had |
| 401 | No key, or a wrong or revoked one |
| 403 | Not allowed: a read-only key writing, or a note you can only view |
| 404 | No such note or notebook, or you can't see it |
| 409 | Can't right now, e.g. search by meaning isn't set up |

## Details

- Ids are UUIDs. Times are RFC 3339 in UTC.
- `role` is `owner`, `editor` or `viewer`. Editors can change text and move notes; only owners trash them.
- A note holds at most 1 MB of text.
- Attachments (photos, voice memos) show in the text as `![Photo](att:<id>)`. Keep those lines when you rewrite a note; the API doesn't upload or download attachments yet.
- `/api/v1` only grows: new endpoints and fields may appear, but what's here won't change shape.
