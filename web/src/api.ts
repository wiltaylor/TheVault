// The typed client of TheVault's HTTP API. Every UI action goes through one of these calls.
// The browser authenticates with the session cookie that POST /api/login sets (D11).

import type { SchemaType } from "./defs/typeModel";
import type { QueryModel } from "./editor/queryModel";

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
  /** `date_range`: whether the ends are dates or datetimes. */
  of?: "date" | "datetime";
  taxonomy?: string;
  target?: string;
  relation?: "one_way" | "two_way" | "peer";
  inverse?: string | null;
  items?: string;
  values?: string;
  /** `location`: the image an image map's pins sit on, null on a real-world map. */
  image?: string | null;
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

export interface Term {
  name: string;
  path: string;
  colour: string | null;
  description: string | null;
  children: Term[];
}

export interface Taxonomy {
  name: string;
  label: string;
  description: string | null;
  terms: Term[];
}

/** A planned type change (D6): what it adds, removes and renames, and every note it edits. */
export interface TypePlan {
  type: string;
  applied: boolean;
  plan: string;
  wcl: string;
  added: string[];
  removed: string[];
  renamed: { from: string; to: string }[];
  changes: { id: string; title: string; version: string; edits: { op: string; [key: string]: Json }[] }[];
  flagged: string[];
}

export interface Row {
  id: string;
  type: string;
  title: string;
  fields: Record<string, Json>;
}

/** An uploaded file, stored at `assets/<id>/<filename>` (D22). */
export interface Asset {
  id: string;
  filename: string;
  size: number;
  path: string;
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
  if (body instanceof Blob) {
    init.headers = { ...init.headers, "Content-Type": "application/octet-stream" };
    init.body = body;
  } else if (body !== undefined) {
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
  /** Every type as schema inspection describes it, with handlers and reference rules. */
  schema: () => request<{ types: SchemaType[]; taxonomies: Taxonomy[] }>("GET", "/api/schema"),
  /** Create a type, or replace one that has no notes. */
  putType: (name: string, wcl: string) => request<{ name: string; wcl: string }>("PUT", `/api/types/${enc(name)}`, { wcl }),
  /** Plan a change to a type, or apply the plan whose token is `plan`. */
  changeType: (name: string, wcl: string, renames: Record<string, string>, plan?: string) =>
    request<TypePlan>("POST", `/api/types/${enc(name)}/change`, { wcl, renames, ...(plan ? { plan } : {}) }),
  putTemplate: (type: string, name: string, wcl: string) =>
    request<{ wcl: string }>("PUT", `/api/types/${enc(type)}/templates/${enc(name)}`, { wcl }),
  deleteTemplate: (type: string, name: string) =>
    request<unknown>("DELETE", `/api/types/${enc(type)}/templates/${enc(name)}`),
  taxonomies: async () => (await request<{ taxonomies: Taxonomy[] }>("GET", "/api/taxonomies")).taxonomies,
  /** Create or replace a taxonomy. `renames` maps old term paths to new ones. */
  putTaxonomy: (name: string, wcl: string, renames: Record<string, string>) =>
    request<Taxonomy & { notes: { id: string }[] }>("PUT", `/api/taxonomies/${enc(name)}`, { wcl, renames }),
  /** One page of `query`. `thisId` is the note that holds the query, bound to `this`. */
  query: (query: string, cursor: string | null, pageSize: number, thisId?: string) =>
    request<Page>("POST", "/api/query", {
      query,
      ...(thisId ? { this: thisId } : {}),
      ...(cursor ? { cursor } : {}),
      page_size: pageSize,
    }),
  /** Check `query`, and get the builder's model of it, or why the builder cannot show it (D8). */
  parseQuery: (query: string) =>
    request<{ text: string; model: QueryModel | null; builder_error: string | null }>("POST", "/api/query/parse", {
      query,
    }),
  /** The canonical text of a builder model. A model that is not a query yet is a 400. */
  queryText: async (model: QueryModel) =>
    (await request<{ text: string }>("POST", "/api/query/text", { model })).text,
  search: (q: string, cursor: string | null, pageSize: number) => {
    const params = new URLSearchParams({ q, page_size: String(pageSize) });
    if (cursor) params.set("cursor", cursor);
    return request<Page>("GET", `/api/search?${params}`);
  },
  assets: async () => (await request<{ assets: Asset[] }>("GET", "/api/assets")).assets,
  /** Upload `file` as a new asset. The body is the file's bytes, unchanged. */
  uploadAsset: (file: File) =>
    request<Asset>("POST", `/api/assets?filename=${enc(file.name)}`, file),
};

/** Where the browser fetches an asset's bytes. */
export function assetUrl(id: string): string {
  return `/api/assets/${enc(id)}`;
}

