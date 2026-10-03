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

  createNotebook: (name: string, parent_id: string | null) =>
    request<{ id: string }>("POST", "/notebooks", { name, parent_id }),
  renameNotebook: (id: string, name: string) => request("PATCH", `/notebooks/${id}`, { name }),
  moveNotebook: (id: string, parent_id: string | null) => request("PATCH", `/notebooks/${id}`, { parent_id }),
  moveNote: (id: string, notebook_id: string | null) => request("PATCH", `/notes/${id}`, { notebook_id }),
  deleteNotebook: (id: string) => request("DELETE", `/notebooks/${id}`),

  createNote: (notebook_id: string | null) => request<{ id: string }>("POST", "/notes", { notebook_id }),
  deleteNote: (id: string) => request("DELETE", `/notes/${id}`),
  adminSettings: () => request<{ whisper: ServiceSettings; vision: ServiceSettings; summary: ServiceSettings }>("GET", "/admin/settings"),
  saveWhisper: (body: ServiceInput) => request<{ whisper: ServiceSettings }>("PUT", "/admin/settings/whisper", body),
  testWhisper: (body: ServiceInput) => request<{ ok: boolean; message: string }>("POST", "/admin/settings/whisper/test", body),
  /** The chat-API services: reading photos and summarizing notes. */
  saveChat: <K extends "vision" | "summary">(kind: K, body: ServiceInput) =>
    request<Record<K, ServiceSettings>>("PUT", `/admin/settings/${kind}`, body).then((r) => r[kind]),
  testChat: (kind: "vision" | "summary", body: ServiceInput) =>
    request<{ ok: boolean; message: string }>("POST", `/admin/settings/${kind}/test`, body),
  summary: (noteId: string) => request<NoteSummary>("GET", `/notes/${noteId}/summary`),
  /** Whose editing session each Loro peer was, for showing who wrote what. */
  authors: (noteId: string) => request<{ peers: Record<string, { user_id: string | null; name: string | null }> }>("GET", `/notes/${noteId}/authors`),
  summarize: (noteId: string) => request<NoteSummary>("POST", `/notes/${noteId}/summary`, {}),
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
