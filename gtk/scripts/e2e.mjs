// End-to-end checks of the GTK client against a real server: the app runs headless (Sway) and is
// driven by a script (GNOTES_DRIVE, gtk/src/ui/drive.rs), while this side changes things on the
// server as other people would and checks what landed. Prints `ok - …` per check and ends with
// `all checks passed`.
//
// Usage: cargo build -p gnotes-server -p gnotes-gtk && node gtk/scripts/e2e.mjs
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { sleep, startDemo } from "./demo.mjs";

const demo = await startDemo();
const { ann, sam, ids } = demo;
const dir = mkdtempSync(join(tmpdir(), "gnotes-e2e-"));
const f = (name) => join(dir, name);
const dump = (name) => JSON.parse(readFileSync(f(name), "utf8"));

let failed = false;
function check(name, ok, detail) {
  if (ok) console.log(`ok - ${name}`);
  else {
    failed = true;
    console.log(`not ok - ${name}${detail === undefined ? "" : `\n  ${JSON.stringify(detail)}`}`);
  }
}

async function waitFile(name, ms = 30_000) {
  for (let t = 0; t < ms; t += 50) {
    if (existsSync(f(name))) return true;
    await sleep(50);
  }
  throw new Error(`the app never signalled ${name}`);
}
const signal = (name) => writeFileSync(f(name), "");

/** Polls the server until `ok` holds for a note's text. */
async function serverText(id, ok, ms = 5000) {
  let text;
  for (let t = 0; t < ms; t += 200) {
    text = await demo.read(ann, id);
    if (ok(text)) return text;
    await sleep(200);
  }
  return text;
}

// A note Sam lets Ann read but not change.
const readOnly = await sam.note();
await demo.write(sam, readOnly, "# Read only\n\nLook, don't touch.");
await sam.call("POST", "/shares", { resource_type: "note", resource_id: readOnly, username: "ann", role: "viewer" });
await sleep(300);

const samHere = await demo.stay(sam, ids.groceries, "Spinach");

const script = `
wait-tree
wait-row Home
dump ${f("tree.json")}
open note:Groceries
wait-loaded
wait-text Spinach
wait-hidden **Saturday**
cursor 2
wait-shown **Saturday**
signal ${f("preview")}
wait-peer Sam
dump ${f("open.json")}
type \\n- [ ] Coffee\\nTea
signal ${f("typed")}
wait-file ${f("remote")}
wait-text Bananas
wait-writer Bananas|Sam
signal ${f("blamed")}
sleep 1200
undo
signal ${f("undone")}
wait-file ${f("undo-checked")}
tick 1
signal ${f("ticked")}
wait-file ${f("garden")}
wait-row Garden
open note:Read only
wait-loaded
dump ${f("viewer.json")}
open search:butter
wait-list Groceries
open recent
wait-list Snippets
new-note
type Fresh idea
signal ${f("fresh")}
wait-file ${f("fresh-checked")}
new-note
open note:Groceries
wait-loaded
signal ${f("blank")}
wait-file ${f("server-down")}
wait-status offline
type \\nWritten offline
dump ${f("offline.json")}
new-note
type Made offline
dump ${f("offline-new.json")}
open note:Groceries
wait-text Written offline
signal ${f("offline-typed")}
wait-status online
wait-file ${f("synced")}
quit
`;

const run = demo.runApp({ env: { GNOTES_DRIVE: script }, timeout: 120_000 });

try {
  await waitFile("preview");
  check("live preview hides marks, and shows them on the line being edited", true);
  await waitFile("typed");
  let d = dump("tree.json");
  check("signs in from a saved session and lists the tree", ["Home", "Personal", "Snippets", "Family"].every((r) => d.sidebar.includes(r)), d.sidebar);
  d = dump("open.json");
  check("opens a note with its text", d.text.startsWith("# Groceries") && d.title === "Groceries", d);
  check("shows who else has the note open", d.peers.includes("Sam"), d.peers);
  check("the open note is highlighted in the sidebar", d.sidebar.includes("> Groceries"), d.sidebar);

  let text = await serverText(ids.groceries, (t) => t.includes("Tea"));
  let ann_ = samHere.peers().find((p) => p.name === "Ann");
  for (let t = 0; t < 3000 && ann_?.at == null; t += 100) {
    await sleep(100);
    ann_ = samHere.peers().find((p) => p.name === "Ann");
  }
  check("others see your cursor where you're typing", ann_?.at === samHere.text().length && /^user-/.test(ann_?.color), { ann_, len: samHere.text().length });
  check("typing reaches the server", text.includes("- [ ] Coffee"), text);
  check("Enter in a checklist starts the next item", text.endsWith("- [ ] Coffee\n- [ ] Tea"), text.slice(-40));

  samHere.insert("\n- [ ] Bananas", samHere.text().indexOf("- [ ] Lemons") + "- [ ] Lemons".length);
  signal("remote");
  await waitFile("blamed");
  check("someone else's edit shows up live", true);
  check("a line someone else wrote is marked as theirs", true);
  await waitFile("undone");
  text = await serverText(ids.groceries, (t) => !t.includes("Coffee"));
  check("undo takes back your own typing", !text.includes("Coffee") && !text.includes("Tea"), text.slice(-60));
  check("undo leaves others' edits alone", text.includes("- [ ] Bananas"), text);
  signal("undo-checked");

  await waitFile("ticked");
  text = await serverText(ids.groceries, (t) => t.includes("- [x] Spinach"));
  check("clicking a checkbox ticks it", text.includes("- [x] Spinach"), text);

  await ann.notebook("Garden");
  signal("garden");
  await waitFile("fresh");
  check("a notebook made elsewhere appears without a restart", true);
  d = dump("viewer.json");
  check("a note shared to view is read-only", d.editable === false && /read this note/.test(d.banner ?? ""), d);
  check("search lists notes by a word in them", true);
  check("Recent lists notes", true);

  let tree;
  for (let t = 0; t < 5000 && !tree?.notes.some((n) => n.title === "Fresh idea"); t += 200) {
    tree = await ann.tree();
    await sleep(200);
  }
  const fresh = tree.notes.find((n) => n.title === "Fresh idea");
  check("New Note makes a note you can type in", !!fresh, tree.notes.map((n) => n.title));
  signal("fresh-checked");

  await waitFile("blank");
  await sleep(800);
  const blanks = (await ann.tree()).notes.filter((n) => !n.title);
  check("a new note left blank is discarded", blanks.length === 0, blanks);

  await demo.stopServer();
  signal("server-down");
  await waitFile("offline-typed");
  d = dump("offline.json");
  check("typing offline keeps the note to push later", d.status === "offline" && d.dirty.includes(ids.groceries) && d.text.endsWith("Written offline"), d);
  const made = dump("offline-new.json");
  check("a note made offline is queued", made.queued.includes(made.note), made);
  await demo.startServer();
  text = await serverText(ids.groceries, (t) => t.includes("Written offline"), 15_000);
  check("edits made offline land after reconnecting", text.includes("Written offline"), text.slice(-60));
  let titles = [];
  for (let t = 0; t < 10_000 && !titles.includes("Made offline"); t += 300) {
    titles = (await ann.tree()).notes.map((n) => n.title);
    await sleep(300);
  }
  check("a note made offline is created on reconnect with its text", titles.includes("Made offline"), titles);
  signal("synced");
} catch (e) {
  failed = true;
  console.log(`not ok - ${e.message}`);
}

let { code, stderr } = await run;
check("the app ran and quit cleanly", code === 0, { code, stderr });

// Notebooks, moving, the trash and sharing, as the sidebar's menus and the Share dialog do them.
const treeRun = demo.runApp({
  env: {
    GNOTES_DRIVE: [
      "wait-tree",
      "make-notebook Recipes",
      "wait-row Recipes",
      "rename-notebook Recipes|Cookbook",
      "wait-row Cookbook",
      "move-note Snippets|Cookbook",
      "open notebook:Cookbook",
      "wait-row Snippets",
      "trash-note Pantry",
      `signal ${f("trashed")}`,
      `wait-file ${f("trash-checked")}`,
      "restore Pantry",
      "sleep 800",
      "share-note Snippets|sam|viewer",
      "drag-note Plumber|Personal",
      "sleep 800",
      `dump ${f("tree2.json")}`,
      "quit",
    ].join("\n"),
  },
});
await waitFile("trashed").catch(() => {});
let t2 = await ann.tree();
for (let t = 0; t < 5000 && t2.notes.some((n) => n.title === "Pantry"); t += 200) {
  await sleep(200);
  t2 = await ann.tree();
}
check("Move to Trash takes a note out of the tree", !t2.notes.some((n) => n.title === "Pantry"), t2.notes.map((n) => n.title));
signal("trash-checked");
({ code, stderr } = await treeRun);
check("the tree's actions ran", code === 0, { code, stderr });
t2 = await ann.tree();
const cookbook = t2.notebooks.find((n) => n.name === "Cookbook");
check("New Notebook and Rename work", !!cookbook && !t2.notebooks.some((n) => n.name === "Recipes"), t2.notebooks.map((n) => n.name));
check("Move to puts a note in another notebook", t2.notes.find((n) => n.title === "Snippets")?.notebook_id === cookbook?.id);
check("Restore brings a note back from the trash", t2.notes.some((n) => n.title === "Pantry"));
check("dragging a note onto a notebook moves it", t2.notes.find((n) => n.title === "Plumber")?.notebook_id === t2.notebooks.find((n) => n.name === "Personal")?.id);
const samTree = await sam.tree();
check("Share gives someone else the note", samTree.notes.some((n) => n.title === "Snippets" && n.role === "viewer"), samTree.notes.map((n) => [n.title, n.role]));

// Photos and recordings.
({ code, stderr } = await demo.runApp({
  env: {
    GNOTES_DRIVE: [
      "wait-tree",
      "open note:Plumber",
      "wait-loaded",
      "wait-media 1",
      `dump ${f("memo.json")}`,
      "open note:Snippets",
      "wait-loaded",
      `add-file ${join(demo.root, "web/public/icon-192.png")}`,
      "wait-media 1",
      `dump ${f("photo.json")}`,
      "quit",
    ].join("\n"),
  },
}));
check("photos and recordings ran", code === 0, { code, stderr });
check("a recording shows a play button", existsSync(f("memo.json")) && dump("memo.json").media[0] === "▶  Play Recording".trim(), existsSync(f("memo.json")) && dump("memo.json").media);
const snippetsText = await serverText(ids.snippets, (t) => t.includes("att:"));
check("Add Photo uploads it and links it in the note", /!\[icon-192\]\(att:[0-9a-f-]+\)/.test(snippetsText), snippetsText.slice(-80));
check("the photo shows in the note", existsSync(f("photo.json")) && dump("photo.json").media.includes("photo"), existsSync(f("photo.json")) && dump("photo.json").media);

// The "/" menu and emoji.
({ code, stderr } = await demo.runApp({
  env: {
    GNOTES_DRIVE: [
      "wait-tree",
      "open note:Snippets",
      "wait-loaded",
      "type \\n/che",
      "wait-menu",
      "pick Checklist",
      "type Buy coffee",
      "type \\nLunch was :taco:",
      "sleep 200",
      "type \\n:smil",
      "wait-menu",
      "pick :smile:",
      "code-lang 0|python",
      "type \\n/table",
      "wait-menu",
      "pick Table",
      "sleep 600",
      "quit",
    ].join("\n"),
  },
}));
check("the slash menu and emoji ran", code === 0, { code, stderr });
const slashed = await demo.read(ann, ids.snippets);
check("the / menu sets a line's style", slashed.includes("- [ ] Buy coffee"), slashed.slice(-200));
check("typing :taco: makes the emoji", slashed.includes("Lunch was 🌮"));
check("the emoji list puts in the one picked", slashed.includes("😄"));
check("a code block's language can be picked", slashed.includes("```python\nkubectl"), slashed);
check("the / menu adds a table", slashed.includes("| Column 1 | Column 2 | Column 3 |"));

// Settings: your name, API keys, invites, AI services and import.
writeFileSync(f("Imported.md"), "# Imported note\n\nFrom a Markdown file.");
({ code, stderr } = await demo.runApp({
  env: {
    GNOTES_DRIVE: [
      "wait-tree",
      "set-name Annie",
      `make-key Agent|read|${f("key.txt")}`,
      "make-key Spare|write|/dev/null",
      "revoke-key Spare",
      `invite ${f("invite.txt")}`,
      "save-service summary|http://127.0.0.1:9/v1|stand-in",
      "sleep 500",
      `import ${f("Imported.md")}`,
      "wait-row Imported note",
      `dump ${f("settings.json")}`,
      "quit",
    ].join("\n"),
  },
}));
check("settings ran", code === 0, { code, stderr });
const settingsDump = existsSync(f("settings.json")) ? dump("settings.json") : {};
check("a new name shows in the app and on the server", settingsDump.user === "Annie" && (await ann.call("GET", "/me")).display_name === "Annie", settingsDump.user);
const keyToken = existsSync(f("key.txt")) ? readFileSync(f("key.txt"), "utf8") : "";
const keyRes = await fetch(`${demo.base}/api/v1/me`, { headers: { authorization: `Bearer ${keyToken}` } });
check("an API key made in Settings works", keyRes.ok, keyRes.status);
const keys = await ann.call("GET", "/me/keys");
check("revoking a key removes it", keys.length === 1 && keys[0].name === "Agent", keys.map((k) => k.name));
check("saving AI Summaries turns summaries on", settingsDump.features?.summaries === true, settingsDump.features);
check("Import brings in Markdown files", (await ann.tree()).notes.some((n) => n.title === "Imported note"));
await ann.call("PUT", "/admin/settings/summary", { url: "", model: "" });

// Joining with that invite link.
const link = existsSync(f("invite.txt")) ? readFileSync(f("invite.txt"), "utf8").trim() : "";
check("an invite link is made", link.startsWith(`${demo.base}/join/`), link);
({ code, stderr } = await demo.runApp({
  signedIn: false,
  env: { GNOTES_JOIN: `${link} carol Carol password123`, GNOTES_DRIVE: `wait-status online\ndump ${f("joined.json")}\nquit` },
}));
check("joining with an invite link makes the account and signs in", code === 0 && existsSync(f("joined.json")) && dump("joined.json").user === "Carol", { code, stderr });

// A brand-new server: the first account is made from the app.
const empty = await demo.startEmptyServer();
({ code, stderr } = await demo.runApp({
  signedIn: false,
  env: { GNOTES_SETUP: `${empty} admin Admin password123`, GNOTES_DRIVE: `wait-status online\ndump ${f("setup.json")}\nquit` },
}));
const setupDump = existsSync(f("setup.json")) ? dump("setup.json") : {};
check("a new server's first account is made from sign-in", code === 0 && setupDump.user === "Admin", { code, stderr, setupDump });

// The AI tools, against stand-in services on a server of their own.
{
  const aiDemo = await startDemo({ ai: true });
  const aiAnn = aiDemo.ann;
  // Sam's open note in the main demo is irrelevant here; this one starts clean.
  const aiStart = Date.now();
  ({ code, stderr } = await aiDemo.runApp({
    timeout: 180_000,
    env: {
      GNOTES_AUDIO_SRC: "audiotestsrc is-live=true wave=sine",
      GNOTES_DRIVE: [
        "wait-tree",
        "open note:Groceries",
        "wait-loaded",
        "summary",
        "wait-dialog Key points",
        `dump ${f("summary.json")}`,
        "tidy",
        "wait-text ## To buy",
        "open note:Pantry",
        "wait-loaded",
        "edit-ai Make it all bullets",
        "wait-text ## Action items",
        "cursor 0",
        "ai-write Add two more staples",
        "wait-text Olive oil",
        "open note:Snippets",
        "wait-loaded",
        "record-start",
        "sleep 1000",
        "record-stop",
        "wait-text Call the plumber about the sink.",
        "record-start",
        "live-delta Remember",
        "live-delta the",
        "live-final Remember the milk.",
        "record-stop",
        "wait-text Remember the milk.",
        "open note:Plumber",
        "wait-loaded",
        "wait-media 1",
        "open note:Groceries",
        "wait-loaded",
        `add-file ${join(demo.root, "web/public/icon-192.png")}`,
        "wait-media 1",
        "get-text 0",
        "wait-text RICE 2 KG",
        "wait-row Ask",
        "open ask:what do I still need to buy for the groceries?",
        "wait-answer spinach",
        `dump ${f("ask.json")}`,
        "cite 1",
        "wait-loaded",
        `dump ${f("cited.json")}`,
        "quit",
      ].join("\n"),
    },
  }));
  check("the AI tools ran", code === 0, { code, stderr, seconds: (Date.now() - aiStart) / 1000 });
  check("Summary shows the note's summary", existsSync(f("summary.json")) && /Spinach, lemons and milk/.test(dump("summary.json").dialog_text));
  const tidied = await aiDemo.read(aiAnn, aiDemo.ids.groceries);
  check("Tidy Up applies the tidied note", tidied.includes("## To buy") && tidied.includes("- [x] Apples"), tidied);
  const edited = await aiDemo.read(aiAnn, aiDemo.ids.pantry);
  check("Edit with AI applies the change", edited.includes("## Action items"), edited);
  const memos = await aiDemo.read(aiAnn, aiDemo.ids.snippets);
  check("a voice memo is recorded, uploaded and transcribed under its player", /!\[Voice memo\]\(att:[0-9a-f-]+\)\nCall the plumber about the sink\./.test(memos), memos);
  check("a live transcript is written as it's heard, under the player", /!\[Voice memo\]\(att:[0-9a-f-]+\)\nRemember the milk\./.test(memos), memos);
  const read = await aiDemo.read(aiAnn, aiDemo.ids.groceries);
  check("Get Text puts a photo's text under it", /att:[0-9a-f-]+\)\n+RICE 2 KG/.test(read), read.slice(-120));
  check("Ask shows in the sidebar and streams an answer", existsSync(f("ask.json")) && /spinach, lemons and milk/.test(dump("ask.json").dialog_text + JSON.stringify(dump("ask.json"))));
  check("an answer's citation opens its note", existsSync(f("cited.json")) && dump("cited.json").title === "Groceries", existsSync(f("cited.json")) && dump("cited.json").title);
  check("the AI Prompt writes at the cursor", /# Pantry\n- Olive oil\n- Coffee beans/.test(edited), edited.slice(0, 80));
}

// Signing in through the form, then starting again signed in, then starting with no server.
({ code, stderr } = await demo.runApp({
  signedIn: false,
  env: { GNOTES_LOGIN: `${demo.base} ann password123`, GNOTES_DRIVE: `wait-tree\nwait-row Home\nquit` },
}));
check("signs in with a server, username and password", code === 0, { code, stderr });
check("keeps the session for next time", existsSync(join(demo.data, "gtk/.config/session.json")));
({ code, stderr } = await demo.runApp({ fresh: false, signedIn: false, env: { GNOTES_DRIVE: `wait-tree\nwait-row Home\nopen note:Groceries\nwait-loaded\nquit` } }));
check("starts signed in after a restart", code === 0, { code, stderr });
await demo.stopServer();
({ code, stderr } = await demo.runApp({
  fresh: false,
  signedIn: false,
  env: { GNOTES_DRIVE: `wait-tree\nwait-status offline\nopen note:Groceries\nwait-text Written offline\ndump ${f("cold.json")}\nquit` },
}));
const cold = existsSync(f("cold.json")) ? dump("cold.json") : {};
check("with no server, starts into the notes kept on this computer", code === 0 && cold.editable === true, { code, stderr, cold });
await demo.startServer();
samHere.stop();
if (failed) {
  console.log("some checks failed");
  process.exit(1);
}
console.log("all checks passed");
process.exit(0);
