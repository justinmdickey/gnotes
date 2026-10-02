// Two browsers, two accounts, one note: sharing, live edits and remote cursors.
// Usage: npm run build && cargo build -p gnotes-server && node e2e/live-edit.mjs
// Env: CHROME (default /usr/bin/chromium), SHOTS (directory for screenshots, optional).
import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import puppeteer from "puppeteer-core";

const root = resolve(import.meta.dirname, "../..");
const bin = join(root, "target/debug/gnotes-server");
const data = mkdtempSync(join(tmpdir(), "gnotes-e2e-"));
const port = 18000 + Math.floor(Math.random() * 1000);
const base = `http://127.0.0.1:${port}`;
// A stand-in speech-to-text service: answers every transcription request the same way.
const whisper = createServer((req, res) => {
  req.resume();
  req.on("end", () => {
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify({ text: "remember the milk" }));
  });
}).listen(port + 1, "127.0.0.1");
// A stand-in realtime speech-to-text service, speaking just enough websocket to stream a few words
// back while audio arrives and the full sentence when the recording is committed.
const realtime = createServer().listen(port + 2, "127.0.0.1");
realtime.on("upgrade", (req, sock) => {
  const accept = createHash("sha1").update(req.headers["sec-websocket-key"] + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest("base64");
  sock.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
  const send = (obj) => {
    const data = Buffer.from(JSON.stringify(obj));
    const head = data.length < 126 ? Buffer.from([0x81, data.length]) : Buffer.from([0x81, 126, data.length >> 8, data.length & 255]);
    sock.write(Buffer.concat([head, data]));
  };
  const words = ["remember ", "the ", "milk "];
  let chunks = 0;
  let buf = Buffer.alloc(0);
  sock.on("error", () => {});
  sock.on("data", (d) => {
    buf = Buffer.concat([buf, d]);
    for (;;) {
      if (buf.length < 2) return;
      let len = buf[1] & 0x7f;
      let off = 2;
      if (len === 126) [len, off] = [buf.readUInt16BE(2), 4];
      else if (len === 127) [len, off] = [Number(buf.readBigUInt64BE(2)), 10];
      const mask = buf.subarray(off, off + 4);
      off += 4;
      if (buf.length < off + len) return;
      const payload = Buffer.from(buf.subarray(off, off + len)).map((b, i) => b ^ mask[i % 4]);
      const op = buf[0] & 0x0f;
      buf = buf.subarray(off + len);
      if (op === 8) return void sock.end();
      if (op !== 1) continue;
      const event = JSON.parse(payload.toString());
      if (event.type === "session.update") send({ type: "session.updated" });
      else if (event.type === "input_audio_buffer.append" && ++chunks % 4 === 0 && chunks / 4 <= words.length)
        send({ type: "conversation.item.input_audio_transcription.delta", delta: words[chunks / 4 - 1] });
      else if (event.type === "input_audio_buffer.commit")
        send({ type: "conversation.item.input_audio_transcription.completed", transcript: "Remember the milk and the eggs." });
    }
  });
});
const env = {
  ...process.env,
  GNOTES_DATA_DIR: data,
  GNOTES_BIND: `127.0.0.1:${port}`,
  GNOTES_WEB_DIR: join(root, "web/dist"),
};

for (const [name, display] of [["alice", "Alice"], ["bob", "Bob"]]) {
  const admin = name === "alice" ? ["--admin"] : [];
  const r = spawnSync(bin, ["create-user", name, "--display-name", display, ...admin], { env: { ...env, GNOTES_PASSWORD: "password123" } });
  if (r.status !== 0) throw new Error(`create-user ${name}: ${r.stderr}`);
}
const server = spawn(bin, [], { env, stdio: "inherit" });

async function waitForServer() {
  for (let i = 0; i < 50; i++) {
    try {
      if ((await fetch(`${base}/api/health`)).ok) return;
    } catch {}
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error("server didn't start");
}

function check(cond, msg) {
  if (!cond) throw new Error(`FAILED: ${msg}`);
  console.log(`ok - ${msg}`);
}

// Dialogs animate out, so wait for them to leave before clicking what's underneath.
const closed = (page) => page.waitForFunction(() => !document.querySelector("dialog"));

const text = (page) => page.$eval(".cm-content", (el) => el.innerText);

/** Sidebar rows lit as drop targets at the end of the last drag. */
let dragLit = [];

/** A real mouse drag from one element to another, the way HTML drag and drop sees it. */
async function dragTo(page, from, to, shot) {
  const box = async (sel) => (await page.waitForSelector(sel)).boundingBox();
  const a = await box(from);
  const b = await box(to);
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
  await page.mouse.down();
  for (let i = 1; i <= 10; i++) await page.mouse.move(a.x + a.width / 2 + ((b.x - a.x) * i) / 10, a.y + a.height / 2 + ((b.y - a.y) * i) / 10);
  await page.mouse.move(b.x + 40, b.y + b.height / 2);
  await new Promise((r) => setTimeout(r, 100));
  if (shot && process.env.SHOTS) await page.screenshot({ path: join(process.env.SHOTS, `${shot}.png`) });
  dragLit = await page.evaluate(() => [...document.querySelectorAll("nav li.drop .label")].map((l) => l.textContent));
  await page.mouse.up();
}

async function login(browser, username) {
  const ctx = await browser.createBrowserContext();
  const page = await ctx.newPage();
  await page.setViewport({ width: 1280, height: 800 });
  page.on("pageerror", (e) => console.error(`[${username}] page error:`, e.message));
  await page.goto(base);
  await page.type("input[name=username]", username);
  await page.type("input[name=password]", "password123");
  await page.click("button[type=submit]");
  await page.waitForSelector("nav");
  return page;
}

let browser;
try {
  await waitForServer();
  browser = await puppeteer.launch({
    executablePath: process.env.CHROME ?? "/usr/bin/chromium",
    // A fake microphone, allowed without a prompt, for the voice memo test.
    args: ["--no-sandbox", "--disable-gpu", "--use-fake-ui-for-media-stream", "--use-fake-device-for-media-stream"],
  });
  const alice = await login(browser, "alice");
  const bob = await login(browser, "bob");

  // Alice writes a note and shares it with Bob.
  await alice.click(".list header button[aria-label='New note']");
  await alice.waitForSelector(".cm-content[contenteditable=true]");
  await alice.click(".cm-content");
  await alice.keyboard.type("# Groceries\nmilk");
  await alice.waitForFunction(() => document.querySelector(".editor .headerbar .title strong")?.textContent === "Groceries", { timeout: 5000 });
  check(true, "title follows the first line");

  await alice.click("button[aria-label='Share']");
  await alice.waitForSelector("dialog button.add");
  await alice.click("dialog button.add");
  await alice.waitForFunction(() => document.querySelector("dialog ul")?.textContent.includes("Bob"));
  await alice.click("dialog .actions button");
  await closed(alice);
  check(true, "alice shared the note with bob");
  await alice.waitForFunction(() => [...document.querySelectorAll(".note")].some((b) => b.textContent.includes("Groceries") && b.querySelector(".shared-badge")));
  check(true, "a shared note shows the shared badge");

  // Bob sees it appear without reloading, and opens it.
  await bob.waitForFunction(() => [...document.querySelectorAll("nav button")].some((b) => b.textContent.includes("Shared with Me")));
  await bob.evaluate(() => [...document.querySelectorAll("nav button")].find((b) => b.textContent.includes("Shared with Me")).click());
  await bob.waitForFunction(() => [...document.querySelectorAll("li button")].some((b) => b.textContent.includes("Groceries")));
  await bob.evaluate(() => [...document.querySelectorAll("li button")].find((b) => b.textContent.includes("Groceries")).click());
  await bob.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("milk"));
  check(true, "bob loads alice's text");

  // Bob types at the end; Alice sees it live.
  await bob.click(".cm-content");
  await bob.keyboard.down("Control");
  await bob.keyboard.press("End");
  await bob.keyboard.up("Control");
  await bob.keyboard.type("\neggs");
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("eggs"), { timeout: 5000 });
  check(true, "bob's edit reaches alice live");

  // Alice types too; both converge.
  await alice.click(".cm-content");
  await alice.keyboard.down("Control");
  await alice.keyboard.press("End");
  await alice.keyboard.up("Control");
  await alice.keyboard.type(" and bread");
  await bob.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("bread"), { timeout: 5000 });
  // Blur both so no line shows its raw Markdown, then compare what each person sees.
  for (const p of [alice, bob]) await p.evaluate(() => document.activeElement?.blur());
  const [a, b] = [await text(alice), await text(bob)];
  check(a === b, `both editors converge (${JSON.stringify(a)})`);

  // Remote cursor and presence avatar.
  await alice.waitForFunction(() => document.querySelector(".loro-cursor")?.style.getPropertyValue("--name") === '"Bob"', { timeout: 5000 });
  check(true, "alice sees bob's cursor labelled Bob");
  await bob.waitForSelector(".peers .avatar[title='Alice is here']", { timeout: 5000 });
  check(true, "bob sees alice in the presence list");

  // The checklist button turns the current line into a task; tapping the circle ticks it for everyone.
  await alice.click(".cm-content");
  await alice.keyboard.down("Control");
  await alice.keyboard.press("End");
  await alice.keyboard.up("Control");
  await alice.keyboard.press("Enter");
  await alice.keyboard.type("butter");
  await alice.click("button[aria-label='Checklist']");
  // Pressing a format button on an empty line leaves the cursor ready to type after the marker.
  await alice.keyboard.press("Enter");
  await alice.keyboard.type("jam");
  await alice.keyboard.press("Enter");
  await alice.keyboard.press("Enter");
  await alice.evaluate(() => [...document.querySelectorAll(".bar button")].find((b) => b.textContent.trim() === "Heading").click());
  await alice.keyboard.type("Later");
  await alice.evaluate(() => document.activeElement?.blur());
  // Marks hide once the editor has processed the blur.
  await alice.waitForFunction(() => [...document.querySelectorAll(".cm-line")].some((l) => l.innerText.trim() === "Later"), { timeout: 2000 }).catch(() => {});
  const lines = await alice.$$eval(".cm-line", (els) => els.map((el) => ({ text: el.innerText, h2: el.classList.contains("cm-h2"), box: !!el.querySelector(".cm-checkbox") })));
  const jam = lines.find((l) => l.text.includes("jam"));
  const later = lines.find((l) => l.text.includes("Later"));
  check(jam?.box && jam.text.trim() === "jam", `typing after Checklist lands in the item (${JSON.stringify(jam)})`);
  check(later?.h2 && later.text.trim() === "Later", `typing after Heading lands in the heading (${JSON.stringify(later)})`);
  await alice.waitForSelector(".cm-checkbox:not(.checked)");
  await alice.$eval(".cm-checkbox", (el) => el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })));
  await bob.waitForSelector(".cm-checkbox.checked", { timeout: 5000 });
  check(true, "checklist item created and ticked live");

  // Revoking removes access live.
  await alice.click("button[aria-label='Share']");
  await alice.waitForSelector("dialog button[aria-label='Remove Bob']");
  await alice.click("dialog button[aria-label='Remove Bob']");
  await alice.waitForSelector("dialog button.add");
  await bob.waitForFunction(() => document.querySelector(".lost")?.textContent.includes("no longer have access"), { timeout: 5000 });
  check(true, "bob loses access when alice unshares");

  // Invite someone new from the same dialog; they join from the link and see the note.
  await alice.click("dialog button.invite");
  const link = await alice.waitForSelector("dialog input.link").then((el) => el.evaluate((i) => i.value));
  await alice.click("dialog .actions button");
  await closed(alice);
  check(link.startsWith(`${base}/join/`), "invite link created");
  const carol = await (await browser.createBrowserContext()).newPage();
  await carol.setViewport({ width: 390, height: 844 });
  await carol.goto(link);
  await carol.waitForFunction(() => document.body.innerText.includes("Alice invited you"));
  await carol.type("input[name=name]", "Carol");
  await carol.type("input[name=password]", "password123");
  await carol.click("button[type=submit]");
  await carol.waitForSelector("nav");
  await carol.evaluate(() => [...document.querySelectorAll(".tabbar button")].find((b) => b.textContent.includes("Shared")).click());
  await carol.waitForFunction(() => [...document.querySelectorAll("li button")].some((b) => b.textContent.includes("Groceries")));
  check(true, "invitee joins from the link and sees the shared note");

  // The phone back button walks back through screens: note -> list -> notebooks.
  await carol.evaluate(() => [...document.querySelectorAll("li button")].find((b) => b.textContent.includes("Groceries")).click());
  await carol.waitForSelector(".cm-content");
  await carol.goBack();
  await carol.waitForFunction(() => location.hash === "#/shared");
  await carol.goBack();
  await carol.waitForFunction(() => location.hash === "#/");
  check(true, "back button goes note -> list -> the previous tab");

  // A new note that's left blank is thrown away, like Apple Notes.
  const count = () => alice.$$eval("li button .note-title", (els) => els.length);
  const before = await count();
  // With a note open on a wide screen, New note sits in the note's headerbar and other notes are in the sidebar.
  await alice.click(".pane.editor header button[aria-label='New note']");
  await alice.waitForFunction((n) => document.querySelectorAll("li button .note-title").length === n + 1, {}, before);
  await alice.waitForSelector(".cm-content[contenteditable=true]");
  await alice.click("nav .note-row ::-p-text(Groceries)");
  await alice.waitForFunction((n) => document.querySelectorAll("li button .note-title").length === n, { timeout: 5000 }, before);
  check(true, "blank new note is discarded on leaving");

  // Content survives a reload.
  await alice.reload();
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("bread"));
  check(true, "alice's note reloads from the server");

  // Settings: an admin resets Bob's password, and Bob's open app drops to the login screen.
  await alice.click("nav button[aria-label='Settings']");
  await alice.waitForSelector("button[aria-label='Manage Bob']");
  await alice.click("button[aria-label='Manage Bob']");
  await alice.evaluate(() => [...document.querySelectorAll("[role=menuitem]")].find((b) => b.textContent.includes("Reset Password")).click());
  await alice.type("#reset-password input", "bobsnewpass");
  await alice.click("dialog button[form=reset-password]");
  await closed(alice);
  await alice.waitForFunction(() => document.body.innerText.includes("password was reset"));
  await bob.waitForFunction(() => document.body.innerText.includes("You were signed out"), { timeout: 10000 });
  check(true, "admin reset signs the user out of their open app");

  // The admin points speech-to-text at a service from Settings, tests it and saves it.
  await alice.type(".stt input[type=url]", `http://127.0.0.1:${port + 1}/v1`);
  await alice.type(".stt input[placeholder^='Optional, e.g. ws']", `ws://127.0.0.1:${port + 2}/v1/realtime`);
  await alice.evaluate(() => [...document.querySelectorAll(".stt button")].find((b) => b.textContent === "Test").click());
  await alice.waitForFunction(() => document.querySelector(".stt .status")?.textContent.includes("live transcription works"), { timeout: 8000 });
  await alice.click(".stt button[type=submit]");
  await alice.waitForFunction(() => document.body.innerText.includes("Speech-to-text saved"));
  check(true, "admin sets up speech-to-text in settings");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-settings.png") });
  await alice.click(".settings-layer .back");
  await alice.waitForFunction(() => !document.querySelector(".settings-layer"));
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("bread"));

  // A zip picked in Settings becomes a notebook with its folders inside, and the toast's Show opens it.
  const beforeImport = await alice.evaluate(() => location.hash);
  const vault = join(data, "vault");
  mkdirSync(join(vault, "Vault/Trips"), { recursive: true });
  writeFileSync(join(vault, "Vault/Trips/Lisbon.md"), "pastel de nata");
  writeFileSync(join(vault, "Vault/Ideas.md"), "# Ideas\nfly");
  spawnSync("zip", ["-qr", join(data, "Vault.zip"), "Vault"], { cwd: vault });
  await alice.click("nav button[aria-label='Settings']");
  const [importer] = await Promise.all([alice.waitForFileChooser(), alice.click(".settings-layer ::-p-text(Import…)")]);
  await importer.accept([join(data, "Vault.zip")]);
  await alice.waitForFunction(() => document.querySelector(".toast")?.innerText.includes("Imported 2 notes"), { timeout: 5000 });
  await alice.click(".toast ::-p-text(Show)");
  await alice.waitForFunction(() => !document.querySelector(".settings-layer") && document.querySelector(".hero h1")?.textContent === "Vault");
  check(await alice.evaluate(() => document.body.innerText.includes("Trips") && document.body.innerText.includes("Ideas")), "an imported zip becomes a notebook with its folders and notes");

  // Markdown dropped on the window lands in the folder you're in, even dropped on the editor.
  if (process.env.SHOTS) {
    // Keep a drag going while the shot is taken; the overlay leaves when dragover stops.
    await alice.evaluate(() => {
      const dt = new DataTransfer();
      dt.items.add(new File(["x"], "x.md"));
      window.__drag = setInterval(() => document.body.dispatchEvent(new DragEvent("dragover", { dataTransfer: dt, bubbles: true, cancelable: true })), 50);
    });
    await alice.waitForSelector(".drop-target");
    await new Promise((r) => setTimeout(r, 300));
    await alice.screenshot({ path: join(process.env.SHOTS, "desktop-drop-import.png") });
    await alice.evaluate(() => clearInterval(window.__drag));
    await alice.waitForFunction(() => !document.querySelector(".drop-target"));
  }
  await alice.evaluate(() => {
    const dt = new DataTransfer();
    dt.items.add(new File(["- tent\n- stove"], "Camping.md", { type: "text/markdown" }));
    const at = document.querySelector(".cm-content") ?? document.body;
    at.dispatchEvent(new DragEvent("dragover", { dataTransfer: dt, bubbles: true, cancelable: true }));
    at.dispatchEvent(new DragEvent("drop", { dataTransfer: dt, bubbles: true, cancelable: true }));
  });
  await alice.waitForFunction(() => [...document.querySelectorAll(".note-title")].some((t) => t.textContent.trim() === "Camping"), { timeout: 5000 });
  check(await alice.evaluate(() => document.querySelector(".hero h1")?.textContent === "Vault" && !document.querySelector(".drop-target")), "dropped Markdown imports into the open notebook");

  // The sidebar lists a folder's notebooks first, then its notes A–Z, and a note row opens the note.
  const sidebar = () => alice.evaluate(() => [...document.querySelectorAll("nav .row .label")].map((l) => l.textContent));
  await alice.waitForFunction(() => [...document.querySelectorAll("nav .note-row .label")].some((l) => l.textContent === "Camping"));
  const nbIdOf = (name) => alice.evaluate((name) => fetch("/api/tree").then((r) => r.json()).then((t) => t.notebooks.find((n) => n.name === name).id), name);
  // Folders start folded; going to Vault unfolded it, but Trips inside stays folded.
  const rows = await sidebar();
  const at = (name) => rows.indexOf(name);
  check(at("Vault") < at("Trips") && at("Trips") < at("Camping") && at("Camping") < at("Ideas") && at("Lisbon") === -1, `sidebar shows folders, then notes, with sub-folders folded (${rows})`);
  await alice.click("nav button[aria-label=\"Show what's in Trips\"]");
  await alice.waitForFunction(() => [...document.querySelectorAll("nav .note-row .label")].some((l) => l.textContent === "Lisbon"));
  check(true, "unfolding a notebook shows its notes");
  const saved = await alice.evaluate(() => JSON.parse(localStorage.getItem("gnotes.folds")));
  check(JSON.stringify(saved) === JSON.stringify([await nbIdOf("Trips")]), "only folders unfolded by hand are remembered");

  // The sidebar's edge drags wider, and the width comes back after a reload.
  const edge = await (await alice.$(".resize")).boundingBox();
  await alice.mouse.move(edge.x + 4, 300);
  await alice.mouse.down();
  await alice.mouse.move(edge.x + 84, 300, { steps: 5 });
  await alice.mouse.up();
  const wide = await alice.evaluate(() => document.querySelector(".pane.sidebar").getBoundingClientRect().width);
  check(Math.abs(wide - 340) <= 2, `the sidebar drags wider (${wide}px)`);
  await alice.reload();
  await alice.waitForSelector("nav .row");
  check(Math.abs((await alice.evaluate(() => document.querySelector(".pane.sidebar").getBoundingClientRect().width)) - 340) <= 2, "the sidebar keeps its width after a reload");
  await alice.click(".resize", { count: 2 });
  await alice.waitForFunction(() => Math.abs(document.querySelector(".pane.sidebar").getBoundingClientRect().width - 260) <= 2);
  await alice.click("nav .note-row ::-p-text(Ideas)");
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("fly"));
  check(await alice.evaluate(() => document.querySelector("nav .row.selected .label")?.textContent === "Ideas"), "a sidebar note opens and takes the highlight from its folder");

  // Wide screens in a folder use two panes: the note covers the folder's page, and Back returns to it.
  const panes = () => alice.evaluate(() => ["list", "editor"].filter((p) => getComputedStyle(document.querySelector(`.pane.${p}`)).display !== "none"));
  check(JSON.stringify(await panes()) === '["editor"]', "an open note fills the main pane in a folder");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-two-pane-note.png") });
  await alice.click(".pane.editor header button[aria-label='Back to Vault']");
  await alice.waitForFunction(() => !location.hash.includes("/note/"));
  check(JSON.stringify(await panes()) === '["list"]', "Back shows the folder's page in its place");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-two-pane-folder.png") });
  await alice.click("nav .row ::-p-text(Recent)");
  await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Recent");
  check(JSON.stringify(await panes()) === '["list"]', "Recent fills the main pane like a folder");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-two-pane-recent.png") });
  await alice.click(".pane.list .note ::-p-text(Ideas)");
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("fly"));
  check(JSON.stringify(await panes()) === '["editor"]', "a note opened from Recent covers it");
  await alice.click(".pane.editor header button[aria-label='Back to Recent']");
  await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Recent" && !location.hash.includes("/note/"));
  await alice.click("nav .row ::-p-text(Vault)");
  await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Vault");

  // Pointing at a folder in the sidebar shows a + that makes a notebook or a note inside it.
  await alice.hover("nav li:has(button[aria-label='New in Vault'])");
  await alice.click("nav button[aria-label='New in Vault']");
  await alice.waitForSelector(".popover [role=menuitem]");
  if (process.env.SHOTS) await (await new Promise((r) => setTimeout(r, 300)), alice.screenshot({ path: join(process.env.SHOTS, "desktop-sidebar-add.png") }));
  if (process.env.SHOTS) {
    // A row without sub-notebooks, in dark: the + sits where its count was.
    await alice.keyboard.press("Escape");
    await alice.emulateMediaFeatures([{ name: "prefers-color-scheme", value: "dark" }]);
    await alice.hover("nav li:has(button[aria-label='New in Trips'])");
    await new Promise((r) => setTimeout(r, 300));
    await alice.screenshot({ path: join(process.env.SHOTS, "desktop-sidebar-add-dark.png"), clip: { x: 0, y: 0, width: 600, height: 260 } });
    await alice.emulateMediaFeatures([]);
    await alice.click("nav button[aria-label='New in Vault']");
    await alice.waitForSelector(".popover [role=menuitem]");
  }
  await alice.click(".popover ::-p-text(New Notebook)");
  await alice.type("dialog input", "Packing");
  await alice.click("dialog button[type=submit]");
  await closed(alice);
  await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Packing" && document.querySelector(".hero .crumbs")?.textContent.includes("Vault"));
  check(true, "the sidebar + makes a notebook inside the folder you point at");
  await alice.hover("nav li:has(button[aria-label='New in Packing'])");
  await alice.click("nav button[aria-label='New in Packing']");
  await alice.click(".popover ::-p-text(New Note)");
  await alice.waitForSelector(".pane.editor .cm-content[contenteditable=true]");
  await alice.keyboard.type("Socks");
  await alice.waitForFunction(() => [...document.querySelectorAll(".note-title")].some((t) => t.textContent.trim() === "Socks"));
  check(await alice.evaluate(() => document.querySelector(".hero h1")?.textContent === "Packing"), "the sidebar + makes a note inside the folder you point at");

  // Desktop drag and drop: a note from the list and a notebook in the sidebar move onto sidebar folders.
  const tree = () => alice.evaluate(() => fetch("/api/tree").then((r) => r.json()));
  const nbId = async (name) => (await tree()).notebooks.find((n) => n.name === name).id;
  const socks = (await tree()).notes.find((n) => n.title === "Socks").id;
  await dragTo(alice, "nav .note-row ::-p-text(Socks)", "nav li:has(button[aria-label='New in Trips']) .row", "desktop-drag-note");
  await alice.waitForFunction(() => document.querySelector(".toast")?.innerText.includes("Moved to Trips"));
  check((await tree()).notes.find((n) => n.id === socks).notebook_id === (await nbId("Trips")), "a note dragged onto a sidebar notebook moves there");
  if (process.env.SHOTS) await alice.emulateMediaFeatures([{ name: "prefers-color-scheme", value: "dark" }]);
  await dragTo(alice, "nav li:has(button[aria-label='New in Packing']) .row", "nav li:has(button[aria-label='New in Notes']) .row", "desktop-drag-notebook-dark");
  if (process.env.SHOTS) await alice.emulateMediaFeatures([]);
  check(dragLit.length === 1 && dragLit[0] === "Notes", `only the folder under the drag lights up (${dragLit})`);
  await alice.waitForFunction(() => document.querySelector(".toast")?.innerText.includes("Moved to Notes"));
  check((await tree()).notebooks.find((n) => n.name === "Packing").parent_id === null, "a notebook dragged onto Notes moves to the top");
  await alice.click(".toast ::-p-text(Undo)");
  await alice.waitForFunction(async (vault) => (await fetch("/api/tree").then((r) => r.json())).notebooks.find((n) => n.name === "Packing").parent_id === vault, {}, await nbId("Vault"));
  check(true, "Undo puts a dragged notebook back");
  await dragTo(alice, "nav li:has(button[aria-label='New in Vault']) .row", "nav li:has(button[aria-label='New in Trips']) .row");
  await new Promise((r) => setTimeout(r, 400));
  check((await tree()).notebooks.find((n) => n.name === "Vault").parent_id === null, "a notebook can't be dropped inside itself");
  await alice.evaluate((hash) => (location.hash = hash), beforeImport);
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("bread"));

  // Inside a nested notebook the list shows its path and name, and its notes show a notebook chip.
  const kitchen = await alice.evaluate(async () => {
    const post = (path, body) => fetch(`/api${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) }).then((r) => r.json());
    const home = await post("/notebooks", { name: "Home", parent_id: null });
    const kitchen = await post("/notebooks", { name: "Kitchen", parent_id: home.id });
    return kitchen.id;
  });
  await alice.evaluate((id) => (location.hash = `#/nb/${id}`), kitchen);
  await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Kitchen" && document.querySelector(".hero .crumbs")?.textContent.includes("Home"));
  check(true, "nested notebook shows its path and name");
  // The notebook menu floats above the editor pane instead of being clipped by the list.
  await alice.click(".list header button[aria-label='Notebook menu']");
  await alice.waitForSelector(".popover [role=menuitem]");
  check(
    await alice.evaluate(() => {
      const pop = document.querySelector(".popover");
      return pop.parentElement === document.body && [...pop.querySelectorAll("[role=menuitem]")].every((it) => {
        const b = it.getBoundingClientRect();
        return pop.contains(document.elementFromPoint(b.right - 6, b.top + b.height / 2));
      });
    }),
    "the notebook menu sits above every pane",
  );
  await alice.keyboard.press("Escape");
  await alice.waitForFunction(() => !document.querySelector(".popover"));
  await alice.click(".list header button[aria-label='New note']");
  await alice.waitForSelector(".cm-content[contenteditable=true]");
  // The "Title" hint is drawn by the line, not an inline widget that would push the caret off the title.
  check(await alice.$eval(".cm-line", (l) => l.classList.contains("cm-title-empty") && !l.querySelector("[contenteditable=false]")), "an empty title line has only the caret on it");
  await alice.keyboard.type("Pantry");
  await alice.waitForFunction(() => document.querySelector(".notebook-chip")?.textContent.trim() === "Home › Kitchen");
  check(true, "a note in a notebook shows the notebook chip");
  if (process.env.SHOTS) await (await new Promise((r) => setTimeout(r, 400)), alice.screenshot({ path: join(process.env.SHOTS, "desktop-notebook.png") }));

  // Folders: make a sub-notebook from the notebook menu, see it listed in its parent, and move a note into it.
  const pantry = await alice.evaluate(() => location.hash.match(/note\/([0-9a-f-]{36})/)[1]);
  await alice.evaluate(() => {
    const kitchen = location.hash.match(/nb\/([0-9a-f-]{36})/)[1];
    location.hash = `#/nb/${kitchen}`;
  });
  await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Kitchen");
  await alice.click(".hero .crumbs button.crumb:not(.root)");
  await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Home");
  await alice.click(".pane.list header button[aria-label='New notebook']");
  await alice.type("#new-notebook input", "Garage");
  await alice.click("dialog button[form=new-notebook]");
  await closed(alice);
  await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Garage" && document.querySelector(".hero .crumbs")?.textContent.includes("Home"));
  check(true, "a notebook is made inside another the same way as at the top");
  await alice.click(".hero .crumbs button.crumb:not(.root)");
  await alice.waitForFunction(() => {
    const names = [...document.querySelectorAll(".pane.list .folder-row .name")].map((e) => e.textContent.trim());
    return names.join(",") === "Garage,Kitchen";
  });
  check(true, "a notebook lists its sub-notebooks as folders");
  await alice.evaluate((id) => (location.hash = `${location.hash}/note/${id}`), pantry);
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("Pantry"));
  await alice.click(".editor header button[aria-label='Note menu']");
  await alice.waitForFunction(() => [...document.querySelectorAll("[role=menuitem]")].some((b) => b.textContent.includes("Move to")));
  await alice.evaluate(() => [...document.querySelectorAll("[role=menuitem]")].find((b) => b.textContent.includes("Move to")).click());
  await alice.waitForSelector("dialog [aria-label=Destinations]");
  await alice.evaluate(() => [...document.querySelectorAll("dialog [aria-label=Destinations] button")].find((b) => b.textContent.trim() === "Garage").click());
  await closed(alice);
  await alice.waitForFunction(() => document.querySelector(".notebook-chip")?.textContent.trim() === "Home › Garage");
  check(true, "a note moves to another notebook");

  // Deep down, the middle levels fold into a "…" crumb that still reaches them.
  const deepest = await alice.evaluate(async () => {
    const post = (path, body) => fetch(`/api${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) }).then((r) => r.json());
    const tree = await fetch("/api/tree").then((r) => r.json());
    let parent = tree.notebooks.find((n) => n.name === "Garage").id;
    for (const name of ["Shelf", "Box", "Bag", "Pocket"]) parent = (await post("/notebooks", { name, parent_id: parent })).id;
    return parent;
  });
  await alice.evaluate((id) => (location.hash = `#/nb/${id}`), deepest);
  await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Pocket" && document.querySelector(".hero .crumb.more"));
  const shown = await alice.$$eval(".hero .crumbs button.crumb:not(.root):not(.more)", (els) => els.map((e) => e.textContent.trim()).join(","));
  check(shown === "Home,Box,Bag", `a deep path shows the top and nearest levels (${shown})`);
  await alice.click(".hero .crumb.more");
  await alice.waitForFunction(() => [...document.querySelectorAll("[role=menuitem]")].some((b) => b.textContent.includes("Garage")));
  await alice.evaluate(() => [...document.querySelectorAll("[role=menuitem]")].find((b) => b.textContent.includes("Shelf")).click());
  await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Shelf");
  check(true, "the folded levels open from the … crumb");
  await alice.evaluate((id) => (location.hash = `#/all/note/${id}`), pantry);
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("Pantry"));

  // A photo picked from the format bar uploads and shows inline.
  const png = join(data, "dot.png");
  writeFileSync(png, Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEklEQVR4nGP4z8CAFWEXHbQSACj/P8Fu7N9hAAAAAElFTkSuQmCC", "base64"));
  const [chooser] = await Promise.all([alice.waitForFileChooser(), alice.click("button[aria-label='Add photo']")]);
  await chooser.accept([png]);
  await alice.waitForFunction(() => document.querySelector(".cm-attachment img")?.naturalWidth === 8, { timeout: 5000 });
  check(true, "a picked photo uploads and shows in the note");

  // A voice memo records, embeds a player, and gets its transcript underneath.
  // With the cursor on the title, the memo goes on the line right under it, marked before any words come.
  await alice.click(".cm-content");
  await alice.keyboard.down("Control");
  await alice.keyboard.press("Home");
  await alice.keyboard.up("Control");
  await alice.click("button[aria-label='Record voice memo']");
  await alice.waitForSelector(".recorder .stop:not([disabled])");
  check(await alice.evaluate(() => !document.querySelector("dialog")), "recording shows a small bar, not a dialog over the note");
  check(
    await alice.evaluate(() => !!document.querySelector(".cm-line:first-child .cm-listening") && !document.querySelector(".cm-content").innerText.includes("remember")),
    "a marker shows where the memo will go before any words arrive",
  );
  // With a live URL set, words land in the note while still recording.
  await alice.waitForFunction(() => document.querySelector(".cm-content").innerText.includes("remember the"), { timeout: 8000 });
  check(await alice.evaluate(() => document.querySelector(".recorder .tag.on")?.textContent.trim() === "Live"), "the live transcript writes into the note while recording");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-recording-live.png") });
  await alice.click(".recorder .stop");
  await alice.waitForSelector(".cm-audio", { timeout: 5000 });
  await alice.waitForFunction(() => document.querySelector(".cm-content").innerText.includes("Remember the milk and the eggs."), { timeout: 5000 });
  check(
    await alice.evaluate(() => {
      const text = document.querySelector(".cm-content").innerText;
      return !text.includes("remember the milk") && text.split("Remember the milk").length === 2;
    }),
    "stopping swaps in the final transcript under the recording",
  );
  check(
    await alice.evaluate(() => {
      const lines = [...document.querySelectorAll(".cm-line")];
      return !!lines[1]?.querySelector(".cm-audio") && lines[2]?.textContent.trim() === "Remember the milk and the eggs." && !document.querySelector(".cm-listening");
    }),
    "the memo lands where the cursor was: player under the title, transcript under it",
  );
  check(true, "a voice memo embeds a player with its transcript");
  if (process.env.SHOTS) {
    await closed(alice);
    await alice.click(".cm-audio-play");
    await alice.waitForSelector(".cm-audio.playing");
    await (await alice.$(".cm-audio")).screenshot({ path: join(process.env.SHOTS, "audio-playing.png") });
    await alice.screenshot({ path: join(process.env.SHOTS, "desktop-attachments.png") });
  }

  // A note that starts with a photo offers a title line above it, and styles leave embeds alone.
  await closed(alice);
  await alice.click(".pane.editor header button[aria-label='New note']");
  await alice.waitForFunction(() => document.querySelector(".editor .headerbar .title strong")?.textContent === "New Note");
  await alice.waitForSelector(".cm-content[contenteditable=true]");
  const photoId = await alice.evaluate(async () => {
    const form = new FormData();
    form.append("note_id", location.hash.split("/note/")[1]);
    const png = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEklEQVR4nGP4z8CAFWEXHbQSACj/P8Fu7N9hAAAAAElFTkSuQmCC"), (c) => c.charCodeAt(0));
    form.append("file", new File([png], "p.png", { type: "image/png" }));
    return (await fetch("/api/attachments", { method: "POST", body: form }).then((r) => r.json())).id;
  });
  await alice.keyboard.type(`![Photo](att:${photoId})`);
  await alice.evaluate(() => document.activeElement?.blur());
  await alice.waitForSelector(".cm-title-hint.add");
  await alice.click(".cm-content");
  await alice.evaluate(() => [...document.querySelectorAll(".bar button")].find((b) => b.textContent.trim() === "Title").click());
  await alice.evaluate(() => document.activeElement?.blur());
  await alice.waitForSelector(".cm-attachment img");
  const raw = await alice.$eval(".cm-content", (el) => el.innerText);
  check(!raw.includes("att:"), "the Title style leaves a photo line alone");
  await alice.$eval(".cm-title-hint.add", (el) => el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })));
  await alice.keyboard.type("Holiday");
  await alice.waitForFunction(() => document.querySelector(".editor .headerbar .title strong")?.textContent === "Holiday", { timeout: 5000 });
  check(await alice.$(".cm-attachment img") !== null, "“Add a title” puts a title above a leading photo");
  await alice.evaluate(() => (location.hash = "#/all"));
  await alice.waitForFunction(() => [...document.querySelectorAll("li button")].some((b) => b.textContent.includes("Groceries")));
  await alice.evaluate(() => [...document.querySelectorAll("li button")].find((b) => b.textContent.includes("Groceries")).click());

  if (process.env.SHOTS) {
    // Let slide and fade animations settle first.
    const shot = async (name) => (await new Promise((r) => setTimeout(r, 450)), alice.screenshot({ path: join(process.env.SHOTS, `${name}.png`) }));
    await alice.click(".cm-content");
    await shot("desktop-editing");
    await alice.evaluate(() => document.activeElement?.blur());
    await shot("desktop");
    // Switching to mobile emulation reloads the page.
    await alice.setViewport({ width: 390, height: 844, isMobile: true, hasTouch: true });
    await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("bread"));
    await shot("phone-editor");
    const noteUrl = alice.url();
    await alice.goto(`${base}/#/settings`);
    await alice.waitForSelector("button[aria-label='Manage Bob']");
    await shot("phone-settings");
    await alice.evaluate(() => document.querySelector(".stt").scrollIntoView({ block: "center" }));
    await shot("phone-settings-stt");
    await alice.goto(noteUrl);
    await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("bread"));
    await alice.tap(".editor header button[aria-label='Share']");
    await alice.waitForSelector("dialog button.add");
    await shot("phone-share");
    await alice.tap("dialog .actions button");
    await closed(alice);
    await alice.tap(".cm-content");
    await shot("phone-editing");
    // Typing a long note on a phone keeps the cursor clear of the format bar, with room below it.
    await alice.keyboard.down("Control");
    await alice.keyboard.press("End");
    await alice.keyboard.up("Control");
    for (let i = 0; i < 40; i++) await alice.keyboard.type(`\nline ${i}`);
    await new Promise((r) => setTimeout(r, 700));
    const gap = await alice.evaluate(() => {
      const caret = [...document.querySelectorAll(".cm-line")].at(-1).getBoundingClientRect();
      const bar = document.querySelector(".format").getBoundingClientRect();
      return Math.round(bar.top - caret.bottom);
    });
    await shot("phone-typing-long");
    check(gap >= 100, `cursor stays above the phone format bar with room (${gap}px)`);
    await shot("phone-typing-long");
    await alice.tap("button[aria-label='Text styles']");
    await shot("phone-editing-styles");
    await alice.tap("button[aria-label='Text styles']");
    // The keyboard bar has its own way out, so Done at the top isn't the only one.
    await alice.tap("button[aria-label='Hide keyboard']");
    await alice.waitForSelector(".tabbar");
    check(await alice.evaluate(() => !document.querySelector(".cm-editor.cm-focused")), "the phone keyboard bar can hide the keyboard");
    await alice.click(".editor .back-icon");
    await new Promise((r) => setTimeout(r, 500));
    await shot("phone-list");
    await alice.emulateMediaFeatures([{ name: "prefers-color-scheme", value: "dark" }]);
    // The tab bar is the way between sections; Notes opens the top folder.
    await alice.evaluate(() => [...document.querySelectorAll(".tabbar button")].find((b) => b.textContent.includes("Notes")).click());
    await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Notes");
    await shot("phone-home-dark");
    await alice.type(".search input", "pantry");
    await alice.waitForFunction(() => [...document.querySelectorAll(".note .where")].some((w) => w.textContent.includes("Home › Garage")));
    check(true, "searching from the top finds notes anywhere and says where they are");
    await shot("phone-search-dark");
    await alice.click(".search .clear");
    await alice.evaluate(() => [...document.querySelectorAll(".pane.list .folder-row")].find((b) => b.textContent.includes("Home")).click());
    await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Home");
    await alice.evaluate(() => [...document.querySelectorAll(".pane.list .folder-row")].find((b) => b.textContent.includes("Kitchen")).click());
    await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Kitchen");
    await shot("phone-subfolder-dark");
    await alice.click(".list .back-icon");
    await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Home");
    await shot("phone-folders-dark");
    await alice.evaluate((id) => (location.hash = `#/nb/${id}`), deepest);
    await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Pocket");
    await shot("phone-deep-dark");
    // The + in the tab bar makes a note in one tap, inside the folder you're in.
    const inPocket = await alice.evaluate(() => document.querySelectorAll(".note").length);
    await alice.tap(".tabbar .compose");
    await alice.waitForSelector(".pane.editor .cm-content[contenteditable=true]");
    await alice.keyboard.type("Lint");
    await alice.waitForFunction(() => document.querySelector(".notebook-chip")?.textContent.includes("Pocket"));
    check(inPocket === 0, "the tab bar + makes a note in the current folder");
    await alice.evaluate(() => document.activeElement?.blur());
    await shot("phone-compose-dark");
    // The recording bar floats above the format bar on a phone, with the note still in view.
    await alice.tap(".cm-content");
    await alice.waitForSelector("button[aria-label='Record voice memo']", { visible: true });
    await alice.tap("button[aria-label='Record voice memo']");
    await alice.waitForSelector(".recorder .stop:not([disabled])");
    await shot("phone-recording-dark");
    await alice.tap(".recorder button[aria-label='Discard recording']");
    await alice.waitForFunction(() => !document.querySelector(".recorder"));
    check(await alice.evaluate(() => !document.querySelector(".cm-listening") && !document.querySelector(".cm-content").innerText.includes("remember")), "discarding takes back the marker and any live text");
    // Account is a tab like the others: the tab bar stays and there's no Back.
    await alice.evaluate(() => [...document.querySelectorAll(".tabbar button")].find((b) => b.textContent.includes("Account")).click());
    await alice.waitForFunction(() => document.querySelector(".tab-page h1")?.textContent === "Account");
    check(await alice.evaluate(() => !!document.querySelector(".tabbar .tab.on") && !document.querySelector(".tab-page .back")), "Account opens as a tab with the tab bar still there");
    await shot("phone-account-dark");
    await alice.evaluate(() => [...document.querySelectorAll(".tabbar button")].find((b) => b.textContent.includes("Recent")).click());
    await alice.waitForFunction(() => !document.querySelector(".tab-page") && document.querySelector(".hero h1")?.textContent === "Recent");
    check(true, "another tab leaves Account");
  }
  console.log("all checks passed");
} finally {
  await browser?.close();
  server.kill();
  whisper.close();
  realtime.close();
  rmSync(data, { recursive: true, force: true });
}
