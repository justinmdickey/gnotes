// A throwaway server with demo data, and the GTK app run headless against it, for shot.mjs and
// e2e.mjs. Accounts: ann (the app's) and sam. Ann has Home › Kitchen (Groceries, Pantry with a
// photo) and Garage, Personal, and Snippets at the top; Sam shares his Family notebook (Weekly sync)
// with Ann, and Ann shares Groceries with Sam.
import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
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

/**
 * Stand-in AI services on `port`, OpenAI-compatible, like web/scripts/shot.mjs's: embeddings that
 * know a few topics, a chat model for summaries, Tidy Up, Edit with AI, the AI Prompt and Ask, a
 * vision model that reads "RICE" off any photo, and speech-to-text that hears "Call the plumber".
 */
function standInAI(port) {
  const topics = [
    ["milk", "dairy", "eggs", "cheese", "bread", "groceries", "breakfast", "buy", "spinach", "lemons"],
    ["rice", "beans", "flour", "pantry", "staples", "dry", "cook"],
    ["kubectl", "flux", "code", "command", "commands", "homelab", "cargo"],
  ];
  const embedding = (text) => {
    const v = new Array(3 + 64).fill(0);
    for (const w of text.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean)) {
      const t = topics.findIndex((words) => words.includes(w));
      if (t >= 0) v[t] += 3;
      else v[3 + ([...w].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7) % 64)] += 1;
    }
    return v;
  };
  const reply = (res, content) => res.end(JSON.stringify({ choices: [{ message: { role: "assistant", content } }] }));
  return createServer((req, res) => {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      res.setHeader("content-type", "application/json");
      if (req.url.endsWith("/audio/transcriptions")) return res.end(JSON.stringify({ text: "Call the plumber about the sink." }));
      const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString() || "{}") : {};
      if (req.url.endsWith("/models")) return res.end(JSON.stringify({ data: [{ id: "stand-in" }] }));
      if (req.url.endsWith("/embeddings")) {
        return res.end(JSON.stringify({ data: body.input.map((t, index) => ({ index, embedding: embedding(t) })) }));
      }
      if (!req.url.endsWith("/chat/completions")) return res.end(JSON.stringify({ data: [] }));
      const [first] = body.messages;
      const last = body.messages.at(-1).content;
      if (Array.isArray(last)) return reply(res, "RICE 2 KG");
      if (first.role === "system") {
        // Ask: a sentence for each note it knows, cited by its number, streamed.
        const lines = { Groceries: "You still need spinach, lemons and milk", Pantry: "Rice, beans and flour are in the pantry" };
        const said = [...first.content.matchAll(/^\[(\d+)\] (\w+)$/gm)].filter((m) => lines[m[2]]).map((m) => `${lines[m[2]]} [${m[1]}].`);
        res.setHeader("content-type", "text/event-stream");
        for (const text of (said.join(" ") || "NO_ANSWER").match(/.{1,8}/g)) {
          res.write(`data: ${JSON.stringify({ choices: [{ delta: { content: text } }] })}\n\n`);
        }
        return res.end("data: [DONE]\n\n");
      }
      const edit = last.match(/^The instruction: (.*)\n\nThe note:\n\n([\s\S]*)$/m);
      if (edit) {
        const [title, ...rest] = edit[2].split("\n");
        const items = rest.filter((l) => l.trim()).map((l) => `- ${l.replace(/^\s*([-*]|\d+\.)\s+(\[[ x]\]\s+)?/, "")}`);
        return reply(res, [title, "", ...items, "", "## Action items", "", "- [ ] Go shopping on Saturday"].join("\n"));
      }
      if (last.includes("<<WRITE HERE>>")) return reply(res, "- Olive oil\n- Coffee beans");
      const [, note] = last.split("The note:\n\n");
      if (note !== undefined) {
        const [title, ...rest] = note.split("\n");
        const items = rest.map((l) => (!l.trim() || /^\s*([-*>#]|\d+\.|```)/.test(l) ? l : `- ${l}`));
        return reply(res, [title.startsWith("#") ? title : `# ${title}`, "", "## To buy", "", ...items].join("\n"));
      }
      const followUp = last.split("\nLast message: ");
      if (followUp.length === 2) return reply(res, `${followUp[0].split("\n").find((l) => l.startsWith("User: "))?.slice(6) ?? ""} ${followUp[1]}`);
      return reply(res, "A shopping list for the weekend.\n\n## Key points\n- Spinach, lemons and milk\n\n## Action items\n- [ ] Check the pantry");
    });
  }).listen(port, "127.0.0.1");
}

/** `ai`: stand-in AI services, all switched on. */
export async function startDemo({ ai = false } = {}) {
  const data = mkdtempSync(join(tmpdir(), "gnotes-gtk-"));
  const port = 19000 + Math.floor(Math.random() * 1000);
  const base = `http://127.0.0.1:${port}`;
  const env = { ...process.env, GNOTES_DATA_DIR: join(data, "server"), GNOTES_BIND: `127.0.0.1:${port}` };
  if (ai) {
    standInAI(port + 2000);
    const url = `http://127.0.0.1:${port + 2000}/v1`;
    Object.assign(env, {
      GNOTES_EMBED_URL: url,
      GNOTES_EMBED_MODEL: "stand-in-embed",
      GNOTES_SUMMARY_URL: url,
      GNOTES_SUMMARY_MODEL: "stand-in-chat",
      GNOTES_VISION_URL: url,
      GNOTES_VISION_MODEL: "stand-in-vision",
      GNOTES_WHISPER_URL: url,
    });
  }
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

  // A voice memo: a second of quiet as WAV.
  const memo = await ann.note();
  const rate = 8000;
  const wav = Buffer.alloc(44 + rate);
  wav.write("RIFF", 0);
  wav.writeUInt32LE(36 + rate, 4);
  wav.write("WAVEfmt ", 8);
  wav.writeUInt32LE(16, 16);
  wav.writeUInt16LE(1, 20);
  wav.writeUInt16LE(1, 22);
  wav.writeUInt32LE(rate, 24);
  wav.writeUInt32LE(rate, 28);
  wav.writeUInt16LE(1, 32);
  wav.writeUInt16LE(8, 34);
  wav.write("data", 36);
  wav.writeUInt32LE(rate, 40);
  wav.fill(128, 44);
  const memoForm = new FormData();
  memoForm.append("note_id", memo);
  memoForm.append("file", new Blob([wav], { type: "audio/wav" }), "Voice memo.wav");
  const recording = await (await fetch(`${base}/api/attachments`, { method: "POST", headers: { cookie: `gnotes_session=${ann.token}` }, body: memoForm })).json();

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
  await write(ann, memo, `# Plumber\n\n![Voice memo](att:${recording.id})\n\nCall about the leak under the sink.`);
  const plans = await ann.note();
  await write(ann, plans, [
    "# Plans",
    "",
    "| Room | Paint | Done by |",
    "| --- | --- | --- |",
    "| Kitchen | Sage | May |",
    "| Hall | White | June |",
    "",
    "```kanban",
    "## To do",
    "- [ ] Buy paint",
    "- Move the couch",
    "",
    "## Doing",
    "- Patch the wall",
    "",
    "## Done",
    "- [x] Pick colors",
    "```",
  ].join("\n"));
  // Due dates, one late and one tomorrow, for Tasks and the editor's date styling.
  const day = (n) => {
    const d = new Date(Date.now() + n * 864e5);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  };
  await write(sam, sync, `# Weekly sync\n\nQuick check-in on the **mobile release**.\n\n## Action items\n- [ ] Sam: write the release notes @${day(-1)}\n- [ ] Ann: try the beta @${day(1)}`);
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

  // The app's own display: a headless Sway, so nothing shows on your screen. Its windows float, so
  // they keep the size the app asks for.
  const runtime = join(data, "run");
  mkdirSync(runtime, { mode: 0o700 });
  writeFileSync(
    join(runtime, "sway.conf"),
    'output HEADLESS-1 resolution 1600x1000\nfor_window [app_id="com.erisdor.Gnotes"] floating enable\ndefault_border none\n',
  );
  const swayEnv = { ...process.env, XDG_RUNTIME_DIR: runtime, WLR_BACKENDS: "headless", WLR_LIBINPUT_NO_DEVICES: "1", WLR_RENDERER: "pixman" };
  for (const v of ["WAYLAND_DISPLAY", "DISPLAY", "SWAYSOCK", "I3SOCK"]) delete swayEnv[v];
  const sway = track(spawn("sway", ["-c", join(runtime, "sway.conf")], { env: swayEnv, stdio: "ignore" }));
  sway.once("exit", (code, signal) => console.error(`sway ended: ${code ?? signal}`));
  let wayland;
  for (let i = 0; i < 100 && !wayland; i++) {
    await sleep(100);
    wayland = readdirSync(runtime).find((f) => /^wayland-\d+$/.test(f));
  }
  if (!wayland) throw new Error("headless sway didn't start");
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
    // GNOTES_GDB: under gdb, for a backtrace when it's interrupted.
    const [cmd, args] = process.env.GNOTES_GDB
      ? ["gdb", ["-batch", "-ex", "run", "-ex", "thread apply 1 bt 40", "--args", join(root, "target/debug/gnotes")]]
      : [join(root, "target/debug/gnotes"), []];
    const app = track(
      spawn(cmd, args, {
        env: {
          ...process.env,
          GDK_BACKEND: "wayland",
          WAYLAND_DISPLAY: wayland,
          XDG_RUNTIME_DIR: runtime,
          // No GPU in a headless compositor.
          GSK_RENDERER: "cairo",
          // A tone stands in for the microphone.
          GNOTES_AUDIO_SRC: "audiotestsrc is-live=true wave=sine volume=0.1",
          GNOTES_GTK_DIR: gtkDir,
          GNOTES_SIZE: size,
          GNOTES_STYLE: style,
          // Stock Adwaita, not a gtk.css in your own config.
          XDG_CONFIG_HOME: join(gtkDir, "xdg"),
          ...extra,
        },
        stdio: ["ignore", process.env.GNOTES_GDB ? "pipe" : "ignore", "pipe"],
      }),
    );
    let stderr = "";
    app.stdout?.on("data", (d) => (stderr += d.toString()));
    app.stderr.on("data", (d) => {
      const s = d.toString();
      // GTK warnings about the headless display aren't ours.
      stderr += s
        .split("\n")
        .filter((l) => l.trim() && (!/-WARNING \*\*/.test(l) || /display/i.test(l)))
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

  /** Another server, with no accounts yet, for first-run setup. Resolves with its address. */
  async function startEmptyServer() {
    const port2 = port + 1000;
    const env2 = { ...env, GNOTES_DATA_DIR: join(data, "empty"), GNOTES_BIND: `127.0.0.1:${port2}` };
    track(spawn(join(root, "target/debug/gnotes-server"), [], { env: env2, stdio: "ignore" }));
    const base2 = `http://127.0.0.1:${port2}`;
    for (let i = 0; i < 100; i++) {
      try {
        if ((await fetch(`${base2}/api/health`)).ok) return base2;
      } catch {}
      await sleep(100);
    }
    throw new Error("the empty server didn't start");
  }

  return {
    root,
    startEmptyServer,
    login,
    base,
    data,
    ann,
    sam,
    ids: { home, kitchen, groceries, pantry, snippets, memo, plans, family, sync, photo: photo.id, recording: recording.id },
    open,
    read,
    write,
    stay,
    runApp,
    startServer,
    stopServer,
  };
}
