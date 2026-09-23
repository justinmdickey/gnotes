import { api, ApiError, type Features, type Tree, type TreeNote, type User } from "./api";
import { sync, type Status } from "./sync";
import { toast } from "./ui.svelte";

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
  /** A note just created here, which should open with the cursor ready. */
  freshNote: null as string | null,
  /** The Settings screen covers the whole app while open. */
  settings: false,
  /** Tablet only: the notebooks sidebar is pulled out over the list. */
  drawer: false,
  /** Optional server abilities, like speech-to-text. */
  features: { transcription: false, max_upload: 0 } as Features,
  /** Why the user was sent back to the login screen, if it wasn't their choice. */
  signedOutReason: "" as string,
});

export function viewTitle(view: View, tree: Tree): string {
  if (view.kind === "all") return "All Notes";
  if (view.kind === "shared-notes") return "Shared Notes";
  return tree.notebooks.find((n) => n.id === view.id)?.name ?? "Notes";
}

/** Apple Notes-style compose: make the note and drop straight into it. */
export async function composeNote(view: View = app.view) {
  const notebook = view.kind === "notebook" ? view.id : null;
  const { id } = await api.createNote(notebook);
  app.freshNote = id;
  navigate(view.kind === "shared-notes" ? { kind: "all" } : view, id);
  void refreshTree();
}

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
  sync.onStatus = (s) => {
    app.status = s;
    // A dropped connection can mean the session ended (password reset, account disabled).
    if (s === "offline" && app.user) {
      api.me().catch((e) => {
        if (e instanceof ApiError && e.status === 401) {
          sync.disconnect();
          app.user = null;
          app.signedOutReason = "You were signed out. Log in again to continue.";
        }
      });
    }
  };
  sync.connect();
  void refreshTree();
  api.features().then((f) => (app.features = f), () => {});
  readHash();
}

export async function endSession() {
  sync.disconnect();
  await api.logout().catch(() => {});
  app.user = null;
  app.settings = false;
  history.replaceState(null, "", "/#/");
}

// Routes live in the hash so every screen change is a history entry and the phone's
// back button steps back through them:
//   #/                       notebooks (the phone's home screen)
//   #/all  #/shared  #/nb/<id>            a note list
//   #/all/note/<id>  #/nb/<id>/note/<id>  a note, remembering the list it came from

function viewPath(view: View): string {
  if (view.kind === "notebook") return `nb/${view.id}`;
  return view.kind === "shared-notes" ? "shared" : "all";
}

function hashFor(view: View | null, noteId: string | null): string {
  if (!view) return "#/";
  return `#/${viewPath(view)}${noteId ? `/note/${noteId}` : ""}`;
}

function apply(view: View | null, noteId: string | null) {
  app.settings = false;
  app.drawer = false;
  if (view) app.view = view;
  app.noteId = noteId;
  app.pane = noteId ? "editor" : view ? "list" : "sidebar";
}

/** Goes to a screen: a view's note list, a note inside it, or home with `view` null. */
export function navigate(view: View | null, noteId: string | null = null, replace = false) {
  apply(view, noteId);
  const hash = hashFor(view, noteId);
  if (location.hash === hash) return;
  if (replace) history.replaceState(history.state, "", hash);
  else history.pushState({ from: location.hash }, "", hash);
}

export function openNote(id: string) {
  navigate(app.view, id);
}

export function openSettings() {
  app.settings = true;
  if (location.hash !== "#/settings") history.pushState({ from: location.hash }, "", "#/settings");
}

/**
 * In-app back always goes to the parent screen (note -> its list -> notebooks). It pops
 * history when the previous entry is that parent, so the phone's back button stays in step.
 */
export function goBack() {
  if (app.settings) {
    if (history.state?.from !== undefined) return history.back();
    return navigate(null, null, true);
  }
  const parent = app.noteId ? app.view : null;
  if (history.state?.from === hashFor(parent, null)) return history.back();
  navigate(parent, null, true);
}

/** Tablet: closes the pulled-out sidebar, leaving home for the list if that's where we were. */
export function closeDrawer() {
  app.drawer = false;
  if (app.pane === "sidebar") navigate(app.view, null, true);
}

export function readHash() {
  const h = location.hash.replace(/^#\/?/, "");
  if (h === "settings") {
    app.settings = true;
    return;
  }
  const id = "([0-9a-f-]{36})";
  const note = h.match(new RegExp(`(?:^|/)note/${id}$`))?.[1] ?? null;
  const base = h.replace(/\/?note\/[0-9a-f-]{36}$/, "");
  let view: View | null = null;
  const nb = base.match(new RegExp(`^nb/${id}$`));
  if (nb) view = { kind: "notebook", id: nb[1] };
  else if (base === "shared") view = { kind: "shared-notes" };
  else if (base === "all" || note) view = { kind: "all" };
  apply(view, note);
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

/** Moves a note to the trash, with Undo instead of an "are you sure?". */
export async function trashNote(id: string) {
  await api.deleteNote(id);
  if (app.noteId === id) goBack();
  toast("Note moved to trash", { label: "Undo", run: () => void api.restore("note", id).then(refreshTree) });
}

export async function trashNotebook(id: string, name: string) {
  await api.deleteNotebook(id);
  if (app.view.kind === "notebook" && app.view.id === id) navigate({ kind: "all" }, null, true);
  toast(`“${name}” moved to trash`, { label: "Undo", run: () => void api.restore("notebook", id).then(refreshTree) });
}
