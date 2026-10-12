import { A } from "@solidjs/router";
import { Show } from "solid-js";
import type { Page } from "../api";
import { createPagedRows } from "./pagedRows";
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
  const { rows, total, error, loadUntil } = createPagedRows(
    (cursor, size) => props.fetchPage(cursor, size),
    () => props.key,
    PAGE_SIZE,
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
