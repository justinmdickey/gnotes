// The app's one icon set: symbolic icons in the Adwaita style, on a 16px grid with
// 1.5px round strokes, drawn inside 2.5–13.5 so every icon has the same weight and size.
// Svelte uses <Icon>; plain DOM (editor widgets) uses iconSvg().

const stroked = {
  plus: "M8 3v10M3 8h10",
  back: "M10.25 3 5.25 8l5 5",
  menu: "M3 4.5h10M3 8h10M3 11.5h10",
  share: "M5.75 6.5H3.5v7h9v-7h-2.25M8 2.5v7.5M5.75 4.75 8 2.5l2.25 2.25",
  trash: "M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.7 8.5h5.6l.7-8.5",
  folder: "M2.25 3.25h4l1.5 1.75h6v7.75H2.25z",
  note: "M4 2.5h5.5l2.5 2.5v8.5H4zM9.5 2.5V5H12",
  people: "M6 7a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM2.5 13c0-2 1.5-3.5 3.5-3.5s3.5 1.5 3.5 3.5M10.5 7a1.7 1.7 0 1 0 0-3.4M11.5 9.5c1.3.3 2 1.6 2 3.5",
  pencil: "M3 13l.5-2.5 7-7 2 2-7 7zM9.5 4.5l2 2",
  logout: "M9.5 3h-6v10h6M7 8h6.5M11 5.5 13.5 8 11 10.5",
  more: "M7.6 3.5h.8M7.6 8h.8M7.6 12.5h.8",
  newfolder: "M2.25 3.25h4l1.5 1.75h6v7.75H2.25zM8 7.25v3.5M6.25 9h3.5",
  person_add: "M6.5 7a2.25 2.25 0 1 0 0-4.5 2.25 2.25 0 0 0 0 4.5zM2.5 13.5c0-2.3 1.8-4 4-4 .9 0 1.7.25 2.35.7M11.5 9v4.5M9.25 11.25h4.5",
  link: "M7 9a2.5 2.5 0 0 0 3.5 0l2-2a2.5 2.5 0 0 0-3.5-3.5l-.7.7M9 7a2.5 2.5 0 0 0-3.5 0l-2 2A2.5 2.5 0 0 0 7 12.5l.7-.7",
  compose: "M13 8.5v4.5H3V3h4.5M7 9l.4-2L12 2.4 13.6 4 9 8.6z",
  search: "M7 11.5a4.5 4.5 0 1 0 0-9 4.5 4.5 0 0 0 0 9zM10.3 10.3 13.5 13.5",
  checklist: "M2.5 4.5l1 1 2-2M2.5 10.5l1 1 2-2M8 4.5h5.5M8 10.5h5.5",
  bullets: "M3.5 4.5h.01M3.5 8h.01M3.5 11.5h.01M6.5 4.5h7M6.5 8h7M6.5 11.5h7",
  numbers: "M2.5 3.5h1v3M2.5 9.5h2l-2 2.5h2M6.5 4.5h7M6.5 8h7M6.5 11.5h7",
  quote: "M3 3.5v9M6 5h7M6 8h7M6 11h5",
  code: "M5.5 4.5 2 8l3.5 3.5M10.5 4.5 14 8l-3.5 3.5",
  textformat: "M2 12.5 5 4l3 8.5M3 10h4M10.5 8.5c.5-.7 1.2-1 2-1 1 0 1.5.6 1.5 1.5v3.5M14 10.5c-2.5-.5-4 .2-4 1.2 0 .6.5 1 1.2 1 1.3 0 2.3-.8 2.8-1.7",
  sidebar: "M2.5 3.5h11v9h-11zM6 3.5v9",
  close: "M3.75 3.75l8.5 8.5M12.25 3.75l-8.5 8.5",
  check: "M3 8.5l3.25 3.25L13 4.75",
  next: "M5.75 3l5 5-5 5",
  expand: "M3 6l5 5 5-5",
  clock: "M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11zM8 5v3.25l2.25 1.5",
  home: "M2.5 7.5 8 3l5.5 4.5M4 6.5v6.5h8V6.5",
  move: "M2.25 3.25h4l1.5 1.75h6v7.75H2.25zM5.75 9h4.5M8.75 7.25 10.5 9l-1.75 1.75",
  key: "M5.25 13.5a2.75 2.75 0 1 1 0-5.5 2.75 2.75 0 0 1 0 5.5zM7.2 8.8 13.5 2.5M11.25 4.75l1.75 1.75M9.5 6.5l1.25 1.25",
  devices: "M2.5 3.5h8v6h-8zM4.5 12.5h4M6.5 9.5v3M11.5 6.5h2v6h-2z",
  rename: "M3 13l.5-2.5 7-7 2 2-7 7zM9.5 4.5l2 2",
  shield: "M8 2.5l5 2v3.5c0 3-2.2 5-5 5.5-2.8-.5-5-2.5-5-5.5V4.5z",
  user: "M8 7.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5zM3 13.5c0-2.5 2.2-4.2 5-4.2s5 1.7 5 4.2",
  camera: "M2.5 5.5h2.5l1-1.5h4l1 1.5h2.5v7.5h-11zM8 11a2 2 0 1 0 0-4 2 2 0 0 0 0 4z",
  mic: "M8 2.5a2 2 0 0 0-2 2V8a2 2 0 1 0 4 0V4.5a2 2 0 0 0-2-2zM4 7.5a4 4 0 0 0 8 0M8 11.5v2",
  image: "M2.5 3.5h11v9h-11zM2.5 10.5l3-3 3 3 2-2 3 3M10.5 6.5h.01",
  // Hide the on-screen keyboard.
  keyboardhide: "M2.5 3h11v6.5h-11zM5 5.25h.01M8 5.25h.01M11 5.25h.01M5.5 7.5h5M6 11.75l2 2 2-2",
  bold: "M4.5 3h4a2.5 2.5 0 0 1 0 5h-4zM4.5 8h4.8a2.5 2.5 0 0 1 0 5H4.5z",
  italic: "M7 3h5M4 13h5M9.5 3l-3 10",
  strikethrough: "M3 8h10M11 4.8C10.5 3.7 9.4 3 8 3 6.3 3 5 4 5 5.3c0 .9.5 1.6 1.4 2M5 11c.5 1.2 1.6 2 3 2 1.8 0 3-1 3-2.4 0-.6-.2-1.1-.6-1.6",
};

/** Solid shapes, for media controls that sit on a colored button. */
const filled = {
  person: "M8 2.25a2.75 2.75 0 1 1 0 5.5 2.75 2.75 0 0 1 0-5.5zM2.75 13.25c0-2.9 2.35-4.5 5.25-4.5s5.25 1.6 5.25 4.5a.75.75 0 0 1-.75.75h-9a.75.75 0 0 1-.75-.75z",
  play: "M5.5 3.4v9.2a.6.6 0 0 0 .9.5l7.3-4.6a.6.6 0 0 0 0-1L6.4 2.9a.6.6 0 0 0-.9.5z",
  pause: "M4.5 3h2.25v10H4.5zM9.25 3h2.25v10H9.25z",
};

export type IconName = keyof typeof stroked | keyof typeof filled;

export function iconPath(name: IconName): { d: string; filled: boolean } {
  return name in filled
    ? { d: filled[name as keyof typeof filled], filled: true }
    : { d: stroked[name as keyof typeof stroked], filled: false };
}

/** The same icon as <Icon>, for code that builds DOM by hand. */
export function iconSvg(name: IconName, size = 16): SVGSVGElement {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 16 16");
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("class", "icon");
  const path = document.createElementNS(ns, "path");
  const { d, filled: solid } = iconPath(name);
  path.setAttribute("d", d);
  if (solid) svg.classList.add("filled");
  svg.append(path);
  return svg;
}
