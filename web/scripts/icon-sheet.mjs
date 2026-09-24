// Every icon in src/lib/icons.ts on one sheet, 4x size, each over its 16px grid with the
// 2.5–13.5 drawing area outlined in pink. Use it to check a new or changed icon matches the rest.
// Usage: node scripts/icon-sheet.mjs [out.png]   (needs rsvg-convert)
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const src = readFileSync(resolve(import.meta.dirname, "../src/lib/icons.ts"), "utf8");
const grab = (name) => {
  const block = src.slice(src.indexOf(`const ${name} = {`)).split("\n};")[0];
  return [...block.matchAll(/^\s+([a-z_]+): "([^"]+)"/gm)].map((m) => [m[1], m[2]]);
};
const items = [...grab("stroked").map(([n, d]) => [n, d, false]), ...grab("filled").map(([n, d]) => [n, d, true])];
const cols = 10;
const cell = 96;
let svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${cols * cell}" height="${Math.ceil(items.length / cols) * cell}"><rect width="100%" height="100%" fill="#fff"/>`;
items.forEach(([name, d, filled], i) => {
  const x = (i % cols) * cell;
  const y = Math.floor(i / cols) * cell;
  const paint = filled ? `fill="#222"` : `fill="none" stroke="#222" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"`;
  svg += `<g transform="translate(${x + 16} ${y + 6}) scale(4)"><rect width="16" height="16" fill="#eef3fb"/><rect x="2.5" y="2.5" width="11" height="11" fill="none" stroke="#f3b" stroke-width=".1"/><path d="${d}" ${paint}/></g>`;
  svg += `<text x="${x + cell / 2}" y="${y + 88}" font-size="11" text-anchor="middle" font-family="sans-serif">${name}</text>`;
});
const out = process.argv[2] ?? "/tmp/gnotes-icons.png";
writeFileSync(`${out}.svg`, `${svg}</svg>`);
execFileSync("rsvg-convert", [`${out}.svg`, "-o", out]);
console.log(out);
