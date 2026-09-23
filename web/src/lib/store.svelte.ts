import { api, type Tree, type TreeNote, type User } from "./api";
import { sync, type Status } from "./sync";

/** What the note list shows. */
export type View =
  | { kind: "all" }
  | { kind: "notebook"; id: string }
  | { kind: "shared-notes" };

export const app = $state({
  user: null as User | null,
  tree: { notebooks: [], notes: [], shared: [] } as Tree,
  status: "connecting" as Status,
  view: { kind: "all" } as View,
  noteId: null as string | null,
  /** Which pane is visible on narrow screens. */
  pane: "list" as "sidebar" | "list" | "editor",
});

let refreshing: Promise<void> | null = null;
let stale = false;

/** Coalesces bursts of tree_changed, but always refetches once more if a change landed mid-request. */
export function refreshTree(): Promise<void> {
  if (refreshing) {
    stale = true;
    return refreshing;
  }
  stale = false;
  refreshing = api
    .tree()
    .then((t) => {
      app.tree = t;
    })
    .catch((e) => console.warn("tree refresh failed", e))
    .finally(() => {
      refreshing = null;
      if (stale) void refreshTree();
    });
  return refreshing;
}

export function startSession(user: User) {
  app.user = user;
  sync.onTreeChanged = () => void refreshTree();
  sync.onStatus = (s) => (app.status = s);
  sync.connect();
  void refreshTree();
  readHash();
}

export async function endSession() {
  sync.disconnect();
  await api.logout().catch(() => {});
  app.user = null;
}

export function openNote(id: string | null) {
  app.noteId = id;
  app.pane = id ? "editor" : "list";
  const hash = id ? `#/note/${id}` : "#/";
  if (location.hash !== hash) history.pushState(null, "", hash);
}

export function readHash() {
  const m = location.hash.match(/^#\/note\/([0-9a-f-]{36})$/);
  app.noteId = m ? m[1] : null;
  if (m) app.pane = "editor";
}

export function notesFor(view: View, tree: Tree): TreeNote[] {
  switch (view.kind) {
    case "all":
      return tree.notes.filter((n) => n.role === "owner");
    case "notebook":
      return tree.notes.filter((n) => n.notebook_id === view.id);
    case "shared-notes": {
      const direct = new Set(
        tree.shared.filter((s) => s.resource_type === "note" && !s.hidden).map((s) => s.resource_id),
      );
      return tree.notes.filter((n) => direct.has(n.id));
    }
  }
}

/** Stable per-user cursor color. */
export function colorFor(userId: string): string {
  const colors = ["user-blue", "user-green", "user-orange", "user-purple", "user-red"];
  let h = 0;
  for (const c of userId) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return colors[h % colors.length];
}
