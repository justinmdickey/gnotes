// A kanban board kept as plain Markdown in a ```kanban fenced block: each "## Name" starts a
// column and each "- text" (or "- [ ] text") under it is a card. The board is drawn from the
// text, and every action writes the smallest change back: typing edits the card's characters,
// and dragging a card moves its one line, in one transaction.
import type { EditorState } from "@codemirror/state";
import { EditorView, WidgetType } from "@codemirror/view";
import { blockWidgets, caretOffset, insertBlock, keepFocus, makeEditable, placeCaret, replaceMinimal, topLevel, typed } from "./blockWidget";
import { iconSvg } from "./icons";

interface Card {
  /** The card's whole line. */
  from: number;
  to: number;
  textFrom: number;
  text: string;
  /** null for a plain card; for a task, whether it's done, and where its "x" or " " is. */
  done: boolean | null;
  mark: number;
}

interface Column {
  title: string;
  titleFrom: number;
  cards: Card[];
  /** End of the column's last card, or of its heading. */
  end: number;
}

interface Board {
  from: number;
  to: number;
  /** Start of the closing fence's line; null while the fence is still open. */
  close: number | null;
  columns: Column[];
}

const OPEN = /^\s{0,3}(`{3,}|~{3,})\s*kanban\s*$/;
const CLOSE = /^\s{0,3}(`{3,}|~{3,})\s*$/;
const HEADING = /^(\s{0,3}##(?:[ \t]+|$))(.*)$/;
const CARD = /^(\s*[-*+](?:[ \t]+|$))(\[([ xX])\](?:[ \t]+|$))?(.*)$/;

function parseBoard(lines: { text: string; from: number }[]): Board {
  const last = lines.at(-1)!;
  const closed = lines.length > 1 && CLOSE.test(last.text);
  const board: Board = { from: lines[0].from, to: last.from + last.text.length, close: closed ? last.from : null, columns: [] };
  for (const line of lines.slice(1, closed ? -1 : undefined)) {
    const h = HEADING.exec(line.text);
    if (h) {
      board.columns.push({ title: h[2], titleFrom: line.from + h[1].length, cards: [], end: line.from + line.text.length });
      continue;
    }
    const c = CARD.exec(line.text);
    const col = board.columns.at(-1);
    // Cards before the first column, and any other lines, stay in the text but aren't drawn.
    if (!c || !col) continue;
    const textFrom = line.from + c[1].length + (c[2]?.length ?? 0);
    const to = line.from + line.text.length;
    col.cards.push({ from: line.from, to, textFrom, text: c[4], done: c[2] ? c[3] !== " " : null, mark: line.from + c[1].length + 1 });
    col.end = to;
  }
  return board;
}

function linesOf(state: EditorState, from: number, to: number) {
  const out = [];
  for (let n = state.doc.lineAt(from).number; n <= state.doc.lineAt(to).number; n++) {
    const line = state.doc.line(n);
    out.push({ text: line.text, from: line.from });
  }
  return out;
}

const { extension: boardField, blockOf, domOf } = blockWidgets(
  (state) => topLevel(state, "FencedCode", (from) => OPEN.test(state.doc.lineAt(from).text)),
  (state, b, editable) => new BoardWidget(state.sliceDoc(b.from, b.to), editable),
);

function boardOf(view: EditorView, dom: HTMLElement): Board | null {
  const b = blockOf(view, dom);
  return b ? parseBoard(linesOf(view.state, b.from, b.to)) : null;
}

/** Text written where a card's or title's text starts, with a space after a bare "-" or "##". */
function textAt(view: EditorView, pos: number, old: string, text: string) {
  if (!old && text && !/\s/.test(view.state.sliceDoc(pos - 1, pos))) text = ` ${text}`;
  replaceMinimal(view, pos, old, text);
}

function writeCard(view: EditorView, dom: HTMLElement, col: number, card: number, text: string) {
  const c = boardOf(view, dom)?.columns[col]?.cards[card];
  if (c) textAt(view, c.textFrom, c.text, text);
}

function writeTitle(view: EditorView, dom: HTMLElement, col: number, text: string) {
  const c = boardOf(view, dom)?.columns[col];
  if (c) textAt(view, c.titleFrom, c.title, text);
}

/** Adds an empty card after card `after` (-1: at the column's end) and starts editing it. */
function addCard(view: EditorView, dom: HTMLElement, col: number, after = -1) {
  const board = boardOf(view, dom);
  const column = board?.columns[col];
  if (!board || !column) return;
  const prev = after < 0 ? column.cards.at(-1) : column.cards[after];
  const index = after < 0 ? column.cards.length : after + 1;
  const at = after < 0 ? column.end : prev!.to;
  view.dispatch({ changes: { from: at, insert: prev?.done != null ? "\n- [ ] " : "\n- " }, userEvent: "input" });
  editCard(view, board.from, col, index);
}

function deleteCard(view: EditorView, dom: HTMLElement, col: number, card: number) {
  const c = boardOf(view, dom)?.columns[col]?.cards[card];
  if (c) view.dispatch({ changes: { from: c.from - 1, to: c.to }, userEvent: "delete" });
}

function toggleCard(view: EditorView, dom: HTMLElement, col: number, card: number) {
  const c = boardOf(view, dom)?.columns[col]?.cards[card];
  if (c && c.done !== null) view.dispatch({ changes: { from: c.mark, to: c.mark + 1, insert: c.done ? " " : "x" }, userEvent: "input" });
}

function addColumn(view: EditorView, dom: HTMLElement) {
  const board = boardOf(view, dom);
  if (!board) return;
  let at: number;
  let insert: string;
  if (board.close === null) {
    at = board.to;
    insert = "\n\n## New column";
  } else {
    at = board.close;
    const prev = view.state.doc.lineAt(at - 1);
    insert = `${prev.length && prev.from !== board.from ? "\n" : ""}## New column\n`;
  }
  view.dispatch({ changes: { from: at, insert }, userEvent: "input" });
  const title = domOf(view, board.from, "cm-kanban-widget")?.querySelector<HTMLElement>(`.cm-kanban-title[data-col="${board.columns.length}"]`);
  if (title) placeCaret(title, true);
}

/** Moves a card's line to sit before card `before` of column `col` (or at its end), in one transaction. */
function moveCard(view: EditorView, dom: HTMLElement, from: { col: number; card: number }, to: { col: number; before: number }) {
  const board = boardOf(view, dom);
  const card = board?.columns[from.col]?.cards[from.card];
  const column = board?.columns[to.col];
  if (!card || !column) return;
  if (from.col === to.col && (to.before === from.card || to.before === from.card + 1)) return;
  const line = view.state.sliceDoc(card.from, card.to);
  const next = column.cards[to.before];
  const insert = next ? { from: next.from, insert: `${line}\n` } : { from: column.end, insert: `\n${line}` };
  const changes = [{ from: card.from - 1, to: card.to }, insert].sort((a, b) => a.from - b.from);
  view.dispatch({ changes, userEvent: "move" });
}

/** Shows the board as its Markdown, with the cursor in it. */
function editMarkdown(view: EditorView, dom: HTMLElement) {
  const b = blockOf(view, dom);
  if (!b) return;
  view.dispatch({ selection: { anchor: view.state.doc.lineAt(b.from).to }, scrollIntoView: true });
  view.focus();
}

const textEl = (dom: HTMLElement, col: number, card: number) => dom.querySelector<HTMLElement>(`.cm-kanban-card[data-col="${col}"][data-card="${card}"] .cm-kanban-text`);

/** Turns a card's text into an editing field and puts the caret in it. */
function startEditing(el: HTMLElement, offset?: number) {
  el.closest(".cm-kanban-card")!.classList.add("editing");
  makeEditable(el);
  placeCaret(el, false, offset);
}

function editCard(view: EditorView, from: number, col: number, card: number) {
  const dom = domOf(view, from, "cm-kanban-widget");
  const el = dom && textEl(dom, col, card);
  if (el) startEditing(el);
}

/** Set while a redraw blurs the card being edited, which isn't the editor leaving it. */
let redrawing = false;

/** Drags in progress, so a redraw from someone else's edit can call one off. */
const drags = new WeakMap<HTMLElement, () => void>();

/** Picks a card up and follows the pointer until it's dropped on a column. */
function beginDrag(card: HTMLElement, dom: HTMLElement, view: EditorView, x: number, y: number) {
  const rect = card.getBoundingClientRect();
  const from = { col: Number(card.dataset.col), card: Number(card.dataset.card) };
  const ghost = card.cloneNode(true) as HTMLElement;
  ghost.classList.add("cm-kanban-ghost");
  Object.assign(ghost.style, { left: `${rect.left}px`, top: `${rect.top}px`, width: `${rect.width}px` });
  const spot = document.createElement("div");
  spot.className = "cm-kanban-spot";
  spot.style.height = `${rect.height}px`;
  card.after(spot);
  card.classList.add("dragging");
  dom.classList.add("dragging");
  dom.append(ghost);
  const scroller = dom.querySelector<HTMLElement>(".cm-kanban")!;
  let target: { col: number; before: number } | null = { col: from.col, before: from.card };
  let px = x;
  let py = y;
  // Near the board's sides it scrolls, so far columns can be reached on a phone.
  let frame = 0;
  const tick = () => {
    const r = scroller.getBoundingClientRect();
    const step = px < r.left + 40 ? -8 : px > r.right - 40 ? 8 : 0;
    if (step) {
      scroller.scrollLeft += step;
      place();
    }
    frame = requestAnimationFrame(tick);
  };
  frame = requestAnimationFrame(tick);

  const place = () => {
    ghost.style.transform = `translate(${px - x}px, ${py - y}px) rotate(2deg)`;
    const col = [...dom.querySelectorAll<HTMLElement>(".cm-kanban-col")].find((c) => {
      const r = c.getBoundingClientRect();
      return px >= r.left && px <= r.right;
    });
    if (!col) {
      spot.remove();
      target = null;
      return;
    }
    const list = col.querySelector<HTMLElement>(".cm-kanban-cards")!;
    const next = [...list.querySelectorAll<HTMLElement>(".cm-kanban-card:not(.dragging)")].find((c) => {
      const r = c.getBoundingClientRect();
      return py < r.top + r.height / 2;
    });
    if (next) {
      if (spot.nextSibling !== next) list.insertBefore(spot, next);
    } else if (list.lastChild !== spot) list.append(spot);
    target = { col: Number(col.dataset.col), before: next ? Number(next.dataset.card) : list.querySelectorAll(".cm-kanban-card").length };
  };

  const end = (drop: boolean) => {
    cancelAnimationFrame(frame);
    ghost.remove();
    spot.remove();
    card.classList.remove("dragging");
    dom.classList.remove("dragging");
    drags.delete(dom);
    if (drop && target) moveCard(view, dom, from, target);
  };
  drags.set(dom, () => end(false));
  return {
    move(nx: number, ny: number) {
      px = nx;
      py = ny;
      place();
    },
    end,
  };
}

/**
 * A press on a card: a mouse drags it as soon as it moves; a finger holds still for a moment
 * first, so swiping still scrolls the board. A press that doesn't drag is a tap, which edits.
 */
function pressCard(e: PointerEvent, card: HTMLElement, dom: HTMLElement, view: EditorView) {
  if (e.button !== 0 || card.classList.contains("editing") || (e.target as Element).closest("button, .cm-checkbox")) return;
  const touch = e.pointerType !== "mouse";
  const sx = e.clientX;
  const sy = e.clientY;
  let drag: ReturnType<typeof beginDrag> | null = null;
  const timer = touch ? setTimeout(() => ((drag = beginDrag(card, dom, view, sx, sy)), navigator.vibrate?.(10)), 350) : undefined;
  const move = (ev: PointerEvent) => {
    if (ev.pointerId !== e.pointerId) return;
    if (drag) return drag.move(ev.clientX, ev.clientY);
    if (Math.hypot(ev.clientX - sx, ev.clientY - sy) > (touch ? 8 : 4)) {
      if (touch) return finish(false);
      drag = beginDrag(card, dom, view, sx, sy);
      drag.move(ev.clientX, ev.clientY);
    }
  };
  const finish = (drop: boolean) => {
    clearTimeout(timer);
    removeEventListener("pointermove", move);
    removeEventListener("pointerup", up);
    removeEventListener("pointercancel", cancel);
    if (!drag) return;
    // The click that ends a drag isn't a tap.
    card.dataset.dragged = "1";
    setTimeout(() => delete card.dataset.dragged, 0);
    drag.end(drop);
  };
  const up = (ev: PointerEvent) => ev.pointerId === e.pointerId && finish(true);
  const cancel = (ev: PointerEvent) => ev.pointerId === e.pointerId && finish(false);
  addEventListener("pointermove", move);
  addEventListener("pointerup", up);
  addEventListener("pointercancel", cancel);
}

function button(cls: string, label: string, text: string, run: () => void) {
  const b = document.createElement("button");
  b.type = "button";
  b.className = cls;
  b.title = label;
  b.setAttribute("aria-label", label);
  b.append(iconSvg(cls === "cm-kanban-del" ? "close" : cls === "cm-kanban-md" ? "code" : "plus", cls === "cm-kanban-del" ? 14 : 16));
  if (text) b.append(text);
  keepFocus(b);
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    run();
  });
  return b;
}

class BoardWidget extends WidgetType {
  constructor(
    readonly src: string,
    readonly canEdit: boolean,
  ) {
    super();
  }

  eq(other: BoardWidget) {
    return other.src === this.src && other.canEdit === this.canEdit;
  }

  private model() {
    let at = 0;
    const lines = this.src.split("\n").map((text) => {
      const line = { text, from: at };
      at += text.length + 1;
      return line;
    });
    const { columns } = parseBoard(lines);
    const shape = `${this.canEdit}:${columns.map((c) => c.cards.map((k) => (k.done === null ? "-" : "t")).join("")).join("|")}`;
    return { columns, shape };
  }

  toDOM(view: EditorView) {
    const dom = document.createElement("div");
    dom.className = "cm-kanban-widget";
    // A click here is the widget's; the note's "tap below the text" handler mustn't take focus after it.
    dom.addEventListener("click", (e) => e.stopPropagation());
    // A finger dragging a card mustn't scroll the page; this has to be listening before the touch starts.
    dom.addEventListener("touchmove", (e) => dom.classList.contains("dragging") && e.preventDefault(), { passive: false });
    this.render(dom, view);
    return dom;
  }

  updateDOM(dom: HTMLElement, view: EditorView) {
    const { columns, shape } = this.model();
    if (dom.dataset.shape !== shape) {
      drags.get(dom)?.();
      // Cards or columns came or went: redraw, keeping the caret where it was.
      const el = document.activeElement as HTMLElement | null;
      const active = el && dom.contains(el) ? { col: el.dataset.col, card: el.closest<HTMLElement>(".cm-kanban-card")?.dataset.card, title: el.classList.contains("cm-kanban-title"), offset: caretOffset(el) } : null;
      // Blur first: a focused element taken out of the page hands focus to the editor, which keeps it.
      if (active) {
        redrawing = true;
        el!.blur();
        redrawing = false;
      }
      this.render(dom, view);
      if (active?.title) {
        const t = dom.querySelector<HTMLElement>(`.cm-kanban-title[data-col="${active.col}"]`);
        if (t) placeCaret(t, false, active.offset);
      } else if (active?.card !== undefined) {
        const t = textEl(dom, Number(active.col), Number(active.card));
        if (t) startEditing(t, active.offset);
      }
      return true;
    }
    const sync = (el: HTMLElement, text: string) => {
      if (el !== document.activeElement) {
        if (el.textContent !== text) el.textContent = text;
      } else if (typed(el) !== text) {
        const offset = caretOffset(el);
        el.textContent = text;
        placeCaret(el, false, offset);
      }
    };
    columns.forEach((col, i) => {
      sync(dom.querySelector<HTMLElement>(`.cm-kanban-title[data-col="${i}"]`)!, col.title);
      col.cards.forEach((card, k) => {
        sync(textEl(dom, i, k)!, card.text);
        dom.querySelector(`.cm-kanban-card[data-col="${i}"][data-card="${k}"]`)!.classList.toggle("done", !!card.done);
      });
    });
    return true;
  }

  private render(dom: HTMLElement, view: EditorView) {
    const { columns, shape } = this.model();
    const editable = this.canEdit;
    dom.dataset.shape = shape;
    dom.classList.toggle("editable", editable);
    const board = document.createElement("div");
    board.className = "cm-kanban";
    columns.forEach((col, i) => {
      const section = document.createElement("section");
      section.className = "cm-kanban-col";
      section.dataset.col = String(i);
      const head = document.createElement("div");
      head.className = "cm-kanban-head";
      const title = document.createElement("div");
      title.className = "cm-kanban-title";
      title.dataset.col = String(i);
      title.textContent = col.title;
      title.setAttribute("aria-label", "Column name");
      const count = document.createElement("span");
      count.className = "cm-kanban-count";
      count.textContent = String(col.cards.length);
      head.append(title, count);
      const list = document.createElement("div");
      list.className = "cm-kanban-cards";
      col.cards.forEach((c, k) => list.append(this.card(dom, view, c, i, k)));
      section.append(head, list);
      if (editable) {
        makeEditable(title);
        title.addEventListener("input", () => writeTitle(view, dom, i, typed(title)));
        title.addEventListener("keydown", (e) => {
          if ((e.key === "Enter" && !e.isComposing) || e.key === "Escape") {
            e.preventDefault();
            title.blur();
          }
        });
        section.append(button("cm-kanban-add", "Add card", "Add card", () => addCard(view, dom, i)));
      }
      board.append(section);
    });
    if (editable) {
      const end = document.createElement("div");
      end.className = "cm-kanban-end";
      end.append(button("cm-kanban-add-col", "Add column", "Add column", () => addColumn(view, dom)), button("cm-kanban-md", "Edit as Markdown", "", () => editMarkdown(view, dom)));
      board.append(end);
    }
    // Keep the sideways scroll position across a redraw.
    const left = dom.querySelector(".cm-kanban")?.scrollLeft ?? 0;
    dom.replaceChildren(board);
    board.scrollLeft = left;
  }

  private card(dom: HTMLElement, view: EditorView, c: Card, col: number, k: number) {
    const card = document.createElement("div");
    card.className = "cm-kanban-card";
    card.classList.toggle("done", !!c.done);
    card.dataset.col = String(col);
    card.dataset.card = String(k);
    if (c.done !== null) {
      const box = document.createElement("span");
      box.className = "cm-checkbox";
      box.setAttribute("role", "checkbox");
      box.setAttribute("aria-checked", String(c.done));
      if (this.canEdit) box.addEventListener("click", () => toggleCard(view, dom, col, k));
      card.append(box);
    }
    const text = document.createElement("div");
    text.className = "cm-kanban-text";
    text.dataset.col = String(col);
    text.textContent = c.text;
    card.append(text);
    if (!this.canEdit) return card;

    card.append(button("cm-kanban-del", "Delete card", "", () => deleteCard(view, dom, col, k)));
    card.addEventListener("pointerdown", (e) => pressCard(e, card, dom, view));
    card.addEventListener("contextmenu", (e) => e.preventDefault());
    card.addEventListener("click", (e) => {
      if (card.dataset.dragged || card.classList.contains("editing") || (e.target as Element).closest("button, .cm-checkbox")) return;
      // The caret goes where the card was tapped.
      const at = document.caretRangeFromPoint?.(e.clientX, e.clientY);
      startEditing(text, at && text.contains(at.startContainer) ? at.startOffset : undefined);
    });
    text.addEventListener("input", () => writeCard(view, dom, col, k, typed(text)));
    text.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.isComposing) {
        e.preventDefault();
        // Enter starts the next card, like a list; on an empty card it's done.
        if (typed(text).trim()) addCard(view, dom, col, k);
        else text.blur();
      } else if (e.key === "Escape") text.blur();
    });
    text.addEventListener("blur", () => {
      card.classList.remove("editing");
      text.removeAttribute("contenteditable");
      // A card left empty goes away.
      if (!redrawing && text.isConnected && !typed(text).trim()) deleteCard(view, dom, col, k);
    });
    return card;
  }

  get estimatedHeight() {
    return 240;
  }

  ignoreEvent() {
    return true;
  }
}

export const kanban = boardField;

const template = "```kanban\n## To do\n- \n\n## Doing\n\n## Done\n```";

/** Puts a new board with To do, Doing and Done at the cursor, ready to type its first card. */
export function insertBoard(view: EditorView) {
  const from = insertBlock(view, template);
  editCard(view, from, 0, 0);
  return true;
}
