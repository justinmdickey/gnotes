// A throwaway server with demo data, and the GTK app run headless against it, for shot.mjs and
// e2e.mjs. Accounts: ann (the app's) and sam. Ann has Home › Kitchen (Groceries, Pantry with a
// photo) and Garage, Personal, and Snippets at the top; Sam shares his Family notebook (Weekly sync)
// with Ann, and Ann shares Groceries with Sam.
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

export const root = resolve(import.meta.dirname, "../..");
const require = createRequire(join(root, "web/package.json"));
const { Cursor, LoroDoc, EphemeralStore } = require("loro-crdt");

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const uuidBytes = (id) => Uint8Array.from(id.replace(/-/g, "").match(/../g), (h) => parseInt(h, 16));
const frame = (kind, id, payload) => {
  const buf = new Uint8Array(17 + payload.length);
  buf[0] = kind;
  buf.set(uuidBytes(id), 1);
  buf.set(payload, 17);
  return buf;
};

export async function startDemo() {
  const data = mkdtempSync(join(tmpdir(), "gnotes-gtk-"));
  const port = 19000 + Math.floor(Math.random() * 1000);
  const base = `http://127.0.0.1:${port}`;
  const env = { ...process.env, GNOTES_DATA_DIR: join(data, "server"), GNOTES_BIND: `127.0.0.1:${port}` };
  const children = new Set();
  const track = (c) => {
    children.add(c);
    c.once("exit", () => children.delete(c));
    return c;
  };
  process.on("exit", () => {
    for (const c of children) c.kill();
    rmSync(data, { recursive: true, force: true });
  });
  process.on("SIGINT", () => process.exit(1));

  for (const [name, display] of [["ann", "Ann"], ["sam", "Sam"]]) {
    const r = spawnSync(join(root, "target/debug/gnotes-server"), ["create-user", name, "--display-name", display, "--admin"], {
      env: { ...env, GNOTES_PASSWORD: "password123" },
    });
    if (r.status !== 0) throw new Error(`create-user ${name}: ${r.stderr}`);
  }

  let server;
  async function startServer() {
    server = track(spawn(join(root, "target/debug/gnotes-server"), [], { env, stdio: "ignore" }));
    for (let i = 0; ; i++) {
      try {
        if ((await fetch(`${base}/api/health`)).ok) return;
      } catch {}
      if (i > 100) throw new Error("server didn't start");
      await sleep(100);
    }
  }
  async function stopServer() {
    const exited = new Promise((r) => server.once("exit", r));
    server.kill();
    await exited;
  }
  await startServer();

  async function login(username) {
    const res = await fetch(`${base}/api/auth/login`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ username, password: "password123" }),
    });
    const token = res.headers.get("set-cookie").match(/gnotes_session=([^;]+)/)[1];
    const user = await res.json();
    const call = async (method, path, body) => {
      const r = await fetch(`${base}/api${path}`, {
        method,
        headers: { cookie: `gnotes_session=${token}`, ...(body ? { "content-type": "application/json" } : {}) },
        body: body ? JSON.stringify(body) : undefined,
      });
      if (!r.ok) throw new Error(`${method} ${path}: ${r.status}`);
      return r.json();
    };
    return {
      token,
      user,
      call,
      notebook: (name, parent_id = null) => call("POST", "/notebooks", { name, parent_id }).then((r) => r.id),
      note: (notebook_id = null) => call("POST", "/notes", { notebook_id }).then((r) => r.id),
      tree: () => call("GET", "/tree"),
    };
  }

  /** Opens a note over the websocket, as the PWA does; resolves once its content has arrived. */
  function open(c, id) {
    return new Promise((done) => {
      const ws = new WebSocket(`ws://127.0.0.1:${port}/api/ws`, { headers: { cookie: `gnotes_session=${c.token}` } });
      ws.binaryType = "arraybuffer";
      const doc = new LoroDoc();
      // Others' presence, as loro-codemirror keeps it.
      const others = new EphemeralStore(30_000);
      let ready = false;
      ws.onopen = () => ws.send(JSON.stringify({ t: "join", note: id, version: null }));
      ws.onmessage = (ev) => {
        if (typeof ev.data === "string") return;
        const bytes = new Uint8Array(ev.data);
        if (bytes[0] === 2) others.apply(bytes.subarray(17));
        if (bytes[0] === 1) {
          doc.import(bytes.subarray(17));
          if (!ready) {
            ready = true;
            done({
              ws,
              doc,
              text: () => doc.getText("body").toString(),
              /** Who else is here, and where their cursor is in the text, as the PWA would show them. */
              peers() {
                const all = others.getAllStates();
                return Object.entries(all)
                  .filter(([k]) => k.endsWith("-cm-user"))
                  .map(([k, user]) => {
                    const cursor = all[k.replace("-cm-user", "-cm-cursor")];
                    const at = cursor ? doc.getCursorPos(Cursor.decode(cursor.anchor)).offset : null;
                    return { name: user.name, color: user.colorClassName, at };
                  });
              },
              /** Inserts `s` at `at` (default: the end) and sends it. */
              insert(s, at) {
                const text = doc.getText("body");
                const before = doc.oplogVersion();
                text.insert(at ?? text.length, s);
                doc.commit();
                ws.send(frame(1, id, doc.export({ mode: "update", from: before })));
              },
              presence() {
                const store = new EphemeralStore(30_000);
                store.subscribeLocalUpdates((b) => ws.send(frame(2, id, b)));
                return store;
              },
              close: () => ws.close(),
            });
          }
        }
      };
    });
  }

  /** A note's text as the server has it now. */
  async function read(c, id) {
    const n = await open(c, id);
    n.close();
    return n.text();
  }

  async function write(c, id, text) {
    const n = await open(c, id);
    n.insert(text, 0);
    await sleep(150);
    n.close();
  }

  const ann = await login("ann");
  const sam = await login("sam");
  const home = await ann.notebook("Home");
  const kitchen = await ann.notebook("Kitchen", home);
  await ann.notebook("Garage", home);
  await ann.notebook("Personal");
  const groceries = await ann.note(kitchen);
  const pantry = await ann.note(kitchen);
  const snippets = await ann.note();
  const family = await sam.notebook("Family");
  const sync = await sam.note(family);
  await sam.call("POST", "/shares", { resource_type: "notebook", resource_id: family, username: "ann", role: "editor" });
  await ann.call("POST", "/shares", { resource_type: "note", resource_id: groceries, username: "sam", role: "editor" });

  const form = new FormData();
  form.append("note_id", pantry);
  form.append("file", new Blob([readFileSync(join(root, "web/public/icon-192.png"))], { type: "image/png" }), "Shelf.png");
  const photo = await (await fetch(`${base}/api/attachments`, { method: "POST", headers: { cookie: `gnotes_session=${ann.token}` }, body: form })).json();

  await write(ann, groceries, [
    "# Groceries",
    "",
    "Weekly shop for **Saturday**. Check the [[Pantry]] first.",
    "",
    "## Produce",
    "- [x] Apples",
    "- [ ] Spinach",
    "- [ ] Lemons",
    "",
    "## Dairy",
    "- [ ] Milk",
    "- [ ] Eggs and *good* butter",
    "",
    "> The farmers market opens at 8.",
    "",
    "Recipes: https://cooking.example.com",
  ].join("\n"));
  await write(ann, pantry, `# Pantry\n\nRice, beans and flour.\n\n![Shelf](att:${photo.id})\n\n1. Rice\n2. Beans\n3. Flour`);
  await write(ann, snippets, [
    "# Snippets",
    "",
    "Handy commands for the homelab.",
    "",
    "```bash",
    "kubectl get pods -A",
    "flux reconcile kustomization apps",
    "```",
    "",
    "Inline `cargo test` runs everything, ~~slowly~~.",
  ].join("\n"));
  await write(sam, sync, "# Weekly sync\n\nQuick check-in on the **mobile release**.\n\n## Action items\n- [ ] Sam: write the release notes\n- [ ] Ann: try the beta");
  await ann.call("DELETE", `/notes/${await ann.note()}`);
  // Titles and previews settle a moment after the last edit.
  await sleep(600);

  /** colorFor in web/src/lib/store.svelte.ts. */
  const colorFor = (userId) => {
    const colors = ["user-blue", "user-green", "user-orange", "user-purple", "user-red"];
    let h = 0;
    for (const ch of userId) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    return colors[h % colors.length];
  };

  /** Keeps someone in a note with the cursor after `after`, as the PWA announces them. */
  async function stay(c, id, after) {
    const [name, color] = [c.user.display_name, colorFor(c.user.id)];
    const n = await open(c, id);
    const store = n.presence();
    const announce = () => {
      const text = n.doc.getText("body");
      const at = text.toString().indexOf(after) + after.length;
      store.set(`${n.doc.peerIdStr}-cm-user`, { name, colorClassName: color });
      store.set(`${n.doc.peerIdStr}-cm-cursor`, { anchor: text.getCursor(at).encode() });
    };
    announce();
    const timer = setInterval(announce, 1000);
    return { ...n, stop: () => (clearInterval(timer), n.close()) };
  }

  const broadwayPort = 18000 + Math.floor(Math.random() * 1000);
  const display = `:${broadwayPort - 8080}`;
  track(spawn("gtk4-broadwayd", ["--port", String(broadwayPort), display], { stdio: "ignore" }));
  await sleep(500);
  const gtkDir = join(data, "gtk");

  /**
   * Runs the app signed in as Ann (unless `signedIn` is false) until it quits. `env` adds
   * GNOTES_OPEN, GNOTES_DRIVE, GNOTES_SHOT and the like. Resolves with its exit code.
   */
  function runApp({ env: extra = {}, signedIn = true, style = "dark", size = "1280x800", fresh = true, timeout = 60_000 } = {}) {
    if (fresh) rmSync(gtkDir, { recursive: true, force: true });
    if (signedIn && !existsSync(join(gtkDir, ".config/session.json"))) {
      mkdirSync(join(gtkDir, ".config"), { recursive: true });
      writeFileSync(join(gtkDir, ".config/session.json"), JSON.stringify({ server: base, token: ann.token, user: ann.user }));
    }
    const app = track(
      spawn(join(root, "target/debug/gnotes"), [], {
        env: {
          ...process.env,
          GDK_BACKEND: "broadway",
          BROADWAY_DISPLAY: display,
          GNOTES_GTK_DIR: gtkDir,
          GNOTES_SIZE: size,
          GNOTES_STYLE: style,
          // Stock Adwaita, not a gtk.css in your own config.
          XDG_CONFIG_HOME: join(gtkDir, "xdg"),
          ...extra,
        },
        stdio: ["ignore", "ignore", "pipe"],
      }),
    );
    let stderr = "";
    app.stderr.on("data", (d) => {
      const s = d.toString();
      // GTK's own warnings about Broadway aren't ours.
      stderr += s
        .split("\n")
        .filter((l) => l.trim() && !/WARNING \*\*|^\(gnotes:\d+\)/.test(l))
        .map((l) => l + "\n")
        .join("");
    });
    const timer = setTimeout(() => app.kill(), timeout);
    return new Promise((r) =>
      app.once("exit", (code, signal) => {
        clearTimeout(timer);
        r({ code: code ?? signal, stderr });
      }),
    );
  }

  return {
    base,
    data,
    ann,
    sam,
    ids: { home, kitchen, groceries, pantry, snippets, family, sync, photo: photo.id },
    open,
    read,
    write,
    stay,
    runApp,
    startServer,
    stopServer,
  };
}
