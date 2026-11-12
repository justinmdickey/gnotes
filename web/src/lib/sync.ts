// One websocket for every open note. Protocol: "Websocket protocol" in docs/DESIGN.md.
import { LoroDoc, VersionVector } from "loro-crdt";
import { api, type Role } from "./api";
import { cache, editing, isQueued, plainText, rememberText, unreachable } from "./offline";

const KIND_UPDATE = 0x01;
const KIND_PRESENCE = 0x02;

export interface NoteHandler {
  /** The local version to rejoin with, or null to get a full snapshot. */
  version(): VersionVector | null;
  joined(role: Role, serverVersion: VersionVector): void;
  update(data: Uint8Array): void;
  presence(data: Uint8Array): void;
  role(role: Role): void;
  /** Access was removed, or the note is gone. */
  lost(reason: "revoked" | "not_found"): void;
  outOfSync(): void;
}

export type Status = "connecting" | "online" | "offline";

function uuidBytes(id: string): Uint8Array {
  const hex = id.replace(/-/g, "");
  const out = new Uint8Array(16);
  for (let i = 0; i < 16; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

function bytesUuid(b: Uint8Array): string {
  const hex = Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function b64(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s);
}

function unb64(s: string): Uint8Array {
  return Uint8Array.from(atob(s), (c) => c.charCodeAt(0));
}

export class SyncClient {
  private ws: WebSocket | null = null;
  private notes = new Map<string, NoteHandler>();
  private retry = 0;
  private stopped = false;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private status: Status = "connecting";

  onTreeChanged = () => {};
  /** A note's tasks changed somewhere: the Tasks list refetches if it's showing. */
  onTasksChanged = () => {};
  onStatus = (_: Status) => {};
  /** The pass that saves every note on this device: how far it is. */
  onNotesSaved = (_done: number, _total: number, _saving: boolean) => {};

  private savingAll = false;
  private saveAgain = false;

  constructor() {
    // A phone back in signal, or a tab brought back, tries right away instead of waiting out the backoff.
    const wake = () => {
      if (!this.ws && !this.stopped && document.visibilityState === "visible") {
        clearTimeout(this.timer);
        this.connect();
      }
    };
    addEventListener("online", wake);
    document.addEventListener("visibilitychange", wake);
  }

  connect() {
    this.stopped = false;
    clearTimeout(this.timer);
    // Retries while offline stay "offline", so the indicator doesn't flicker.
    if (this.status !== "offline") this.setStatus("connecting");
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    const ws = new WebSocket(`${proto}//${location.host}/api/ws`);
    ws.binaryType = "arraybuffer";
    ws.onopen = () => {
      this.retry = 0;
      this.setStatus("online");
      for (const [id, h] of this.notes) this.sendJoin(id, h);
      // Catch up on anything that changed while disconnected.
      this.onTreeChanged();
    };
    ws.onclose = () => {
      if (this.ws !== ws) return;
      this.ws = null;
      this.setStatus("offline");
      if (this.stopped) return;
      const delay = Math.min(10_000, 500 * 2 ** this.retry++);
      this.timer = setTimeout(() => this.connect(), delay);
    };
    ws.onmessage = (ev) => this.receive(ev.data);
    this.ws = ws;
  }

  disconnect() {
    this.stopped = true;
    clearTimeout(this.timer);
    const ws = this.ws;
    this.ws = null;
    ws?.close();
    this.status = "connecting";
  }

  private setStatus(s: Status) {
    this.status = s;
    this.onStatus(s);
  }

  open(id: string, handler: NoteHandler) {
    this.notes.set(id, handler);
    this.sendJoin(id, handler);
  }

  /** With `handler`, only if it's still the one listening (another may have taken the note over). */
  close(id: string, handler?: NoteHandler) {
    if (handler && this.notes.get(id) !== handler) return;
    if (!this.notes.delete(id)) return;
    this.sendJson({ t: "leave", note: id });
  }

  /** Re-sends join so the server fills any gap in both directions. */
  rejoin(id: string) {
    const h = this.notes.get(id);
    if (h) this.sendJoin(id, h);
  }

  /**
   * Keeps every note in the tree saved on this device: a background pass, a few notes at a time,
   * that asks the server for what each saved copy lacks, drops copies of notes gone from the
   * tree, and never touches a note being edited or with unsent edits. Re-queued while running.
   */
  saveAll(ids: () => string[]) {
    if (this.savingAll) {
      this.saveAgain = true;
      return;
    }
    this.savingAll = true;
    this.onNotesSaved(0, 0, true);
    void this.runSave(ids).finally(() => {
      this.savingAll = false;
      if (this.saveAgain) {
        this.saveAgain = false;
        this.saveAll(ids);
      }
    });
  }

  private async runSave(ids: () => string[]) {
    for (;;) {
      this.saveAgain = false;
      if (this.status !== "online") return;
      const treeIds = new Set(ids());
      const dirty = new Set(await cache.dirtyIds());
      // Notes gone from the tree: drop their copies, unless they still have unsent edits (a
      // flush on reconnect decides those) or an editor owns them.
      for (const id of await cache.noteIds()) {
        if (!treeIds.has(id) && !dirty.has(id) && !editing.has(id) && !isQueued(id)) {
          await cache.forget(id);
        }
      }
      let batch = [...treeIds].filter((id) => !editing.has(id) && !isQueued(id) && !dirty.has(id));
      let done = 0;
      while (batch.length) {
        const chunk = batch.splice(0, 20);
        // Re-read the dirty set per chunk: a tick made offline since the pass began must not be
        // overwritten by the server's copy, so skip anything now dirty or under an editor.
        const dirtyNow = new Set(await cache.dirtyIds());
        const have: Record<string, string> = {};
        for (const id of chunk) {
          if (dirtyNow.has(id) || editing.has(id)) continue;
          const v = await cache.noteVersion(id);
          have[id] = v ? b64(v) : "";
        }
        let resp: { notes: { id: string; data: string }[]; rest: string[] };
        try {
          resp = await api.noteSnapshots(have);
        } catch (err) {
          if (unreachable(err)) return;
          return;
        }
        for (const n of resp.notes) {
          // Skip a note that went dirty or got an editor while the snapshot was in flight.
          if (dirtyNow.has(n.id) || editing.has(n.id)) continue;
          const doc = new LoroDoc();
          doc.import(unb64(n.data));
          await cache.saveNote(n.id, doc);
          rememberText(n.id, plainText(doc.getText("body").toString()));
          done++;
        }
        batch = resp.rest.concat(batch);
        this.onNotesSaved(done, treeIds.size, true);
      }
      this.onNotesSaved(treeIds.size, treeIds.size, false);
      if (!this.saveAgain) return;
    }
  }

  /** Dropped while offline (returns false); the rejoin after reconnecting uploads whatever the server is missing. */
  sendUpdate(id: string, data: Uint8Array): boolean {
    return this.sendFrame(KIND_UPDATE, id, data);
  }

  sendPresence(id: string, data: Uint8Array) {
    this.sendFrame(KIND_PRESENCE, id, data);
  }

  private get isOpen() {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  private sendJoin(id: string, h: NoteHandler) {
    const v = h.version();
    this.sendJson({ t: "join", note: id, version: v ? b64(v.encode()) : null });
  }

  private sendJson(msg: unknown) {
    if (this.isOpen) this.ws!.send(JSON.stringify(msg));
  }

  private sendFrame(kind: number, id: string, payload: Uint8Array): boolean {
    if (!this.isOpen) return false;
    const buf = new Uint8Array(17 + payload.length);
    buf[0] = kind;
    buf.set(uuidBytes(id), 1);
    buf.set(payload, 17);
    this.ws!.send(buf);
    return true;
  }

  private receive(data: string | ArrayBuffer) {
    if (typeof data !== "string") {
      const bytes = new Uint8Array(data);
      if (bytes.length < 17) return;
      const h = this.notes.get(bytesUuid(bytes.subarray(1, 17)));
      const payload = bytes.subarray(17);
      if (bytes[0] === KIND_UPDATE) h?.update(payload);
      else if (bytes[0] === KIND_PRESENCE) h?.presence(payload);
      return;
    }
    const msg = JSON.parse(data);
    if (msg.t === "tree_changed") return this.onTreeChanged();
    if (msg.t === "tasks_changed") return this.onTasksChanged();
    const h = msg.note ? this.notes.get(msg.note) : undefined;
    if (!h) return;
    switch (msg.t) {
      case "joined":
        h.joined(msg.role, VersionVector.decode(unb64(msg.version)));
        break;
      case "role":
        h.role(msg.role);
        break;
      case "revoked":
        this.notes.delete(msg.note);
        h.lost("revoked");
        break;
      case "error":
        if (msg.code === "not_found") {
          this.notes.delete(msg.note);
          h.lost("not_found");
        } else if (msg.code === "out_of_sync") {
          h.outOfSync();
        } else {
          console.warn("sync error", msg);
        }
        break;
    }
  }
}

export const sync = new SyncClient();
