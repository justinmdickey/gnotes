// Shared plumbing for Markdown blocks drawn as interactive widgets (tables, kanban boards).
// The Markdown text is the only state: widgets are rebuilt from it, and every edit they make
// is a small change to the text, so other people's edits elsewhere in the block survive.
import { ensureSyntaxTree, syntaxTree } from "@codemirror/language";
import { type EditorState, type Extension, type Range, StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, type WidgetType } from "@codemirror/view";

export interface Span {
  from: number;
  to: number;
}

const focusEffect = StateEffect.define<boolean>();
/** Whether the editor itself (not a widget inside it) has focus. */
const editorFocus = StateField.define<boolean>({
  create: () => false,
  update(v, tr) {
    for (const e of tr.effects) if (e.is(focusEffect)) v = e.value;
    return v;
  },
});
const trackFocus = [editorFocus, EditorView.focusChangeEffect.of((_s, focusing) => focusEffect.of(focusing))];

/** Whole lines from a syntax node's start to its end, leaving out a trailing line break. */
export function nodeLines(state: EditorState, from: number, to: number): Span {
  if (to > from && state.sliceDoc(to - 1, to) === "\n") to--;
  return { from: state.doc.lineAt(from).from, to: state.doc.lineAt(to).to };
}

/** Top-level syntax nodes of one kind, for blocks that start at the start of a line. */
export function topLevel(state: EditorState, name: string, keep: (from: number, to: number) => boolean = () => true): Span[] {
  const tree = ensureSyntaxTree(state, state.doc.length, 50) ?? syntaxTree(state);
  const out: Span[] = [];
  for (let node = tree.topNode.firstChild; node; node = node.nextSibling) {
    if (node.name !== name || state.doc.lineAt(node.from).from !== node.from || !keep(node.from, node.to)) continue;
    out.push(nodeLines(state, node.from, node.to));
  }
  return out;
}

/**
 * A field of block widgets, one per span `find` returns. A block shows as its Markdown while the
 * editor has focus with the cursor in it ("Edit as Markdown"), and as the widget otherwise.
 */
export function blockWidgets<B extends Span>(find: (state: EditorState) => B[], widget: (state: EditorState, block: B, editable: boolean) => WidgetType) {
  const compute = (state: EditorState): { blocks: B[]; decos: DecorationSet } => {
    const blocks = find(state);
    const editable = state.facet(EditorView.editable);
    const focused = state.field(editorFocus, false) ?? false;
    const head = state.selection.main.head;
    const decos: Range<Decoration>[] = [];
    for (const b of blocks) {
      // Strictly inside: focus moving into a widget can leave the selection on its edge.
      if (focused && editable && head > b.from && head < b.to) continue;
      decos.push(Decoration.replace({ widget: widget(state, b, editable), block: true }).range(b.from, b.to));
    }
    return { blocks, decos: Decoration.set(decos) };
  };
  const field = StateField.define({
    create: compute,
    update(v, tr) {
      const changed =
        tr.docChanged ||
        tr.selection ||
        tr.effects.some((e) => e.is(focusEffect)) ||
        syntaxTree(tr.state) !== syntaxTree(tr.startState) ||
        tr.startState.facet(EditorView.editable) !== tr.state.facet(EditorView.editable);
      return changed ? compute(tr.state) : v;
    },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });
  /** The block a widget's DOM stands for, in the current text. */
  const blockOf = (view: EditorView, dom: HTMLElement): B | undefined => {
    if (!dom.isConnected) return undefined;
    const pos = view.posAtDOM(dom);
    return view.state.field(field).blocks.find((b) => pos >= b.from && pos <= b.to);
  };
  /** The widget DOM now drawn for the block starting at `from`, after a change. */
  const domOf = (view: EditorView, from: number, cls: string): HTMLElement | undefined =>
    [...view.contentDOM.querySelectorAll<HTMLElement>(`.${cls}`)].find((el) => blockOf(view, el)?.from === from);
  const extension: Extension = [trackFocus, field];
  return { extension, blockOf, domOf };
}

/** Replaces `oldText` (at `from`) with `newText`, changing only the part that differs. */
export function replaceMinimal(view: EditorView, from: number, oldText: string, newText: string) {
  if (oldText === newText) return;
  let a = 0;
  while (a < oldText.length && a < newText.length && oldText[a] === newText[a]) a++;
  let b = 0;
  while (b < oldText.length - a && b < newText.length - a && oldText[oldText.length - 1 - b] === newText[newText.length - 1 - b]) b++;
  // Never split a surrogate pair.
  if (a > 0 && /[\uD800-\uDBFF]/.test(oldText[a - 1])) a--;
  if (b > 0 && /[\uDC00-\uDFFF]/.test(oldText[oldText.length - b])) b--;
  view.dispatch({ changes: { from: from + a, to: from + oldText.length - b, insert: newText.slice(a, newText.length - b) }, userEvent: "input.type" });
}

/** Where a new block goes: on the cursor's blank line, or after its line, with blank lines around it. */
export function insertBlock(view: EditorView, text: string): number {
  const { state } = view;
  const line = state.doc.lineAt(state.selection.main.head);
  const blank = line.length === 0 && line.number > 1;
  const at = blank ? line.from : line.to;
  const before = blank ? (state.doc.line(line.number - 1).length ? "\n" : "") : "\n\n";
  const insert = `${before}${text}\n`;
  const from = at + before.length;
  // The cursor waits on the line after the block, so the block shows as its widget.
  view.dispatch({ changes: { from: at, insert }, selection: { anchor: at + insert.length }, userEvent: "input", scrollIntoView: true });
  return from;
}

/** Keeps focus where it is when pressing a widget's buttons. */
export function keepFocus(el: HTMLElement) {
  el.addEventListener("mousedown", (e) => e.preventDefault());
}

/** Puts the caret at the end of an editable element, or selects all of it. */
export function placeCaret(el: HTMLElement, selectAll = false, offset?: number) {
  el.focus({ preventScroll: true });
  const sel = getSelection();
  if (!sel) return;
  const range = document.createRange();
  range.selectNodeContents(el);
  if (!selectAll) {
    const text = el.firstChild;
    if (offset !== undefined && text?.nodeType === Node.TEXT_NODE) {
      range.setStart(text, Math.min(offset, text.textContent!.length));
      range.collapse(true);
    } else range.collapse(false);
  }
  sel.removeAllRanges();
  sel.addRange(range);
  el.scrollIntoView({ block: "nearest", inline: "nearest" });
}

/** The caret's offset in an element with a single text node, to put it back after a redraw. */
export function caretOffset(el: HTMLElement): number | undefined {
  const sel = getSelection();
  if (!sel?.rangeCount || !el.contains(sel.focusNode)) return undefined;
  return sel.focusNode === el ? (sel.focusOffset ? el.textContent!.length : 0) : sel.focusOffset;
}

/** The text typed into an editable element, on one line. */
export function typed(el: HTMLElement) {
  return (el.textContent ?? "").replace(/[\r\n]+/g, " ");
}

export function makeEditable(el: HTMLElement) {
  el.contentEditable = "plaintext-only";
  // Browsers without plaintext-only fall back to rich editing; only text is read back either way.
  if (el.contentEditable !== "plaintext-only") el.contentEditable = "true";
  el.spellcheck = true;
}
