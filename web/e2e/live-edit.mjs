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
// It's also the chat model: a photo always reads as a short shopping list, a note (text only)
// summarizes to a fixed summary, and a question (Ask) gets a one-line answer citing note [1],
// streamed a few characters at a time, each a moment later. A follow-up is rewritten as the first
// question plus the follow-up. Tidy Up adds a heading and makes plain lines bullets. And the embedding model: dairy words land together; any other word is its own
// direction (hashed), so texts only come close when they share a word or are both about dairy.
const whisper = createServer((req, res) => {
  const chunks = [];
  req.on("data", (c) => chunks.push(c));
  req.on("end", () => {
    res.setHeader("content-type", "application/json");
    if (req.url.endsWith("/chat/completions")) {
      const body = JSON.parse(Buffer.concat(chunks).toString());
      const photo = Array.isArray(body.messages[0].content);
      const last = body.messages.at(-1).content;
      if (body.messages[0].role === "system") {
        const content = last.includes("cheese") ? "Cheese is on your list too [1]." : "You have milk and cheese on your list [1].";
        res.setHeader("content-type", "text/event-stream");
        const pieces = content.match(/.{1,6}/g).map((text) => `data: ${JSON.stringify({ choices: [{ delta: { content: text } }] })}\n\n`);
        let i = 0;
        const next = () => (i < pieces.length ? (res.write(pieces[i++]), setTimeout(next, 60)) : res.end("data: [DONE]\n\n"));
        setTimeout(next, 800);
        return;
      }
      // Tidy Up: a heading under the title, and plain lines outside code as bullets.
      const [, note] = photo ? [] : last.split("The note:\n\n");
      if (note !== undefined) {
        let code = false;
        const [first, ...rest] = note.split("\n");
        const lines = rest.map((l) => {
          if (l.startsWith("```")) code = !code;
          return code || l.startsWith("```") || !l.trim() || /^\s*([-*#>|!]|\d+\.)/.test(l) ? l : `- ${l}`;
        });
        const content = ["<think>Tidying.</think>", first, "", "## To buy", "", ...lines].join("\n");
        setTimeout(() => res.end(JSON.stringify({ choices: [{ message: { role: "assistant", content } }] })), 800);
        return;
      }
      const followUp = !photo && last.split("\nLast message: ");
      const content = photo
        ? "SHOPPING LIST\nOat milk"
        : followUp.length === 2
          ? `${followUp[0].split("\n").find((l) => l.startsWith("User: ")).slice(6)} ${followUp[1]}`
          : "Things to buy this week.\n\n## Key points\n- **Milk** and eggs\n\n## Action items\n- [ ] Buy bread";
      setTimeout(() => res.end(JSON.stringify({ choices: [{ message: { role: "assistant", content } }] })), 800);
    } else if (req.url.endsWith("/embeddings")) {
      const dairy = ["milk", "dairy", "cheese", "butter", "yogurt"];
      const embed = (t) => {
        const v = new Array(65).fill(0);
        for (const w of t.toLowerCase().split(/\W+/).filter(Boolean)) {
          if (dairy.includes(w)) v[0] += 3;
          else v[1 + ([...w].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7) % 64)] += 1;
        }
        return v;
      };
      const { input } = JSON.parse(Buffer.concat(chunks).toString());
      res.end(JSON.stringify({ data: input.map((t, index) => ({ index, embedding: embed(t) })) }));
    } else res.end(JSON.stringify({ text: "remember the milk" }));
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

function createUser(name, display) {
  const r = spawnSync(bin, ["create-user", name, "--display-name", display], { env: { ...env, GNOTES_PASSWORD: "password123" } });
  if (r.status !== 0) throw new Error(`create-user ${name}: ${r.stderr}`);
}
let server = spawn(bin, [], { env, stdio: "inherit" });

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

/** The note with a table and a kanban board, for screenshots. */
let plansId;

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
  // A fresh server asks for its first account on the login screen; Alice makes it and is the admin.
  const alice = await (await browser.createBrowserContext()).newPage();
  await alice.setViewport({ width: 1280, height: 800 });
  alice.on("pageerror", (e) => console.error("[alice] page error:", e.message));
  await alice.goto(base);
  await alice.waitForSelector("input[name=name]");
  await alice.type("input[name=name]", "Alice");
  check((await alice.$eval("input[name=username]", (el) => el.value)) === "alice", "first-run setup suggests a username from the name");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "first-run.png") });
  await alice.type("input[name=password]", "password123");
  await alice.click("button[type=submit]");
  await alice.waitForSelector("nav");
  check((await (await fetch(`${base}/api/auth/setup`)).json()).needed === false, "first-run setup makes the first account");
  createUser("bob", "Bob");
  const bob = await login(browser, "bob");

  // Alice writes a note and shares it with Bob.
  await alice.click(".list header button[aria-label='New note']");
  await alice.waitForSelector(".cm-content[contenteditable=true]");
  await alice.click(".cm-content");
  await alice.keyboard.type("# Groceries\nmilk");
  await alice.waitForFunction(() => document.querySelector(".editor .headerbar .title strong")?.textContent === "Groceries", { timeout: 5000 });
  check(true, "title follows the first line");

  // Undo and redo, from the keyboard and the format bar. A pause ends an undo step.
  const undoState = () => alice.evaluate(() => ["Undo", "Redo"].map((l) => document.querySelector(`.format button[aria-label=${l}]`).disabled).join(","));
  const press = async (...keys) => {
    for (const k of keys.slice(0, -1)) await alice.keyboard.down(k);
    await alice.keyboard.press(keys.at(-1));
    for (const k of keys.slice(0, -1).reverse()) await alice.keyboard.up(k);
  };
  const oats = (has) => alice.waitForFunction((has) => document.querySelector(".cm-content").innerText.includes("milk oats") === has && document.querySelector(".cm-content").innerText.includes("milk"), { timeout: 3000 }, has);
  await new Promise((r) => setTimeout(r, 1100));
  await alice.keyboard.type(" oats");
  await press("Control", "z");
  await oats(false);
  check(true, "Ctrl+Z takes back the last thing typed");
  await press("Control", "Shift", "Z");
  await oats(true);
  check(true, "Ctrl+Shift+Z redoes it");
  await press("Control", "z");
  await oats(false);
  await press("Control", "y");
  await oats(true);
  check(true, "Ctrl+Y redoes too");
  check((await undoState()) === "false,true", "with nothing undone, Redo is disabled");
  await alice.click(".format button[aria-label=Undo]");
  await oats(false);
  await alice.click(".format button[aria-label=Redo]");
  await oats(true);
  await alice.click(".format button[aria-label=Undo]");
  await oats(false);
  check((await undoState()) === "false,false", "the Undo and Redo buttons step through the same history");

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

  // Who wrote what: in a shared note, a line someone else typed in last gets a bar in their color.
  await bob.click(".cm-content");
  await bob.keyboard.down("Control");
  await bob.keyboard.press("End");
  await bob.keyboard.up("Control");
  await bob.keyboard.type("\ncheese");
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("cheese"), { timeout: 5000 });
  await bob.evaluate(() => document.activeElement?.blur());
  const blameOf = (t) => alice.evaluate((t) => [...document.querySelectorAll(".cm-line")].find((l) => l.textContent.includes(t))?.querySelector(".cm-blame")?.title ?? "", t);
  await alice.waitForFunction(() => [...document.querySelectorAll(".cm-line")].find((l) => l.textContent.includes("cheese"))?.querySelector(".cm-blame")?.title.startsWith("Bob"), { timeout: 8000 });
  const [titleBy, cheeseBy] = [await blameOf("Groceries"), await blameOf("cheese")];
  check(titleBy === "" && cheeseBy.startsWith("Bob ·"), `in a shared note, lines someone else wrote get a bar and yours don't (${cheeseBy})`);
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-blame.png") });
  // Search reads whole notes on the server: "cheese" is past the title and preview, and half-typed.
  for (const p of [alice, bob]) {
    await p.evaluate(() => (location.hash = "#/all"));
    await p.waitForSelector(".pane.list .search input", { visible: true });
    await p.type(".pane.list .search input", "chee");
    await p.waitForFunction(
      () => [...document.querySelectorAll(".pane.list .note")].some((b) => b.textContent.includes("Groceries") && b.querySelector(".snippet mark")?.textContent === "cheese"),
      { timeout: 5000 },
    );
  }
  check(true, "search finds a word deep in a note, for its owner and who it's shared with, and marks it");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-search-deep.png") });
  for (const p of [alice, bob]) {
    await p.click(".pane.list .search .clear");
    await p.goBack();
    await p.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("cheese"));
  }
  // Pointing at a line's grip says who wrote it.
  const milk = await alice.evaluate(() => {
    const r = [...document.querySelectorAll(".cm-line")].find((l) => l.textContent.includes("milk")).getBoundingClientRect();
    return { x: r.x, y: r.y, height: r.height };
  });
  await alice.mouse.move(milk.x + 40, milk.y + milk.height / 2);
  await alice.waitForFunction(() => document.querySelector(".cm-drag-grip.shown")?.title.startsWith("You ·"));
  check(true, "a line's grip says who wrote it");
  await alice.mouse.move(5, 5);

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

  // ---- Tables and kanban boards (lib/tables.ts, lib/kanban.ts) ----
  // Alice builds both in a new note; Bob, who can only view it, watches them change live.
  const markdown = (page) => page.evaluate(() => document.querySelector(".cm-content").cmTile.root.view.state.doc.toString());
  await alice.click(".pane.editor header button[aria-label='New note']");
  await alice.waitForFunction(() => document.querySelector(".editor .headerbar .title strong")?.textContent === "New Note" && document.activeElement?.matches(".cm-content[contenteditable=true]"));
  await alice.keyboard.type("Plans\n");
  plansId = await alice.evaluate(() => location.hash.split("/note/")[1]);
  await alice.evaluate((id) => fetch("/api/shares", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ resource_type: "note", resource_id: id, username: "bob", role: "viewer" }) }), plansId);
  await bob.waitForFunction((id) => fetch("/api/tree").then((r) => r.json()).then((t) => t.notes.some((n) => n.id === id)), {}, plansId);
  await bob.evaluate((id) => (location.hash = `#/shared/note/${id}`), plansId);
  await bob.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("Plans"));

  await alice.click("button[aria-label='Insert table']");
  await alice.waitForSelector(".cm-table-cell[data-r='-1'][data-c='0']:focus");
  await alice.keyboard.type("Item");
  await alice.click(".cm-table-cell[data-r='0'][data-c='0']");
  await alice.keyboard.type("Milk");
  await alice.keyboard.press("Tab");
  await alice.keyboard.type("2");
  await alice.waitForFunction(() => document.querySelector(".cm-content").cmTile.root.view.state.doc.toString().includes("| Milk | 2 |  |"));
  check((await markdown(alice)).includes("| Item | Column 2 | Column 3 |\n| --- | --- | --- |\n| Milk | 2 |  |\n"), "typing in table cells writes them into the Markdown table");
  await alice.click(".cm-table-tools button[aria-label='Add row']");
  await alice.keyboard.type("6");
  await alice.waitForFunction(() => document.querySelector(".cm-content").cmTile.root.view.state.doc.toString().includes("|  | 6 |  |"));
  const tableLines = (await markdown(alice)).split("\n").filter((l) => l.startsWith("|"));
  check(tableLines.length === 5 && tableLines[3] === "|  | 6 |  |", `Add row puts a row under the cell being edited (${JSON.stringify(tableLines)})`);
  await bob.waitForFunction(() => document.querySelector(".cm-table-cell[data-r='1'][data-c='1']")?.textContent === "6", { timeout: 5000 });
  check(await bob.evaluate(() => !document.querySelector(".cm-table-widget [contenteditable]") && !document.querySelector(".cm-table-tools")), "a viewer sees the table live, without editing tools");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-table-editing.png") });
  await alice.keyboard.press("Escape");

  await alice.click(".cm-line");
  await alice.keyboard.down("Control");
  await alice.keyboard.press("End");
  await alice.keyboard.up("Control");
  await alice.click("button[aria-label='Insert board']");
  await alice.waitForSelector(".cm-kanban-card.editing .cm-kanban-text:focus");
  await alice.keyboard.type("Bread");
  await alice.keyboard.press("Escape");
  await alice.click(".cm-kanban-col[data-col='1'] .cm-kanban-add");
  await alice.waitForSelector(".cm-kanban-card[data-col='1'].editing .cm-kanban-text:focus");
  await alice.keyboard.type("Butter");
  await alice.keyboard.press("Escape");
  check((await markdown(alice)).includes("```kanban\n## To do\n- Bread\n\n## Doing\n- Butter\n\n## Done\n```"), "a new board takes its first card, and Add card adds one to its column");
  const card = await (await alice.$(".cm-kanban-card[data-col='0'][data-card='0']")).boundingBox();
  const done = await (await alice.$(".cm-kanban-col[data-col='2']")).boundingBox();
  await alice.mouse.move(card.x + card.width / 2, card.y + card.height / 2);
  await alice.mouse.down();
  await alice.mouse.move(done.x + done.width / 2, done.y + done.height / 2, { steps: 12 });
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-board-drag.png") });
  await alice.mouse.up();
  await alice.waitForFunction(() => document.querySelector(".cm-kanban-col[data-col='2'] .cm-kanban-text")?.textContent === "Bread");
  check((await markdown(alice)).includes("## To do\n\n## Doing\n- Butter\n\n## Done\n- Bread\n```"), "dragging a card to another column moves its line there");
  await bob.waitForFunction(() => document.querySelector(".cm-kanban-col[data-col='2'] .cm-kanban-text")?.textContent === "Bread", { timeout: 5000 });
  check(await bob.evaluate(() => !document.querySelector(".cm-kanban-widget [contenteditable]") && !document.querySelector(".cm-kanban-add")), "a viewer sees the card move live, without editing tools");
  for (const p of [alice, bob]) await p.goBack();
  for (const p of [alice, bob]) await p.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("jam"));
  // ---- end of tables and kanban boards ----

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

  // ---- Offline (lib/offline.ts): with the server stopped, saved notes open and take edits, then merge back. ----
  // Carol (phone) has the shared note open too, so edits on both sides have to merge.
  await carol.evaluate(() => [...document.querySelectorAll(".tabbar button")].find((b) => b.textContent.includes("Shared")).click());
  await carol.waitForFunction(() => [...document.querySelectorAll("li button")].some((b) => b.textContent.includes("Groceries")));
  await carol.evaluate(() => [...document.querySelectorAll("li button")].find((b) => b.textContent.includes("Groceries")).click());
  await carol.waitForSelector(".cm-content[contenteditable=true]");
  await alice.waitForFunction(() => !!navigator.serviceWorker?.controller, { timeout: 10000 });
  // Snapshots are saved a moment after the last change.
  await new Promise((r) => setTimeout(r, 1200));
  const stopped = new Promise((r) => server.once("exit", r));
  server.kill();
  await stopped;
  await carol.waitForSelector(".offline-bar", { visible: true, timeout: 5000 });
  await alice.waitForFunction(() => document.querySelector("nav footer")?.textContent.includes("Offline"), { timeout: 5000 });
  check(true, "losing the server shows Offline: a strip on the phone, the sidebar footer on desktop");
  if (process.env.SHOTS) await carol.screenshot({ path: join(process.env.SHOTS, "phone-offline.png") });
  const typeAtEnd = async (page, words) => {
    await page.click(".cm-content");
    await page.keyboard.down("Control");
    await page.keyboard.press("End");
    await page.keyboard.up("Control");
    await page.keyboard.type(words);
  };
  await typeAtEnd(carol, "\ncarol offline");
  // A reload with no server: the app shell comes from the service worker, the account, tree and note from the device.
  await alice.reload();
  await alice.waitForFunction(() => document.querySelector(".cm-content[contenteditable=true]")?.innerText.includes("bread"), { timeout: 10000 });
  check(await alice.evaluate(() => document.querySelector("nav")?.innerText.includes("Groceries")), "with the server stopped, the app reopens with its tree and the saved note, editable");
  await typeAtEnd(alice, "\nalice offline");
  await new Promise((r) => setTimeout(r, 1200));
  // A note made offline gets its id here and waits to be sent.
  await alice.click(".pane.editor header button[aria-label='New note']");
  await alice.waitForSelector(".cm-content[contenteditable=true]");
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.trim() === "");
  await alice.keyboard.type("# Packed offline\nsocks");
  await alice.waitForFunction(() => document.querySelector("nav")?.innerText.includes("Packed offline"), { timeout: 5000 });
  check(true, "a note made offline opens for typing and is listed by its title");
  await new Promise((r) => setTimeout(r, 1200));
  server = spawn(bin, [], { env, stdio: "inherit" });
  await waitForServer();
  // Carol's open note rejoins; Alice's closed one is pushed up in the background; the new note is made with its own id.
  await carol.waitForFunction(() => document.querySelector(".cm-content").innerText.includes("alice offline"), { timeout: 30000 });
  const offlineNote = await alice.evaluate(() => location.hash.match(/note\/([0-9a-f-]{36})/)[1]);
  await alice.waitForFunction(
    async (id) => (await fetch("/api/tree").then((r) => r.json())).notes.some((n) => n.id === id && n.title === "Packed offline"),
    { timeout: 30000, polling: 500 },
    offlineNote,
  );
  await alice.click("nav .note-row ::-p-text(Groceries)");
  await alice.waitForFunction(() => /carol offline[\s\S]*alice offline|alice offline[\s\S]*carol offline/.test(document.querySelector(".cm-content")?.innerText ?? ""), { timeout: 15000 });
  check(true, "back online, edits made offline on both sides merge, and the note made offline reaches the server");
  await alice.evaluate(async (id) => {
    await fetch(`/api/notes/${id}`, { method: "DELETE" });
    await fetch(`/api/trash/note/${id}`, { method: "DELETE" });
  }, offlineNote);
  // ---- end of offline ----

  // Beside the text is still the editor: a drag that starts out in the left margin selects from the line's start.
  const title = await (await alice.$(".cm-line")).boundingBox();
  await alice.mouse.move(title.x - 120, title.y + title.height / 2);
  await alice.mouse.down();
  await alice.mouse.move(title.x + 60, title.y + title.height / 2, { steps: 6 });
  await alice.mouse.up();
  const picked = await alice.evaluate(() => document.getSelection().toString());
  check(picked.length >= 1 && "Groceries".startsWith(picked), `a drag from the margin selects from the line start ("${picked}")`);
  await alice.mouse.click(title.x + title.width + 200, title.y + title.height / 2);

  // Before any AI service is set up, the sidebar's top row is Search: it puts the cursor in the Notes list's search, and nothing offers to ask.
  const noteHash = await alice.evaluate(() => location.hash);
  await alice.click("nav .row ::-p-text(Search)");
  await alice.waitForFunction(() => location.hash === "#/" && document.activeElement === document.querySelector(".pane.list .search input"));
  await alice.keyboard.type("bread");
  await alice.waitForFunction(() => [...document.querySelectorAll(".pane.list .note")].some((c) => c.textContent.includes("Groceries") && c.querySelector(".snippet mark")));
  check(
    await alice.evaluate(() => !document.querySelector(".pane.list .ask") && ![...document.querySelectorAll("nav .row")].some((r) => r.textContent.trim() === "Ask")),
    "without AI services the sidebar's Search row opens the Notes list's search, which finds notes as you type and offers nothing about asking",
  );
  await alice.click(".pane.list .search .clear");
  await alice.evaluate((h) => (location.hash = h), noteHash);
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("bread"));
  // Tidy Up needs the AI chat model.
  await alice.click(".editor header button[aria-label='Note menu']");
  await alice.waitForSelector("[role=menuitem]");
  check(await alice.evaluate(() => ![...document.querySelectorAll("[role=menuitem]")].some((b) => b.textContent.includes("Tidy Up"))), "without AI, the note menu has no Tidy Up");
  await alice.keyboard.press("Escape");
  await alice.waitForFunction(() => !document.querySelector("[role=menuitem]"));

  // Settings: an admin resets Bob's password, and Bob's open app drops to the login screen.
  await alice.click("nav button[aria-label='Settings']");
  await alice.waitForSelector("button[aria-label='Manage Bob']");
  check((await alice.$eval(".version", (el) => el.textContent)) === "Gnotes dev", "Settings shows the server's version");
  await alice.click("button[aria-label='Manage Bob']");
  await alice.evaluate(() => [...document.querySelectorAll("[role=menuitem]")].find((b) => b.textContent.includes("Reset Password")).click());
  await alice.type("#reset-password input", "bobsnewpass");
  await alice.click("dialog button[form=reset-password]");
  await closed(alice);
  await alice.waitForFunction(() => document.body.innerText.includes("password was reset"));
  await bob.waitForFunction(() => document.body.innerText.includes("You were signed out"), { timeout: 10000 });
  check(true, "admin reset signs the user out of their open app");

  // AI services start folded away behind a switch; switching one on opens its form.
  check(
    await alice.evaluate(() => [".stt", ".vision", ".summaries", ".embed"].every((c) => !document.querySelector(`${c} input`))),
    "AI services are folded away while off",
  );
  for (const c of [".stt", ".vision", ".summaries", ".embed"]) await alice.click(`${c} .switch`);
  // The admin points speech-to-text at a service from Settings, tests it and saves it.
  await alice.type(".stt input[type=url]", `http://127.0.0.1:${port + 1}/v1`);
  await alice.type(".stt input[placeholder^='Optional, e.g. ws']", `ws://127.0.0.1:${port + 2}/v1/realtime`);
  await alice.evaluate(() => [...document.querySelectorAll(".stt button")].find((b) => b.textContent === "Test").click());
  await alice.waitForFunction(() => document.querySelector(".stt .status")?.textContent.includes("live transcription works"), { timeout: 8000 });
  await alice.click(".stt button[type=submit]");
  await alice.waitForFunction(() => document.body.innerText.includes("Speech-to-text saved"));
  check(true, "admin sets up speech-to-text in settings");
  await alice.type(".vision input[type=url]", `http://127.0.0.1:${port + 1}/v1`);
  await alice.type(".vision input[placeholder='qwen2.5vl']", "qwen2.5vl");
  await alice.click(".vision button[type=submit]");
  await alice.waitForFunction(() => document.body.innerText.includes("Text from photos saved"));
  check(true, "admin sets up text from photos in settings");
  await alice.type(".summaries input[type=url]", `http://127.0.0.1:${port + 1}/v1`);
  await alice.type(".summaries input[placeholder='llama3.1']", "llama3.1");
  await alice.click(".summaries button[type=submit]");
  await alice.waitForFunction(() => document.body.innerText.includes("Summaries saved"));
  check(true, "admin sets up summaries in settings");
  await alice.type(".embed input[type=url]", `http://127.0.0.1:${port + 1}/v1`);
  await alice.type(".embed input[placeholder='nomic-embed-text']", "nomic-embed-text");
  await alice.click(".embed button[type=submit]");
  await alice.waitForFunction(() => document.body.innerText.includes("Semantic search saved"));
  check(true, "admin sets up semantic search in settings");
  // Switching a running service off folds it away but keeps its settings; switching it back on restores them.
  await alice.click(".vision .switch");
  await alice.waitForFunction(() => document.body.innerText.includes("Text from photos turned off") && !document.querySelector(".vision input"));
  await alice.click(".vision .switch");
  await alice.waitForFunction(() => document.querySelector(".vision .status")?.textContent.includes("On"));
  check(
    await alice.$eval(".vision input[type=url]", (i, url) => i.value === url, `http://127.0.0.1:${port + 1}/v1`),
    "switching a service off and on keeps its settings",
  );
  // Re-index asks first, then starts over.
  await alice.evaluate(() => [...document.querySelectorAll(".embed button")].find((b) => b.textContent === "Re-index").click());
  await alice.waitForSelector("dialog[open]");
  await alice.evaluate(() => [...document.querySelectorAll("dialog[open] button")].find((b) => b.textContent.trim() === "Re-index").click());
  await closed(alice);
  await alice.waitForFunction(() => /Re-indexing \d+ notes?/.test(document.body.innerText));
  check(true, "Re-index embeds every note again");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-settings.png") });
  await alice.click(".settings-layer .back");
  await alice.waitForFunction(() => !document.querySelector(".settings-layer"));
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("bread"));

  // The Summary tab summarizes the note the first time it's opened, and says when the note has moved on.
  await alice.waitForFunction(() => document.querySelector(".tabs button:last-child")?.innerText.trim() === "Summarize");
  check(true, "with no summary yet, the tab offers to Summarize");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-summarize-tab.png") });
  await alice.click(".tabs button:last-child");
  await alice.waitForSelector(".summary-state .spinner");
  await alice.waitForFunction(() => document.querySelector(".summary-page.shown .cm-content")?.innerText.includes("Things to buy this week."), { timeout: 5000 });
  check(
    await alice.evaluate(() => {
      const page = document.querySelector(".summary-page.shown");
      return !!page.querySelector(".cm-checkbox") && page.innerText.includes("Key points") && !page.innerText.includes("##") && !document.querySelector(".format");
    }),
    "opening Summary makes one, drawn like a note, with the format bar put away",
  );
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-summary.png") });
  await alice.click(".tabs ::-p-text(Note)");
  await alice.waitForFunction(() => !document.querySelector(".summary-page.shown") && !!document.querySelector(".format"));
  await alice.reload();
  await alice.waitForFunction(() => document.querySelector(".tabs button:last-child")?.innerText.trim() === "Summary");
  check(true, "once a note has a summary, the tab reads Summary before it's opened");
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("bread"));
  await alice.click(".cm-content");
  await alice.keyboard.down("Control");
  await alice.keyboard.press("End");
  await alice.keyboard.up("Control");
  await alice.keyboard.type(" and jam");
  await new Promise((r) => setTimeout(r, 600));
  await alice.click(".tabs ::-p-text(Summary)");
  await alice.waitForFunction(() => document.querySelector(".summary-state")?.innerText.includes("changed since"), { timeout: 5000 });
  check(true, "reopening Summary after an edit says the note has changed");
  await alice.click(".summary-state ::-p-text(Update)");
  await alice.waitForFunction(() => document.querySelector(".summary-state")?.innerText.includes("Made"), { timeout: 5000 });
  check(true, "Update makes a fresh summary");
  await alice.click(".tabs ::-p-text(Note)");
  await alice.keyboard.press("Backspace");
  for (let i = 0; i < 7; i++) await alice.keyboard.press("Backspace");

  // Tidy Up, from the note menu: a rendered preview, then Apply writes it into the note, and Undo takes it back.
  await new Promise((r) => setTimeout(r, 600));
  const untidy = await markdown(alice);
  await alice.click(".editor header button[aria-label='Note menu']");
  await alice.waitForSelector("[role=menuitem]");
  await alice.evaluate(() => [...document.querySelectorAll("[role=menuitem]")].find((b) => b.textContent.includes("Tidy Up")).click());
  await alice.waitForSelector("dialog .tidy-state .spinner");
  await alice.waitForFunction(() => document.querySelector("dialog .tidy-page .cm-content")?.innerText.includes("To buy"), { timeout: 5000 });
  check(await alice.evaluate(() => !document.querySelector("dialog .tidy-page").innerText.includes("##")), "Tidy Up shows the tidied note, drawn like a note");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-tidy.png") });
  check(await markdown(alice) === untidy, "the note doesn't change until Apply");
  await alice.click("dialog ::-p-text(Apply)");
  await closed(alice);
  const tidied = await markdown(alice);
  check(tidied.includes("\n\n## To buy\n\n") && tidied.split("\n")[0] === untidy.split("\n")[0], `Apply writes the tidied note in (${JSON.stringify(tidied.slice(0, 60))})`);
  await alice.click(".toast ::-p-text(Undo)");
  await alice.waitForFunction((t) => document.querySelector(".cm-content").cmTile.root.view.state.doc.toString() === t, { timeout: 3000 }, untidy);
  check(true, "the toast's Undo puts the note back as it was");
  // The same from the slash menu, by another of its words.
  await alice.click(".cm-content");
  await alice.keyboard.down("Control");
  await alice.keyboard.press("End");
  await alice.keyboard.up("Control");
  await alice.keyboard.type(" /form");
  await alice.waitForFunction(() => document.querySelector(".cm-slash li[aria-selected]")?.textContent === "Tidy Up");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-slash-tidy.png") });
  await new Promise((r) => setTimeout(r, 150));
  await alice.keyboard.press("Enter");
  await alice.waitForFunction(() => document.querySelector("dialog .tidy-page .cm-content")?.innerText.includes("To buy"), { timeout: 5000 });
  check(await markdown(alice) === `${untidy} `, "/format finds Tidy Up in the slash menu, which takes its words out and opens the preview");
  await alice.click("dialog ::-p-text(Cancel)");
  await closed(alice);
  check(await markdown(alice) === `${untidy} `, "Cancel leaves the note as it was");
  await alice.click(".cm-content");
  await alice.keyboard.down("Control");
  await alice.keyboard.press("End");
  await alice.keyboard.up("Control");
  await alice.keyboard.press("Backspace");

  // Finding by meaning is Ask's: "dairy" isn't in Groceries, but milk and cheese are. The list's search is words only, and offers Ask.
  await alice.evaluate(() => document.activeElement?.blur());
  // Notes are embedded a few seconds after typing stops.
  await alice.waitForFunction(async () => (await fetch("/api/search/meaning?q=dairy").then((r) => r.json())).results.length > 0, { timeout: 40000, polling: 500 });
  await alice.evaluate(() => (location.hash = "#/all"));
  await alice.waitForSelector(".pane.list .search input", { visible: true });
  await alice.type(".pane.list .search input", "dairy");
  await alice.waitForSelector(".pane.list .status .pill");
  // Give the word search time to answer, too.
  await new Promise((r) => setTimeout(r, 600));
  check(
    await alice.evaluate(() => !document.querySelector(".pane.list .note") && !document.querySelector(".pane.list .meaning") && document.querySelector(".pane.list .status").textContent.includes("No Results")),
    "a list's search finds notes by their words only, and offers Ask when none match",
  );
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-search-ask.png") });
  await alice.click(".pane.list .status .pill");
  // The row opens the Ask screen and asks it there. The notes found show as cards first; the answer streams in under the question after.
  await alice.waitForSelector(".pane.list .turn .card", { timeout: 5000 });
  check(
    await alice.evaluate(
      () =>
        location.hash === "#/ask" &&
        document.querySelector(".pane.list .question").textContent === "dairy" &&
        !document.querySelector(".pane.list .answer") &&
        document.querySelector(".pane.list .card").textContent.includes("Groceries") &&
        !!document.querySelector(".pane.list .card .snippet") &&
        document.querySelector("nav .row.selected")?.textContent.trim() === "Ask",
    ),
    "a list's Ask Your Notes row opens the Ask screen, lit in the sidebar, which shows the notes it found as cards before the answer",
  );
  await alice.waitForSelector(".pane.list .turn[aria-busy=true] .answer", { timeout: 5000 });
  check(true, "the answer shows while it's still being written");
  await alice.waitForSelector(".pane.list .turn[aria-busy=false] .answer", { timeout: 5000 });
  check(await alice.evaluate(() => document.querySelector(".pane.list .answer").textContent.includes("milk and cheese on your list")), "the answer comes from the notes found");
  check(await alice.evaluate(() => !!document.querySelector(".pane.list .answer .cite")), "Ask streams an answer from your notes with a numbered link");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-ask.png") });
  // A follow-up builds on the question before; both stay in the conversation.
  await alice.type(".pane.list .composer input", "and the cheese?");
  await alice.keyboard.press("Enter");
  await alice.waitForSelector(".pane.list .turn:nth-child(2)[aria-busy=false]", { timeout: 8000 });
  check(
    await alice.evaluate(() => {
      const [first, second] = document.querySelectorAll(".pane.list .turn");
      const cards = second.querySelectorAll(".card");
      return (
        first.querySelector(".answer").textContent.includes("milk and cheese") &&
        second.querySelector(".question").textContent === "and the cheese?" &&
        second.querySelector(".answer").textContent.includes("Cheese is on your list too") &&
        // Once answered, only the note it cites is left, with the word asked about marked.
        cards.length === 1 &&
        cards[0].textContent.includes("Groceries") &&
        cards[0].querySelector(".snippet mark")?.textContent === "cheese"
      );
    }),
    "a follow-up is answered from the notes the conversation is about, under the first answer",
  );
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-ask-follow-up.png") });
  await alice.click(".pane.list .turn:last-child .card");
  await alice.waitForFunction(() => location.hash.startsWith("#/ask/note/") && document.querySelector(".cm-content")?.innerText.includes("cheese"));
  check(true, "an Ask card opens its note");
  // Later checks start from the note opened from Recent, as before.
  const askedNote = await alice.evaluate(() => location.hash.replace("#/ask/", "#/all/"));
  // The sidebar's Ask row, at the top, opens the same screen in the main pane, the conversation still there.
  check(await alice.evaluate(() => document.querySelector("nav .scroll .row")?.textContent.trim() === "Ask"), "with AI set up, the sidebar's top row is Ask");
  await alice.click("nav .row ::-p-text(Ask)");
  await alice.waitForFunction(() => location.hash === "#/ask" && document.querySelectorAll(".pane.list .turn").length === 2);
  check(true, "the sidebar's Ask row opens the Ask screen with the conversation kept");
  // New Chat starts over; typing a question lists nothing until it's sent.
  await alice.click(".pane.list .new-chat");
  await alice.waitForFunction(() => !document.querySelector(".pane.list .turn") && document.querySelector(".pane.list .composer input").placeholder === "Ask your notes…");
  await alice.type(".pane.list .composer input", "dairy");
  await new Promise((r) => setTimeout(r, 600));
  check(await alice.evaluate(() => !document.querySelector(".pane.list .card")), "New Chat starts over, and Ask lists nothing until a question is sent");
  await alice.evaluate(() => {
    const input = document.querySelector(".pane.list .composer input");
    input.value = "";
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await alice.evaluate((h) => (location.hash = h), askedNote);
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("cheese"));

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
  check(rows[0] === "Ask" && rows[1] === "Recent" && rows.at(-1) === "Trash", "the sidebar starts with Ask and Recent and ends with Trash, after the folders");
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
  // Nested rows carry a colored line per level, and Collapse All folds every notebook under the heading.
  check(await alice.evaluate(() => [...document.querySelectorAll("nav .row")].find((r) => r.textContent.trim() === "Lisbon")?.querySelectorAll(".guide").length === 2), "a row two levels down shows two level lines");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-sidebar-guides.png") });
  await alice.click("nav button[aria-label='Collapse all in Notes']");
  await alice.waitForFunction(() => ![...document.querySelectorAll("nav .row .guide")].length);
  check(await alice.evaluate(() => !document.querySelector("nav button[aria-label='Collapse all in Notes']")), "Collapse All folds every notebook, then steps aside");
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
  // A code block without a language gets colors from a best guess; its picker writes the language after the fence.
  await alice.click(".list header button[aria-label='New note']");
  await alice.waitForSelector(".cm-content[contenteditable=true]");
  await alice.keyboard.type("Snippets\n```\nconst greet = (name) => name;\n```\nend");
  await alice.evaluate(() => document.activeElement?.blur());
  await alice.waitForFunction(() => document.querySelector(".cm-code .tok-keyword")?.textContent === "const", { timeout: 5000 });
  check(
    await alice.$eval(".cm-code-lang select", (s) => s.options[s.selectedIndex].text === "JavaScript (guess)" && !document.querySelector(".cm-content").innerText.includes("```")),
    "an unlabeled code block is colored as its guessed language, with the fences hidden",
  );
  await alice.select(".cm-code-lang select", "python");
  await alice.waitForFunction(() => document.querySelector(".cm-code-lang select")?.value === "python");
  // The picker is drawn from the text, so a new selection that sticks means the fence changed.
  check(await alice.$eval(".cm-code-lang select", (s) => s.options[s.selectedIndex].text === "Python"), "the language picker sets the block's language");
  await alice.select(".cm-code-lang select", "lua");
  await alice.waitForFunction(() => document.querySelector(".cm-code-lang select")?.value === "lua");
  check(await alice.$eval(".cm-code-lang select", (s) => s.options[s.selectedIndex].text === "Lua"), "the picker offers Lua");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-code-block.png") });
  // "/" opens a menu of styles and things to add; typing narrows it and Enter picks.
  await alice.click(".cm-content");
  await alice.keyboard.down("Control");
  await alice.keyboard.press("End");
  await alice.keyboard.up("Control");
  await alice.keyboard.type("\n/");
  await alice.waitForSelector(".cm-slash li");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-slash-menu.png") });
  await alice.keyboard.type("subh");
  await alice.waitForFunction(() => document.querySelector(".cm-slash li[aria-selected]")?.textContent === "Subheading");
  // The menu ignores Enter for a moment after it changes, so a fast typist doesn't pick by accident.
  await new Promise((r) => setTimeout(r, 150));
  await alice.keyboard.press("Enter");
  await alice.keyboard.type("Later");
  await alice.waitForFunction(() => [...document.querySelectorAll(".cm-line.cm-h3")].some((l) => l.textContent.includes("Later")));
  check(await alice.evaluate(() => !document.querySelector(".cm-slash") && !document.querySelector(".cm-content").innerText.includes("/")), "the slash menu turns the line into a subheading and takes the slash with it");
  await alice.keyboard.type(" /code");
  await alice.waitForFunction(() => document.querySelector(".cm-slash li[aria-selected]")?.textContent === "Code Block");
  await new Promise((r) => setTimeout(r, 150));
  await alice.keyboard.press("Enter");
  await alice.keyboard.type("SELECT 1 FROM t;");
  await alice.evaluate(() => document.activeElement?.blur());
  await alice.waitForFunction(() => document.querySelectorAll(".cm-code-lang").length === 2);
  check(await alice.$$eval(".cm-code-lang select", (s) => s[1].options[s[1].selectedIndex].text === "SQL (guess)"), "the slash menu adds a code block under the line, ready to type into");
  const sql = await alice.evaluateHandle(() => [...document.querySelectorAll(".cm-code")].find((l) => l.textContent.includes("SELECT")));
  await sql.click();
  await alice.keyboard.press("End");
  await alice.keyboard.type(" /");
  await new Promise((r) => setTimeout(r, 300));
  check(!(await alice.$(".cm-slash")), "a slash inside code is just a slash");
  for (let i = 0; i < 2; i++) await alice.keyboard.press("Backspace");
  for (const [word, widget] of [["table", ".cm-table-widget"], ["board", ".cm-kanban-widget"]]) {
    // Back into the note's text from wherever the last insert left the cursor.
    await alice.evaluate(() => document.activeElement?.blur());
    await (await alice.evaluateHandle(() => [...document.querySelectorAll(".cm-line")].find((l) => l.textContent === "end"))).click();
    await alice.keyboard.down("Control");
    await alice.keyboard.press("End");
    await alice.keyboard.up("Control");
    await alice.keyboard.type(`\n/${word}`);
    await alice.waitForFunction((w) => document.querySelector(".cm-slash li[aria-selected]")?.textContent.toLowerCase() === w, {}, word);
    await new Promise((r) => setTimeout(r, 150));
    await alice.keyboard.press("Enter");
    await alice.waitForSelector(widget, { timeout: 5000 });
  }
  check(true, "the slash menu adds a table and a board");
  // Emoji by name: ":tac" opens a list, and a whole ":thumbsup:" turns into the emoji as you type it.
  await alice.evaluate(() => document.activeElement?.blur());
  await (await alice.evaluateHandle(() => [...document.querySelectorAll(".cm-line")].find((l) => l.textContent === "end"))).click();
  await alice.keyboard.press("End");
  await alice.keyboard.type(" :tac");
  await alice.waitForFunction(() => document.querySelector(".cm-slash li[aria-selected]")?.textContent.includes(":taco:"), { timeout: 5000 });
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-emoji.png") });
  await new Promise((r) => setTimeout(r, 150));
  await alice.keyboard.press("Enter");
  await alice.keyboard.type(" :thumbsup: at 10:30");
  await alice.waitForFunction(() => [...document.querySelectorAll(".cm-line")].some((l) => l.textContent === "end 🌮 👍 at 10:30"), { timeout: 3000 });
  check(true, "emoji come from :name, and a whole :name: turns into one, leaving times alone");
  // Pointing at a block shows a grip beside it; dragging the grip moves the block, nested items and all.
  await alice.keyboard.down("Control");
  await alice.keyboard.press("End");
  await alice.keyboard.up("Control");
  // Lists continue on Enter; two spaces before the marker nest an item.
  await alice.keyboard.type("\n- one\ntwo\n");
  await alice.keyboard.press("Home");
  await alice.keyboard.type("  ");
  await alice.keyboard.press("End");
  // Enter on an empty nested item steps back out a level.
  await alice.keyboard.type("two child\n\nthree");
  await alice.evaluate(() => document.activeElement?.blur());
  const lineBox = (t) => alice.evaluate((t) => {
    const r = [...document.querySelectorAll(".cm-line")].find((l) => l.textContent.replace("•", "").trim() === t).getBoundingClientRect();
    return { x: r.x, y: r.y, w: r.width, h: r.height };
  }, t);
  const two = await lineBox("two");
  await alice.mouse.move(two.x + 60, two.y + two.h / 2);
  await alice.waitForSelector(".cm-drag-grip.shown");
  const grip = await (await alice.$(".cm-drag-grip")).boundingBox();
  check(Math.abs(grip.y + grip.height / 2 - (two.y + two.h / 2)) < 4 && grip.x + grip.width <= two.x + 30, "pointing at a list item shows a grip beside it");
  check(await alice.$eval(".cm-drag-grip.shown", (g) => g.title === "Drag to move"), "a note only you edit doesn't say who wrote each line");
  const one = await lineBox("one");
  // Edits less than a second apart undo together, so let the typing settle into its own step.
  await new Promise((r) => setTimeout(r, 1100));
  await alice.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
  await alice.mouse.down();
  for (let i = 1; i <= 8; i++) await alice.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2 + ((one.y + 2 - grip.y - grip.height / 2) * i) / 8);
  await alice.waitForSelector(".cm-drop-marker.shown");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-drag-block.png") });
  await alice.mouse.up();
  const order = await alice.$$eval(".cm-line", (ls) => ls.map((l) => l.textContent.replace("•", "").trim()).filter((t) => ["one", "two", "two child", "three"].includes(t)).join(","));
  check(order === "two,two child,one,three", `dragging a list item's grip moves it with its nested item (${order})`);
  await alice.click(".cm-content");
  await alice.keyboard.down("Control");
  await alice.keyboard.press("z");
  await alice.keyboard.up("Control");
  await alice.evaluate(() => document.activeElement?.blur());
  const undone = await alice.$$eval(".cm-line", (ls) => ls.map((l) => l.textContent.replace("•", "").trim()).filter((t) => ["one", "two", "two child", "three"].includes(t)).join(","));
  check(undone === "one,two,two child,three", `one Ctrl+Z puts a dragged block back (${undone})`);
  // The note covers the folder's page; Back returns to it.
  await alice.goBack();
  await alice.waitForSelector(".list header button[aria-label='New note']", { visible: true });
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

  // Trash: a deleted note waits in the Trash screen and Restore puts it back where it was.
  await alice.click(".editor header button[aria-label='Note menu']");
  await alice.waitForFunction(() => [...document.querySelectorAll("[role=menuitem]")].some((b) => b.textContent.includes("Move to Trash")));
  await alice.evaluate(() => [...document.querySelectorAll("[role=menuitem]")].find((b) => b.textContent.includes("Move to Trash")).click());
  await alice.waitForFunction(() => [...document.querySelectorAll(".toast")].some((t) => t.textContent.includes("moved to trash")));
  await alice.evaluate(() => [...document.querySelectorAll("nav .row")].find((b) => b.textContent.trim() === "Trash").click());
  await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Trash" && document.querySelector(".boxed-list .row")?.textContent.includes("Pantry"));
  check(await alice.evaluate(() => /Deleted .+ · 30 days left/.test(document.querySelector(".boxed-list .row small").textContent)), "the Trash screen lists a deleted note with when it goes for good");
  await alice.evaluate(() => [...document.querySelectorAll(".boxed-list .row")].find((r) => r.textContent.includes("Pantry")).querySelector("button").click());
  await alice.waitForFunction(() => [...document.querySelectorAll(".toast")].some((t) => t.textContent.includes("restored")) && !document.querySelector(".boxed-list"));
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-trash-empty.png") });
  await alice.click(".toast ::-p-text(Show)");
  await alice.waitForFunction(() => document.querySelector(".notebook-chip")?.textContent.trim() === "Home › Garage");
  check(true, "Restore on the Trash screen puts the note back in its notebook");

  // Delete Forever takes one item out of the trash for good; Empty Trash takes the rest. Both ask first.
  await alice.evaluate(async () => {
    const post = (path) => fetch(`/api${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: "{}" }).then((r) => r.json());
    for (let i = 0; i < 3; i++) await fetch(`/api/notes/${(await post("/notes")).id}`, { method: "DELETE" });
  });
  await alice.evaluate(() => [...document.querySelectorAll("nav .row")].find((b) => b.textContent.trim() === "Trash").click());
  await alice.waitForFunction(() => document.querySelectorAll(".boxed-list .row").length === 3);
  await alice.click(".boxed-list .row button[aria-label$='forever']");
  await alice.waitForSelector("dialog .destructive-action");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-trash-delete-forever.png") });
  await alice.click("dialog ::-p-text(Cancel)");
  await closed(alice);
  check((await alice.$$(".boxed-list .row")).length === 3, "Cancel keeps the item");
  await alice.click(".boxed-list .row button[aria-label$='forever']");
  await alice.click("dialog .destructive-action");
  await closed(alice);
  await alice.waitForFunction(() => document.querySelectorAll(".boxed-list .row").length === 2);
  check((await (await fetch(`${base}/api/trash`, { headers: { cookie: (await alice.cookies()).map((c) => `${c.name}=${c.value}`).join("; ") } })).json()).length === 2, "Delete Forever removes one item from the trash for good");
  await alice.click("button.empty");
  await alice.click("dialog .destructive-action");
  await closed(alice);
  await alice.waitForFunction(() => document.body.innerText.includes("Trash Is Empty") && !document.querySelector("button.empty"));
  check(true, "Empty Trash deletes the rest and shows the empty state");

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
  // A photo-sized picture of a shopping list, drawn in the page.
  const png = join(data, "list.png");
  const listPng = await alice.evaluate(() => {
    const c = Object.assign(document.createElement("canvas"), { width: 320, height: 200 });
    const g = c.getContext("2d");
    g.fillStyle = "#f6f1e3";
    g.fillRect(0, 0, 320, 200);
    g.fillStyle = "#333";
    g.font = "bold 26px sans-serif";
    g.fillText("SHOPPING LIST", 24, 70);
    g.font = "22px sans-serif";
    g.fillText("Oat milk", 24, 120);
    return c.toDataURL("image/png").split(",")[1];
  });
  writeFileSync(png, Buffer.from(listPng, "base64"));
  const [chooser] = await Promise.all([alice.waitForFileChooser(), alice.click("button[aria-label='Add photo']")]);
  await chooser.accept([png]);
  await alice.waitForFunction(() => document.querySelector(".cm-attachment img")?.naturalWidth === 320, { timeout: 5000 });
  check(true, "a picked photo uploads and shows in the note");
  // Reading its text is on demand: nothing happens until Get Text, shown while pointing at the photo.
  await new Promise((r) => setTimeout(r, 1200));
  check(await alice.evaluate(() => !document.querySelector(".cm-content").innerText.includes("SHOPPING LIST") && !document.querySelector(".cm-attachment.reading")), "a photo isn't read until asked");
  await alice.hover(".cm-attachment img");
  await alice.waitForFunction(() => getComputedStyle(document.querySelector(".is-image .cm-att-tools")).opacity === "1");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-photo-get-text.png") });
  check(
    await alice.evaluate(() => {
      const a = document.querySelector(".is-image .cm-att-tools .tool-download");
      return a?.getAttribute("href")?.startsWith("/api/attachments/") && a.getAttribute("download") === "list.png";
    }),
    "a photo's tools offer a download of the original file",
  );
  await alice.click(".is-image .cm-att-tools .tool-read");
  await alice.waitForSelector(".cm-attachment.reading .cm-reading", { timeout: 5000 });
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-photo-reading.png") });
  await alice.waitForFunction(() => document.querySelector(".cm-content").innerText.includes("SHOPPING LIST"), { timeout: 5000 });
  check(
    await alice.evaluate(() => {
      const lines = [...document.querySelectorAll(".cm-line")];
      const at = lines.findIndex((l) => l.querySelector(".cm-attachment img"));
      return lines[at + 1]?.textContent === "SHOPPING LIST" && lines[at + 2]?.textContent === "Oat milk" && !document.querySelector(".cm-attachment.reading");
    }),
    "Get Text writes the photo's text under it",
  );
  // Delete takes the photo out of the note, keeping the text under it, and Undo puts it back.
  await alice.hover(".cm-attachment img");
  await alice.click(".is-image .cm-att-tools .tool-delete");
  await alice.waitForFunction(() => !document.querySelector(".cm-attachment.is-image") && document.querySelector(".toast")?.innerText.includes("Photo removed"));
  check(await alice.evaluate(() => document.querySelector(".cm-content").innerText.includes("SHOPPING LIST")), "deleting a photo removes it and keeps its text");
  await alice.click(".toast ::-p-text(Undo)");
  await alice.waitForFunction(() => document.querySelector(".cm-attachment.is-image img")?.naturalWidth === 320);
  check(true, "Undo brings the photo back");

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
  // The bar's input menu lists the browser's inputs and offers meeting audio; a pick is remembered.
  await alice.click(".recorder button[aria-label='Audio input']");
  await alice.waitForSelector(".popover [role=menuitemradio]");
  const inputs = await alice.$$eval(".popover [role=menuitemradio]", (els) => els.map((e) => [e.textContent.trim(), e.getAttribute("aria-checked")]));
  const meetingOffered = await alice.evaluate(() => [...document.querySelectorAll(".popover [role=menuitem]")].some((e) => e.textContent.includes("Add Meeting Audio")));
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-recording-inputs.png") });
  check(inputs.length >= 2 && inputs.filter(([, on]) => on === "true").length === 1 && meetingOffered, `the input menu lists inputs with one checked, and meeting audio (${JSON.stringify(inputs)})`);
  const other = inputs.find(([, on]) => on !== "true")[0];
  const pick = await alice.evaluateHandle((label) => [...document.querySelectorAll(".popover [role=menuitemradio]")].find((e) => e.textContent.trim() === label), other);
  await pick.click();
  await alice.waitForFunction(() => !!localStorage.getItem("gnotes.mic") && !document.querySelector(".popover"));
  await alice.click(".recorder button[aria-label='Audio input']");
  await alice.waitForSelector(".popover [role=menuitemradio]");
  check(
    await alice.evaluate((label) => [...document.querySelectorAll(".popover [role=menuitemradio]")].find((e) => e.getAttribute("aria-checked") === "true")?.textContent.trim() === label, other),
    "switching input mid-recording takes effect and is remembered",
  );
  await alice.keyboard.press("Escape");
  await alice.waitForFunction(() => !document.querySelector(".popover"));
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
  // A memo's player has a speed button, and its tools can transcribe the whole recording again.
  await alice.click(".cm-audio-speed");
  check(await alice.evaluate(() => document.querySelector(".cm-audio-speed").textContent === "1.5×"), "the memo's speed button steps to 1.5×");
  await alice.click(".cm-audio-speed");
  await alice.click(".cm-audio-speed");
  await alice.hover(".cm-audio");
  if (process.env.SHOTS) await (await new Promise((r) => setTimeout(r, 250)), alice.screenshot({ path: join(process.env.SHOTS, "desktop-memo-tools.png") }));
  await alice.click(".is-audio .cm-att-tools .tool-transcribe");
  await alice.waitForFunction(() => document.querySelector(".cm-content").innerText.includes("remember the milk"), { timeout: 5000 });
  check(
    await alice.evaluate(() => {
      const lines = [...document.querySelectorAll(".cm-line")];
      const at = lines.findIndex((l) => l.querySelector(".cm-audio"));
      return lines[at + 1]?.textContent === "remember the milk";
    }),
    "Transcribe adds the recording's transcript under the player",
  );
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
    await alice.keyboard.type("\n/");
    await alice.waitForSelector(".cm-slash li");
    await shot("phone-slash-menu");
    await alice.keyboard.press("Escape");
    for (let i = 0; i < 2; i++) await alice.keyboard.press("Backspace");
    // On a touch screen the grip sits beside the line with the cursor.
    await alice.waitForSelector(".cm-drag-grip.shown");
    check(true, "on a phone the line being typed on shows its grip");
    await shot("phone-drag-grip");
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
    // The top of Notes carries the app's icon and name, in the middle of the headerbar even with New Notebook on one side.
    const brand = await alice.evaluate(() => {
      const bar = document.querySelector(".pane.list .headerbar").getBoundingClientRect();
      const title = document.querySelector(".pane.list .headerbar .brand");
      const box = title?.getBoundingClientRect();
      return { text: title?.textContent.trim(), icon: !!title?.querySelector("img[src='/icon.svg']")?.naturalWidth, off: box ? box.left + box.width / 2 - (bar.left + bar.width / 2) : NaN };
    });
    check(brand.text === "Gnotes" && brand.icon && Math.abs(brand.off) <= 2, `the phone's Notes headerbar centers the app icon and name (${brand.off.toFixed(1)}px off)`);
    await shot("phone-home-dark");
    await alice.type(".search input", "pantry");
    await alice.waitForFunction(() => [...document.querySelectorAll(".note .where")].some((w) => w.textContent.includes("Home › Garage")));
    check(true, "searching from the top finds notes anywhere and says where they are");
    await shot("phone-search-dark");
    // The Notes tab drops the search and goes back to the plain list.
    await alice.evaluate(() => [...document.querySelectorAll(".tabbar button")].find((b) => b.textContent.includes("Notes")).click());
    await alice.waitForFunction(() => document.querySelector(".search input")?.value === "" && !!document.querySelector(".pane.list .folder-row"));
    check(true, "the Notes tab clears a search");
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
    // A note with a photo and a memo: on a touch screen their tools are always there.
    await alice.evaluate(async () => {
      const tree = await fetch("/api/tree").then((r) => r.json());
      location.hash = `#/all/note/${tree.notes.find((n) => n.title === "Pantry").id}`;
    });
    await alice.waitForSelector(".cm-attachment.is-image img");
    await alice.evaluate(() => document.activeElement?.blur());
    // A memo player and a photo fit the line, so the note never pans sideways.
    const sideways = () => alice.evaluate(() => [document.scrollingElement, document.querySelector(".pane.editor .scroll")].every((el) => el.scrollWidth <= el.clientWidth));
    check(await sideways(), "a note with a memo and a photo doesn't scroll sideways on a phone");
    await shot("phone-attachments-dark");
    // The Summary tab on a phone, on a note with plenty in it.
    await alice.tap(".tabs button:last-child");
    await alice.waitForFunction(() => document.querySelector(".summary-page.shown .cm-content")?.innerText.includes("Things to buy"), { timeout: 5000 });
    await shot("phone-summary-dark");
    await alice.tap(".tabs button:first-child");
    // Tidy Up on a phone: the note menu's sheet, then the preview as a bottom sheet with Apply in reach.
    await alice.tap(".editor header button[aria-label='Note menu']");
    await alice.waitForSelector("[role=menuitem]");
    await shot("phone-note-menu-dark");
    await alice.evaluate(() => [...document.querySelectorAll("[role=menuitem]")].find((b) => b.textContent.includes("Tidy Up")).click());
    await alice.waitForFunction(() => document.querySelector("dialog .tidy-page .cm-content")?.innerText.includes("To buy"), { timeout: 5000 });
    await shot("phone-tidy-dark");
    check(
      await alice.evaluate(() => {
        const apply = [...document.querySelectorAll("dialog button")].find((b) => b.textContent.trim() === "Apply").getBoundingClientRect();
        return apply.bottom <= innerHeight && apply.top > innerHeight / 2;
      }),
      "on a phone, Tidy Up's preview is a bottom sheet with Apply in thumb reach",
    );
    await alice.evaluate(() => [...document.querySelectorAll("dialog button")].find((b) => b.textContent.trim() === "Cancel").click());
    await closed(alice);
    // The tab bar is Notes, Recent, +, Ask, Shared; Account is your avatar at the top right of each tab.
    check(
      await alice.evaluate(() => [...document.querySelectorAll(".tabbar > *")].map((t) => t.textContent.trim() || "+").join(",") === "Notes,Recent,+,Ask,Shared"),
      "the phone tab bar is Notes, Recent, +, Ask, Shared",
    );
    await alice.evaluate(() => [...document.querySelectorAll(".tabbar button")].find((b) => b.textContent.includes("Recent")).click());
    await alice.waitForFunction(() => document.querySelector(".pane.list .hero h1")?.textContent === "Recent");
    const avatar = await alice.evaluate(() => {
      const bar = document.querySelector(".pane.list .headerbar").getBoundingClientRect();
      const button = document.querySelector(".pane.list .headerbar .account").getBoundingClientRect();
      const face = document.querySelector(".pane.list .headerbar .account .avatar").getBoundingClientRect();
      const touch = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--icon-touch"));
      return { right: Math.round(bar.right - button.right), size: button.height, face: face.width, touch };
    });
    check(avatar.right <= 8 && avatar.size >= 44 && avatar.face === avatar.touch, `the avatar sits at the top right, touch-sized, its face icon-sized (${JSON.stringify(avatar)})`);
    await alice.$eval(".pane.list .headerbar .account", (b) => b.click());
    await alice.waitForFunction(() => document.querySelector(".tab-page h1")?.textContent === "Account");
    check(await alice.evaluate(() => !!document.querySelector(".tabbar") && !document.querySelector(".tabbar .tab.on")), "the avatar opens Account, with the tab bar still there and no tab lit");
    await shot("phone-account-dark");
    check(await alice.evaluate(() => ![...document.querySelectorAll(".tab-page li")].some((li) => li.textContent.includes("Trash"))), "Account no longer has a Trash entry");
    await alice.$eval(".tab-page .back-icon", (b) => b.click());
    await alice.waitForFunction(() => !document.querySelector(".tab-page") && document.querySelector(".pane.list .hero h1")?.textContent === "Recent");
    check(true, "Account's back chevron returns to the tab it was opened from");
    await alice.$eval(".pane.list .headerbar .account", (b) => b.click());
    await alice.waitForFunction(() => document.querySelector(".tab-page h1")?.textContent === "Account");
    await alice.evaluate(() => [...document.querySelectorAll(".tabbar button")].find((b) => b.textContent.includes("Shared")).click());
    await alice.waitForFunction(() => !document.querySelector(".tab-page") && location.hash === "#/shared");
    check(true, "another tab leaves Account");
    // Trash is the last row of the Notes tab, as on the desktop sidebar; it keeps Notes lit and Back returns there.
    await alice.evaluate(() => [...document.querySelectorAll(".tabbar button")].find((b) => b.textContent.includes("Notes")).click());
    await alice.waitForFunction(() => document.querySelector(".pane.list .hero h1")?.textContent === "Notes");
    check(await alice.evaluate(() => document.querySelector(".pane.list .scroll > :last-child")?.textContent.trim() === "Trash"), "Trash is the last row on the phone's Notes screen");
    await alice.click(".pane.list .trash-link button");
    await alice.waitForFunction(() => document.querySelector(".pane.list .hero h1")?.textContent === "Trash");
    check(await alice.evaluate(() => document.querySelector(".tabbar .tab.on")?.textContent.includes("Notes")), "Trash opens from the Notes tab and keeps it lit");
    await shot("phone-trash-dark");
    await alice.click(".pane.list .back-icon");
    await alice.waitForFunction(() => document.querySelector(".pane.list .hero h1")?.textContent === "Notes" && location.hash === "#/");
    check(true, "Back from Trash returns to Notes");

    // Ask, a tab of its own: sending chats about your notes, follow-ups continue it, and Back leaves.
    await alice.evaluate(() => [...document.querySelectorAll(".tabbar button")].find((b) => b.textContent.includes("Ask")).click());
    await alice.waitForFunction(() => location.hash === "#/ask" && document.querySelector(".tabbar .tab.on")?.textContent.includes("Ask") && !!document.querySelector(".pane.list .headerbar .account"));
    check(await alice.evaluate(() => document.querySelector(".pane.list .status")?.textContent.includes("Ask Your Notes")), "the Ask tab opens on a plain page with its field");
    await shot("phone-ask-dark");
    await alice.tap(".pane.list .composer input");
    await alice.keyboard.type("cheese");
    check(await alice.evaluate(() => !document.querySelector(".tabbar") && !document.querySelector(".pane.list .card")), "typing a question in Ask, the tab bar steps aside for the keyboard");
    await shot("phone-ask-typing-dark");
    await alice.keyboard.press("Enter");
    await alice.waitForSelector(".pane.list .turn[aria-busy=false] .answer", { timeout: 8000 });
    await alice.tap(".pane.list .composer input");
    await alice.keyboard.type("and the cheese?");
    await alice.keyboard.press("Enter");
    await alice.waitForSelector(".pane.list .turn:nth-child(2)[aria-busy=false] .answer", { timeout: 8000 });
    await new Promise((r) => setTimeout(r, 500));
    const pinned = await alice.evaluate(() => {
      const field = document.querySelector(".pane.list .composer").getBoundingClientRect();
      const tabs = document.querySelector(".tabbar")?.getBoundingClientRect();
      const scroll = document.querySelector(".pane.list .scroll");
      return { gap: tabs ? Math.round(tabs.top - field.bottom) : NaN, scrolls: getComputedStyle(scroll).overflowY === "auto" && !scroll.contains(document.querySelector(".pane.list .composer")), turns: document.querySelectorAll(".pane.list .turn").length };
    });
    check(pinned.turns === 2 && pinned.gap >= 0 && pinned.gap <= 2 && pinned.scrolls, `sending chats and a follow-up continues it, the chat scrolling over a field pinned above the tab bar (${JSON.stringify(pinned)})`);
    await shot("phone-ask-chat-dark");
    await alice.emulateMediaFeatures([]);
    await shot("phone-ask-chat");
    await alice.emulateMediaFeatures([{ name: "prefers-color-scheme", value: "dark" }]);
    await alice.goBack();
    await alice.waitForFunction(() => location.hash === "#/" && document.querySelector(".pane.list .hero h1")?.textContent === "Notes");
    check(true, "the back button leaves Ask");

    // ---- Tables and kanban boards on a phone ----
    await alice.evaluate((id) => (location.hash = `#/all/note/${id}`), plansId);
    await alice.waitForSelector(".cm-kanban-card");
    await shot("phone-table-board-dark");
    check(await sideways(), "a wide board scrolls inside itself, not the page");
    // A finger holds a card still for a moment, then drags it to another column.
    const butter = await (await alice.$(".cm-kanban-col[data-col='1'] .cm-kanban-card")).boundingBox();
    const todo = await (await alice.$(".cm-kanban-col[data-col='0']")).boundingBox();
    await alice.touchscreen.touchStart(butter.x + 30, butter.y + butter.height / 2);
    await new Promise((r) => setTimeout(r, 500));
    for (let i = 1; i <= 8; i++) await alice.touchscreen.touchMove(butter.x + 30 + ((todo.x + 40 - butter.x - 30) * i) / 8, butter.y + butter.height / 2);
    await shot("phone-board-drag-dark");
    await alice.touchscreen.touchEnd();
    await alice.waitForFunction(() => document.querySelector(".cm-kanban-col[data-col='0'] .cm-kanban-text")?.textContent === "Butter");
    check((await markdown(alice)).includes("## To do\n- Butter\n\n## Doing\n\n## Done\n- Bread"), "a long press drags a card to another column on a phone");
    await alice.emulateMediaFeatures([]);
    await shot("phone-table-board");
    await alice.tap(".cm-table-cell[data-r='0'][data-c='0']");
    await shot("phone-table-editing");
    await alice.evaluate(() => document.activeElement?.blur());
    await alice.evaluate(() => document.querySelector(".cm-kanban").scrollBy(1000, 0));
    await shot("phone-board-scrolled");
    await alice.setViewport({ width: 1280, height: 800 });
    await alice.waitForSelector(".cm-kanban-card");
    await shot("desktop-table-board");
    await alice.emulateMediaFeatures([{ name: "prefers-color-scheme", value: "dark" }]);
    await shot("desktop-table-board-dark");
  }
  console.log("all checks passed");
} finally {
  await browser?.close();
  server.kill();
  whisper.close();
  realtime.close();
  rmSync(data, { recursive: true, force: true });
}
