// Lists of links, such as a project's documentation: which fields are one, and how a link is
// read, shown and stored. The editor and the read-only links are in components/LinkList.tsx.

import type { FieldDef, Json } from "./api";

/** How a list of links stores each item: whether `name` must be written. */
export interface LinkShape {
  nameRequired: boolean;
}

/**
 * The shape of `f`'s items when it is a list of links: a list whose item is a dict with exactly
 * two declared keys, `name` (text) and `url` (a URL), in either order, and no other entries.
 * Else undefined.
 */
export function linkShape(f: FieldDef): LinkShape | undefined {
  const item = f.kind === "list" && !f.listed ? f.item : undefined;
  if (!item || item.kind !== "dict" || (item.values !== null && item.values !== undefined)) return undefined;
  const keys = item.keys ?? [];
  if (keys.length !== 2 || keys.some((k) => k.multiple)) return undefined;
  const name = keys.find((k) => k.name === "name" && k.kind === "string");
  const url = keys.find((k) => k.name === "url" && k.kind === "url");
  return name && url ? { nameRequired: name.required === true } : undefined;
}

/** One link: its name ("" for none) and its URL. */
export interface Link {
  name: string;
  url: string;
}

const isRecord = (v: Json | undefined): v is Record<string, Json> => !!v && typeof v === "object" && !Array.isArray(v);
const text = (v: Json | undefined): string => (typeof v === "string" ? v : v === undefined || v === null ? "" : String(v));

/** A stored list's links; a bare string is a link with no name. */
export function linksOf(value: Json | undefined): Link[] {
  const items = Array.isArray(value) ? value : value === undefined || value === null ? [] : [value];
  return items.map((x) => (isRecord(x) ? { name: text(x.name), url: text(x.url) } : { name: "", url: text(x) }));
}

/**
 * The list as it is stored: one record per link, `name` left out where a link has none (unless
 * the shape requires it), and nothing at all for an empty list.
 */
export function linksValue(links: Link[], shape: LinkShape): Json | undefined {
  if (!links.length) return undefined;
  return links.map((l): Json => (l.name || shape.nameRequired ? { name: l.name, url: l.url } : { url: l.url }));
}

/**
 * Whether `s` is an absolute URL, as a `url` field takes it (the schema's `is_url`): a scheme,
 * then something; after `//` a host; no whitespace anywhere.
 */
export function isUrl(s: string): boolean {
  const colon = s.indexOf(":");
  if (colon < 1) return false;
  const scheme = s.slice(0, colon);
  const rest = s.slice(colon + 1);
  if (!/^[A-Za-z][A-Za-z0-9+.-]*$/.test(scheme) || !rest || /[\s\u0000-\u001f\u007f]/.test(s)) return false;
  return rest.startsWith("//") ? rest.slice(2).split(/[/?#]/)[0] !== "" : true;
}

/** A URL a link may open: a web page or an email address, never a script. */
export const SAFE_LINK = /^(https?:|mailto:)/i;

/** A URL's host without a leading `www.`, as `example.com`; "" when it has none. */
export function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./i, "");
  } catch {
    return "";
  }
}

/** What a link shows: its name, else its URL's host and path, as `example.com/docs`, else the URL. */
export function linkText(l: Link): string {
  if (l.name.trim()) return l.name;
  try {
    const u = new URL(l.url);
    if (!u.hostname) return l.url;
    const path = u.pathname === "/" ? "" : u.pathname.replace(/\/$/, "");
    return `${u.hostname.replace(/^www\./i, "")}${path}`;
  } catch {
    return l.url;
  }
}

/** Read-only text: "Docs, example.com/blog". */
export function linksText(links: Link[]): string {
  return links.map(linkText).join(", ");
}
