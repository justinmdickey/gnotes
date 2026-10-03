import { Prec } from "@codemirror/state";
import { keymap } from "@codemirror/view";
import type { UndoManager } from "loro-crdt";

/**
 * Undo and redo through Loro's UndoManager, so they take back only your own edits, never a
 * collaborator's. loro-codemirror's own commands run the undo inside a CodeMirror transaction,
 * and the doc change it causes is then dispatched mid-update, which breaks the view. Running it
 * here, outside any transaction, lets loro-codemirror's listener apply the change normally.
 * `changed` runs after each step so buttons can follow canUndo/canRedo.
 */
export function undoCommands(undoManager: UndoManager, changed: () => void) {
  const step = (run: () => boolean) => () => {
    run();
    changed();
    return true;
  };
  const undo = step(() => undoManager.undo());
  const redo = step(() => undoManager.redo());
  // Above loro-codemirror's Prec.high keymap for the same keys.
  const keys = Prec.highest(
    keymap.of([
      { key: "Mod-z", run: undo, preventDefault: true },
      { key: "Mod-y", mac: "Mod-Shift-z", run: redo, preventDefault: true },
      { key: "Mod-Shift-z", run: redo, preventDefault: true },
    ]),
  );
  return { undo, redo, keymap: keys };
}
