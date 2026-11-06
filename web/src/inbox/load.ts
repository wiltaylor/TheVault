// Loading the unified inbox: one query per source, merged (see sources.ts).

import { type Page, api } from "../api";
import { type Filter, type Item, type Source, merge, sourceQuery, sourcesOf, total } from "./sources";

export interface InboxPage {
  items: Item[];
  /** Items in the whole result, over every source the filter lists. */
  total: number;
}

/**
 * The first `limit` items of the sources `filter` lists, newest first. A source whose query fails,
 * such as one whose type the vault lacks, adds nothing; when every source fails, so does this.
 * `extra` names more fields a captures row needs.
 */
export async function loadInbox(filter: Filter, withProcessed: boolean, limit: number, extra: string[] = []): Promise<InboxPage> {
  const sources = sourcesOf(filter);
  const results = await Promise.allSettled(sources.map((s) => api.query(sourceQuery(s, withProcessed, extra), null, limit)));
  const pages: { source: Source; page: Page }[] = [];
  let failure: unknown = null;
  sources.forEach((source, i) => {
    const r = results[i];
    if (r?.status === "fulfilled") pages.push({ source, page: r.value });
    else if (r) failure ??= r.reason;
  });
  if (pages.length === 0 && failure !== null) throw failure;
  return { items: merge(pages, limit), total: total(pages) };
}

/** How many items wait in the All view: what the sidebar badge shows. */
export async function countInbox(): Promise<number> {
  return (await loadInbox("all", false, 1)).total;
}
