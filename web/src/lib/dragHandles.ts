// A grip to the left of a block (a line, a list item with the items under it, or a whole code
// block) that drags it somewhere else in the note. It shows beside the line you point at, or on
// touch screens beside the line with the cursor. A move is one change, so one undo puts it back.
import { syntaxTree } from "@codemirror/language";
import { type EditorState, StateEffect, StateField } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";
import { EMBED } from "./attachments";
import { blockOf } from "./format";
import { iconSvg } from "./icons";

/** A run of whole lines, by line number. */
interface Block {
  first: number;
  last: number;
}

const indentOf = (text: string) => text.match(/^\s*/)![0].length;

function fencedAt(state: EditorState, pos: number): Block | null {
  for (let node: { name: string; from: number; to: number; parent: unknown } | null = syntaxTree(state).resolveInner(pos, 1); node; node = node.parent as typeof node) {
    if (node.name === "FencedCode") return { first: state.doc.lineAt(node.from).number, last: state.doc.lineAt(node.to).number };
  }
  return null;
}

/** The block a line belongs to. The title line has none: it stays first. */
export function blockAt(state: EditorState, lineNo: number): Block | null {
  if (lineNo <= 1) return null;
  const line = state.doc.line(lineNo);
  const code = fencedAt(state, line.from);
  if (code) return code.first > 1 ? code : null;
  if (!line.text.trim()) return null;
  const kind = blockOf(line.text).block;
  if (kind !== "check" && kind !== "bullet" && kind !== "number") return { first: lineNo, last: lineNo };
  // A list item carries the items nested under it.
  const indent = indentOf(line.text);
  let last = lineNo;
  while (last < state.doc.lines) {
    const next = state.doc.line(last + 1).text;
    if (!next.trim() || indentOf(next) <= indent) break;
    last++;
  }
  return { first: lineNo, last };
}

/** Where a block can land: before another block or a blank line, or at the end (lines + 1). */
function dropPoints(state: EditorState, moving: Block): number[] {
  const points: number[] = [];
  for (let n = 2; n <= state.doc.lines; ) {
    if (n <= moving.first || n > moving.last) points.push(n);
    n = (blockAt(state, n)?.last ?? n) + 1;
  }
  points.push(state.doc.lines + 1);
  return points;
}

/** Moves a block so it starts at `before` (a line number, or lines + 1 for the end). */
export function moveBlock(view: EditorView, moving: Block, before: number): boolean {
  const { state } = view;
  const { doc } = state;
  if (before === moving.first || before === moving.last + 1) return false;
  const from = doc.line(moving.first).from;
  const to = doc.line(moving.last).to;
  const text = doc.sliceString(from, to);
  const atEnd = moving.last === doc.lines;
  // Take the block out with one of its line breaks, so no blank line is left behind.
  const remove = atEnd ? { from: from - 1, to } : { from, to: to + 1 };
  const insert = before > doc.lines ? { from: doc.length, insert: `\n${text}` } : { from: doc.line(before).from, insert: `${text}\n` };
  const changes = state.changes([remove, insert]);
  const start = changes.mapPos(insert.from, before > doc.lines ? 1 : -1) + (before > doc.lines ? 1 : 0);
  view.dispatch({ changes, selection: { anchor: Math.min(start, changes.newLength) }, scrollIntoView: true, userEvent: "move.drag" });
  return true;
}

const setDragging = StateEffect.define<Block | null>();
const dimmed = Decoration.line({ class: "cm-drag-source" });

/** The block being dragged, drawn faded where it was. */
const dragging = StateField.define<Block | null>({
  create: () => null,
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setDragging)) return e.value;
    return value && tr.docChanged ? null : value;
  },
  provide: (f) =>
    EditorView.decorations.from(f, (b) => (view) => {
      if (!b) return Decoration.none;
      const ranges = [];
      for (let n = b.first; n <= Math.min(b.last, view.state.doc.lines); n++) ranges.push(dimmed.range(view.state.doc.line(n).from));
      return Decoration.set(ranges);
    }),
});

const touch = () => matchMedia("(hover: none)").matches;

/** The nearest ancestor that scrolls, so a drag near its edges can scroll the note. */
function scrollParent(el: HTMLElement): HTMLElement | null {
  for (let p = el.parentElement; p; p = p.parentElement) {
    if (/auto|scroll/.test(getComputedStyle(p).overflowY) && p.scrollHeight > p.clientHeight) return p;
  }
  return null;
}

const handles = ViewPlugin.fromClass(
  class {
    grip: HTMLButtonElement;
    marker: HTMLDivElement;
    block: Block | null = null;
    pointerLine = 0;
    drag: { block: Block; before: number; y: number; scroller: HTMLElement | null; frame: number } | null = null;

    constructor(readonly view: EditorView) {
      this.grip = document.createElement("button");
      this.grip.className = "cm-drag-grip";
      this.grip.type = "button";
      this.grip.tabIndex = -1;
      this.grip.setAttribute("aria-label", "Drag to move");
      this.grip.title = "Drag to move";
      this.grip.append(iconSvg("grip"));
      this.marker = document.createElement("div");
      this.marker.className = "cm-drop-marker";
      view.dom.append(this.grip, this.marker);

      this.grip.addEventListener("mousedown", (e) => e.preventDefault());
      this.grip.addEventListener("pointerdown", this.start);
      view.dom.addEventListener("mousemove", this.hover);
      view.dom.addEventListener("mouseleave", this.leave);
      this.place();
    }

    hover = (e: MouseEvent) => {
      if (this.drag || e.target === this.grip || this.grip.contains(e.target as Node)) return;
      const r = this.view.contentDOM.getBoundingClientRect();
      const pos = this.view.posAtCoords({ x: Math.max(r.left + 1, Math.min(e.clientX, r.right - 1)), y: e.clientY }, false);
      const n = this.view.state.doc.lineAt(pos).number;
      if (n !== this.pointerLine) {
        this.pointerLine = n;
        this.place();
      }
    };

    leave = (e: MouseEvent) => {
      if (this.drag || this.view.dom.contains(e.relatedTarget as Node)) return;
      this.pointerLine = 0;
      this.place();
    };

    update(u: ViewUpdate) {
      if (this.drag) return;
      if (u.docChanged || u.selectionSet || u.focusChanged || u.geometryChanged || u.viewportChanged) this.place();
    }

    /** Puts the grip beside the block that's pointed at, or has the cursor on touch screens. */
    place() {
      const { view } = this;
      const { state } = view;
      const typing = touch() && view.hasFocus;
      const lineNo = typing ? state.doc.lineAt(state.selection.main.head).number : this.pointerLine;
      const block = lineNo && state.facet(EditorView.editable) ? blockAt(state, lineNo) : null;
      this.block = block;
      this.grip.classList.toggle("shown", !!block);
      if (!block) return;
      view.requestMeasure({
        read: () => {
          const line = state.doc.line(block.first);
          const text = line.from + (EMBED.test(line.text) ? 0 : indentOf(line.text));
          const at = view.coordsAtPos(text, 1) ?? view.coordsAtPos(line.from, 1);
          const box = view.dom.getBoundingClientRect();
          const content = view.contentDOM.getBoundingClientRect();
          if (!at) return null;
          // The first row of the line: a heading's row is taller than body text.
          const row = Math.min(at.bottom - at.top, view.lineBlockAt(line.from).height);
          return { top: at.top - box.top + row / 2, left: Math.max(content.left, at.left) - box.left };
        },
        write: (m) => {
          if (!m) return void this.grip.classList.remove("shown");
          this.grip.style.top = `${m.top}px`;
          this.grip.style.left = `${m.left}px`;
        },
      });
    }

    start = (e: PointerEvent) => {
      if (!this.block || e.button !== 0) return;
      e.preventDefault();
      e.stopPropagation();
      this.grip.setPointerCapture(e.pointerId);
      this.drag = { block: this.block, before: this.block.first, y: e.clientY, scroller: scrollParent(this.view.dom), frame: 0 };
      this.view.dom.classList.add("cm-drag-active");
      this.view.dispatch({ effects: setDragging.of(this.block) });
      this.grip.addEventListener("pointermove", this.move);
      this.grip.addEventListener("pointerup", this.end);
      this.grip.addEventListener("pointercancel", this.cancel);
      this.drag.frame = requestAnimationFrame(this.autoscroll);
      this.track();
    };

    move = (e: PointerEvent) => {
      if (!this.drag) return;
      this.drag.y = e.clientY;
      this.track();
    };

    /** Works out where the block would land for the pointer's height, and draws the marker there. */
    track() {
      const { view, drag } = this;
      if (!drag) return;
      const { state } = view;
      const docY = drag.y - view.documentTop;
      // The drop point whose top edge is nearest the pointer.
      const edge = (n: number) => (n > state.doc.lines ? view.lineBlockAt(state.doc.length).bottom : view.lineBlockAt(state.doc.line(n).from).top);
      let best = drag.block.first;
      let gap = Infinity;
      for (const n of dropPoints(state, drag.block)) {
        const d = Math.abs(edge(n) - docY);
        if (d < gap) [best, gap] = [n, d];
      }
      drag.before = best;
      const box = view.dom.getBoundingClientRect();
      const content = view.contentDOM.getBoundingClientRect();
      const style = getComputedStyle(view.contentDOM);
      const y = edge(drag.before);
      this.marker.style.top = `${y + view.documentTop - box.top - 1}px`;
      this.marker.style.left = `${content.left - box.left + parseFloat(style.paddingLeft)}px`;
      this.marker.style.width = `${content.width - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight)}px`;
      const still = drag.before === drag.block.first || drag.before === drag.block.last + 1;
      this.marker.classList.toggle("shown", !still);
      this.grip.style.top = `${drag.y - box.top}px`;
    }

    /** Near the top or bottom of the note's scroll area, a drag scrolls it. */
    autoscroll = () => {
      const { drag } = this;
      if (!drag) return;
      const s = drag.scroller;
      if (s) {
        const r = s.getBoundingClientRect();
        const edge = 56;
        const step = drag.y < r.top + edge ? -Math.ceil((r.top + edge - drag.y) / 4) : drag.y > r.bottom - edge ? Math.ceil((drag.y - (r.bottom - edge)) / 4) : 0;
        if (step) {
          s.scrollTop += step;
          this.track();
        }
      }
      drag.frame = requestAnimationFrame(this.autoscroll);
    };

    end = () => {
      const drag = this.finish();
      if (drag) moveBlock(this.view, drag.block, drag.before);
      this.place();
    };

    cancel = () => {
      this.finish();
      this.place();
    };

    finish(redraw = true) {
      const { drag } = this;
      if (!drag) return null;
      cancelAnimationFrame(drag.frame);
      this.drag = null;
      this.grip.removeEventListener("pointermove", this.move);
      this.grip.removeEventListener("pointerup", this.end);
      this.grip.removeEventListener("pointercancel", this.cancel);
      this.marker.classList.remove("shown");
      this.view.dom.classList.remove("cm-drag-active");
      if (redraw) this.view.dispatch({ effects: setDragging.of(null) });
      return drag;
    }

    destroy() {
      this.finish(false);
      this.grip.remove();
      this.marker.remove();
      this.view.dom.removeEventListener("mousemove", this.hover);
      this.view.dom.removeEventListener("mouseleave", this.leave);
    }
  },
);

export const dragHandles = [dragging, handles];
