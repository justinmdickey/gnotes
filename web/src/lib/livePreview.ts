// Makes Markdown read like formatted text: marks are hidden except on lines being edited,
// bullets and checkboxes render as widgets, and the first line is styled as the title.
import { syntaxTree } from "@codemirror/language";
import type { EditorState, Range } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin, type ViewUpdate, WidgetType } from "@codemirror/view";
import { iconSvg } from "./icons";
import { EMBED, attachmentMeta, attachmentUrl, isTranscribing, onTranscribingChange } from "./attachments";

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
  body.append(title, bar);
  wrap.append(play, body, time, status);

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

/** Shown on a blank first line so it's clear the note's title goes there. */
class TitleHint extends WidgetType {
  constructor(readonly addLine: boolean) {
    super();
  }

  eq(other: TitleHint) {
    return other.addLine === this.addLine;
  }

  toDOM(view: EditorView) {
    const el = document.createElement("span");
    el.className = this.addLine ? "cm-title-hint add" : "cm-title-hint";
    el.textContent = this.addLine ? "Add a title" : "Title";
    if (this.addLine) {
      // The note starts with a photo or memo: make a title line above it.
      el.addEventListener("mousedown", (e) => {
        e.preventDefault();
        if (!view.state.facet(EditorView.editable)) return;
        view.dispatch({ changes: { from: 0, insert: "\n" }, selection: { anchor: 0 }, userEvent: "input" });
        view.focus();
      });
    }
    return el;
  }

  ignoreEvent() {
    return !this.addLine;
  }
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
    decos.push(Decoration.widget({ widget: new TitleHint(true), side: -1 }).range(0));
  } else if (first.length === 0 && state.doc.lines > 1 && !active.has(1)) {
    decos.push(Decoration.widget({ widget: new TitleHint(false), side: 1 }).range(0));
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
          case "QuoteMark":
            decos.push(quoteLine.range(lineAt(node.from).from));
            if (!editing) decos.push(hidden.range(node.from, withSpace(state, node.to)));
            break;
          case "ListMark": {
            const item = node.node.parent;
            if (item?.parent?.name !== "BulletList") break;
            if (item.getChild("Task")) decos.push(hidden.range(node.from, withSpace(state, node.to)));
            else decos.push(bullet.range(node.from, node.to));
            break;
          }
          case "TaskMarker": {
            const checked = /x/i.test(state.sliceDoc(node.from, node.to));
            decos.push(Decoration.replace({ widget: new CheckboxWidget(checked, node.from) }).range(node.from, node.to));
            const textFrom = withSpace(state, node.to);
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
    // Only replaced ranges (hidden marks, bullets, checkboxes) are skipped by the cursor.
    atomic: Decoration.set(decos.filter((d) => d.value.point), true),
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
      if (u.docChanged || u.viewportChanged || u.selectionSet || u.focusChanged) {
        ({ decorations: this.decorations, atomic: this.atomic } = build(u.view));
      }
    }
  },
  { decorations: (v) => v.decorations },
);

export const livePreview = [
  livePreviewPlugin,
  // The cursor steps over bullets and checkboxes instead of into them.
  EditorView.atomicRanges.of((view) => view.plugin(livePreviewPlugin)?.atomic ?? Decoration.none),
];
