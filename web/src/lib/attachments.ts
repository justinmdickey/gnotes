// Photos and voice memos in notes. A note embeds them as `![label](att:<id>)` on a line of its own.
import type { EditorView } from "@codemirror/view";
import { api, type AttachmentMeta } from "./api";

/**
 * An embed line. It may carry a block mark (heading, quote, list) that a text style put
 * there; the embed still shows as the photo or player.
 */
export const EMBED = /^\s*(?:#{1,6}\s+|>\s?|[-*+]\s+(?:\[[ xX]\]\s+)?|\d+[.)]\s+)?!\[([^\]\n]*)\]\(att:([0-9a-f-]{36})\)\s*$/;
export const attachmentUrl = (id: string) => `/api/attachments/${id}`;

const metas = new Map<string, Promise<AttachmentMeta>>();

/** Cached, so every redraw of an embed doesn't refetch it. */
export function attachmentMeta(id: string): Promise<AttachmentMeta> {
  let m = metas.get(id);
  if (!m) {
    m = api.attachmentMeta(id);
    m.catch(() => metas.delete(id));
    metas.set(id, m);
  }
  return m;
}

/** Recordings being transcribed right now; their players show a spinner. */
const transcribing = new Set<string>();
const listeners = new Set<() => void>();

export function isTranscribing(id: string) {
  return transcribing.has(id);
}

export function onTranscribingChange(fn: () => void) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function setTranscribing(id: string, on: boolean) {
  if (on) transcribing.add(id);
  else transcribing.delete(id);
  for (const fn of listeners) fn();
}

const MAX_SIDE = 2048;

/**
 * Phone photos are often 5-12 MB. Shrink big ones to 2048px JPEG before upload;
 * GIFs keep their animation and small images go as they are.
 */
export async function prepareImage(file: File): Promise<File> {
  if (file.type === "image/gif" || !file.type.startsWith("image/")) return file;
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file, { imageOrientation: "from-image" });
  } catch {
    return file;
  }
  const scale = Math.min(1, MAX_SIDE / Math.max(bitmap.width, bitmap.height));
  if (scale === 1 && file.size < 1.5 * 1024 * 1024) {
    bitmap.close();
    return file;
  }
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(bitmap.width * scale);
  canvas.height = Math.round(bitmap.height * scale);
  canvas.getContext("2d")!.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close();
  const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, "image/jpeg", 0.85));
  if (!blob || blob.size >= file.size) return file;
  return new File([blob], file.name.replace(/\.\w+$/, "") + ".jpg", { type: "image/jpeg" });
}

/**
 * Puts an embed on its own line at the cursor (or at the end, when nobody is typing)
 * and leaves the cursor on the line after it.
 */
export function insertEmbed(view: EditorView, id: string, label: string, atEnd = false) {
  const { state } = view;
  const line = atEnd ? state.doc.line(state.doc.lines) : state.doc.lineAt(state.selection.main.head);
  const embed = `![${label.replace(/[[\]\n]/g, "")}](att:${id})`;
  // The first line is the note's title, so an embed never takes it over.
  const empty = line.text.trim() === "" && line.number > 1;
  const from = empty ? line.from : line.to;
  const to = line.to;
  const before = empty ? "" : "\n";
  // Keep one line after the embed to type on.
  const nextLine = line.number < state.doc.lines ? state.doc.line(line.number + 1) : null;
  const after = nextLine && nextLine.text.trim() === "" ? "" : "\n";
  const insert = before + embed + after;
  // Start of the (possibly just inserted) line after the embed.
  const cursor = from + before.length + embed.length + 1;
  view.dispatch({
    changes: { from, to, insert },
    selection: { anchor: cursor },
    scrollIntoView: true,
  });
}

export async function addImages(view: EditorView, noteId: string, files: File[], atEnd = false) {
  for (const file of files) {
    const att = await api.upload(noteId, await prepareImage(file));
    insertEmbed(view, att.id, "Photo", atEnd);
  }
}

/**
 * Uploads a recording, embeds it, then (when the server has speech-to-text) puts the
 * transcript on the line under it. The embed is found again by id, since other people
 * may have edited the note in the meantime.
 */
export async function addRecording(view: EditorView, noteId: string, audio: File, transcribe: boolean, atEnd = false) {
  const att = await api.upload(noteId, audio);
  insertEmbed(view, att.id, "Voice memo", atEnd);
  if (!transcribe) return;
  setTranscribing(att.id, true);
  try {
    const { text } = await api.transcribe(att.id);
    // The note was closed meanwhile; the recording is still there to transcribe later.
    if (!text || !view.dom.isConnected) return;
    const { doc } = view.state;
    for (let n = 1; n <= doc.lines; n++) {
      const line = doc.line(n);
      if (!line.text.includes(`att:${att.id}`)) continue;
      view.dispatch({ changes: { from: line.to, insert: `\n${text}` } });
      break;
    }
  } finally {
    setTranscribing(att.id, false);
  }
}
