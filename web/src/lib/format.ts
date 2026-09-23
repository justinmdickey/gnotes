// Formatting commands for the toolbar and shortcuts. They edit the Markdown directly.
import { syntaxTree } from "@codemirror/language";
import { EditorSelection, type EditorState } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";

export type Block = "title" | "heading" | "subheading" | "body" | "check" | "bullet" | "number" | "quote";
export type Inline = "bold" | "italic" | "strike" | "code";

const PREFIX: Record<Block, string> = {
  title: "# ",
  heading: "## ",
  subheading: "### ",
  body: "",
  check: "- [ ] ",
  bullet: "- ",
  number: "1. ",
  quote: "> ",
};

const MARK: Record<Inline, string> = { bold: "**", italic: "*", strike: "~~", code: "`" };

const BLOCK_PATTERNS: [Block, RegExp][] = [
  ["check", /^(\s*)[-*+] \[[ xX]\] /],
  ["bullet", /^(\s*)[-*+] /],
  ["number", /^(\s*)\d+[.)] /],
  ["quote", /^(\s*)> ?/],
  ["subheading", /^(\s*)### /],
  ["heading", /^(\s*)## /],
  ["title", /^(\s*)# /],
];

export function blockOf(text: string): { block: Block; prefixLength: number; indent: string } {
  for (const [block, re] of BLOCK_PATTERNS) {
    const m = text.match(re);
    if (m) return { block, prefixLength: m[0].length, indent: m[1] };
  }
  return { block: "body", prefixLength: 0, indent: "" };
}

/** Sets every selected line to `block`, or back to body text if they all already are. */
export function setBlock(view: EditorView, block: Block): boolean {
  const { state } = view;
  const lines = new Map<number, { from: number; text: string }>();
  for (const r of state.selection.ranges) {
    for (let l = state.doc.lineAt(r.from).number; l <= state.doc.lineAt(r.to).number; l++) {
      const line = state.doc.line(l);
      lines.set(l, { from: line.from, text: line.text });
    }
  }
  const all = [...lines.values()];
  const target = all.every((l) => blockOf(l.text).block === block) ? "body" : block;
  let number = 1;
  const changes = all.map(({ from, text }) => {
    const cur = blockOf(text);
    const prefix = target === "number" ? `${number++}. ` : PREFIX[target];
    return { from: from + cur.indent.length, to: from + cur.prefixLength, insert: prefix };
  });
  view.dispatch(state.update({ changes, scrollIntoView: true, userEvent: "input.format" }));
  view.focus();
  return true;
}

/** Wraps or unwraps the selection in an inline mark. With no selection, inserts a pair to type into. */
export function toggleInline(view: EditorView, kind: Inline): boolean {
  const mark = MARK[kind];
  const n = mark.length;
  const tr = view.state.changeByRange((range) => {
    const doc = view.state.doc;
    const before = doc.sliceString(range.from - n, range.from);
    const after = doc.sliceString(range.to, range.to + n);
    if (before === mark && after === mark) {
      return {
        changes: [
          { from: range.from - n, to: range.from },
          { from: range.to, to: range.to + n },
        ],
        range: EditorSelection.range(range.from - n, range.to - n),
      };
    }
    return {
      changes: [
        { from: range.from, insert: mark },
        { from: range.to, insert: mark },
      ],
      range: EditorSelection.range(range.from + n, range.to + n),
    };
  });
  view.dispatch(view.state.update(tr, { scrollIntoView: true, userEvent: "input.format" }));
  view.focus();
  return true;
}

/** What the toolbar should show as active at the main cursor. */
export function activeFormats(state: EditorState): { block: Block; inline: Set<Inline> } {
  const head = state.selection.main.head;
  const block = blockOf(state.doc.lineAt(head).text).block;
  const inline = new Set<Inline>();
  for (let node: { name: string; parent: unknown } | null = syntaxTree(state).resolveInner(head, -1); node; node = node.parent as typeof node) {
    if (node.name === "StrongEmphasis") inline.add("bold");
    else if (node.name === "Emphasis") inline.add("italic");
    else if (node.name === "Strikethrough") inline.add("strike");
    else if (node.name === "InlineCode") inline.add("code");
  }
  return { block, inline };
}

/** Apple Notes shortcuts where the browser allows them. */
export const formatKeymap = [
  { key: "Mod-b", run: (v: EditorView) => toggleInline(v, "bold") },
  { key: "Mod-i", run: (v: EditorView) => toggleInline(v, "italic") },
  { key: "Mod-Shift-x", run: (v: EditorView) => toggleInline(v, "strike") },
  { key: "Mod-Shift-t", run: (v: EditorView) => setBlock(v, "title") },
  { key: "Mod-Shift-h", run: (v: EditorView) => setBlock(v, "heading") },
  { key: "Mod-Shift-j", run: (v: EditorView) => setBlock(v, "subheading") },
  { key: "Mod-Shift-b", run: (v: EditorView) => setBlock(v, "body") },
  { key: "Mod-Shift-l", run: (v: EditorView) => setBlock(v, "check") },
  { key: "Mod-Shift-7", run: (v: EditorView) => setBlock(v, "number") },
  { key: "Mod-Shift-8", run: (v: EditorView) => setBlock(v, "bullet") },
];
