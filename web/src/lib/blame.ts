// Who wrote what. Loro knows which peer (one editing session) wrote every character; the server
// knows whose session each peer was. In a shared note, lines someone else typed in last get a bar
// in their color. Any line's drag grip says who wrote it and when.
import { Facet, type Range } from "@codemirror/state";
import { Decoration, type DecorationSet, type EditorView, ViewPlugin, type ViewUpdate, WidgetType } from "@codemirror/view";
import type { LoroDoc } from "loro-crdt";
import { api } from "./api";
import { app, colorFor } from "./store.svelte";

/** Who last typed in a line, as the grip and the bars show it. */
export interface LineAuthor {
  /** "Bob · Oct 2, 3:14 PM", or a note that it's from before authors were kept. */
  label: string;
  /** A user-* color class. */
  color: string;
  mine: boolean;
  known: boolean;
}

type AuthorOf = (view: EditorView, line: number) => LineAuthor | null;

/** Looks up who last typed in a line, by line number. Provided by `blame`, read by the drag grip. */
export const lineAuthor = Facet.define<AuthorOf, AuthorOf | null>({
  combine: (values) => values[0] ?? null,
});

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
    return bar;
  }
}

const blamedLine = Decoration.line({ class: "cm-blamed" });

const when = (seconds: number) =>
  new Date(seconds * 1000).toLocaleString([], { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });

/** The bars and the author lookup, for one note's Loro doc. `shared` says whether others can edit it. */
export function blame(doc: LoroDoc, noteId: string, shared: () => boolean) {
  let authors = new Map<string, { user_id: string | null; name: string | null }>();
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

  function authorOf(view: EditorView, lineNo: number): LineAuthor | null {
    const line = view.state.doc.line(lineNo);
    const text = doc.getText("body");
    // The line goes to whoever typed in it last: the character with the highest Lamport time.
    let last: { peer: string; lamport: number; timestamp: number } | null = null;
    for (let i = line.from; i < line.to; i++) {
      const id = text.getCursor(i, 0)?.pos();
      if (!id) continue;
      const change = doc.getChangeAt(id);
      const lamport = change.lamport + (id.counter - change.counter);
      if (!last || lamport > last.lamport) last = { peer: id.peer, lamport, timestamp: change.timestamp };
    }
    if (!last) return null;
    // This session's own edits are known before the server has seen them.
    const who = last.peer === doc.peerIdStr && app.user ? { user_id: app.user.id, name: app.user.display_name } : authors.get(last.peer);
    if (!who) {
      refresh(view);
      return { label: "Written before authors were kept", color: "user-unknown", mine: false, known: false };
    }
    const mine = who.user_id === app.user?.id;
    const name = mine ? "You" : (who.name ?? "Someone");
    return {
      label: last.timestamp ? `${name} · ${when(last.timestamp)}` : name,
      color: who.user_id ? colorFor(who.user_id) : "user-unknown",
      mine,
      known: true,
    };
  }

  function build(view: EditorView): DecorationSet {
    if (!shared()) return Decoration.none;
    const decos: Range<Decoration>[] = [];
    for (const { from, to } of view.visibleRanges) {
      for (let pos = from; pos <= to; ) {
        const line = view.state.doc.lineAt(pos);
        const who = authorOf(view, line.number);
        if (who?.known && !who.mine) {
          decos.push(blamedLine.range(line.from));
          decos.push(Decoration.widget({ widget: new BlameBar(who.color, who.label), side: -1 }).range(line.from));
        }
        pos = line.to + 1;
      }
    }
    return Decoration.set(decos, true);
  }

  const plugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = build(view);
        if (shared()) refresh(view);
      }
      update(u: ViewUpdate) {
        // Edits, scrolling, and the empty dispatch after authors arrive.
        if (u.docChanged || u.viewportChanged || (u.transactions.length && !u.selectionSet)) this.decorations = build(u.view);
      }
    },
    { decorations: (v) => v.decorations },
  );

  return [lineAuthor.of(authorOf), plugin];
}
