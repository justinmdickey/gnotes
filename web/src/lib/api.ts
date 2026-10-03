export type Role = "viewer" | "editor" | "owner";

export interface User {
  id: string;
  username: string;
  display_name: string;
  is_admin: boolean;
}

export interface TreeNotebook {
  id: string;
  parent_id: string | null;
  name: string;
  owner: string;
  role: Role;
  updated_at: number;
  /** Has a share of its own, either given by you or to you. */
  shared: boolean;
}

export interface TreeNote {
  id: string;
  notebook_id: string | null;
  title: string;
  preview: string;
  owner: string;
  role: Role;
  updated_at: number;
  shared: boolean;
}

export interface SharedRoot {
  share_id: string;
  resource_type: "note" | "notebook";
  resource_id: string;
  hidden: boolean;
}

/** Something you deleted directly, not what was swept along inside a deleted notebook. */
export interface TrashItem {
  resource_type: "note" | "notebook";
  id: string;
  /** A notebook's name or a note's title. */
  name: string;
  deleted_at: number;
  /** When the server removes it for good. */
  purge_at: number;
}

export interface Tree {
  notebooks: TreeNotebook[];
  notes: TreeNote[];
  shared: SharedRoot[];
}

export interface ShareInfo {
  id: string;
  username: string;
  display_name: string;
  role: "viewer" | "editor";
}

export interface UserSummary {
  id: string;
  username: string;
  display_name: string;
}

export interface CreatedInvite {
  id: string;
  token: string;
  expires_at: number;
}

export interface PendingInvite {
  id: string;
  resource_type: "note" | "notebook" | null;
  resource_id: string | null;
  role: string;
  created_at: number;
  expires_at: number;
}

export interface InvitePreview {
  inviter: string;
  shared: string | null;
  expires_at: number;
}

export interface AdminUser {
  id: string;
  username: string;
  display_name: string;
  is_admin: boolean;
  disabled: boolean;
  created_at: number;
}

export interface AttachmentMeta {
  id: string;
  note_id: string;
  filename: string;
  mime: string;
  size: number;
}

export interface ImportResult {
  notes: number;
  notebooks: number;
  attachments: number;
  /** Files that weren't notes or something a note links to. */
  skipped: string[];
  /** The notebook the first zip became. */
  notebook_id: string | null;
}

/** A piece of a search snippet; `hit` pieces are the words that matched. */
export interface SnippetPart {
  text: string;
  hit: boolean;
}

/** A note that matched a search, best first. Only notes you can see come back. */
export interface SearchResult {
  note: string;
  title: string;
  /** A few words around the match, from anywhere in the note. */
  snippet: SnippetPart[];
  /** Which search found it: "text" (full-text) or "meaning" (semantic search, whose snippet has no hits). */
  source: string;
}

/** One message of an Ask conversation. */
export interface AskMessage {
  role: "user" | "assistant";
  text: string;
}

/** A note Ask found for a question; `n` is its number in the answer's [n] citations. */
export interface AskNote {
  n: number;
  note: string;
  title: string;
  /** The passage that matched, with the question's words as hits. */
  snippet: SnippetPart[];
}

/** The notes Ask found, which come before the answer. */
export interface AskSources {
  /** What was searched: the question, or a follow-up rewritten to stand on its own. */
  query: string;
  /** Its words, lowercased, for marking them in titles. */
  terms: string[];
  notes: AskNote[];
}

/** Ask's finished answer, with [n] citations; null when the notes don't say. `cited` are numbers of notes it was given. */
export interface AskDone {
  answer: string | null;
  cited: number[];
}

/** A note's AI summary. `summary` is null until someone asks for one. */
export interface NoteSummary {
  summary: string | null;
  /** The note has changed since this summary was made. */
  stale?: boolean;
  created_at?: number;
}

export interface Features {
  transcription: boolean;
  /** Text appears in the note while a voice memo is being recorded. */
  live_transcription: boolean;
  /** Photos can have the text in them read out, on request. */
  photo_text: boolean;
  /** Notes can be summarized, on request, in their Summary tab. */
  summaries: boolean;
  /** Search also finds notes by meaning. Absent from older servers. */
  semantic_search?: boolean;
  /** Questions can be asked of your notes (semantic search plus the summary chat model). */
  ask?: boolean;
  max_upload: number;
  /** The server's release, e.g. "0.7.0", or "dev". */
  version: string;
}

/** An outside service the server calls (speech-to-text, photo reading), as Settings shows it. */
export interface ServiceSettings {
  enabled: boolean;
  url: string;
  model: string;
  has_key: boolean;
  from_env: boolean;
  /** Speech-to-text only: the live (websocket) endpoint. */
  realtime_url?: string;
}

/** `key` absent keeps the saved one; null clears it. */
export interface ServiceInput {
  url: string;
  model: string;
  key?: string | null;
  realtime_url?: string;
  /** False switches the service off but keeps what's filled in. */
  enabled?: boolean;
}

export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
  ) {
    super(message);
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api${path}`, {
    method,
    headers: body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    credentials: "same-origin",
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new ApiError(res.status, data.error ?? "unknown", data.message ?? res.statusText);
  return data as T;
}

/**
 * The next turn of an Ask conversation (`messages` ends with the new question). The server streams
 * it: `onSources` gets the notes found, `onText` each piece of the answer as it's written, and the
 * promise resolves with the whole answer.
 */
async function ask(
  messages: AskMessage[],
  on: { onSources: (s: AskSources) => void; onText: (piece: string) => void },
  signal?: AbortSignal,
): Promise<AskDone> {
  const res = await fetch("/api/ask", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ messages }),
    credentials: "same-origin",
    signal,
  });
  if (!res.ok || !res.body) {
    const data = await res.json().catch(() => ({}));
    throw new ApiError(res.status, data.error ?? "unknown", data.message ?? res.statusText);
  }
  // Server-sent events: blocks of "event: <name>" and "data: <json>" lines, a blank line apart.
  const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
  let buffer = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buffer += value;
    let end;
    while ((end = buffer.indexOf("\n\n")) >= 0) {
      const block = buffer.slice(0, end);
      buffer = buffer.slice(end + 2);
      const name = /^event: ?(.*)$/m.exec(block)?.[1];
      const data = /^data: ?(.*)$/m.exec(block)?.[1];
      if (!name || data === undefined) continue;
      const body = JSON.parse(data);
      if (name === "sources") on.onSources(body);
      else if (name === "delta") on.onText(body.text);
      else if (name === "done") return body;
      else if (name === "error") throw new ApiError(409, "conflict", body.message);
    }
  }
  throw new ApiError(0, "unknown", "The answer stopped partway");
}

export const api = {
  me: () => request<User>("GET", "/me"),
  login: (username: string, password: string) => request<User>("POST", "/auth/login", { username, password }),
  logout: () => request("POST", "/auth/logout", {}),
  setupNeeded: () => request<{ needed: boolean }>("GET", "/auth/setup"),
  setup: (username: string, display_name: string, password: string) =>
    request<User>("POST", "/auth/setup", { username, display_name, password }),
  users: () => request<UserSummary[]>("GET", "/users"),
  updateMe: (display_name: string) => request<User>("PATCH", "/me", { display_name }),
  changePassword: (current: string, next: string) => request("POST", "/me/password", { current, new: next }),
  logoutOthers: () => request("POST", "/me/logout-others", {}),
  adminUsers: () => request<AdminUser[]>("GET", "/admin/users"),
  adminUpdateUser: (id: string, patch: { is_admin?: boolean; disabled?: boolean }) =>
    request("PATCH", `/admin/users/${id}`, patch),
  adminResetPassword: (id: string, password: string) => request("POST", `/admin/users/${id}/password`, { password }),
  tree: () => request<Tree>("GET", "/tree"),
  search: (q: string) => request<{ results: SearchResult[] }>("GET", `/search?q=${encodeURIComponent(q)}`),
  /** Notes about what `q` means, even without its words. Empty when semantic search is off. */
  searchMeaning: (q: string) => request<{ results: SearchResult[] }>("GET", `/search/meaning?q=${encodeURIComponent(q)}`),
  ask,

  createNotebook: (name: string, parent_id: string | null) =>
    request<{ id: string }>("POST", "/notebooks", { name, parent_id }),
  renameNotebook: (id: string, name: string) => request("PATCH", `/notebooks/${id}`, { name }),
  moveNotebook: (id: string, parent_id: string | null) => request("PATCH", `/notebooks/${id}`, { parent_id }),
  moveNote: (id: string, notebook_id: string | null) => request("PATCH", `/notes/${id}`, { notebook_id }),
  deleteNotebook: (id: string) => request("DELETE", `/notebooks/${id}`),

  /** `id` is a client-made UUIDv7, so a note made offline keeps the same id. */
  createNote: (notebook_id: string | null, id?: string) => request<{ id: string }>("POST", "/notes", { id, notebook_id }),
  deleteNote: (id: string) => request("DELETE", `/notes/${id}`),
  adminSettings: () =>
    request<{ whisper: ServiceSettings; vision: ServiceSettings; summary: ServiceSettings; embed: ServiceSettings }>("GET", "/admin/settings"),
  saveWhisper: (body: ServiceInput) => request<{ whisper: ServiceSettings }>("PUT", "/admin/settings/whisper", body),
  testWhisper: (body: ServiceInput) => request<{ ok: boolean; message: string }>("POST", "/admin/settings/whisper/test", body),
  /** The services set by URL, model and key: reading photos, summarizing notes, and embeddings for semantic search. */
  saveChat: <K extends "vision" | "summary" | "embed">(kind: K, body: ServiceInput) =>
    request<Record<K, ServiceSettings>>("PUT", `/admin/settings/${kind}`, body).then((r) => r[kind]),
  testChat: (kind: "vision" | "summary" | "embed", body: ServiceInput) =>
    request<{ ok: boolean; message: string }>("POST", `/admin/settings/${kind}/test`, body),
  /** Embeds every note again, for a model swapped under the same name. */
  reindexEmbed: () => request<{ notes: number }>("POST", "/admin/semantic/reindex"),
  summary: (noteId: string) => request<NoteSummary>("GET", `/notes/${noteId}/summary`),
  /** Whose editing session each Loro peer was, for showing who wrote what. */
  authors: (noteId: string) => request<{ peers: Record<string, { user_id: string | null; name: string | null }> }>("GET", `/notes/${noteId}/authors`),
  summarize: (noteId: string) => request<NoteSummary>("POST", `/notes/${noteId}/summary`, {}),
  /** `text` improved by the summary chat model, with nothing of it lost. Nothing is saved. */
  tidy: (noteId: string, text: string) => request<{ original: string; text: string }>("POST", `/notes/${noteId}/tidy`, { text }),
  photoText: (id: string) => request<{ text: string }>("POST", `/attachments/${id}/text`, {}),
  features: () => request<Features>("GET", "/features"),
  attachmentMeta: (id: string) => request<AttachmentMeta>("GET", `/attachments/${id}/meta`),
  transcribe: (id: string) => request<{ text: string }>("POST", `/attachments/${id}/transcribe`, {}),
  async upload(noteId: string, file: File): Promise<AttachmentMeta> {
    const form = new FormData();
    form.append("note_id", noteId);
    form.append("file", file, file.name);
    const res = await fetch("/api/attachments", { method: "POST", body: form, credentials: "same-origin" });
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw new ApiError(res.status, data.error ?? "unknown", data.message ?? res.statusText);
    return data as AttachmentMeta;
  },
  /** Markdown files or zips of them; a zip becomes a notebook. `notebookId` null is the top folder. */
  async importNotes(files: File[], notebookId: string | null): Promise<ImportResult> {
    const form = new FormData();
    if (notebookId) form.append("notebook_id", notebookId);
    for (const file of files) form.append("file", file, file.name);
    const res = await fetch("/api/import", { method: "POST", body: form, credentials: "same-origin" });
    const data = await res.json().catch(() => ({}));
    if (res.status === 413) throw new ApiError(413, "too_large", "Imports can be up to 200 MB at a time");
    if (!res.ok) throw new ApiError(res.status, data.error ?? "unknown", data.message ?? res.statusText);
    return data as ImportResult;
  },
  trash: () => request<TrashItem[]>("GET", "/trash"),
  restore: (kind: "note" | "notebook", id: string) => request("POST", `/trash/${kind}/${id}/restore`, {}),
  /** Deletes a trashed item for good, without waiting out the 30 days. */
  deleteForever: (kind: "note" | "notebook", id: string) => request("DELETE", `/trash/${kind}/${id}`),
  emptyTrash: () => request<{ deleted: number }>("DELETE", "/trash"),
  /** Permanently removes the note only if it's blank; the server refuses otherwise. */
  discardNote: (id: string) => request("DELETE", `/notes/${id}?discard=true`),

  shares: (kind: "note" | "notebook", id: string) => request<ShareInfo[]>("GET", `/${kind}s/${id}/shares`),
  share: (resource_type: "note" | "notebook", resource_id: string, username: string, role: string) =>
    request<{ id: string }>("POST", "/shares", { resource_type, resource_id, username, role }),
  setShareRole: (id: string, role: string) => request("PATCH", `/shares/${id}`, { role }),
  unshare: (id: string) => request("DELETE", `/shares/${id}`),

  invite: (resource?: { kind: "note" | "notebook"; id: string }, role = "editor") =>
    request<CreatedInvite>("POST", "/invites", {
      resource_type: resource?.kind ?? null,
      resource_id: resource?.id ?? null,
      role,
    }),
  invites: () => request<PendingInvite[]>("GET", "/invites"),
  revokeInvite: (id: string) => request("DELETE", `/invites/${id}`),
  previewJoin: (token: string) => request<InvitePreview>("GET", `/join/${token}`),
  join: (token: string, username: string, display_name: string, password: string) =>
    request<User>("POST", `/join/${token}`, { username, display_name, password }),
};

export const inviteUrl = (token: string) => `${location.origin}/join/${token}`;
