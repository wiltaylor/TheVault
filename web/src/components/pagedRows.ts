import { type Accessor, batch, createEffect, createSignal, on } from "solid-js";
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
 * them (D33). Starts again from the first page whenever `key` changes. Whenever `refresh`
 * changes, loads the rows it already holds again and swaps them in at once, so a result that
 * changed under it updates in place rather than emptying first.
 */
export function createPagedRows(
  fetchPage: (cursor: string | null, pageSize: number) => Promise<Page>,
  key: Accessor<string>,
  pageSize: number,
  refresh?: Accessor<unknown>,
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

  async function reload() {
    const gen = ++generation;
    loading = true;
    let fresh: Row[] = [];
    let cursor: string | null = null;
    let count: number | null = null;
    try {
      do {
        const page = await fetchPage(cursor, pageSize);
        if (gen !== generation) return;
        cursor = page.next;
        fresh = fresh.concat(page.rows);
        count = page.total;
      } while (cursor && fresh.length <= wanted);
      next = cursor;
      batch(() => {
        setRows(fresh);
        setTotal(count);
        setError(null);
      });
    } catch (e) {
      if (gen === generation) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (gen === generation) loading = false;
    }
  }

  if (refresh) createEffect(on(refresh, () => void reload(), { defer: true }));

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
