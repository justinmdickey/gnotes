// Screenshots of real screens, for looking at a UI change instead of guessing.
// Starts a throwaway server with demo data, logs in, and saves one PNG per screen.
//
// Usage: npm run build && cargo build -p gnotes-server && node scripts/shot.mjs [options] [screen...]
//   screens: home recent shared account trash notebook:<name> note:<title> edit:<title> search:<query>
//            ask:<question> (with --ai) service:<form> (a Settings service form, e.g. service:embed)
//            (default: home notebook:Kitchen note:Groceries account)
//   --phone | --desktop   viewport (default: both)
//   --dark | --light      color scheme (default: dark)
//   --ai                  stand-in AI services: summaries, search by meaning and Ask
//   --out <dir>           where PNGs go (default: /tmp/gnotes-shots)
//   --offline             visit each screen, then stop the server and shoot them offline
// Env: CHROME (default /usr/bin/chromium).
import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import puppeteer from "puppeteer-core";

const args = process.argv.slice(2);
const flag = (f) => args.includes(f);
const outIdx = args.indexOf("--out");
const out = outIdx >= 0 ? args[outIdx + 1] : "/tmp/gnotes-shots";
const screens = args.filter((a, i) => !a.startsWith("--") && args[i - 1] !== "--out");
const targets = screens.length ? screens : ["home", "notebook:Kitchen", "note:Groceries", "account"];
const viewports = [
  ...(flag("--desktop") ? [] : [["phone", { width: 390, height: 844, isMobile: true, hasTouch: true, deviceScaleFactor: 2 }]]),
  ...(flag("--phone") ? [] : [["desktop", { width: 1280, height: 800 }]]),
];
const scheme = flag("--light") ? "light" : "dark";
const offline = flag("--offline");

const root = resolve(import.meta.dirname, "../..");
const bin = join(root, "target/debug/gnotes-server");
const data = mkdtempSync(join(tmpdir(), "gnotes-shot-"));
const port = 19000 + Math.floor(Math.random() * 1000);
const base = `http://127.0.0.1:${port}`;
const env = { ...process.env, GNOTES_DATA_DIR: data, GNOTES_BIND: `127.0.0.1:${port}`, GNOTES_WEB_DIR: join(root, "web/dist") };
// --ai: stand-in summary, embeddings and chat services, so search by meaning and Ask have something to show.
// Embeddings know a few topics ("dairy" lands on milk); the chat model answers about groceries.
let ai;
if (flag("--ai")) {
  const topics = [
    ["milk", "dairy", "eggs", "cheese", "bread", "groceries", "breakfast", "buy"],
    ["rice", "beans", "flour", "pantry", "staples", "dry", "cook"],
    ["npm", "build", "code", "console", "run", "script", "program"],
  ];
  // Topic words weigh in on their topic; any other word is its own (hashed) direction.
  const embedding = (text) => {
    const v = new Array(3 + 64).fill(0);
    for (const w of text.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean)) {
      const t = topics.findIndex((words) => words.includes(w));
      if (t >= 0) v[t] += 3;
      else v[3 + ([...w].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7) % 64)] += 1;
    }
    return v;
  };
  ai = createServer((req, res) => {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      res.setHeader("content-type", "application/json");
      const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString()) : {};
      if (req.url.endsWith("/embeddings")) {
        res.end(JSON.stringify({ data: body.input.map((t, index) => ({ index, embedding: embedding(t) })) }));
      } else if (req.url.endsWith("/chat/completions")) {
        const prompt = body.messages[0].content;
        const content = prompt.includes("Question:")
          ? `You still need milk, eggs and bread [1].${prompt.includes("\n[2] ") ? " Rice, beans and flour are in the pantry [2]." : ""}`
          : "A shopping list.\n\n## Key points\n- Milk, eggs and bread";
        res.end(JSON.stringify({ choices: [{ message: { role: "assistant", content } }] }));
      } else res.end(JSON.stringify({ data: [] }));
    });
  }).listen(port + 1, "127.0.0.1");
  const url = `http://127.0.0.1:${port + 1}/v1`;
  Object.assign(env, { GNOTES_EMBED_URL: url, GNOTES_EMBED_MODEL: "stub-embed", GNOTES_SUMMARY_URL: url, GNOTES_SUMMARY_MODEL: "stub-chat" });
}
for (const [name, display, admin] of [["alice", "Alice", ["--admin"]], ["bob", "Bob", []]]) {
  const r = spawnSync(bin, ["create-user", name, "--display-name", display, ...admin], { env: { ...env, GNOTES_PASSWORD: "password123" } });
  if (r.status !== 0) throw new Error(`create-user ${name}: ${r.stderr}`);
}
const server = spawn(bin, [], { env, stdio: "ignore" });
const settle = (ms = 500) => new Promise((r) => setTimeout(r, ms));

async function login(page) {
  for (let i = 0; i < 50; i++) {
    try {
      if ((await fetch(`${base}/api/health`)).ok) break;
    } catch {}
    await settle(100);
  }
  await page.goto(base);
  await page.type("input[name=username]", "alice");
  await page.type("input[name=password]", "password123");
  await page.click("button[type=submit]");
  await page.waitForSelector("nav");
}

/** Notebooks Home › Kitchen and Home › Garage, two loose notes, a note in Kitchen, Home shared with Bob, and a note and notebook in the trash. */
async function seed(page) {
  const ids = await page.evaluate(async () => {
    const post = (path, body) =>
      fetch(`/api${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) }).then((r) => r.json());
    const home = (await post("/notebooks", { name: "Home", parent_id: null })).id;
    const kitchen = (await post("/notebooks", { name: "Kitchen", parent_id: home })).id;
    await post("/notebooks", { name: "Garage", parent_id: home });
    await post("/notebooks", { name: "Personal", parent_id: null });
    await post("/shares", { resource_type: "notebook", resource_id: home, username: "bob", role: "editor" });
    return { kitchen };
  });
  // Note text only exists through the editor, so type it in.
  const write = async (hash, text) => {
    await page.evaluate((h) => (location.hash = h), hash);
    await settle(300);
    await page.click(".pane.list header button[aria-label='New note']");
    await page.waitForSelector(".cm-content[contenteditable=true]");
    await page.keyboard.type(text);
    await settle(600);
  };
  // Lists continue on Enter, so only the first item gets its marker.
  await write("#/", "# Groceries\nmilk\n- [ ] eggs\nbread");
  await write(`#/nb/${ids.kitchen}`, "# Pantry\nrice, beans, flour");
  // Code blocks: one labeled, one left for the guesser.
  await write("#/", '# Snippets\nRun this first:\n```bash\nnpm run build\n```\nThen:\n```\n// say hi\nconst greet = (name) => `Hello ${name}`;\nconsole.log(greet("Ada"), 42);\n```\nAnd Lua:\n```\nlocal M = {}\nfunction M.setup(opts)\nfor k, v in pairs(opts) do print(k, v) end\nend\nreturn M\n```\nDone.');
  await write("#/", "# Old packing list\nsocks");
  await page.evaluate(() => document.activeElement?.blur());
  // The notes are embedded a few seconds after typing stops.
  if (flag("--ai")) {
    await page.waitForFunction(async () => (await fetch("/api/search/meaning?q=dairy").then((r) => r.json())).results.length > 0, { timeout: 20000, polling: 500 });
  }
  await page.evaluate(async () => {
    const tree = await fetch("/api/tree").then((r) => r.json());
    const del = (path) => fetch(`/api${path}`, { method: "DELETE" });
    await del(`/notes/${tree.notes.find((n) => n.title === "Old packing list").id}`);
    const old = await fetch("/api/notebooks", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ name: "Archive", parent_id: null }) }).then((r) => r.json());
    await del(`/notebooks/${old.id}`);
  });
}

let tree;

async function go(page, target) {
  const [kind, ...rest] = target.split(":");
  const arg = rest.join(":");
  // Offline there's no /tree to ask, so the last one fetched online names the screens.
  tree = offline && tree ? tree : await page.evaluate(() => fetch("/api/tree").then((r) => r.json()));
  const hash = {
    home: "#/",
    recent: "#/all",
    shared: "#/shared",
    account: "#/settings",
    trash: "#/trash",
    search: "#/",
    ask: "#/",
    service: "#/settings",
    notebook: `#/nb/${tree.notebooks.find((n) => n.name === arg)?.id}`,
    note: `#/note/${tree.notes.find((n) => n.title === arg)?.id}`,
    edit: `#/note/${tree.notes.find((n) => n.title === arg)?.id}`,
  }[kind];
  if (!hash || hash.endsWith("undefined")) throw new Error(`unknown screen ${target}`);
  // No dialog left over from the screen before.
  if (await page.$("dialog")) {
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => !document.querySelector("dialog"));
  }
  await page.evaluate((h) => (location.hash = h), hash);
  if (kind === "search" || kind === "ask") {
    await page.waitForSelector(".pane.list .search input");
    // Start from an empty field, whatever the screen before left.
    await page.$eval(".pane.list .search input", (el) => el.select());
    await page.keyboard.press("Backspace");
    await page.type(".pane.list .search input", arg);
    // Meaning matches come a moment after the word matches.
    if (flag("--ai")) await page.waitForSelector(".pane.list .meaning", { timeout: 5000 }).catch(() => {});
  }
  if (kind === "ask") {
    await settle(400);
    await page.click(".pane.list .ask button, .pane.list .status button");
    await page.waitForSelector("dialog .answer, dialog .status, dialog .error", { timeout: 5000 });
  }
  if (kind === "service") {
    await page.waitForSelector(`.service.${arg}`);
    await page.evaluate((name) => document.querySelector(`.service.${name}`).scrollIntoView({ block: "center" }), arg);
  }
  // Typing: the cursor in the note, with the format bar (on phones, the keyboard bar) up.
  if (kind === "edit") {
    await page.waitForSelector(".cm-content[contenteditable=true]");
    await page.click(".cm-content");
  }
  await settle();
}

let browser;
try {
  mkdirSync(out, { recursive: true });
  browser = await puppeteer.launch({ executablePath: process.env.CHROME ?? "/usr/bin/chromium", args: ["--no-sandbox", "--disable-gpu"] });
  const seeder = await browser.newPage();
  await seeder.setViewport({ width: 1280, height: 800 });
  await login(seeder);
  await seed(seeder);
  const cookies = await seeder.browserContext().cookies();
  const pages = [];
  for (const [name, viewport] of viewports) {
    const page = await browser.newPage();
    await page.browserContext().setCookie(...cookies);
    await page.setViewport(viewport);
    await page.emulateMediaFeatures([{ name: "prefers-color-scheme", value: scheme }]);
    await page.goto(base);
    await page.waitForSelector("nav");
    pages.push([name, page]);
  }
  if (offline) {
    // Opening each screen once saves its notes on the device; then the server goes away.
    for (const [, page] of pages) for (const target of targets) await go(page, target);
    await settle(1200);
    server.kill();
    await settle(1500);
  }
  for (const [name, page] of pages) {
    // Only the tab in front can be shot.
    await page.bringToFront();
    for (const target of targets) {
      await go(page, target);
      const file = join(out, `${name}-${scheme}${offline ? "-offline" : ""}-${target.replace(/[^a-z0-9]+/gi, "-").toLowerCase()}.png`);
      await page.screenshot({ path: file });
      console.log(file);
    }
    await page.close();
  }
} finally {
  await browser?.close();
  server.kill();
  ai?.close();
  rmSync(data, { recursive: true, force: true });
}
