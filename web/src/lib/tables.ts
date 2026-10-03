// GFM pipe tables drawn as a grid whose cells are edited in place. Each cell edit changes only
// the characters that differ inside that cell, and adding or removing rows and columns inserts or
// deletes just those pipes and cells, so edits elsewhere in the table survive.
import type { EditorState } from "@codemirror/state";
import { EditorView, WidgetType } from "@codemirror/view";
import { blockWidgets, caretOffset, insertBlock, keepFocus, makeEditable, placeCaret, replaceMinimal, topLevel, typed } from "./blockWidget";
import { iconSvg, type IconName } from "./icons";

interface Cell {
  /** The cell's text, trimmed, as it is in the Markdown (pipes escaped). */
  from: number;
  to: number;
  raw: string;
  /** Between its pipes: segFrom is just after the opening pipe (or the line start), segTo the closing pipe (or line end). */
  segFrom: number;
  segTo: number;
  open: boolean;
  closed: boolean;
}

interface Row {
  from: number;
  to: number;
  cells: Cell[];
}

interface Table {
  from: number;
  to: number;
  header: Row;
  delimiter: Row;
  rows: Row[];
}

/** Splits one table line at its unescaped pipes. */
function parseRow(text: string, offset: number): Row {
  const pipes: number[] = [];
  for (let i = 0; i < text.length; i++) {
    if (text[i] === "\\") i++;
    else if (text[i] === "|") pipes.push(i);
  }
  const start = text.length - text.trimStart().length;
  const end = text.trimEnd().length;
  const lead = pipes[0] === start;
  const trail = pipes.length > (lead ? 1 : 0) && pipes.at(-1) === end - 1;
  const bounds = [lead ? pipes[0] : -1, ...pipes.slice(lead ? 1 : 0, trail ? -1 : undefined), trail ? pipes.at(-1)! : text.length];
  const cells: Cell[] = [];
  for (let k = 0; k + 1 < bounds.length; k++) {
    const segFrom = bounds[k] + 1;
    const segTo = bounds[k + 1];
    const seg = text.slice(segFrom, segTo);
    const raw = seg.trim();
    const lpad = raw ? seg.length - seg.trimStart().length : Math.min(1, seg.length);
    cells.push({
      from: offset + segFrom + lpad,
      to: offset + segFrom + lpad + raw.length,
      raw,
      segFrom: offset + segFrom,
      segTo: offset + segTo,
      open: bounds[k] >= 0,
      closed: segTo < text.length,
    });
  }
  return { from: offset, to: offset + text.length, cells };
}

function parseTable(state: EditorState, from: number, to: number): Table | null {
  const first = state.doc.lineAt(from).number;
  const last = state.doc.lineAt(to).number;
  if (last - first < 1) return null;
  const rows: Row[] = [];
  for (let n = first; n <= last; n++) {
    const line = state.doc.line(n);
    rows.push(parseRow(line.text, line.from));
  }
  const [header, delimiter, ...body] = rows;
  return { from, to, header, delimiter, rows: body };
}

const unescape = (raw: string) => raw.replace(/\\\|/g, "|");
const escape = (text: string) => text.trim().replace(/\\?\|/g, "\\|");

const { extension: tableField, blockOf, domOf } = blockWidgets(
  (state) => topLevel(state, "Table"),
  (state, b, editable) => new TableWidget(state.sliceDoc(b.from, b.to), editable),
);

/** The table a widget stands for, parsed from the current text. */
function tableOf(view: EditorView, dom: HTMLElement): Table | null {
  const b = blockOf(view, dom);
  return b ? parseTable(view.state, b.from, b.to) : null;
}

/** Row r of a table, where -1 is the header. */
const rowAt = (t: Table, r: number) => (r < 0 ? t.header : t.rows[r]);

function cellEl(dom: HTMLElement, r: number, c: number) {
  return dom.querySelector<HTMLElement>(`.cm-table-cell[data-r="${r}"][data-c="${c}"]`);
}

function focusCell(view: EditorView, from: number, r: number, c: number, selectAll = false) {
  const dom = domOf(view, from, "cm-table-widget");
  const el = dom && cellEl(dom, r, c);
  if (el) placeCaret(el, selectAll);
}

/** Writes a cell's typed text back into its row, adding cells the row is missing. */
function writeCell(view: EditorView, dom: HTMLElement, r: number, c: number, text: string) {
  const t = tableOf(view, dom);
  const row = t && rowAt(t, r);
  if (!row) return;
  const raw = escape(text);
  const cell = row.cells[c];
  if (cell) return replaceMinimal(view, cell.from, cell.raw, raw);
  // A short row: fill in the cells up to this one at its end.
  const lastCell = row.cells.at(-1);
  let insert = lastCell && !lastCell.closed ? " |" : "";
  if (!row.cells.length) insert = "|";
  for (let k = row.cells.length; k <= c; k++) insert += k === c ? ` ${raw} |` : "  |";
  view.dispatch({ changes: { from: row.to, insert }, userEvent: "input.type" });
}

function addRow(view: EditorView, dom: HTMLElement, after: number) {
  const t = tableOf(view, dom);
  if (!t) return;
  const at = after < 0 ? t.delimiter.to : rowAt(t, after)?.to ?? t.to;
  view.dispatch({ changes: { from: at, insert: `\n|${"  |".repeat(t.header.cells.length)}` }, userEvent: "input" });
  return t.from;
}

function deleteRow(view: EditorView, dom: HTMLElement, r: number) {
  const t = tableOf(view, dom);
  const row = t?.rows[r];
  if (!row) return;
  view.dispatch({ changes: { from: row.from - 1, to: row.to }, userEvent: "delete" });
  return t.from;
}

function addColumn(view: EditorView, dom: HTMLElement, after: number) {
  const t = tableOf(view, dom);
  if (!t) return;
  const changes = [t.header, t.delimiter, ...t.rows].flatMap((row) => {
    const cell = row.cells[after];
    if (!cell) return [];
    const filler = row === t.delimiter ? " --- " : "  ";
    return [{ from: cell.segTo, insert: cell.closed ? `|${filler}` : ` |${filler}|` }];
  });
  view.dispatch({ changes, userEvent: "input" });
  return t.from;
}

function deleteColumn(view: EditorView, dom: HTMLElement, c: number) {
  const t = tableOf(view, dom);
  if (!t || t.header.cells.length < 2) return;
  const changes = [t.header, t.delimiter, ...t.rows].flatMap((row) => {
    const cell = row.cells[c];
    if (!cell) return [];
    return [cell.open ? { from: cell.segFrom - 1, to: cell.segTo } : { from: cell.segFrom, to: cell.segTo + (cell.closed ? 1 : 0) }];
  });
  view.dispatch({ changes, userEvent: "delete" });
  return t.from;
}

/** Shows the table as its Markdown, with the cursor in it. */
function editMarkdown(view: EditorView, dom: HTMLElement) {
  const t = tableOf(view, dom);
  if (!t) return;
  view.dispatch({ selection: { anchor: t.header.to }, scrollIntoView: true });
  view.focus();
}

/** Which cell has focus in a table widget, if any. */
function activeCell(dom: HTMLElement): { r: number; c: number; offset?: number } | null {
  const el = document.activeElement;
  if (!(el instanceof HTMLElement) || !el.classList.contains("cm-table-cell") || !dom.contains(el)) return null;
  return { r: Number(el.dataset.r), c: Number(el.dataset.c), offset: caretOffset(el) };
}

function tool(icon: IconName, label: string, text: string, run: () => void) {
  const b = document.createElement("button");
  b.type = "button";
  b.title = label;
  b.setAttribute("aria-label", label);
  b.append(iconSvg(icon, 14));
  if (text) b.append(text);
  keepFocus(b);
  b.addEventListener("click", run);
  return b;
}

class TableWidget extends WidgetType {
  constructor(
    readonly src: string,
    readonly canEdit: boolean,
  ) {
    super();
  }

  eq(other: TableWidget) {
    return other.src === this.src && other.canEdit === this.canEdit;
  }

  /** The table's text, laid out as rows of display text. */
  private model() {
    const lines = this.src.split("\n").map((l) => parseRow(l, 0));
    const [header, delimiter, ...rows] = lines;
    const cols = header.cells.length;
    const align = delimiter?.cells.map(({ raw }) => (raw.startsWith(":") && raw.endsWith(":") ? "center" : raw.endsWith(":") ? "right" : "")) ?? [];
    const texts = [header, ...rows].map((row) => Array.from({ length: cols }, (_, c) => unescape(row.cells[c]?.raw ?? "")));
    return { cols, align, texts };
  }

  toDOM(view: EditorView) {
    const dom = document.createElement("div");
    dom.className = "cm-table-widget";
    // A click here is the widget's; the note's "tap below the text" handler mustn't take focus after it.
    dom.addEventListener("click", (e) => e.stopPropagation());
    this.render(dom, view);
    // The header isn't a row to delete, and the last column stays.
    dom.addEventListener("focusin", () => {
      const button = (label: string) => dom.querySelector<HTMLButtonElement>(`.cm-table-tools [aria-label='${label}']`);
      const row = button("Delete row");
      const col = button("Delete column");
      if (row) row.disabled = activeCell(dom)?.r === -1;
      if (col) col.disabled = (dom.querySelector("tr")?.children.length ?? 0) < 2;
    });
    return dom;
  }

  updateDOM(dom: HTMLElement, view: EditorView) {
    const { cols, align, texts } = this.model();
    const cells = dom.querySelectorAll<HTMLElement>(".cm-table-cell");
    const sameShape = dom.dataset.editable === String(this.canEdit) && dom.dataset.shape === `${texts.length}x${cols}`;
    if (!sameShape) {
      // Rows or columns came or went: redraw, keeping the caret in the cell it was in.
      const active = activeCell(dom);
      // Blur first: a focused cell taken out of the page hands focus to the editor, which keeps it.
      if (active) (document.activeElement as HTMLElement).blur();
      this.render(dom, view);
      const el = active && cellEl(dom, active.r, active.c);
      if (el) placeCaret(el, false, active.offset);
      return true;
    }
    for (const el of cells) {
      const r = Number(el.dataset.r) + 1;
      const c = Number(el.dataset.c);
      const text = texts[r][c];
      el.style.textAlign = align[c] ?? "";
      if (el === document.activeElement) {
        // Leave the cell being typed in alone unless someone else changed it.
        if (escape(typed(el)) !== escape(text)) {
          const offset = caretOffset(el);
          el.textContent = text;
          placeCaret(el, false, offset);
        }
      } else if (el.textContent !== text) el.textContent = text;
    }
    return true;
  }

  private render(dom: HTMLElement, view: EditorView) {
    const { cols, align, texts } = this.model();
    dom.dataset.shape = `${texts.length}x${cols}`;
    dom.dataset.editable = String(this.canEdit);
    dom.classList.toggle("editable", this.canEdit);
    dom.replaceChildren();
    const scroll = document.createElement("div");
    scroll.className = "cm-table-scroll";
    const table = document.createElement("table");
    table.className = "cm-table";
    texts.forEach((row, i) => {
      const r = i - 1;
      const tr = document.createElement("tr");
      row.forEach((text, c) => {
        const td = document.createElement(r < 0 ? "th" : "td");
        const el = document.createElement("div");
        el.className = "cm-table-cell";
        el.dataset.r = String(r);
        el.dataset.c = String(c);
        el.textContent = text;
        el.style.textAlign = align[c] ?? "";
        el.setAttribute("aria-label", r < 0 ? `Column ${c + 1} heading` : `Row ${r + 1}, column ${c + 1}`);
        if (this.canEdit) this.wireCell(el, dom, view, r, c, texts.length - 2, cols);
        td.append(el);
        tr.append(td);
      });
      (r < 0 ? (table.createTHead() as HTMLElement) : (table.tBodies[0] ?? table.createTBody())).append(tr);
    });
    scroll.append(table);
    dom.append(scroll);
    if (!this.canEdit) return;

    // Row and column tools, for the cell being edited.
    const tools = document.createElement("div");
    tools.className = "cm-table-tools";
    const at = () => activeCell(dom) ?? { r: -1, c: 0 };
    const then = (from: number | undefined, r: number, c: number) => from !== undefined && focusCell(view, from, r, c);
    tools.append(
      tool("plus", "Add row", "Row", () => {
        const { r, c } = at();
        then(addRow(view, dom, r), r + 1, c);
      }),
      tool("plus", "Add column", "Column", () => {
        const { r, c } = at();
        then(addColumn(view, dom, c), r, c + 1);
      }),
      tool("trash", "Delete row", "Row", () => {
        const { r, c } = at();
        if (r >= 0) then(deleteRow(view, dom, r), Math.min(r, texts.length - 3), c);
      }),
      tool("trash", "Delete column", "Column", () => {
        const { r, c } = at();
        then(deleteColumn(view, dom, c), r, Math.max(0, c - 1));
      }),
      tool("code", "Edit as Markdown", "", () => editMarkdown(view, dom)),
    );
    dom.append(tools);
  }

  private wireCell(el: HTMLElement, dom: HTMLElement, view: EditorView, r: number, c: number, lastRow: number, cols: number) {
    makeEditable(el);
    el.addEventListener("input", () => writeCell(view, dom, r, c, typed(el)));
    el.addEventListener("keydown", (e) => {
      const move = (r2: number, c2: number) => {
        e.preventDefault();
        const t = tableOf(view, dom);
        if (!t) return;
        if (r2 > lastRow) return focusCell(view, addRow(view, dom, lastRow)!, r2, c2);
        focusCell(view, t.from, r2, c2, true);
      };
      if (e.key === "Enter" && !e.isComposing) move(r + 1, c);
      else if (e.key === "Tab" && !e.shiftKey) move(c + 1 < cols ? r : r + 1, c + 1 < cols ? c + 1 : 0);
      else if (e.key === "Tab" && e.shiftKey && (r > -1 || c > 0)) move(c > 0 ? r : r - 1, c > 0 ? c - 1 : cols - 1);
      else if (e.key === "Escape") el.blur();
    });
  }

  get estimatedHeight() {
    return 44 * this.src.split("\n").length;
  }

  ignoreEvent() {
    return true;
  }
}

export const tables = tableField;

const template = "| Column 1 | Column 2 | Column 3 |\n| --- | --- | --- |\n|  |  |  |\n|  |  |  |";

/** Puts a new 3×2 table at the cursor, with its first heading selected for typing. */
export function insertTable(view: EditorView) {
  const from = insertBlock(view, template);
  focusCell(view, from, -1, 0, true);
  return true;
}
