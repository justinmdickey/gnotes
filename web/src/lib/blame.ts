// Who wrote what: a colored bar beside each line, in the color of the person who last typed in it.
// Loro knows which peer (one editing session) wrote every character; the server knows whose
// session each peer was. Pointing at a bar says who and when; tapping it says so in a toast.
import { StateEffect, StateField, type Range } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin, type ViewUpdate, WidgetType } from "@codemirror/view";
import type { LoroDoc } from "loro-crdt";
import { api } from "./api";
import { app, colorFor } from "./store.svelte";
import { toast } from "./ui.svelte";

export const setBlame = StateEffect.define<boolean>();

/** Whether the bars are showing. */
export const blameOn = StateField.define<boolean>({
  create: () => false,
  update(on, tr) {
    for (const e of tr.effects) if (e.is(setBlame)) on = e.value;
    return on;
  },
  // The drag grips sit where the bars go, so they step aside while the bars show.
  provide: (f) => EditorView.editorAttributes.from(f, (on) => (on ? { class: "cm-blame-on" } : ({} as Record<string, string>))),
});

interface Author {
  user_id: string | null;
  name: string | null;
}

class BlameBar extends WidgetType {
  constructor(
    readonly color: string,
    readonly label: string,
  ) {
    super();
  }

  eq(other: BlameBar) {
    return other.color === this.color && other.label === this.label;
  }

  toDOM() {
    const bar = document.createElement("span");
    bar.className = `cm-blame ${this.color}`;
    bar.title = this.label;
    bar.setAttribute("aria-label", this.label);
    bar.addEventListener("click", (e) => {
      e.stopPropagation();
      toast(this.label);
    });
    return bar;
  }

  ignoreEvent() {
    return true;
  }
}

const blamedLine = Decoration.line({ class: "cm-blamed" });

const when = (seconds: number) =>
  new Date(seconds * 1000).toLocaleString([], { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });

/** The bars, for one note's Loro doc. */
export function blame(doc: LoroDoc, noteId: string) {
  let authors = new Map<string, Author>();
  let fetched = 0;
  let fetching: Promise<void> | null = null;

  /** Asks the server whose sessions these are; again later if a session turns up it didn't know. */
  function refresh(view: EditorView) {
    if (fetching || Date.now() - fetched < 5000) return;
    fetching = api
      .authors(noteId)
      .then((r) => {
        authors = new Map(Object.entries(r.peers));
        fetched = Date.now();
        view.dispatch({});
      })
      .catch(() => {})
      .finally(() => (fetching = null));
  }

  function build(view: EditorView): DecorationSet {
    if (!view.state.field(blameOn)) return Decoration.none;
    const text = doc.getText("body");
    const decos: Range<Decoration>[] = [];
    let unknown = false;
    for (const { from, to } of view.visibleRanges) {
      for (let pos = from; pos <= to; ) {
        const line = view.state.doc.lineAt(pos);
        // The line goes to whoever typed in it last: the character with the highest Lamport time.
        let last: { peer: string; lamport: number; timestamp: number } | null = null;
        for (let i = line.from; i < line.to; i++) {
          const id = text.getCursor(i, 0)?.pos();
          if (!id) continue;
          const change = doc.getChangeAt(id);
          const lamport = change.lamport + (id.counter - change.counter);
          if (!last || lamport > last.lamport) last = { peer: id.peer, lamport, timestamp: change.timestamp };
        }
        if (last) {
          // This session's own edits are known before the server has seen them.
          const who = last.peer === doc.peerIdStr && app.user ? { user_id: app.user.id, name: app.user.display_name } : authors.get(last.peer);
          if (!who) unknown = true;
          const name = who?.name ?? "Someone, before authors were kept";
          const label = who && last.timestamp ? `${name} · ${when(last.timestamp)}` : name;
          decos.push(blamedLine.range(line.from));
          decos.push(Decoration.widget({ widget: new BlameBar(who?.user_id ? colorFor(who.user_id) : "user-unknown", label), side: -1 }).range(line.from));
        }
        pos = line.to + 1;
      }
    }
    if (unknown) refresh(view);
    return Decoration.set(decos, true);
  }

  const plugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = build(view);
      }
      update(u: ViewUpdate) {
        const turned = u.startState.field(blameOn) !== u.state.field(blameOn);
        if (turned && u.state.field(blameOn)) {
          fetched = 0;
          refresh(u.view);
        }
        if (turned || u.docChanged || u.viewportChanged || (u.transactions.length && !u.docChanged && !u.selectionSet)) {
          this.decorations = build(u.view);
        }
      }
    },
    { decorations: (v) => v.decorations },
  );

  return [blameOn, plugin];
}
