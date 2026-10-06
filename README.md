# Gnotes

Self-hosted Markdown notes with live shared editing. Write on your phone or desktop, share a note or a whole notebook with other people on your server, and watch each other type. Works offline, installs as an app, and keeps a plain Markdown copy of every note.

![A shared note open on desktop, with someone else typing in it](docs/screenshots/desktop.png)

## Run your own

You need Docker and a domain (or Tailscale) for HTTPS.

**1. Start it.**

```sh
docker run -d --name gnotes --restart unless-stopped \
  -p 127.0.0.1:8080:8080 -v gnotes-data:/data \
  -e GNOTES_PUBLIC_URL=https://notes.example.com \
  ghcr.io/justinmdickey/gnotes:latest
```

Or with Compose: `docker compose up -d` using [`docker-compose.yml`](docker-compose.yml).

**2. Make yourself an admin.** Do this before the server is reachable from the internet.

```sh
docker exec -it gnotes gnotes-server create-user alice --admin
```

**3. Put it behind HTTPS.** Phones need HTTPS to install the app, work offline and record voice memos. With [Caddy](https://caddyserver.com) on the same host:

```caddyfile
notes.example.com {
	reverse_proxy localhost:8080
}
```

**4. Invite people.** There's no open sign-up. Go to **Account › People › Invite Someone New** and send them the link.

Tailscale instead of a public domain, settings, optional AI features, backups and upgrades: [docs/SELF-HOSTING.md](docs/SELF-HOSTING.md).

To let an agent or script read and write your notes, make an API key in **Account › API Keys**: [docs/API.md](docs/API.md).

## Development

How to run it locally, check changes and the project rules: [`AGENTS.md`](AGENTS.md). How it's built and why: [`docs/DESIGN.md`](docs/DESIGN.md).

There's no end-to-end encryption, so whoever runs the server can read its notes.

## License

MIT. See [`LICENSE`](LICENSE).
