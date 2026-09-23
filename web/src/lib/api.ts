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
}

export interface TreeNote {
  id: string;
  notebook_id: string | null;
  title: string;
  preview: string;
  owner: string;
  role: Role;
  updated_at: number;
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
  tree: () => request<Tree>("GET", "/tree"),

  createNotebook: (name: string, parent_id: string | null) =>
    request<{ id: string }>("POST", "/notebooks", { name, parent_id }),
  renameNotebook: (id: string, name: string) => request("PATCH", `/notebooks/${id}`, { name }),
  deleteNotebook: (id: string) => request("DELETE", `/notebooks/${id}`),

  createNote: (notebook_id: string | null) => request<{ id: string }>("POST", "/notes", { notebook_id }),
  deleteNote: (id: string) => request("DELETE", `/notes/${id}`),
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
