// What the app keeps on the device so it opens and edits with no server: each opened note's
// Loro snapshot, the last /tree, notes with edits the server may not have yet, and notes made
// while offline. One IndexedDB database per account, deleted on logout. See "PWA → Offline" in docs/DESIGN.md.
import { LoroDoc, type VersionVector } from "loro-crdt";
import { api, ApiError, type Tree, type User } from "./api";
import { sync, type NoteHandler } from "./sync";

const STORES = ["notes", "dirty", "pending", "kv"] as const;
type Store = (typeof STORES)[number];

let db: Promise<IDBDatabase | null> = Promise.resolve(null);
let dbName = "";

const USER_KEY = "gnotes.user";

/** Opens this account's cache. Without IndexedDB (some private windows) everything is a no-op. */
export function openCache(user: User) {
  try {
    localStorage.setItem(USER_KEY, JSON.stringify(user));
  } catch {}
  if (dbName === `gnotes-${user.id}`) return;
  dbName = `gnotes-${user.id}`;
  db = new Promise((resolve) => {
    try {
      const req = indexedDB.open(dbName, 1);
      req.onupgradeneeded = () => {
        for (const s of STORES) if (!req.result.objectStoreNames.contains(s)) req.result.createObjectStore(s);
      };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => resolve(null);
    } catch {
      resolve(null);
    }
  });
}

/** Who was signed in last, so the app can open with no network. */
export function cachedUser(): User | null {
  try {
    return JSON.parse(localStorage.getItem(USER_KEY) ?? "null");
  } catch {
    return null;
  }
}

/** Forgets the signed-in account. `wipe` also deletes its saved notes, for Log Out. */
export async function closeCache(wipe: boolean) {
  try {
    localStorage.removeItem(USER_KEY);
  } catch {}
  const name = dbName;
  const open = await db;
  db = Promise.resolve(null);
  dbName = "";
  open?.close();
  if (wipe && name) {
    try {
      indexedDB.deleteDatabase(name);
    } catch {}
  }
}

function run<T>(store: Store, mode: IDBTransactionMode, work: (s: IDBObjectStore) => IDBRequest): Promise<T | undefined> {
  return db.then(
    (d) =>
      new Promise<T | undefined>((resolve) => {
        if (!d) return resolve(undefined);
        try {
          const req = work(d.transaction(store, mode).objectStore(store));
          req.onsuccess = () => resolve(req.result as T);
          req.onerror = () => resolve(undefined);
        } catch {
          resolve(undefined);
        }
      }),
  );
}

const get = <T>(store: Store, key: string) => run<T>(store, "readonly", (s) => s.get(key));
const put = (store: Store, key: string, value: unknown) => run(store, "readwrite", (s) => s.put(value, key));
const del = (store: Store, key: string) => run(store, "readwrite", (s) => s.delete(key));
const keys = (store: Store) => run<IDBValidKey[]>(store, "readonly", (s) => s.getAllKeys()).then((k) => (k ?? []) as string[]);

export const cache = {
  /** A note's last saved snapshot; it carries its own version vector. */
  note: (id: string) => get<Uint8Array>("notes", id),
  saveNote: (id: string, doc: LoroDoc) => put("notes", id, doc.export({ mode: "snapshot" })),
  tree: () => get<Tree>("kv", "tree"),
  saveTree: (tree: Tree) => put("kv", "tree", tree),
  /** Marks a note as having edits the server may not have. */
  markDirty: (id: string) => put("dirty", id, true),
  clearDirty: (id: string) => del("dirty", id),
  /** Drops a note's copy: it was thrown away, or isn't yours to see anymore. */
  forget: (id: string) => Promise.all([del("notes", id), del("dirty", id)]),
};

/** Whether an error means the server couldn't be reached (no network, or a proxy saying it's down). */
export function unreachable(err: unknown): boolean {
  return !(err instanceof ApiError) || err.status >= 500;
}

/** A time-ordered UUIDv7, so a note made offline already has the id the server will keep. */
export function uuidv7(): string {
  const b = crypto.getRandomValues(new Uint8Array(16));
  let ms = Date.now();
  for (let i = 5; i >= 0; i--) {
    b[i] = ms % 256;
    ms = Math.floor(ms / 256);
  }
  b[6] = 0x70 | (b[6] & 0x0f);
  b[8] = 0x80 | (b[8] & 0x3f);
  const hex = Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

// Notes made offline wait here until the server takes them. Their editors wait too: joining a
// note the server doesn't have yet would come back not_found.
const waiting = new Map<string, { promise: Promise<void>; resolve: () => void }>();

function wait(id: string) {
  let resolve = () => {};
  const promise = new Promise<void>((r) => (resolve = r));
  waiting.set(id, { promise, resolve });
}

/** Queues a note made offline. */
export async function queueNote(id: string, notebookId: string | null) {
  wait(id);
  await put("pending", id, { notebook_id: notebookId });
}

/** Resolves once the server has the note (right away for notes it already had). */
export function whenCreated(id: string): Promise<void> {
  return waiting.get(id)?.promise ?? Promise.resolve();
}

export const isQueued = (id: string) => waiting.has(id);

/** A queued note left blank never needs to reach the server. */
export async function dropQueued(id: string) {
  waiting.delete(id);
  await del("pending", id);
  await del("notes", id);
}

/** Picks up notes queued in an earlier visit, so their editors wait for them to be made. */
export async function loadQueue() {
  for (const id of await keys("pending")) if (!waiting.has(id)) wait(id);
}

/** Notes open in an editor, which syncs them itself. */
const open = new Set<string>();
export const editing = {
  add: (id: string) => open.add(id),
  delete: (id: string) => open.delete(id),
};

let flushing = false;
let again = false;

/**
 * Back online: make the notes queued offline, then push edits made offline to notes that
 * aren't open (an open note's editor rejoins on its own).
 */
export async function flush(): Promise<void> {
  if (flushing) {
    again = true;
    return;
  }
  flushing = true;
  again = false;
  try {
    for (const id of await keys("pending")) {
      const queued = await get<{ notebook_id: string | null }>("pending", id);
      if (!(await create(id, queued?.notebook_id ?? null))) return;
      await del("pending", id);
      waiting.get(id)?.resolve();
      waiting.delete(id);
    }
    for (const id of await keys("dirty")) {
      if (!open.has(id)) await upload(id);
    }
  } finally {
    flushing = false;
  }
  if (again) return flush();
}

/** Makes a queued note. False means the server is out of reach again, so try later. */
async function create(id: string, notebookId: string | null): Promise<boolean> {
  try {
    await api.createNote(notebookId, id);
  } catch (err) {
    if (unreachable(err)) return false;
    // 409: an earlier try got through. Otherwise the notebook went away or became view-only
    // while offline; keep the note at the top level rather than lose it.
    if (!(err instanceof ApiError && err.status === 409)) {
      try {
        await api.createNote(null, id);
      } catch (again) {
        if (unreachable(again)) return false;
      }
    }
  }
  return true;
}

/**
 * Syncs one saved note without an editor: join with its version, send what the server lacks,
 * then join again to check the server has it all before forgetting it's dirty.
 */
function upload(id: string): Promise<void> {
  return new Promise((done) => {
    void cache.note(id).then((saved) => {
      if (!saved || open.has(id)) {
        if (!saved) void cache.clearDirty(id);
        return done();
      }
      const doc = new LoroDoc();
      doc.import(saved);
      let pushed = false;
      const finish = () => {
        clearTimeout(timer);
        sync.close(id, handler);
        if (open.has(id)) return done();
        void cache.saveNote(id, doc).then(() => done());
      };
      // Gives up quietly if the connection drops; the next reconnect tries again.
      const timer = setTimeout(finish, 10_000);
      const handler: NoteHandler = {
        version: () => doc.oplogVersion(),
        joined(role, server: VersionVector) {
          const cmp = doc.oplogVersion().compare(server);
          if (cmp === 0 || cmp === -1 || role === "viewer" || pushed) {
            if (cmp === 0 || cmp === -1 || role === "viewer") void cache.clearDirty(id);
            return finish();
          }
          pushed = true;
          sync.sendUpdate(id, doc.export({ mode: "update", from: server }));
          sync.rejoin(id);
        },
        update: (data) => void doc.import(data),
        presence() {},
        role() {},
        lost() {
          void cache.clearDirty(id);
          finish();
        },
        outOfSync: () => sync.rejoin(id),
      };
      sync.open(id, handler);
    });
  });
}
