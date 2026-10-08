// Screenshots of the GTK client, headless: a throwaway server with demo data (demo.mjs), the app
// signed in on a Broadway display (no window on your screen), one PNG per screen. The app renders
// each PNG itself (GNOTES_SHOT, gtk/src/ui/shot.rs), so it shows exactly what GTK drew.
//
// Usage: cargo build -p gnotes-server -p gnotes-gtk && node gtk/scripts/shot.mjs [options] [screen...]
//   screens: login home note:<title> notebook:<name> recent search:<query> shared trash
//            share:<title> move:<title> new-notebook   (default: home note:Groceries)
//   --narrow | --wide     window size (default: both)
//   --dark | --light      style (default: both)
//   --out <dir>           where PNGs go (default: /tmp/gnotes-shots)
// Needs web/node_modules (loro-crdt, to write the demo notes) and gtk4-broadwayd.
import { mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { startDemo } from "./demo.mjs";

const args = process.argv.slice(2);
const flag = (f) => args.includes(f);
const outIdx = args.indexOf("--out");
const out = outIdx >= 0 ? args[outIdx + 1] : "/tmp/gnotes-shots";
const screens = args.filter((a, i) => !a.startsWith("--") && args[i - 1] !== "--out");
const targets = screens.length ? screens : ["home", "note:Groceries"];
const sizes = [...(flag("--narrow") ? [] : [["wide", "1280x800"]]), ...(flag("--wide") ? [] : [["narrow", "400x820"]])];
const styles = [...(flag("--light") ? [] : ["dark"]), ...(flag("--dark") ? [] : ["light"])];

const demo = await startDemo();
// Sam keeps Groceries open with the cursor in it, so the app has someone to show.
const sam = await demo.stay(demo.sam, demo.ids.groceries, "Spinach");

mkdirSync(out, { recursive: true });
const saved = [];
for (const target of targets) {
  for (const style of styles) {
    for (const [sizeName, size] of sizes) {
      const file = join(out, `gtk-${target.replace(/[^\w.-]+/g, "_")}-${sizeName}-${style}.png`);
      rmSync(file, { force: true });
      const { code, stderr } = await demo.runApp({
        signedIn: target !== "login",
        style,
        size,
        env: { GNOTES_OPEN: target, GNOTES_SHOT: file },
        timeout: 20_000,
      });
      if (code !== 0 || stderr) console.error(`${target}: exit ${code}\n${stderr}`);
      saved.push(file);
    }
  }
}
sam.stop();
console.log(saved.join("\n"));
process.exit(0);
