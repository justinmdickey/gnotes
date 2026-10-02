# Gnotes

Self-hosted Markdown notes with live shared editing. Write on your phone or desktop, share a note or a whole notebook with other people on your server, and watch each other type.

<p>
  <img src="docs/screenshots/desktop.png" alt="A note open on desktop, with notebooks in the sidebar" width="640">
  <img src="docs/screenshots/phone.png" alt="The same note on a phone, in dark mode" width="195">
</p>

- **Markdown that looks finished:** headings, checklists and bold render as you type.
- **Live editing:** several people in one note at once, with cursors. Works offline and merges when you're back.
- **Notebooks inside notebooks,** drag and drop, search, trash, and import from a zip of Markdown files.
- **Sharing** with other accounts on the server, as editor or viewer, plus invite links.
- **Photos and voice memos** in notes.
- **Optional AI, on your own hardware or a hosted API:** transcribe voice memos (or a meeting, live), pull text out of photos, and summarize a note. Nothing runs until you ask.
- **Installable app** (PWA) on phones and desktops, light and dark.

## Quick start

```sh
docker run -d --name gnotes -p 8080:8080 -v gnotes-data:/data ghcr.io/justinmdickey/gnotes:latest
```

Open http://localhost:8080 and create the first account. It's the admin: invite everyone else from **Account → People → Invite Someone New**, or from a note's **Share** button.

With Compose, use [`docker-compose.yml`](docker-compose.yml): `docker compose up -d`.

## Put it on your network

The browser only allows recording and installing the app over **HTTPS** (or on `localhost`). For anyone else to use it, put it behind a reverse proxy with a certificate and tell Gnotes its address:

```sh
-e GNOTES_PUBLIC_URL=https://notes.example.com
```

The proxy must pass WebSocket upgrades through (`/api/ws` and `/api/transcribe/live`). Caddy does this by default:

```
notes.example.com {
    reverse_proxy localhost:8080
}
```

## AI features (optional)

Each one talks to an **OpenAI-compatible API**, so it can be a local server (Ollama, llama.cpp, speaches) or a hosted one (OpenAI, Groq, ...). Set them up as the admin under **Settings**, where **Test** checks the connection, or with environment variables. Settings saved in the app win over the variables. API keys are never sent back to the browser.

| Feature | Settings group | Environment | Example |
|---|---|---|---|
| Transcribe voice memos | Speech-to-Text | `GNOTES_WHISPER_URL`, `_MODEL`, `_KEY` | `http://speaches:8000/v1`, `Systran/faster-whisper-small` |
| Live transcripts and meeting audio | Speech-to-Text → Live URL | `GNOTES_WHISPER_REALTIME_URL` | `ws://speaches:8000/v1/realtime` |
| Text from photos | Text from Photos | `GNOTES_VISION_URL`, `_MODEL`, `_KEY` | `http://ollama:11434/v1`, `qwen2.5vl` |
| Note summaries | AI Summaries | `GNOTES_SUMMARY_URL`, `_MODEL`, `_KEY` | `http://ollama:11434/v1`, `llama3.2` |

URLs include the API version (`/v1`). The model for photos must be able to read images. The live URL is a WebSocket that speaks OpenAI's realtime transcription events; the server connects to it, so it can stay on a private network.

## Configuration

| Variable | Default | What it does |
|---|---|---|
| `GNOTES_DATA_DIR` | `/data` in Docker, `./data` otherwise | Where the database and attachments live |
| `GNOTES_BIND` | `0.0.0.0:8080` in Docker | Address and port to listen on |
| `GNOTES_PUBLIC_URL` | unset | The address people open; checked against WebSocket origins, and `https://` makes cookies secure |
| `RUST_LOG` | `info` | Log level |

Accounts can also be made from the command line:

```sh
docker exec -it gnotes gnotes-server create-user sam --display-name "Sam"
```

## Backups

Everything is in the data folder: `gnotes.db` (SQLite) and `blobs/` (photos and recordings). Stop the container and copy the folder, or back up the database live with `sqlite3 gnotes.db ".backup backup.db"` and copy `blobs/` beside it.

## Build from source

You need Rust (see `rust-toolchain.toml`) and Node 22.

```sh
cd web && npm ci && npm run build && cd ..
cargo run -p gnotes-server
```

That serves on http://localhost:8080 with data in `./data`. For development with hot reload, run `npx vite` in `web/` too and open http://localhost:5173. Tests: `cargo test`, and `cd web && npm run check && npm run e2e`.

How it's built and why: [`docs/DESIGN.md`](docs/DESIGN.md).

## Status

Early and used daily by a few people. Expect rough edges; there's no end-to-end encryption, so whoever runs the server can read its notes.

## License

MIT. See [`LICENSE`](LICENSE).
