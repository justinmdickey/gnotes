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

export interface Features {
  transcription: boolean;
  max_upload: number;
}

export interface WhisperSettings {
  enabled: boolean;
  url: string;
  model: string;
  has_key: boolean;
  from_env: boolean;
}

/** `key` absent keeps the saved one; null clears it. */
export interface WhisperInput {
  url: string;
  model: string;
  key?: string | null;
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
  adminSettings: () => request<{ whisper: WhisperSettings }>("GET", "/admin/settings"),
  saveWhisper: (body: WhisperInput) => request<{ whisper: WhisperSettings }>("PUT", "/admin/settings/whisper", body),
  testWhisper: (body: WhisperInput) => request<{ ok: boolean; message: string }>("POST", "/admin/settings/whisper/test", body),
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
