// The unified inbox (TV-20): unprocessed captures, incoming mail, incoming messages and missed
// calls in one list, newest first by each item's own time. The query language sorts by a named
// field, and each type keeps its time in another field, so the inbox runs one query per source
// and merges the rows here. Every query goes to /api/query, which the apps answer from their
// local index, so the inbox works offline there too.

import type { Json, Page, Row } from "../api";
import { FLEETING, PROCESSED } from "../capture/model";

export type SourceKey = "captures" | "mail" | "messages" | "calls";

export interface Source {
  key: SourceKey;
  /** The filter chip's label. */
  label: string;
  type: string;
  /** What the source lists besides `!processed`: incoming items only. */
  where: string | null;
  /** The field holding the item's own time. */
  time: string;
  /** Fields to select, or none for every stored field. */
  select: (extra: string[]) => string[] | null;
}

/** Gmail's label for mail the user sent, a term of the `gmail_label` taxonomy. */
export const SENT_LABEL = "system/SENT";

export const SOURCES: readonly Source[] = [
  {
    key: "captures",
    label: "Captures",
    type: FLEETING,
    where: null,
    // A row holds the note's stored fields, and `created` is not one of them.
    time: "created",
    select: (extra) => ["created", PROCESSED, ...extra],
  },
  { key: "mail", label: "Mail", type: "email", where: `!under(labels, "${SENT_LABEL}")`, time: "sent", select: () => null },
  { key: "messages", label: "Messages", type: "message", where: `direction == "in"`, time: "sent", select: () => null },
  { key: "calls", label: "Calls", type: "call", where: `direction == "missed"`, time: "started", select: () => null },
];

/** A filter chip: every source, or one. */
export type Filter = "all" | SourceKey;

export const FILTERS: readonly { key: Filter; label: string }[] = [
  { key: "all", label: "All" },
  ...SOURCES.map((s) => ({ key: s.key, label: s.label })),
];

/** The sources a filter lists. */
export function sourcesOf(filter: Filter): Source[] {
  return SOURCES.filter((s) => filter === "all" || s.key === filter);
}

/**
 * The query of source `s`: its unprocessed items, or with `withProcessed` all of them, newest
 * first. `extra` names more fields a captures row needs, such as its image field.
 */
export function sourceQuery(s: Source, withProcessed: boolean, extra: string[] = []): string {
  const conds = [withProcessed ? null : `!${PROCESSED}`, s.where].filter((c): c is string => c !== null);
  const where = conds.length ? ` where ${conds.join(" && ")}` : "";
  const fields = s.select(extra);
  const select = fields ? ` select ${fields.join(", ")}` : "";
  return `from ${s.type}${where} sort ${s.time} desc, id desc${select}`;
}

/** The time of row `r` of source `s` in ms since the epoch, or null when it has none. */
export function rowTime(s: Source, r: Row): number | null {
  const v: Json | undefined = r.fields[s.time];
  if (typeof v !== "string") return null;
  const t = Date.parse(v);
  return Number.isNaN(t) ? null : t;
}

/** A row of the inbox with the source it came from. */
export interface Item {
  source: Source;
  row: Row;
  time: number | null;
}

/**
 * Merge the pages of several sources into one list, newest first, at most `limit` items. An item
 * with no time goes last, as the query puts it; ties go by id, newest first, as within a source.
 */
export function merge(pages: { source: Source; page: Page }[], limit: number): Item[] {
  const items: Item[] = pages.flatMap(({ source, page }) =>
    page.rows.map((row) => ({ source, row, time: rowTime(source, row) })),
  );
  items.sort((a, b) => {
    if (a.time !== b.time) {
      if (a.time === null) return 1;
      if (b.time === null) return -1;
      return b.time - a.time;
    }
    return a.row.id < b.row.id ? 1 : a.row.id > b.row.id ? -1 : 0;
  });
  return items.slice(0, limit);
}

/** How many items the pages hold in all, which the badge and "Show more" use. */
export function total(pages: { page: Page }[]): number {
  return pages.reduce((n, { page }) => n + page.total, 0);
}
