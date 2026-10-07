// Makes Markdown read like formatted text: marks are hidden except on lines being edited,
// bullets and checkboxes render as widgets, and the first line is styled as the title.
import { syntaxTree } from "@codemirror/language";
import { StateEffect, StateField } from "@codemirror/state";
import type { EditorState, Range } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin, type ViewUpdate, WidgetType } from "@codemirror/view";
import { iconSvg } from "./icons";
import { EMBED, attachmentMeta, attachmentUrl, isTranscribing, onTranscribingChange, readPhoto, removeEmbed, transcribeMemo } from "./attachments";
import { openNote } from "./store.svelte";
import type { IconName } from "./icons";

/** Full-screen photo viewer that zooms out of the tapped image. */
function openLightbox(src: string, from: HTMLElement) {
  const r = from.getBoundingClientRect();
  const shade = document.createElement("div");
  shade.className = "cm-lightbox";
  const img = document.createElement("img");
  img.src = src;
  img.alt = "";
  shade.append(img);
  document.body.append(shade);
  // Start where the thumbnail is, then grow to fit the screen.
  const scale = Math.min(r.width / innerWidth, r.height / innerHeight);
  img.style.transform = `translate(${r.left + r.width / 2 - innerWidth / 2}px, ${r.top + r.height / 2 - innerHeight / 2}px) scale(${scale})`;
  requestAnimationFrame(() => {
    shade.classList.add("open");
    img.style.transform = "";
  });
  const close = () => {
    shade.classList.remove("open");
    shade.addEventListener("transitionend", () => shade.remove(), { once: true });
    setTimeout(() => shade.remove(), 400);
    removeEventListener("keydown", onKey);
  };
  const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
  shade.addEventListener("click", close);
  addEventListener("keydown", onKey);
}

function formatTime(s: number) {
  if (!Number.isFinite(s)) return "0:00";
  return `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
}

/** A compact player: play/pause, a scrubbable progress bar and the time. */
function audioPlayer(src: string, label: string): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "cm-audio";
  const audio = new Audio();
  audio.preload = "metadata";
  audio.src = src;
  const play = document.createElement("button");
  play.className = "cm-audio-play";
  play.setAttribute("aria-label", `Play ${label}`);
  const body = document.createElement("div");
  body.className = "cm-audio-body";
  const title = document.createElement("span");
  title.className = "cm-audio-title";
  title.textContent = label;
  const bar = document.createElement("div");
  bar.className = "cm-audio-bar";
  const fill = document.createElement("div");
  bar.append(fill);
  const time = document.createElement("span");
  time.className = "cm-audio-time";
  time.textContent = "0:00";
  const status = document.createElement("span");
  status.className = "cm-audio-status";
  // Playback speed, for long memos and meetings: 1×, 1.5×, 2×, and round again.
  const speed = document.createElement("button");
  speed.className = "cm-audio-speed";
  speed.textContent = "1×";
  speed.title = "Playback speed";
  speed.setAttribute("aria-label", "Playback speed 1×");
  speed.addEventListener("click", () => {
    const next = { 1: 1.5, 1.5: 2, 2: 1 }[audio.playbackRate] ?? 1;
    audio.playbackRate = next;
    speed.textContent = `${next}×`;
    speed.setAttribute("aria-label", `Playback speed ${next}×`);
  });
  body.append(title, bar);
  wrap.append(play, body, speed, time, status);

  const playIcon = iconSvg("play");
  const pauseIcon = iconSvg("pause");
  play.append(playIcon);
  let shown = playIcon;
  const sync = () => {
    wrap.classList.toggle("playing", !audio.paused);
    const want = audio.paused ? playIcon : pauseIcon;
    if (want !== shown) {
      shown.replaceWith(want);
      shown = want;
      play.setAttribute("aria-label", `${audio.paused ? "Play" : "Pause"} ${label}`);
    }
    const d = audio.duration;
    fill.style.width = Number.isFinite(d) && d > 0 ? `${(audio.currentTime / d) * 100}%` : "0";
    time.textContent = audio.paused && audio.currentTime === 0 ? formatTime(d) : formatTime(audio.currentTime);
  };
  for (const ev of ["play", "pause", "timeupdate", "loadedmetadata", "durationchange", "ended"]) audio.addEventListener(ev, sync);
  audio.addEventListener("ended", () => (audio.currentTime = 0));
  play.addEventListener("click", () => (audio.paused ? audio.play() : audio.pause()));
  bar.addEventListener("click", (e) => {
    const r = bar.getBoundingClientRect();
    if (Number.isFinite(audio.duration)) audio.currentTime = ((e.clientX - r.left) / r.width) * audio.duration;
  });
  return wrap;
}

const emptyTitle = Decoration.line({ class: "cm-title-empty" });

/** Above a note that starts with a photo or memo: tap to make a title line over it. */
class TitleHint extends WidgetType {
  eq() {
    return true;
  }

  toDOM(view: EditorView) {
    const el = document.createElement("span");
    el.className = "cm-title-hint add";
    el.textContent = "Add a title";
    el.addEventListener("mousedown", (e) => {
      e.preventDefault();
      if (!view.state.facet(EditorView.editable)) return;
      view.dispatch({ changes: { from: 0, insert: "\n" }, selection: { anchor: 0 }, userEvent: "input" });
      view.focus();
    });
    return el;
  }

  ignoreEvent() {
    return false;
  }
}

/**
 * The tools on a photo or memo, shown while you point at it (always on touch screens). Which ones
 * show is up to the note's page classes: .can-edit (Delete), .can-read (Get Text), .can-transcribe.
 */
function tools(items: { cls: string; icon: IconName; label: string; text?: string; run: () => void }[], download: { href: string; name: string }) {
  const bar = document.createElement("div");
  bar.className = "cm-att-tools";
  for (const it of items) {
    const b = document.createElement("button");
    b.className = it.cls;
    b.title = it.label;
    b.setAttribute("aria-label", it.label);
    b.append(iconSvg(it.icon, 14));
    if (it.text) b.append(it.text);
    b.addEventListener("click", (e) => {
      e.stopPropagation();
      it.run();
    });
    bar.append(b);
  }
  const a = document.createElement("a");
  a.className = "tool-download";
  a.href = download.href;
  a.download = download.name;
  a.title = "Download";
  a.setAttribute("aria-label", "Download");
  a.append(iconSvg("download", 14));
  a.addEventListener("click", (e) => e.stopPropagation());
  // Download sits before Delete, which stays last.
  bar.insertBefore(a, bar.querySelector(".tool-delete"));
  return bar;
}

/** A photo or voice memo, drawn in place of its `![label](att:id)` line. */
class AttachmentWidget extends WidgetType {
  private off?: () => void;

  constructor(
    readonly id: string,
    readonly label: string,
  ) {
    super();
  }

  eq(other: AttachmentWidget) {
    return other.id === this.id && other.label === this.label;
  }

  toDOM(view: EditorView) {
    const box = document.createElement("div");
    box.className = "cm-attachment loading";
    const src = attachmentUrl(this.id);
    attachmentMeta(this.id).then(
      (meta) => {
        box.classList.remove("loading");
        if (meta.mime.startsWith("audio/")) {
          box.classList.add("is-audio");
          box.append(audioPlayer(src, this.label || "Voice memo"));
          box.append(
            tools(
              [
                { cls: "tool-transcribe", icon: "textformat", label: "Transcribe this recording", text: "Transcribe", run: () => void transcribeMemo(view, this.id) },
                { cls: "tool-delete", icon: "trash", label: "Remove from note", run: () => removeEmbed(view, this.id, "Voice memo") },
              ],
              { href: src, name: meta.filename },
            ),
          );
          const update = () => box.classList.toggle("transcribing", isTranscribing(this.id));
          update();
          this.off = onTranscribingChange(update);
        } else {
          box.classList.add("is-image");
          const img = document.createElement("img");
          img.src = src;
          img.alt = this.label;
          img.decoding = "async";
          img.addEventListener("load", () => view.requestMeasure());
          img.addEventListener("click", () => openLightbox(src, img));
          box.append(img);
          box.append(
            tools(
              [
                { cls: "tool-read", icon: "textformat", label: "Get the text in this photo", text: "Get Text", run: () => void readPhoto(view, this.id) },
                { cls: "tool-delete", icon: "trash", label: "Remove from note", run: () => removeEmbed(view, this.id, "Photo") },
              ],
              { href: src, name: meta.filename },
            ),
          );
          // While the server reads the text in it, a line under it says so.
          const badge = document.createElement("span");
          badge.className = "cm-reading";
          badge.textContent = "Reading text…";
          box.append(badge);
          const update = () => box.classList.toggle("reading", isTranscribing(this.id));
          update();
          this.off = onTranscribingChange(update);
        }
        view.requestMeasure();
      },
      () => {
        box.classList.remove("loading");
        box.classList.add("missing");
        box.textContent = "Attachment unavailable";
      },
    );
    return box;
  }

  destroy() {
    this.off?.();
  }

  get estimatedHeight() {
    return 200;
  }

  ignoreEvent() {
    return true;
  }
}

class CheckboxWidget extends WidgetType {
  constructor(
    readonly checked: boolean,
    readonly pos: number,
  ) {
    super();
  }

  eq(other: CheckboxWidget) {
    return other.checked === this.checked && other.pos === this.pos;
  }

  toDOM(view: EditorView) {
    const box = document.createElement("span");
    box.className = this.checked ? "cm-checkbox checked" : "cm-checkbox";
    box.setAttribute("role", "checkbox");
    box.setAttribute("aria-checked", String(this.checked));
    box.addEventListener("mousedown", (e) => {
      e.preventDefault();
      if (!view.state.facet(EditorView.editable)) return;
      // Flip the character inside "[ ]" / "[x]".
      view.dispatch({ changes: { from: this.pos + 1, to: this.pos + 2, insert: this.checked ? " " : "x" } });
    });
    return box;
  }

  ignoreEvent() {
    return true;
  }
}

class BulletWidget extends WidgetType {
  eq() {
    return true;
  }

  toDOM() {
    const dot = document.createElement("span");
    dot.className = "cm-bullet";
    dot.textContent = "•";
    return dot;
  }
}

const bullet = Decoration.replace({ widget: new BulletWidget() });
const hidden = Decoration.replace({});
const lineClass = (cls: string) => Decoration.line({ class: cls });
const headingLines = [1, 2, 3, 4, 5, 6].map((n) => lineClass(`cm-h${Math.min(n, 3)}`));
const quoteLine = lineClass("cm-quote");
const doneText = Decoration.mark({ class: "cm-task-done" });
const embedLine = lineClass("cm-embed-line");
/**
 * The space after a bullet or checkbox stays real text, so on an empty item the cursor sits in a line
 * of text instead of against the drawn marker, which put it too low and too far left. It's set in a
 * monospace face so its width is a known 1ch, which the marker's own width makes up to --marker.
 */
const markerSpace = Decoration.mark({ class: "cm-marker-space" });
// List items whose wrapped lines hang under the text rather than under the bullet.
const itemLine = lineClass("cm-item");

// A wiki link, [[Title]] or [[Title|alias]], drawn as a link to the note with that title.
// `data-note` carries the target so a click can open it; a missing link has none.
const wikiMark = (id: string | null) =>
  Decoration.mark({ class: id ? "cm-wikilink" : "cm-wikilink cm-wikilink-missing", attributes: id ? { "data-note": id } : undefined });
const WIKI = /\[\[([^\[\]\n]+)\]\]/g;

// A web link: [text](url), <url> or a bare address. `data-href` is what a click opens.
const linkMark = (href: string) => Decoration.mark({ class: "cm-link", attributes: { "data-href": href } });
/** What a bare address opens: GFM autolinks come without a scheme, and emails need mailto. */
function hrefFor(text: string): string {
  const t = text.replace(/^<|>$/g, "");
  if (/^[a-z][a-z0-9+.-]*:/i.test(t)) return t;
  if (t.includes("@")) return `mailto:${t}`;
  return `https://${t}`;
}

/** The notes a [[link]] can point at, by lowercased title. The editor keeps it in step with the tree. */
export const setWikiNotes = StateEffect.define<Map<string, string>>();
export const wikiNotesField = StateField.define<Map<string, string>>({
  create: () => new Map(),
  update(value, tr) {
    for (const e of tr.effects)
      if (e.is(setWikiNotes)) value = e.value;
    return value;
  },
});

/** Line numbers touched by the selection. Their marks stay visible so they can be edited. */
function activeLines(view: EditorView): Set<number> {
  const active = new Set<number>();
  if (!view.hasFocus) return active;
  const { doc } = view.state;
  for (const r of view.state.selection.ranges) {
    for (let l = doc.lineAt(r.from).number; l <= doc.lineAt(r.to).number; l++) active.add(l);
  }
  return active;
}

/** End of a mark plus one following space, so "# Title" hides as a unit. */
function withSpace(state: EditorState, to: number) {
  return state.sliceDoc(to, to + 1) === " " ? to + 1 : to;
}

function build(view: EditorView): { decorations: DecorationSet; atomic: DecorationSet } {
  const { state } = view;
  const active = activeLines(view);
  const decos: Range<Decoration>[] = [];
  /** Ranges the cursor steps over without them being replaced. */
  const stepOver: Range<Decoration>[] = [];
  const lineAt = (pos: number) => state.doc.lineAt(pos);
  let firstLineIsHeading = false;

  // Embeds always show as the photo or player; deleting the line removes them.
  const embedLines = new Set<number>();
  for (const { from, to } of view.visibleRanges) {
    for (let pos = from; pos <= to; ) {
      const line = state.doc.lineAt(pos);
      const m = EMBED.exec(line.text);
      if (m && line.length) {
        embedLines.add(line.number);
        decos.push(embedLine.range(line.from));
        decos.push(Decoration.replace({ widget: new AttachmentWidget(m[2], m[1]) }).range(line.from, line.to));
      }
      pos = line.to + 1;
    }
  }
  if (embedLines.has(1)) firstLineIsHeading = true;
  const first = state.doc.line(1);
  if (embedLines.has(1) && view.state.facet(EditorView.editable)) {
    decos.push(Decoration.widget({ widget: new TitleHint(), side: -1 }).range(0));
  } else if (first.length === 0 && (state.doc.lines === 1 || !active.has(1))) {
    // A blank title line says "Title" through CSS on the line itself. An inline widget
    // there would sit before the caret and throw the caret off the big title text.
    decos.push(emptyTitle.range(0));
  }

  // Wiki links: [[Title]] or [[Title|alias]] point at the note with that title. The brackets
  // hide like other marks, except on the line being edited.
  const wiki = state.field(wikiNotesField);
  for (const { from, to } of view.visibleRanges) {
    for (let pos = from; pos <= to; ) {
      const line = state.doc.lineAt(pos);
      pos = line.to + 1;
      if (embedLines.has(line.number)) continue;
      const editing = active.has(line.number);
      WIKI.lastIndex = 0;
      let m: RegExpExecArray | null;
      while ((m = WIKI.exec(line.text))) {
        const raw = m[1];
        const pipe = raw.indexOf("|");
        const target = (pipe >= 0 ? raw.slice(0, pipe) : raw).trim();
        const id = wiki.get(target.toLowerCase()) ?? null;
        const spanFrom = line.from + m.index;
        const spanTo = spanFrom + m[0].length;
        decos.push(wikiMark(id).range(spanFrom, spanTo));
        if (!editing) {
          decos.push(hidden.range(spanFrom, spanFrom + 2));
          decos.push(hidden.range(spanTo - 2, spanTo));
          // [[Target|alias]] shows only the alias.
          if (pipe >= 0) decos.push(hidden.range(spanFrom + 2, spanFrom + 2 + pipe + 1));
        }
      }
    }
  }

  for (const { from, to } of view.visibleRanges) {
    syntaxTree(state).iterate({
      from,
      to,
      enter(node) {
        // Embed lines are drawn whole by their widget.
        const home = lineAt(node.from);
        if (embedLines.has(home.number) && node.to <= home.to) return false;
        const editing = active.has(lineAt(node.from).number);
        switch (node.name) {
          case "ATXHeading1":
          case "ATXHeading2":
          case "ATXHeading3":
          case "ATXHeading4":
          case "ATXHeading5":
          case "ATXHeading6": {
            const line = lineAt(node.from);
            if (line.number === 1) firstLineIsHeading = true;
            decos.push(headingLines[Number(node.name.slice(-1)) - 1].range(line.from));
            break;
          }
          case "HeaderMark":
            if (!editing) decos.push(hidden.range(node.from, withSpace(state, node.to)));
            break;
          case "EmphasisMark":
          case "StrikethroughMark":
            if (!editing) decos.push(hidden.range(node.from, node.to));
            break;
          case "CodeMark":
            if (!editing && node.node.parent?.name === "InlineCode") decos.push(hidden.range(node.from, node.to));
            break;
          case "Link": {
            // [text](url) reads as the text alone; the brackets and address hide like other marks.
            const url = node.node.getChild("URL");
            const marks = node.node.getChildren("LinkMark");
            if (!url || marks.length < 2) break;
            decos.push(linkMark(hrefFor(state.sliceDoc(url.from, url.to))).range(node.from, node.to));
            if (!editing) {
              decos.push(hidden.range(marks[0].from, marks[0].to));
              decos.push(hidden.range(marks[1].from, node.to));
            }
            break;
          }
          case "Autolink": {
            // <https://…> reads as the address without its angle brackets.
            const url = node.node.getChild("URL");
            if (!url) break;
            decos.push(linkMark(hrefFor(state.sliceDoc(url.from, url.to))).range(node.from, node.to));
            if (!editing) for (const m of node.node.getChildren("LinkMark")) decos.push(hidden.range(m.from, m.to));
            break;
          }
          case "URL": {
            // A bare address in the text. Inside a link or image the parent draws it.
            const parent = node.node.parent?.name;
            if (parent === "Link" || parent === "Autolink" || parent === "Image") break;
            decos.push(linkMark(hrefFor(state.sliceDoc(node.from, node.to))).range(node.from, node.to));
            break;
          }
          case "QuoteMark":
            decos.push(quoteLine.range(lineAt(node.from).from));
            if (!editing) decos.push(hidden.range(node.from, withSpace(state, node.to)));
            break;
          case "ListMark": {
            const item = node.node.parent;
            if (item?.parent?.name !== "BulletList") break;
            // The marker's space goes with it, so the text starts a fixed width in and wrapped lines
            // can hang under it. Nested items keep their leading spaces, so only top-level ones hang.
            if (lineAt(node.from).from === node.from) decos.push(itemLine.range(node.from));
            if (item.getChild("Task")) decos.push(hidden.range(node.from, withSpace(state, node.to)));
            else {
              decos.push(bullet.range(node.from, node.to));
              const space = withSpace(state, node.to);
              if (space > node.to) {
                decos.push(markerSpace.range(node.to, space));
                stepOver.push(markerSpace.range(node.to, space));
              }
            }
            break;
          }
          case "TaskMarker": {
            const checked = /x/i.test(state.sliceDoc(node.from, node.to));
            const textFrom = withSpace(state, node.to);
            decos.push(Decoration.replace({ widget: new CheckboxWidget(checked, node.from) }).range(node.from, node.to));
            if (textFrom > node.to) {
              decos.push(markerSpace.range(node.to, textFrom));
              stepOver.push(markerSpace.range(node.to, textFrom));
            }
            const lineEnd = lineAt(node.from).to;
            if (checked && lineEnd > textFrom) decos.push(doneText.range(textFrom, lineEnd));
            break;
          }
        }
      },
    });
  }
  // Like Apple Notes, the first line is the title even without a "#".
  if (!firstLineIsHeading) decos.push(headingLines[0].range(0));
  return {
    decorations: Decoration.set(decos, true),
    // Replaced ranges (hidden marks, bullets, checkboxes) and the spaces after markers are skipped by the cursor.
    atomic: Decoration.set([...decos.filter((d) => d.value.point), ...stepOver], true),
  };
}

const livePreviewPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    atomic: DecorationSet;

    constructor(view: EditorView) {
      ({ decorations: this.decorations, atomic: this.atomic } = build(view));
    }

    update(u: ViewUpdate) {
      if (u.docChanged || u.viewportChanged || u.selectionSet || u.focusChanged || u.state !== u.startState) {
        ({ decorations: this.decorations, atomic: this.atomic } = build(u.view));
      }
    }
  },
  { decorations: (v) => v.decorations },
);

export const livePreview = [
  wikiNotesField,
  livePreviewPlugin,
  // The cursor steps over bullets and checkboxes instead of into them.
  EditorView.atomicRanges.of((view) => view.plugin(livePreviewPlugin)?.atomic ?? Decoration.none),
];

// The click that follows a link: a [[wiki link]] opens its note here, a web link opens in a new tab.
// The notes field lives in livePreview so every editor has it.
export const linkClicks = [
  EditorView.domEventHandlers({
    click(e) {
      const link = (e.target as HTMLElement).closest?.(".cm-wikilink, .cm-link") as HTMLElement | null;
      if (!link) return false;
      const { note, href } = link.dataset;
      if (note) {
        e.preventDefault();
        openNote(note);
        return true;
      }
      if (href && !/^\s*javascript:/i.test(href)) {
        e.preventDefault();
        window.open(href, "_blank", "noopener,noreferrer");
        return true;
      }
      return false;
    },
  }),
];
