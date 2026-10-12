import { A } from "@solidjs/router";
import { Show, createEffect, createSignal, on } from "solid-js";
import type { Page, Row } from "../api";
import { VirtualList } from "./VirtualList";

const ROW_HEIGHT = 44;
const PAGE_SIZE = 500;

export interface PagedListProps {
  /** Fetches the page after `cursor`, or the first page when it is null. */
  fetchPage: (cursor: string | null, pageSize: number) => Promise<Page>;
  /** Changes whenever the list should start again from its first page. */
  key: string;
  empty: string;
  label: string;
  showType?: boolean;
}

/**
 * Notes from a paged API result in a virtualised list. Pages load in order as the list scrolls
 * towards rows it does not have yet (D33).
 */
export function PagedList(props: PagedListProps) {
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
        const page = await props.fetchPage(first ? null : next, PAGE_SIZE);
        if (gen !== generation) return;
        next = page.next;
        setTotal(page.total);
        setRows((r) => r.concat(page.rows));
      }
    } catch (e) {
      if (gen === generation) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (gen === generation) loading = false;
    }
  }

  createEffect(
    on(
      () => props.key,
      () => {
        generation++;
        loading = false;
        next = null;
        wanted = 0;
        setRows([]);
        setTotal(null);
        setError(null);
        void loadUntil(0);
      },
    ),
  );

  return (
    <div class="paged-list">
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={total() !== null} fallback={<p class="muted">Loading…</p>}>
        <Show when={total() !== 0} fallback={<p class="muted">{props.empty}</p>}>
          <p class="muted list-count">{total()} notes</p>
          <VirtualList
            count={total() ?? 0}
            rowHeight={ROW_HEIGHT}
            data-testid="note-list"
            aria-label={props.label}
            onRange={(_, end) => {
              if (end > rows().length - PAGE_SIZE / 4) void loadUntil(end);
            }}
          >
            {(i) => (
              <Show when={rows()[i]} fallback={<span class="note-row placeholder">…</span>}>
                {(row) => (
                  <A class="note-row" href={`/notes/${row().id}`} data-testid="note-row">
                    <span class="note-title">{row().title || "Untitled"}</span>
                    <Show when={props.showType}>
                      <span class="note-type">{row().type}</span>
                    </Show>
                  </A>
                )}
              </Show>
            )}
          </VirtualList>
        </Show>
      </Show>
    </div>
  );
}
