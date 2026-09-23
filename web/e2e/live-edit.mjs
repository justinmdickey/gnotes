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
  const r = spawnSync(bin, ["create-user", name, "--display-name", display], { env: { ...env, GNOTES_PASSWORD: "password123" } });
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
  await alice.click("button[aria-label='New note']");
  await alice.waitForSelector(".cm-content[contenteditable=true]");
  await alice.click(".cm-content");
  await alice.keyboard.type("# Groceries\nmilk");
  await alice.waitForFunction(() => document.querySelector("main section header .title")?.textContent === "Groceries", { timeout: 5000 });
  check(true, "title follows the first line");

  await alice.click("button[aria-label='Share']");
  await alice.waitForSelector("dialog form select");
  await alice.click("dialog form button[type=submit]");
  await alice.waitForFunction(() => document.querySelector("dialog ul li")?.textContent.includes("Bob"));
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
  const [a, b] = [await text(alice), await text(bob)];
  check(a === b, `both editors converge (${JSON.stringify(a)})`);

  // Remote cursor and presence avatar.
  await alice.waitForFunction(() => document.querySelector(".loro-cursor")?.style.getPropertyValue("--name") === '"Bob"', { timeout: 5000 });
  check(true, "alice sees bob's cursor labelled Bob");
  await bob.waitForSelector(".peers .avatar[title='Alice is here']", { timeout: 5000 });
  check(true, "bob sees alice in the presence list");

  // Revoking removes access live.
  await alice.click("button[aria-label='Share']");
  await alice.waitForSelector("dialog ul li button");
  await alice.click("dialog ul li button");
  await alice.waitForFunction(() => document.querySelector("dialog ul li")?.textContent.includes("Only you"));
  await alice.click("dialog .actions button");
  await bob.waitForFunction(() => document.querySelector(".banner")?.textContent.includes("no longer have access"), { timeout: 5000 });
  check(true, "bob loses access when alice unshares");

  // Content survives a reload.
  await alice.reload();
  await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("bread"));
  check(true, "alice's note reloads from the server");

  if (process.env.SHOTS) {
    await alice.screenshot({ path: join(process.env.SHOTS, "desktop.png") });
    // Switching to mobile emulation reloads the page.
    await alice.setViewport({ width: 390, height: 844, isMobile: true, hasTouch: true });
    await alice.waitForFunction(() => document.querySelector(".cm-content")?.innerText.includes("bread"));
    await alice.screenshot({ path: join(process.env.SHOTS, "phone-editor.png") });
    await alice.click("button[aria-label='Back']");
    await alice.screenshot({ path: join(process.env.SHOTS, "phone-list.png") });
    await alice.emulateMediaFeatures([{ name: "prefers-color-scheme", value: "dark" }]);
    await alice.click("button[aria-label='Show sidebar']");
    await alice.screenshot({ path: join(process.env.SHOTS, "phone-sidebar-dark.png") });
  }
  console.log("all checks passed");
} finally {
  await browser?.close();
  server.kill();
  rmSync(data, { recursive: true, force: true });
}
