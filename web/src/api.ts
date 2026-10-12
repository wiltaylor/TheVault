// The typed client of TheVault's HTTP API. Every UI action goes through one of these calls.
// The browser authenticates with the session cookie that POST /api/login sets (D11).

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

/** A content block in its JSON form: the block's kind, labels, items and nested blocks. */
export interface Block {
  kind: string;
  labels?: string[];
  fields?: Record<string, Json>;
  blocks?: Block[];
}

/** A note's JSON form: its fields by name and its compound fields as labelled blocks. */
export interface NoteDoc {
  fields: Record<string, Json>;
  blocks: Block[];
}

export interface Note {
  id: string;
  type: string;
  version: string;
  wcl: string;
  note: NoteDoc;
}

export interface FieldDef {
  name: string;
  label: string;
  description: string | null;
  kind: string;
  required: boolean;
  computed: boolean;
  multiple: boolean;
  compound: boolean;
  default?: Json;
  options?: string[];
}

export type TemplateItem = { slot: string; label: string | null } | { block: Block };

export interface Tab {
  title: string;
  items: TemplateItem[];
}

export interface Template {
  name: string;
  label: string | null;
  tabs: Tab[];
}

export interface TypeDef {
  name: string;
  label: string;
  plural: string;
  description: string | null;
  icon: string | null;
  colour: string | null;
  fields: FieldDef[];
  templates: Template[];
}

export interface Row {
  id: string;
  type: string;
  title: string;
  fields: Record<string, Json>;
}

export interface Page {
  rows: Row[];
  next: string | null;
  total: number;
}

/** A response other than 2xx, with the API's JSON error body when it sent one. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly body: { error?: string; message?: string; [key: string]: unknown },
  ) {
    super(body.message ?? `request failed with ${status}`);
  }

  /** A save whose base version is no longer the note's current version (D12). */
  get isConflict(): boolean {
    return this.status === 409 && this.body.error === "conflict";
  }
}

let onUnauthorized: () => void = () => {};

/** Called when a request other than login answers 401, so the app can return to the login page. */
export function setUnauthorizedHandler(handler: () => void) {
  onUnauthorized = handler;
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const init: RequestInit = {
    method,
    credentials: "same-origin",
    headers: { Accept: "application/json" },
  };
  if (body !== undefined) {
    init.headers = { ...init.headers, "Content-Type": "application/json" };
    init.body = JSON.stringify(body);
  }
  const res = await fetch(path, init);
  if (res.status === 204) {
    return undefined as T;
  }
  const text = await res.text();
  let parsed: unknown = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = { message: text };
  }
  if (!res.ok) {
    if (res.status === 401 && path !== "/api/login") {
      onUnauthorized();
    }
    const errBody = parsed && typeof parsed === "object" ? (parsed as ApiError["body"]) : {};
    throw new ApiError(res.status, errBody);
  }
  return parsed as T;
}

const enc = encodeURIComponent;

export const api = {
  login: (password: string) => request<{ expires_in: number }>("POST", "/api/login", { password }),
  logout: () => request<void>("POST", "/api/logout"),
  types: async () => (await request<{ types: TypeDef[] }>("GET", "/api/types")).types,
  note: (id: string) => request<Note>("GET", `/api/notes/${enc(id)}`),
  createNote: (type: string, note: NoteDoc) =>
    request<Note>("POST", `/api/notes?type=${enc(type)}`, { note }),
  /** Save `note` over version `base`. A stale base is an ApiError whose `isConflict` is true. */
  updateNote: (id: string, note: NoteDoc, base: string) =>
    request<Note>("PUT", `/api/notes/${enc(id)}`, { note, base_version: base }),
  query: (query: string, cursor: string | null, pageSize: number) =>
    request<Page>("POST", "/api/query", {
      query,
      ...(cursor ? { cursor } : {}),
      page_size: pageSize,
    }),
  search: (q: string, cursor: string | null, pageSize: number) => {
    const params = new URLSearchParams({ q, page_size: String(pageSize) });
    if (cursor) params.set("cursor", cursor);
    return request<Page>("GET", `/api/search?${params}`);
  },
};

