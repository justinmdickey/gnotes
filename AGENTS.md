# Gnotes

Self-hosted Markdown notes with live shared editing. `server/` is Rust (axum, SQLite via sqlx, Loro CRDT per note); `web/` is a Svelte 5 (runes) PWA with a CodeMirror 6 editor. Why things are built this way: `docs/DESIGN.md`.

## Run it

- Server: `cargo run -p gnotes-server` from the repo root serves on `:8080`, data in `./data`. Make an account with `GNOTES_PASSWORD=… cargo run -p gnotes-server -- create-user <name> --admin`.
- Web: `cd web && npx vite --host 0.0.0.0` on `:5173`, proxying `/api` to `:8080`. Phones on the LAN open `http://<this-machine>:5173`.
- The running `:8080` server keeps its old code: restart it after any change under `server/`.

## Check a change

Run the narrowest one that exercises it, and paste its result:

- `cargo test` for server changes (`server/tests/api.rs` drives the real HTTP and WebSocket API).
- `cd web && npm run check` for types.
- `cd web && npm run build && cargo build -p gnotes-server && npm run e2e` for UI behavior. The e2e runs against `web/dist` and `target/debug`, so a stale build tests stale code. It prints `ok - …` per check and ends with `all checks passed`.

Add an e2e check for each new UI behavior, in the flow where it naturally happens.

## Look at the UI

Tests prove behavior, not looks. After a visual change, take screenshots and look at them:

- `node web/scripts/shot.mjs [--phone|--desktop] [--light] [screen...]` starts a throwaway server with demo data (Home › Kitchen and Garage, Personal, a shared notebook, two notes), logs in, and writes PNGs to `/tmp/gnotes-shots`. Screens: `home recent shared account notebook:<name> note:<title> search:<query>`. Build `web/dist` first.
- `SHOTS=/some/dir npm run e2e` also saves screenshots from inside the e2e flow (phone editing, keyboard bar, dialogs).
- `magick a.png b.png +append side.png` puts shots side by side, to compare before/after or neighboring screens.
- `node web/scripts/icon-sheet.mjs` renders every icon over its grid; check a new icon against its neighbors there.

Judge the phone shot first; most use is on a phone. Compare each element against its neighbors: same icon size and weight, same color role, aligned edges, no text clipped or wrapped, both themes readable.

## Design rules

The look is GNOME Adwaita. Keep it one system:

- **Tokens only.** Colors, `--text-*` sizes, `--radius-*` and `--icon-touch` live in `web/src/app.css`, with dark values under `prefers-color-scheme`. Add a token when a new value is needed.
- **One icon set.** Icons come from `web/src/lib/icons.ts` through `<Icon>` (or `iconSvg()` in plain DOM): 16px grid, 1.5 round strokes, drawn inside 2.5–13.5.
- **Blue means primary or here.** Accent color marks the main action (the tab bar +, Done, suggested buttons) and the current place (active tab, crumbs, folder icons in lists). Headerbar tools are plain foreground, all the same touch size on phones. Yellow `--shared` is the shared badge.
- **Phone layout.** One bottom tab bar on every screen: Notes, Recent, +, Shared, Account. + makes a note in one tap in the current folder. New Notebook lives in the headerbar. Every folder level looks and works the same, with breadcrumbs showing depth.
- **Shared pieces.** Popups use `lib/Dialog.svelte` (a bottom sheet on phones) and `lib/Menu.svelte`; feedback uses `toast()`; confirmations use `ask()`; empty states use `lib/StatusPage.svelte`. Visibility helpers: `.phone-only`, `.wide-only`, `.tablet-only`.

## Gotchas

- Routes live in the URL hash (`web/src/lib/store.svelte.ts`: `navigate`, `goBack`, `readHash`), so every screen change is a history entry and the phone back button works.
- Svelte props are live getters during component teardown; pin an id you need in cleanup with `untrack` at mount (see `Editor.svelte`).
- Note text exists only in the Loro doc over the WebSocket. REST creates notes and notebooks; titles and previews come from the text. To seed content in a test, type it in the editor.
- A new field on tree items needs the Rust struct in `server/src/tree.rs` and the type in `web/src/lib/api.ts`.
- In e2e, dialogs animate out: wait with `closed(page)` before clicking what's under them. Switching puppeteer to a mobile viewport reloads the page.

## Git and releases

- Conventional Commits (`feat(web): …`, `fix: …`), subject under 50 characters, staged by file name.
- Push and tag only when the user asks. A `vX.Y.Z` tag builds the image in Forgejo CI and Flux rolls it out to the homelab cluster.
