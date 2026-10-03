// The "/" menu: type a slash at the start of a line or after a space, then pick a style or
// something to add. Typing more narrows the list; the slash and what was typed go away on pick.
import { autocompletion, type Completion, type CompletionContext, type CompletionResult } from "@codemirror/autocomplete";
import { syntaxTree } from "@codemirror/language";
import { EditorSelection } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import { emojiSource, startEmoji, type EmojiOption } from "./emoji";
import { setBlock, type Block } from "./format";
import { iconSvg, type IconName } from "./icons";
import { insertBoard } from "./kanban";
import { insertTable } from "./tables";

interface SlashItem extends Completion {
  icon: IconName;
}

export interface SlashActions {
  photo: () => void;
  record: () => void;
  /** Tidy Up the note with AI; listed only while this says it can. */
  tidy: () => void;
  canTidy: () => boolean;
}

/** Takes the "/word" out (the menu's range starts after the slash), then runs the item. */
const pick =
  (run: (view: EditorView) => void) =>
  (view: EditorView, _c: Completion, from: number, to: number) => {
    view.dispatch({ changes: { from: from - 1, to }, userEvent: "delete" });
    run(view);
  };

const block = (b: Block) => pick((view) => setBlock(view, b));

/** An empty fenced block on its own lines, with the cursor inside. */
function codeBlock(view: EditorView) {
  const { state } = view;
  const head = state.selection.main.head;
  const line = state.doc.lineAt(head);
  // On a line with text, the block goes under it.
  const before = line.text.trim() ? "\n" : "";
  const from = before ? line.to : line.from;
  view.dispatch({
    changes: { from, to: line.to, insert: `${before}\`\`\`\n\n\`\`\`` },
    selection: EditorSelection.cursor(from + before.length + 4),
    scrollIntoView: true,
    userEvent: "input",
  });
  view.focus();
}

const TEXT = { name: "Text", rank: 0 };
const ADD = { name: "Add", rank: 1 };
const AI = { name: "AI", rank: 2 };

const TIDY = "Tidy Up";
/** Other words for Tidy Up: typing the start of one finds it too. */
const TIDY_WORDS = ["clean up", "format", "improve"];
const tidyWord = (typed: string) => (typed ? TIDY_WORDS.find((w) => w.startsWith(typed.toLowerCase())) : undefined);

function items(actions: SlashActions): SlashItem[] {
  const list: SlashItem[] = [
    { label: "Title", icon: "textformat", apply: block("title"), section: TEXT },
    { label: "Heading", icon: "textformat", apply: block("heading"), section: TEXT },
    { label: "Subheading", icon: "textformat", apply: block("subheading"), section: TEXT },
    { label: "Body", icon: "textformat", apply: block("body"), section: TEXT },
    { label: "Checklist", icon: "checklist", apply: block("check"), section: TEXT },
    { label: "Bulleted List", icon: "bullets", apply: block("bullet"), section: TEXT },
    { label: "Numbered List", icon: "numbers", apply: block("number"), section: TEXT },
    { label: "Quote", icon: "quote", apply: block("quote"), section: TEXT },
    { label: "Code Block", icon: "code", apply: pick(codeBlock), section: ADD },
    { label: "Table", icon: "table", apply: pick(insertTable), section: ADD },
    { label: "Board", icon: "board", apply: pick(insertBoard), section: ADD },
    { label: "Emoji", icon: "emoji", apply: pick(startEmoji), section: ADD },
    { label: "Photo", icon: "camera", apply: pick(actions.photo), section: ADD },
    { label: "Voice Memo", icon: "mic", apply: pick(actions.record), section: ADD },
    { label: TIDY, icon: "broom", apply: pick(actions.tidy), section: AI },
  ];
  // Listed in this order until a typed word ranks them by how well they match.
  return list.map((item, i) => ({ ...item, boost: -i }));
}

/** Code is code: a slash there is just a slash. */
function inCode(context: CompletionContext) {
  for (let node: { name: string; parent: unknown } | null = syntaxTree(context.state).resolveInner(context.pos, -1); node; node = node.parent as typeof node) {
    if (node.name === "FencedCode" || node.name === "InlineCode" || node.name === "CodeBlock") return true;
  }
  return false;
}

export function slashMenu(actions: SlashActions) {
  const all = items(actions);
  const source = (context: CompletionContext): CompletionResult | null => {
    const match = context.matchBefore(/(?:^|\s)\/(\w+( \w*)?)?$/);
    if (!match || inCode(context)) return null;
    const from = match.from + match.text.indexOf("/") + 1;
    let options = actions.canTidy() ? all : all.filter((o) => o.label !== TIDY);
    // Typing another word for Tidy Up lists it under that word, still shown as Tidy Up.
    const alias = tidyWord(context.state.sliceDoc(from, context.pos));
    if (alias) options = options.map((o) => (o.label === TIDY ? { ...o, label: alias, displayLabel: TIDY } : o));
    // Narrowed as you type, and asked again only when that changes which word Tidy Up is under.
    return { from, options, validFor: (text) => /^(\w+( \w*)?)?$/.test(text) && tidyWord(text) === alias };
  };
  return autocompletion({
    override: [source, emojiSource],
    icons: false,
    activateOnTyping: true,
    closeOnBlur: true,
    tooltipClass: () => "cm-slash",
    addToOptions: [
      {
        // A style's icon, or the emoji itself.
        render: (c) => {
          if (!("emoji" in c)) return iconSvg((c as SlashItem).icon);
          const glyph = document.createElement("span");
          glyph.className = "cm-emoji-glyph";
          glyph.textContent = (c as EmojiOption).emoji;
          return glyph;
        },
        position: 20,
      },
    ],
  });
}
