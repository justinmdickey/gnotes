// Makes Markdown read like formatted text: marks are hidden except on lines being edited,
// bullets and checkboxes render as widgets, and the first line is styled as the title.
import { syntaxTree } from "@codemirror/language";
import type { EditorState, Range } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin, type ViewUpdate, WidgetType } from "@codemirror/view";

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

  for (const { from, to } of view.visibleRanges) {
    syntaxTree(state).iterate({
      from,
      to,
      enter(node) {
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
