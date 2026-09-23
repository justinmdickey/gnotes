// Two browsers, two accounts, one note: sharing, live edits and remote cursors.
// Usage: npm run build && cargo build -p gnotes-server && node e2e/live-edit.mjs
// Env: CHROME (default /usr/bin/chromium), SHOTS (directory for screenshots, optional).
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import puppeteer from "puppeteer-core";

const root = resolve(import.meta.dirname, "../..");
const bin = join(root, "target/debug/gnotes-server");
const data = mkdtempSync(join(tmpdir(), "gnotes-e2e-"));
const port = 18000 + Math.floor(Math.random() * 1000);
const base = `http://127.0.0.1:${port}`;
const env = { ...process.env, GNOTES_DATA_DIR: data, GNOTES_BIND: `127.0.0.1:${port}`, GNOTES_WEB_DIR: join(root, "web/dist") };

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
    args: ["--no-sandbox", "--disable-gpu"],
  });
  const alice = await login(browser, "alice");
  const bob = await login(browser, "bob");

  // Alice writes a note and shares it with Bob.
  await alice.click(".list header button[aria-label='New note']");
  await alice.waitForSelector(".cm-content[contenteditable=true]");
  await alice.click(".cm-content");
  await alice.keyboard.type("# Groceries\nmilk");
  await alice.waitForFunction(() => document.querySelector("main section header .title")?.textContent === "Groceries", { timeout: 5000 });
  check(true, "title follows the first line");

  await alice.click("button[aria-label='Share']");
  await alice.waitForSelector("dialog button.add");
  await alice.click("dialog button.add");
  await alice.waitForFunction(() => document.querySelector("dialog ul")?.textContent.includes("Bob"));
  await alice.click("dialog .actions button");
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
  await bob.waitForFunction(() => document.querySelector(".banner")?.textContent.includes("no longer have access"), { timeout: 5000 });
  check(true, "bob loses access when alice unshares");

  // Invite someone new from the same dialog; they join from the link and see the note.
  await alice.click("dialog button.invite");
  const link = await alice.waitForSelector("dialog input.link").then((el) => el.evaluate((i) => i.value));
  await alice.click("dialog .actions button");
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
  await alice.waitForFunction(() => document.body.innerText.includes("password was reset"));
  await bob.waitForFunction(() => document.body.innerText.includes("You were signed out"), { timeout: 10000 });
  check(true, "admin reset signs the user out of their open app");
  if (process.env.SHOTS) await alice.screenshot({ path: join(process.env.SHOTS, "desktop-settings.png") });
  await alice.click(".page header .back");
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("bread"));

  if (process.env.SHOTS) {
    const shot = (name) => alice.screenshot({ path: join(process.env.SHOTS, `${name}.png`) });
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
    await alice.goto(noteUrl);
    await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("bread"));
    await alice.tap("button[aria-label='Share']");
    await alice.waitForSelector("dialog button.add");
    await shot("phone-share");
    await alice.tap("dialog .actions button");
    await alice.tap(".cm-content");
    await shot("phone-editing");
    await alice.tap("button[aria-label='Text styles']");
    await shot("phone-editing-styles");
    await alice.tap("button[aria-label='Text styles']");
    await alice.evaluate(() => document.activeElement?.blur());
    await alice.click("main .back");
    await shot("phone-list");
    await alice.emulateMediaFeatures([{ name: "prefers-color-scheme", value: "dark" }]);
    await alice.click(".list .back");
    await shot("phone-sidebar-dark");
  }
  console.log("all checks passed");
} finally {
  await browser?.close();
  server.kill();
  rmSync(data, { recursive: true, force: true });
}
