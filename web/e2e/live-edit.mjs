// Two browsers, two accounts, one note: sharing, live edits and remote cursors.
// Usage: npm run build && cargo build -p gnotes-server && node e2e/live-edit.mjs
// Env: CHROME (default /usr/bin/chromium), SHOTS (directory for screenshots, optional).
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
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

  // Bob sees it appear without reloading, and opens it.
  await bob.waitForFunction(() => [...document.querySelectorAll("nav button")].some((b) => b.textContent.includes("Shared Notes")));
  await bob.evaluate(() => [...document.querySelectorAll("nav button")].find((b) => b.textContent.includes("Shared Notes")).click());
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
  await carol.evaluate(() => [...document.querySelectorAll("nav button")].find((b) => b.textContent.includes("Shared Notes")).click());
  await carol.waitForFunction(() => [...document.querySelectorAll("li button")].some((b) => b.textContent.includes("Groceries")));
  check(true, "invitee joins from the link and sees the shared note");

  // The phone back button walks back through screens: note -> list -> notebooks.
  await carol.evaluate(() => [...document.querySelectorAll("li button")].find((b) => b.textContent.includes("Groceries")).click());
  await carol.waitForSelector(".cm-content");
  await carol.goBack();
  await carol.waitForFunction(() => location.hash === "#/shared");
  await carol.goBack();
  await carol.waitForFunction(() => location.hash === "#/");
  check(true, "back button goes note -> list -> notebooks");

  // A new note that's left blank is thrown away, like Apple Notes.
  const count = () => alice.$$eval("li button .note-title", (els) => els.length);
  const before = await count();
  await alice.click(".list header button[aria-label='New note']");
  await alice.waitForFunction((n) => document.querySelectorAll("li button .note-title").length === n + 1, {}, before);
  await alice.waitForSelector(".cm-content[contenteditable=true]");
  await alice.evaluate(() => [...document.querySelectorAll("li button")].find((b) => b.textContent.includes("Groceries")).click());
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
  await alice.evaluate(() => [...document.querySelectorAll(".stt button")].find((b) => b.textContent === "Test").click());
  await alice.waitForFunction(() => document.querySelector(".stt .status")?.textContent.includes("Connected"), { timeout: 5000 });
  await alice.click(".stt button[type=submit]");
  await alice.waitForFunction(() => document.body.innerText.includes("Speech-to-text saved"));
  check(true, "admin sets up speech-to-text in settings");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-settings.png") });
  await alice.click(".settings-layer .back");
  await alice.waitForFunction(() => !document.querySelector(".settings-layer"));
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("bread"));

  // Inside a nested notebook the list shows its path and name, and its notes show a notebook chip.
  const kitchen = await alice.evaluate(async () => {
    const post = (path, body) => fetch(`/api${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) }).then((r) => r.json());
    const home = await post("/notebooks", { name: "Home", parent_id: null });
    const kitchen = await post("/notebooks", { name: "Kitchen", parent_id: home.id });
    return kitchen.id;
  });
  await alice.evaluate((id) => (location.hash = `#/nb/${id}`), kitchen);
  await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Kitchen" && document.querySelector(".hero .path")?.textContent.includes("Home"));
  check(true, "nested notebook shows its path and name");
  await alice.click(".list header button[aria-label='New note']");
  await alice.waitForSelector(".cm-content[contenteditable=true]");
  await alice.keyboard.type("Pantry");
  await alice.waitForFunction(() => document.querySelector(".notebook-chip")?.textContent.trim() === "Kitchen");
  check(true, "a note in a notebook shows the notebook chip");
  if (process.env.SHOTS) await (await new Promise((r) => setTimeout(r, 400)), alice.screenshot({ path: join(process.env.SHOTS, "desktop-notebook.png") }));

  // Folders: make a sub-notebook from the notebook menu, see it listed in its parent, and move a note into it.
  const pantry = await alice.evaluate(() => location.hash.match(/note\/([0-9a-f-]{36})/)[1]);
  await alice.evaluate(() => {
    const kitchen = location.hash.match(/nb\/([0-9a-f-]{36})/)[1];
    location.hash = `#/nb/${kitchen}`;
  });
  await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Kitchen");
  await alice.click(".hero .path button");
  await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Home");
  await alice.click(".list header button[aria-label='Notebook menu']");
  await alice.waitForFunction(() => [...document.querySelectorAll("[role=menuitem]")].some((b) => b.textContent.includes("New Notebook Here")));
  await alice.evaluate(() => [...document.querySelectorAll("[role=menuitem]")].find((b) => b.textContent.includes("New Notebook Here")).click());
  await alice.type("#new-sub-notebook input", "Garage");
  await alice.click("dialog button[form=new-sub-notebook]");
  await closed(alice);
  await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Garage" && document.querySelector(".hero .path")?.textContent.includes("Home"));
  check(true, "a sub-notebook is made from the notebook menu");
  await alice.click(".hero .path button");
  await alice.waitForFunction(() => {
    const names = [...document.querySelectorAll(".folder-row .folder-name")].map((e) => e.textContent);
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
  await alice.waitForFunction(() => document.querySelector(".notebook-chip")?.textContent.trim() === "Garage");
  check(true, "a note moves to another notebook");

  // A photo picked from the format bar uploads and shows inline.
  const png = join(data, "dot.png");
  writeFileSync(png, Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEklEQVR4nGP4z8CAFWEXHbQSACj/P8Fu7N9hAAAAAElFTkSuQmCC", "base64"));
  const [chooser] = await Promise.all([alice.waitForFileChooser(), alice.click("button[aria-label='Add photo']")]);
  await chooser.accept([png]);
  await alice.waitForFunction(() => document.querySelector(".cm-attachment img")?.naturalWidth === 8, { timeout: 5000 });
  check(true, "a picked photo uploads and shows in the note");

  // A voice memo records, embeds a player, and gets its transcript underneath.
  await alice.click(".cm-content");
  await alice.click("button[aria-label='Record voice memo']");
  await alice.waitForSelector("dialog .stop:not([disabled])");
  await new Promise((r) => setTimeout(r, 1500));
  await alice.click("dialog .stop");
  await alice.waitForSelector(".cm-audio", { timeout: 5000 });
  await alice.waitForFunction(() => document.querySelector(".cm-content").innerText.includes("remember the milk"), { timeout: 5000 });
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
  await alice.click(".list header button[aria-label='New note']");
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
    await alice.evaluate(() => [...document.querySelectorAll(".toolbar button")].find((b) => b.textContent.includes("Share")).click());
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
    await alice.waitForSelector(".toolbar button");
    check(await alice.evaluate(() => !document.querySelector(".cm-editor.cm-focused")), "the phone keyboard bar can hide the keyboard");
    await alice.click(".editor .back");
    await new Promise((r) => setTimeout(r, 500));
    await shot("phone-list");
    await alice.emulateMediaFeatures([{ name: "prefers-color-scheme", value: "dark" }]);
    await alice.click(".list .back");
    await new Promise((r) => setTimeout(r, 500));
    await shot("phone-sidebar-dark");
    await alice.evaluate(() => [...document.querySelectorAll(".pane.sidebar .row")].find((b) => b.textContent.includes("Home")).click());
    await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Home");
    await alice.evaluate(() => [...document.querySelectorAll(".folder-row")].find((b) => b.textContent.includes("Kitchen")).click());
    await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Kitchen");
    await shot("phone-subfolder-dark");
    await alice.click(".list .back");
    await alice.waitForFunction(() => document.querySelector(".hero h1")?.textContent === "Home");
    await shot("phone-folders-dark");
  }
  console.log("all checks passed");
} finally {
  await browser?.close();
  server.kill();
  whisper.close();
  rmSync(data, { recursive: true, force: true });
}
