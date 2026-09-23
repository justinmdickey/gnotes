// One websocket for every open note. Protocol: "Websocket protocol" in docs/DESIGN.md.
import { VersionVector } from "loro-crdt";
import type { Role } from "./api";

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

  onTreeChanged = () => {};
  onStatus = (_: Status) => {};

  connect() {
    this.stopped = false;
    this.onStatus("connecting");
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    const ws = new WebSocket(`${proto}//${location.host}/api/ws`);
    ws.binaryType = "arraybuffer";
    ws.onopen = () => {
      this.retry = 0;
      this.onStatus("online");
      for (const [id, h] of this.notes) this.sendJoin(id, h);
      // Catch up on anything that changed while disconnected.
      this.onTreeChanged();
    };
    ws.onclose = () => {
      this.ws = null;
      this.onStatus("offline");
      if (this.stopped) return;
      const delay = Math.min(10_000, 500 * 2 ** this.retry++);
      setTimeout(() => this.connect(), delay);
    };
    ws.onmessage = (ev) => this.receive(ev.data);
    this.ws = ws;
  }

  disconnect() {
    this.stopped = true;
    this.ws?.close();
  }

  open(id: string, handler: NoteHandler) {
    this.notes.set(id, handler);
    this.sendJoin(id, handler);
  }

  close(id: string) {
    if (!this.notes.delete(id)) return;
    this.sendJson({ t: "leave", note: id });
  }

  /** Re-sends join so the server fills any gap in both directions. */
  rejoin(id: string) {
    const h = this.notes.get(id);
    if (h) this.sendJoin(id, h);
  }

  /** Dropped while offline; the rejoin after reconnecting uploads whatever the server is missing. */
  sendUpdate(id: string, data: Uint8Array) {
    this.sendFrame(KIND_UPDATE, id, data);
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

  private sendFrame(kind: number, id: string, payload: Uint8Array) {
    if (!this.isOpen) return;
    const buf = new Uint8Array(17 + payload.length);
    buf[0] = kind;
    buf.set(uuidBytes(id), 1);
    buf.set(payload, 17);
    this.ws!.send(buf);
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
