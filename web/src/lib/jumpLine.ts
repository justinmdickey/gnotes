// The line a task in Tasks pointed at, lit for a moment when its note opens. A line decoration
// rather than a class on the DOM, so it survives the editor redrawing the line.
import { StateEffect, StateField } from "@codemirror/state";
import { Decoration, EditorView, type DecorationSet } from "@codemirror/view";

/** Lights the line holding this position, or `null` to let it go. */
export const lightLine = StateEffect.define<number | null>();

const lit = Decoration.line({ class: "cm-jumped" });

export const jumpLine = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    deco = deco.map(tr.changes);
    for (const e of tr.effects) {
      if (e.is(lightLine)) deco = e.value === null ? Decoration.none : Decoration.set([lit.range(tr.state.doc.lineAt(e.value).from)]);
    }
    return deco;
  },
  provide: (f) => EditorView.decorations.from(f),
});
