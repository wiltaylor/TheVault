// What changed between two versions of a note, worked out in the browser from the parsed
// documents: the fields by name, and the compound fields (content, ink, canvas, dicts) as the
// note's top-level blocks, paired by label, or by position among blocks of the same kind.

import type { Block, Json, NoteDoc } from "../api";

/** Fields the server writes on every save, which say nothing about what the version changed. */
const SERVER_FIELDS = new Set(["created", "updated"]);

export type ChangeSummary =
  | { created: true }
  | { created: false; added: string[]; removed: string[]; changed: string[] };

/** Whether two JSON values are the same, whatever order their object keys are in. */
export function sameJson(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (Array.isArray(a)) {
    const bb = b as unknown[];
    return a.length === bb.length && a.every((x, i) => sameJson(x, bb[i]));
  }
  const ao = a as Record<string, unknown>;
  const bo = b as Record<string, unknown>;
  const keys = Object.keys(ao).filter((k) => ao[k] !== undefined);
  if (keys.length !== Object.keys(bo).filter((k) => bo[k] !== undefined).length) return false;
  return keys.every((k) => k in bo && sameJson(ao[k], bo[k]));
}

/** A note's top-level blocks by name: a compound field's label, else its kind and its place among that kind. */
function blocksByName(blocks: Block[]): Map<string, Block> {
  const out = new Map<string, Block>();
  const seen: Record<string, number> = {};
  for (const b of blocks) {
    const label = b.labels?.[0];
    if (label) {
      out.set(label, b);
      continue;
    }
    const n = seen[b.kind] ?? 0;
    seen[b.kind] = n + 1;
    out.set(n === 0 ? b.kind : `${b.kind} ${n + 1}`, b);
  }
  return out;
}

function compare<T>(prev: Map<string, T>, next: Map<string, T>, into: { added: string[]; removed: string[]; changed: string[] }) {
  for (const [name, v] of next) {
    if (!prev.has(name)) into.added.push(name);
    else if (!sameJson(prev.get(name), v)) into.changed.push(name);
  }
  for (const name of prev.keys()) if (!next.has(name)) into.removed.push(name);
}

function fieldMap(doc: NoteDoc): Map<string, Json> {
  const out = new Map<string, Json>();
  // The title first, as the page shows it first.
  if (doc.fields.title !== undefined) out.set("title", doc.fields.title);
  for (const [k, v] of Object.entries(doc.fields)) if (v !== undefined && !SERVER_FIELDS.has(k)) out.set(k, v);
  return out;
}

/** What `next` changed over `prev`, the version before it; with no `prev`, the note was created. */
export function changeSummary(prev: NoteDoc | null | undefined, next: NoteDoc): ChangeSummary {
  if (!prev) return { created: true };
  const out = { added: [] as string[], removed: [] as string[], changed: [] as string[] };
  compare(fieldMap(prev), fieldMap(next), out);
  compare(blocksByName(prev.blocks ?? []), blocksByName(next.blocks ?? []), out);
  return { created: false, ...out };
}

/** The summary as a line: "created", or "changed: title, body; added: due". */
export function summaryText(s: ChangeSummary): string {
  if (s.created) return "created";
  const parts: string[] = [];
  if (s.changed.length) parts.push(`changed: ${s.changed.join(", ")}`);
  if (s.added.length) parts.push(`added: ${s.added.join(", ")}`);
  if (s.removed.length) parts.push(`removed: ${s.removed.join(", ")}`);
  return parts.length ? parts.join("; ") : "no changes";
}

const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["year", 365 * 24 * 3600_000],
  ["month", 30 * 24 * 3600_000],
  ["week", 7 * 24 * 3600_000],
  ["day", 24 * 3600_000],
  ["hour", 3600_000],
  ["minute", 60_000],
];

/** `at` relative to `now`: "just now", "5 minutes ago", "yesterday". */
export function relativeTime(at: number, now = Date.now()): string {
  const diff = at - now;
  if (Math.abs(diff) < 60_000) return "just now";
  const fmt = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  for (const [unit, ms] of UNITS) {
    if (Math.abs(diff) >= ms) return fmt.format(Math.round(diff / ms), unit);
  }
  return "just now";
}
