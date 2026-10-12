import { type Accessor, createEffect, createSignal, on } from "solid-js";
import type { Page, Row } from "../api";

export interface PagedRows {
  /** The rows loaded so far, from the first. */
  rows: Accessor<Row[]>;
  /** Rows in the whole result, or null before the first page arrives. */
  total: Accessor<number | null>;
  error: Accessor<string | null>;
  /** Load pages in order until row `index` is loaded or the result ends. */
  loadUntil: (index: number) => Promise<void>;
}

/**
 * The rows of a paged API result, loaded a page at a time in cursor order as callers ask for
 * them (D33). Starts again from the first page whenever `key` changes.
 */
export function createPagedRows(
  fetchPage: (cursor: string | null, pageSize: number) => Promise<Page>,
  key: Accessor<string>,
  pageSize: number,
): PagedRows {
  const [rows, setRows] = createSignal<Row[]>([]);
  const [total, setTotal] = createSignal<number | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  let next: string | null = null;
  let loading = false;
  let wanted = 0;
  let generation = 0;

  async function loadUntil(index: number) {
    wanted = Math.max(wanted, index);
    if (loading) return;
    const gen = generation;
    loading = true;
    try {
      while (gen === generation && rows().length <= wanted) {
        const first = total() === null;
        if (!first && !next) break;
        const page = await fetchPage(first ? null : next, pageSize);
        if (gen !== generation) return;
        next = page.next;
        setRows((r) => r.concat(page.rows));
        setTotal(page.total);
      }
    } catch (e) {
      if (gen === generation) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (gen === generation) loading = false;
    }
  }

  createEffect(
    on(key, () => {
      generation++;
      loading = false;
      next = null;
      wanted = 0;
      setRows([]);
      setTotal(null);
      setError(null);
      void loadUntil(0);
    }),
  );

  return { rows, total, error, loadUntil };
}
