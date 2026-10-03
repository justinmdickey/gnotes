// Fenced code blocks: drawn as a tinted block with syntax colors, and a language picker on the
// opening fence. Blocks without a language get a best guess, used for colors only; the text is
// left alone so nothing is written into a shared note behind anyone's back.
import { LanguageDescription, syntaxTree } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import type { Range } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin, type ViewUpdate, WidgetType } from "@codemirror/view";
import { classHighlighter, highlightTree } from "@lezer/highlight";
import { iconSvg } from "./icons";

/** The picker's choices: what's written after the fence, and what it's called. */
export const COMMON: [string, string][] = [
  ["bash", "Shell"],
  ["c", "C"],
  ["cpp", "C++"],
  ["css", "CSS"],
  ["go", "Go"],
  ["html", "HTML"],
  ["java", "Java"],
  ["javascript", "JavaScript"],
  ["json", "JSON"],
  ["markdown", "Markdown"],
  ["python", "Python"],
  ["rust", "Rust"],
  ["sql", "SQL"],
  ["typescript", "TypeScript"],
  ["yaml", "YAML"],
];

/** Written after the fence to turn guessing off for a block. */
const PLAIN = "text";

// Each rule adds to a language's score when it matches; the highest score wins.
const RULES: [string, RegExp, number][] = [
  ["python", /^\s*(def \w+\(|class \w+(\(.*\))?:|from [\w.]+ import |import \w+$|elif |if __name__)/m, 3],
  ["python", /:\s*$\n\s{4}\S/m, 1],
  ["python", /\b(self\.|print\(|None\b|True\b|False\b)/, 1],
  ["rust", /\b(fn \w+|let mut |impl\b|pub fn|println!|-> \w+ \{|&mut |use \w+::)/, 3],
  ["go", /^package \w+|\bfunc (\(\w+ \*?\w+\) )?\w+\(|:= |fmt\./m, 3],
  ["typescript", /\b(interface \w+|type \w+ =|: (string|number|boolean)\b|as const\b|<\w+>\()/, 3],
  ["javascript", /\b(const|let|var) \w+ =|=> |function\s*\w*\(|console\.log|require\(|import .+ from ['"]|export (default|const|function)/, 2],
  ["java", /\b(public|private|protected) (static )?(class|void|final|\w+ \w+\()|System\.out\./, 3],
  ["cpp", /#include\s*<|std::|cout\s*<</, 3],
  ["c", /#include\s*<\w+\.h>|\bprintf\(|int main\(/, 2],
  ["bash", /^\s*(\$ |#!\/.*\b(ba|z)?sh|sudo |apt |brew |npm |npx |cd |ls |echo |export |git |curl |docker |kubectl |cargo )/m, 3],
  ["sql", /^\s*(select .+ from|insert into|update \w+ set|delete from|create (table|index)|alter table)\b/im, 4],
  ["css", /^\s*[.#]?[\w-]+(\s*[,>+~]?\s*[.#]?[\w-]+)*\s*\{\s*$/m, 1],
  ["css", /^\s*[\w-]+:\s*[^;{]+;\s*$/m, 2],
  ["html", /^\s*<(!doctype|html|head|body|div|span|p|a|ul|li|section|script)\b/im, 4],
  ["yaml", /^\s*[\w-]+:( .+)?$/m, 1],
  ["yaml", /^\s*- [\w-]+: /m, 2],
];

/** A best guess at the language of some code, by its name in COMMON, or null when nothing fits. */
export function guessLanguage(code: string): string | null {
  const text = code.trim();
  if (!text) return null;
  if (/^[[{]/.test(text)) {
    try {
      JSON.parse(text);
      return "json";
    } catch {}
  }
  const score = new Map<string, number>();
  for (const [lang, re, n] of RULES) if (re.test(text)) score.set(lang, (score.get(lang) ?? 0) + n);
  // Braces and semicolons mean it isn't YAML.
  if (/[{};]/.test(text)) score.delete("yaml");
  let best: string | null = null;
  let top = 1;
  for (const [lang, n] of score) if (n > top || (n === top && !best)) [best, top] = [lang, n];
  return best;
}

const nameOf = (lang: string) => COMMON.find(([id]) => id === lang)?.[1] ?? lang;
const describe = (lang: string) => LanguageDescription.matchLanguageName(languages, lang, true);

class LanguagePicker extends WidgetType {
  constructor(
    readonly info: string,
    readonly guess: string | null,
    /** Where the language goes: just after the opening fence. */
    readonly from: number,
    readonly to: number,
    readonly editable: boolean,
  ) {
    super();
  }

  eq(other: LanguagePicker) {
    return other.info === this.info && other.guess === this.guess && other.from === this.from && other.to === this.to && other.editable === this.editable;
  }

  toDOM(view: EditorView) {
    const wrap = document.createElement("span");
    wrap.className = "cm-code-lang";
    const select = document.createElement("select");
    select.setAttribute("aria-label", "Code language");
    select.disabled = !this.editable;
    const option = (value: string, label: string) => select.append(new Option(label, value, false, value === this.info));
    option("", this.guess ? `${nameOf(this.guess)} (guess)` : "Plain text");
    option(PLAIN, "Plain text");
    if (this.info && this.info !== PLAIN && !COMMON.some(([id]) => id === this.info)) option(this.info, this.info);
    for (const [id, label] of COMMON) option(id, label);
    select.addEventListener("change", () => {
      view.dispatch({ changes: { from: this.from, to: this.to, insert: select.value }, userEvent: "input" });
    });
    wrap.append(select);
    if (!select.disabled) wrap.append(iconSvg("expand", 12));
    return wrap;
  }

  ignoreEvent() {
    return true;
  }
}

const codeLine = Decoration.line({ class: "cm-code" });
const firstLine = Decoration.line({ class: "cm-code cm-code-first" });
const lastLine = Decoration.line({ class: "cm-code cm-code-last" });
const closedLine = Decoration.line({ class: "cm-code cm-code-last cm-code-closed" });
const hidden = Decoration.replace({});

/** Syntax colors for a block's code, by offset in that code. Kept between redraws. */
const colorCache = new Map<string, [number, number, string][]>();

function colors(lang: LanguageDescription, code: string): [number, number, string][] | null {
  if (!lang.support) return null;
  const key = `${lang.name}\n${code}`;
  let spans = colorCache.get(key);
  if (!spans) {
    spans = [];
    const out = spans;
    highlightTree(lang.support.language.parser.parse(code), classHighlighter, (from, to, cls) => out.push([from, to, cls]));
    if (colorCache.size > 100) colorCache.clear();
    colorCache.set(key, spans);
  }
  return spans;
}

function build(view: EditorView, onLoaded: () => void): { decorations: DecorationSet; atomic: DecorationSet } {
  const { state } = view;
  const decos: Range<Decoration>[] = [];
  // Raw fences show on the lines being edited, like every other mark.
  const active = new Set<number>();
  if (view.hasFocus) for (const r of state.selection.ranges) for (let l = state.doc.lineAt(r.from).number; l <= state.doc.lineAt(r.to).number; l++) active.add(l);

  for (const { from, to } of view.visibleRanges) {
    syntaxTree(state).iterate({
      from,
      to,
      enter(node) {
        if (node.name !== "FencedCode") return;
        const open = state.doc.lineAt(node.from);
        const close = state.doc.lineAt(node.to);
        const marks = node.node.getChildren("CodeMark");
        const closed = marks.length > 1 && close.number > open.number;
        const infoNode = node.node.getChild("CodeInfo");
        const textNode = node.node.getChild("CodeText");
        const info = infoNode ? state.sliceDoc(infoNode.from, infoNode.to).trim().toLowerCase() : "";
        // Boards are fenced blocks too, drawn by kanban.ts.
        if (info === "kanban") return false;
        const code = textNode ? state.sliceDoc(textNode.from, textNode.to) : "";
        const guess = info ? null : guessLanguage(code);

        for (let l = open.number; l <= close.number; l++) {
          const line = state.doc.line(l);
          const last = l === close.number && closed ? (active.has(l) ? lastLine : closedLine) : codeLine;
          decos.push((l === open.number ? firstLine : last).range(line.from));
        }
        if (!active.has(open.number)) {
          const fenceEnd = marks[0]?.to ?? open.from;
          decos.push(Decoration.replace({ widget: new LanguagePicker(info, guess, fenceEnd, open.to, state.facet(EditorView.editable)) }).range(open.from, open.to));
        }
        if (closed && !active.has(close.number)) decos.push(hidden.range(close.from, close.to));

        const lang = info === PLAIN ? null : describe(info || guess || "");
        if (lang && textNode) {
          const spans = colors(lang, code);
          if (spans) for (const [f, t, cls] of spans) decos.push(Decoration.mark({ class: cls }).range(textNode.from + f, textNode.from + t));
          else lang.load().then(onLoaded, () => {});
        }
        return false;
      },
    });
  }
  return {
    decorations: Decoration.set(decos, true),
    atomic: Decoration.set(decos.filter((d) => d.value.point), true),
  };
}

const codeBlocksPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    atomic: DecorationSet;
    private redraw: () => void;

    constructor(readonly view: EditorView) {
      // A language's grammar loads on first use; draw again once it's here.
      this.redraw = () => {
        ({ decorations: this.decorations, atomic: this.atomic } = build(view, this.redraw));
        view.dispatch({});
      };
      ({ decorations: this.decorations, atomic: this.atomic } = build(view, this.redraw));
    }

    update(u: ViewUpdate) {
      const editableChanged = u.startState.facet(EditorView.editable) !== u.state.facet(EditorView.editable);
      if (u.docChanged || u.viewportChanged || u.selectionSet || u.focusChanged || editableChanged) {
        ({ decorations: this.decorations, atomic: this.atomic } = build(u.view, this.redraw));
      }
    }
  },
  { decorations: (v) => v.decorations },
);

export const codeBlocks = [codeBlocksPlugin, EditorView.atomicRanges.of((view) => view.plugin(codeBlocksPlugin)?.atomic ?? Decoration.none)];
