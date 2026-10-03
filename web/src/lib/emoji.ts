// Emoji by name, for keyboards without an emoji key: type ":" and a few letters ("tac" for 🌮)
// and pick from the same popup as the "/" menu. The list (GitHub's names) loads on first use.
import { startCompletion, type Completion, type CompletionContext, type CompletionResult } from "@codemirror/autocomplete";
import { syntaxTree } from "@codemirror/language";
import type { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

export interface EmojiOption extends Completion {
  emoji: string;
}

interface Entry extends EmojiOption {
  name: string;
  tags: string[];
  order: number;
}

let loading: Promise<Entry[]> | null = null;
/** Name to emoji, once the list has loaded. */
const byName = new Map<string, string>();

function load(): Promise<Entry[]> {
  loading ??= import("gemoji").then(({ gemoji }) => {
    for (const g of gemoji) for (const name of g.names) byName.set(name, g.emoji);
    return gemoji.flatMap((g, i) =>
      g.names.map((name) => ({ label: `:${name}:`, detail: g.description, apply: g.emoji, emoji: g.emoji, name, tags: g.tags, order: i })),
    );
  });
  return loading;
}

/** Like Slack: typing the closing colon of a whole ":name:" turns it into the emoji. */
export const emojiOnColon = EditorView.inputHandler.of((view, from, to, text) => {
  if (text !== ":" || from !== to || inCode(view.state, from)) return false;
  const line = view.state.doc.lineAt(from);
  const m = /(?:^|[\s(]):([\w+-]+)$/.exec(line.text.slice(0, from - line.from));
  const emoji = m && byName.get(m[1]);
  if (!emoji) {
    if (m) void load();
    return false;
  }
  const start = from - m![1].length - 1;
  view.dispatch({ changes: { from: start, to, insert: emoji }, selection: { anchor: start + emoji.length }, userEvent: "input" });
  return true;
});

/** Code keeps its colons. */
function inCode(state: EditorState, pos: number) {
  for (let node: { name: string; parent: unknown } | null = syntaxTree(state).resolveInner(pos, -1); node; node = node.parent as typeof node) {
    if (node.name === "FencedCode" || node.name === "InlineCode" || node.name === "CodeBlock") return true;
  }
  return false;
}

/** Completes ":name" after a space or at the start of a line, so times like 10:30 are left alone. */
export async function emojiSource(context: CompletionContext): Promise<CompletionResult | null> {
  const match = context.matchBefore(/(?:^|[\s(])::?[\w+-]*$/);
  if (!match || inCode(context.state, context.pos)) return null;
  const at = match.text.lastIndexOf(":");
  const typed = match.text.slice(at + 1);
  // Two letters before the list opens on its own; "/emoji" opens it straight away.
  if (typed.length < 2 && !context.explicit) return null;
  const all = await load();
  // Names that start with what's typed, then a word in the name, then a tag; each in the list's
  // own order (smileys first). Loose matching found "hot_face" for "tac".
  const q = typed.toLowerCase();
  const rank = (e: Entry) =>
    e.name.startsWith(q) ? 0 : e.name.includes(`_${q}`) ? 1 : e.tags.some((t) => t.startsWith(q)) ? 2 : -1;
  const options = all
    .map((e) => ({ e, r: rank(e) }))
    .filter((x) => x.r >= 0)
    .sort((a, b) => a.r - b.r || a.e.order - b.e.order)
    .slice(0, 60)
    .map((x) => x.e);
  return { from: match.from + at, options, filter: false };
}

/** Starts an emoji at the cursor: a ":" with the list open under it. */
export function startEmoji(view: EditorView) {
  const head = view.state.selection.main.head;
  const before = view.state.sliceDoc(head - 1, head);
  const insert = head > 0 && before.trim() ? " :" : ":";
  view.dispatch({ changes: { from: head, insert }, selection: { anchor: head + insert.length }, userEvent: "input" });
  view.focus();
  startCompletion(view);
}
