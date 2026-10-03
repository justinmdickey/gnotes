import { api, ApiError, type Features, type Tree, type TreeNote, type User } from "./api";
import { sync, type Status } from "./sync";
import { media, toast } from "./ui.svelte";

/** What the note list shows. */
export type View =
  | { kind: "root" }
  | { kind: "all" }
  | { kind: "notebook"; id: string }
  | { kind: "shared-notes" }
  | { kind: "trash" };

export const app = $state({
  user: null as User | null,
  tree: { notebooks: [], notes: [], shared: [] } as Tree,
  status: "connecting" as Status,
  view: { kind: "root" } as View,
  noteId: null as string | null,
  /** Which pane is visible on narrow screens. */
  pane: "list" as "sidebar" | "list" | "editor",
  /** A note just created here, which should open with the cursor ready. */
  freshNote: null as string | null,
  /** The Settings screen covers the whole app while open. */
  settings: false,
  /** The note editor has focus, so the phone's tab bar steps aside for the keyboard bar. */
  typing: false,
  /** Tablet only: the notebooks sidebar is pulled out over the list. */
  drawer: false,
  /** Optional server abilities, like speech-to-text. */
  features: { transcription: false, live_transcription: false, photo_text: false, summaries: false, max_upload: 0, version: "" } as Features,
  /** Why the user was sent back to the login screen, if it wasn't their choice. */
  signedOutReason: "" as string,
});

/** Wide screens use two panes: the sidebar tree, and either the current page (a folder, Recent, Shared) or the open note. */
export function twoPane(): boolean {
  return media.wide;
}

export function viewTitle(view: View, tree: Tree): string {
  if (view.kind === "root") return "Notes";
  if (view.kind === "all") return "Recent";
  if (view.kind === "shared-notes") return "Shared with Me";
  if (view.kind === "trash") return "Trash";
  return tree.notebooks.find((n) => n.id === view.id)?.name ?? "Notes";
}

/** Where new notes from `view` go: Shared, Trash and view-only notebooks can't take them, so the top folder does. */
export function addableView(view: View): View {
  if (view.kind === "shared-notes" || view.kind === "trash") return { kind: "root" };
  if (view.kind === "notebook") {
    const id = view.id;
    if (app.tree.notebooks.find((n) => n.id === id)?.role === "viewer") return { kind: "root" };
  }
  return view;
}

/** Apple Notes-style compose: make the note and drop straight into it. */
export async function composeNote(view: View = app.view) {
  view = addableView(view);
  const notebook = view.kind === "notebook" ? view.id : null;
  const { id } = await api.createNote(notebook);
  app.freshNote = id;
  navigate(view, id);
  void refreshTree();
}

/** Files an import takes: Markdown and zips of it. */
export const importable = (f: File) => /\.(md|markdown|txt|zip)$/i.test(f.name);

/** Brings Markdown notes in from files or zips, then offers to show where they went. */
export async function importNotes(files: File[], view: View = { kind: "root" }) {
  view = addableView(view);
  toast(files.length === 1 ? `Importing ${files[0].name}…` : `Importing ${files.length} files…`, undefined, 60_000);
  try {
    const r = await api.importNotes(files, view.kind === "notebook" ? view.id : null);
    void refreshTree();
    const where: View = r.notebook_id ? { kind: "notebook", id: r.notebook_id } : view;
    const skipped = r.skipped.length ? ` · ${r.skipped.length} other ${r.skipped.length === 1 ? "file" : "files"} skipped` : "";
    toast(`Imported ${r.notes} ${r.notes === 1 ? "note" : "notes"}${skipped}`, { label: "Show", run: () => navigate(where) });
  } catch (err) {
    toast(err instanceof ApiError ? err.message : "Couldn't reach the server");
  }
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
//   #/                                the top folder: top-level notebooks and loose notes
//   #/all  #/shared  #/nb/<id>        Recent, Shared with Me, a notebook
//   #/trash                           deleted notes and notebooks
//   #/note/<id>  #/nb/<id>/note/<id>  a note, remembering the list it came from

function viewPath(view: View): string {
  if (view.kind === "notebook") return `nb/${view.id}`;
  if (view.kind === "root") return "";
  if (view.kind === "trash") return "trash";
  return view.kind === "shared-notes" ? "shared" : "all";
}

function hashFor(view: View | null, noteId: string | null): string {
  const path = [viewPath(view ?? { kind: "root" }), noteId ? `note/${noteId}` : ""].filter(Boolean).join("/");
  return `#/${path}`;
}

function apply(view: View | null, noteId: string | null) {
  app.settings = false;
  app.drawer = false;
  app.view = view ?? { kind: "root" };
  app.noteId = noteId;
  app.pane = noteId ? "editor" : "list";
}

/** Goes to a screen: a view's note list, or a note inside it. `null` is the top folder. */
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
  const parent = app.noteId ? app.view : parentView(app.view);
  // The top folder, Recent and Shared are tabs; there's nothing above them.
  if (!app.noteId && !parent) return;
  if (history.state?.from === hashFor(parent, null)) return history.back();
  navigate(parent, null, true);
}

/** The screen above a note list: a notebook's parent notebook or the top folder, or nothing for tabs. */
export function parentView(view: View): View | null {
  // Trash sits at the bottom of the top folder.
  if (view.kind === "trash") return { kind: "root" };
  if (view.kind !== "notebook") return null;
  const nb = app.tree.notebooks.find((n) => n.id === view.id);
  if (nb?.parent_id && app.tree.notebooks.some((n) => n.id === nb.parent_id)) return { kind: "notebook", id: nb.parent_id };
  // A shared notebook's top sits in Shared with Me, not in your own folders.
  return nb && nb.role !== "owner" ? { kind: "shared-notes" } : { kind: "root" };
}

/** Where a note or notebook lives, outermost first, for "in Home › Kitchen" labels. */
export function pathOf(notebookId: string | null): string[] {
  const out: string[] = [];
  let id = notebookId;
  while (id) {
    const nb = app.tree.notebooks.find((n) => n.id === id);
    if (!nb) break;
    out.unshift(nb.name);
    id = nb.parent_id;
  }
  return out;
}

/** A notebook and everything inside it, for searching a folder. */
export function subtree(id: string | null): Set<string | null> {
  const out = new Set<string | null>([id]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const nb of app.tree.notebooks) {
      if (!out.has(nb.id) && (id === null ? nb.role === "owner" && (nb.parent_id === null || out.has(nb.parent_id)) : out.has(nb.parent_id))) {
        out.add(nb.id);
        grew = true;
      }
    }
  }
  return out;
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
  let view: View = { kind: "root" };
  const nb = base.match(new RegExp(`^nb/${id}$`));
  if (nb) view = { kind: "notebook", id: nb[1] };
  else if (base === "shared") view = { kind: "shared-notes" };
  else if (base === "all") view = { kind: "all" };
  else if (base === "trash") view = { kind: "trash" };
  apply(view, note);
}

export function notesFor(view: View, tree: Tree): TreeNote[] {
  switch (view.kind) {
    case "root":
      return tree.notes.filter((n) => n.role === "owner" && n.notebook_id === null);
    case "all":
      // Recent: every note you can see, newest first.
      return tree.notes.toSorted((a, b) => b.updated_at - a.updated_at);
    case "notebook":
      return tree.notes.filter((n) => n.notebook_id === view.id);
    case "shared-notes": {
      const direct = new Set(
        tree.shared.filter((s) => s.resource_type === "note" && !s.hidden).map((s) => s.resource_id),
      );
      return tree.notes.filter((n) => direct.has(n.id));
    }
    case "trash":
      return [];
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
  if (app.view.kind === "notebook" && app.view.id === id) navigate(parentView(app.view), null, true);
  toast(`“${name}” moved to trash`, { label: "Undo", run: () => void api.restore("notebook", id).then(refreshTree) });
}

/** A note or notebook being dragged onto a sidebar folder (desktop only). */
export type Movable = { kind: "note" | "notebook"; id: string };
export const drag = $state({ item: null as Movable | null });

function placeOf(item: Movable) {
  if (item.kind === "note") {
    const n = app.tree.notes.find((x) => x.id === item.id);
    return n && { role: n.role, owner: n.owner, at: n.notebook_id };
  }
  const nb = app.tree.notebooks.find((x) => x.id === item.id);
  return nb && { role: nb.role, owner: nb.owner, at: nb.parent_id };
}

/** Whether `item` can move into `target` (a notebook id, or null for the top folder); the same rules as Move to…. */
export function canMoveTo(item: Movable, target: string | null): boolean {
  const from = placeOf(item);
  if (!from || from.role === "viewer" || from.at === target) return false;
  // Only the owner can put things at the top level.
  if (target === null) return from.role === "owner";
  const nb = app.tree.notebooks.find((x) => x.id === target);
  if (!nb || nb.role === "viewer" || nb.owner !== from.owner) return false;
  // A notebook can't go inside itself or anything in it.
  return item.kind === "note" || !subtree(item.id).has(target);
}

/** Moves a note or notebook, with Undo to put it back. */
export async function moveTo(item: Movable, target: string | null) {
  const from = placeOf(item)?.at ?? null;
  const send = (to: string | null) => (item.kind === "note" ? api.moveNote(item.id, to) : api.moveNotebook(item.id, to));
  try {
    await send(target);
    await refreshTree();
    const label = target ? (app.tree.notebooks.find((n) => n.id === target)?.name ?? "notebook") : "Notes";
    toast(`Moved to ${label}`, { label: "Undo", run: () => void send(from).then(refreshTree) });
  } catch (err) {
    toast(err instanceof ApiError ? err.message : "Couldn't move it");
  }
}
