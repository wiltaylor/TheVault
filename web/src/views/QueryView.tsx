import { A } from "@solidjs/router";
import {
  For,
  Match,
  Show,
  Switch,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  on,
  onCleanup,
  onMount,
} from "solid-js";
import { type Block, type NoteDoc, type Row, type Template, api } from "../api";
import { BlockView, Blocks, formatValue } from "../components/Render";
import { VirtualList } from "../components/VirtualList";
import { type PagedRows, createPagedRows } from "../components/pagedRows";
import { MapView } from "../map/MapView";
import { compoundBlock, slotField } from "../notes";
import { typeDef } from "../session";
import { type ViewDef, readQueryView, viewTemplate } from "./model";
import { ThisNote, useThisNote } from "./thisNote";

/** Rows per request: the API's largest page, so long results take the fewest round trips. */
const PAGE_SIZE = 1000;
const ROW_HEIGHT = 40;
const CARD_HEIGHT = 176;
const CARD_MIN_WIDTH = 240;
const DETAILS_PER_PAGE = 20;

/** Ask for more rows once the viewport is this close to the end of what is loaded. */
const LOOKAHEAD = PAGE_SIZE / 4;

function count(n: number): string {
  return n === 1 ? "1 note" : `${n} notes`;
}

function NoteLink(props: { row: Row }) {
  return (
    <A class="query-title" href={`/notes/${props.row.id}`} data-testid="query-title">
      {props.row.title || "Untitled"}
    </A>
  );
}

/**
 * The slots and blocks of a template for one result note, below its title. Values come from
 * `doc` when the full note is loaded, else from the row. Blocks see the result note as `this`.
 */
function TemplateBody(props: { row: Row; template: Template | undefined; doc?: NoteDoc; tabTitles?: boolean }) {
  const type = () => typeDef(props.row.type);
  return (
    <For each={props.template?.tabs ?? []}>
      {(tab) => (
        <section class="template-tab">
          <Show when={props.tabTitles && (props.template?.tabs.length ?? 0) > 1}>
            <h4>{tab.title}</h4>
          </Show>
          <For each={tab.items}>
            {(item) => {
              if (!("slot" in item)) {
                return (
                  <ThisNote.Provider value={props.row.id}>
                    <div class="template-block">
                      <BlockView block={item.block} />
                    </div>
                  </ThisNote.Provider>
                );
              }
              if (item.slot === "title") return null;
              const t = type();
              const f = t ? slotField(t, item.slot) : undefined;
              if (!f) return null;
              const label = item.label ?? f.label;
              return (
                <div class="query-slot" data-field={f.name}>
                  <span class="query-slot-label">{label}</span>
                  <span class="query-slot-value">
                    <Show
                      when={f.compound}
                      fallback={formatValue(props.doc ? props.doc.fields[f.name] : props.row.fields[f.name])}
                    >
                      <Show when={props.doc}>{(doc) => <Blocks blocks={compoundBlock(doc(), f)?.blocks ?? []} />}</Show>
                    </Show>
                  </span>
                </div>
              );
            }}
          </For>
        </section>
      )}
    </For>
  );
}

/** Load rows up to `end` once the viewport gets near the end of what is loaded. */
function follow(data: PagedRows, end: number) {
  if (end > data.rows().length - LOOKAHEAD) void data.loadUntil(end);
}

function Placeholder() {
  return <span class="query-placeholder" data-testid="query-placeholder">…</span>;
}

function TableView(props: { view: ViewDef; data: PagedRows; total: number }) {
  const columns = () => props.view.columns;
  const label = (c: string) => {
    const t = typeDef(props.data.rows()[0]?.type ?? "");
    const f = t ? slotField(t, c) : undefined;
    return f?.label ?? c.charAt(0).toUpperCase() + c.slice(1);
  };
  const grid = () => ({ "grid-template-columns": `repeat(${columns().length}, minmax(8rem, 1fr))` });
  return (
    <div class="query-table">
      <div class="query-table-head" style={grid()}>
        <For each={columns()}>{(c) => <span data-testid="query-columns">{label(c)}</span>}</For>
      </div>
      <VirtualList
        count={props.total}
        rowHeight={ROW_HEIGHT}
        aria-label="Results"
        onRange={(_, end) => follow(props.data, end)}
      >
        {(i) => (
          <Show when={props.data.rows()[i]} fallback={<Placeholder />}>
            {(row) => (
              <div class="query-table-row" style={grid()} data-testid="query-row">
                <For each={columns()}>
                  {(c) => (
                    <span class="query-cell">
                      {c === "title" ? <NoteLink row={row()} /> : formatValue(row().fields[c])}
                    </span>
                  )}
                </For>
              </div>
            )}
          </Show>
        )}
      </VirtualList>
    </div>
  );
}

function ListView(props: { data: PagedRows; total: number }) {
  return (
    <VirtualList
      count={props.total}
      rowHeight={ROW_HEIGHT}
      aria-label="Results"
      onRange={(_, end) => follow(props.data, end)}
    >
      {(i) => (
        <Show when={props.data.rows()[i]} fallback={<Placeholder />}>
          {(row) => (
            <div class="query-list-row" data-testid="query-row">
              <NoteLink row={row()} />
            </div>
          )}
        </Show>
      )}
    </VirtualList>
  );
}

/** A grid of cards, virtualised a line of cards at a time. */
function CardsView(props: { view: ViewDef; data: PagedRows; total: number }) {
  let box!: HTMLDivElement;
  const [width, setWidth] = createSignal(960);
  onMount(() => {
    const measure = () => setWidth(box.clientWidth || 960);
    measure();
    if (typeof ResizeObserver !== "undefined") {
      const observer = new ResizeObserver(measure);
      observer.observe(box);
      onCleanup(() => observer.disconnect());
    }
  });
  const cols = () => Math.max(1, Math.floor(width() / CARD_MIN_WIDTH));
  const line = (n: number) => {
    const out: number[] = [];
    for (let i = n * cols(); i < Math.min(props.total, (n + 1) * cols()); i++) out.push(i);
    return out;
  };
  return (
    <div ref={box} class="query-cards">
      <VirtualList
        count={Math.ceil(props.total / cols())}
        rowHeight={CARD_HEIGHT}
        overscan={3}
        aria-label="Results"
        onRange={(_, end) => follow(props.data, end * cols())}
      >
        {(n) => (
          <div class="query-card-line" style={{ "grid-template-columns": `repeat(${cols()}, 1fr)` }}>
            <For each={line(n)}>
              {(i) => (
                <Show when={props.data.rows()[i]} fallback={<Placeholder />}>
                  {(row) => (
                    <article class="query-card" data-testid="query-card">
                      <NoteLink row={row()} />
                      <TemplateBody
                        row={row()}
                        template={viewTemplate(typeDef(row().type), props.view.template, "card")}
                      />
                    </article>
                  )}
                </Show>
              )}
            </For>
          </div>
        )}
      </VirtualList>
    </div>
  );
}

function Detail(props: { row: Row; view: ViewDef }) {
  const [note] = createResource(
    () => props.row.id,
    (id) => api.note(id),
  );
  return (
    <article class="query-detail" data-testid="query-detail">
      <h3>
        <NoteLink row={props.row} />
      </h3>
      <TemplateBody
        row={props.row}
        template={viewTemplate(typeDef(props.row.type), props.view.template, "detail")}
        doc={note.error ? undefined : note()?.note}
        tabTitles
      />
    </article>
  );
}

/** Each note expanded with its detail template, a page at a time. */
function DetailsView(props: { view: ViewDef; data: PagedRows; total: number }) {
  const [page, setPage] = createSignal(0);
  const pages = () => Math.max(1, Math.ceil(props.total / DETAILS_PER_PAGE));
  createEffect(on(pages, (n) => page() >= n && setPage(n - 1)));
  createEffect(
    on([page, () => props.total], ([p, total]) => void props.data.loadUntil(Math.min(total, (p + 1) * DETAILS_PER_PAGE) - 1)),
  );
  const shown = () => props.data.rows().slice(page() * DETAILS_PER_PAGE, (page() + 1) * DETAILS_PER_PAGE);
  return (
    <div class="query-details">
      <Show when={pages() > 1}>
        <nav class="pager" aria-label="Pages">
          <button type="button" onClick={() => setPage(0)} disabled={page() === 0}>
            First page
          </button>
          <button type="button" onClick={() => setPage(page() - 1)} disabled={page() === 0}>
            Previous page
          </button>
          <span class="muted">
            Page {page() + 1} of {pages()}
          </span>
          <button type="button" onClick={() => setPage(page() + 1)} disabled={page() >= pages() - 1}>
            Next page
          </button>
          <button type="button" onClick={() => setPage(pages() - 1)} disabled={page() >= pages() - 1}>
            Last page
          </button>
        </nav>
      </Show>
      <Show when={shown().length > 0} fallback={<p class="muted">Loading…</p>}>
        <For each={shown()}>{(row) => <Detail row={row} view={props.view} />}</For>
      </Show>
    </div>
  );
}

/**
 * A query view block: one query, shown through its saved views (D8, D33). `this` is the note
 * that holds the block. Every view keeps only what is on screen in the DOM: table, list and cards
 * virtualise, details and map page.
 */
export function QueryView(props: { block: Block }) {
  const thisId = useThisNote();
  const def = createMemo(() => readQueryView(props.block));
  const [selected, setSelected] = createSignal<string | null>(null);
  const current = () => def().views.find((v) => v.name === selected()) ?? def().first;
  const data = createPagedRows(
    (cursor, size) => api.query(def().query, cursor, size, thisId),
    () => `${def().query}\u0000${thisId ?? ""}`,
    PAGE_SIZE,
  );

  return (
    <section class="query-view" data-testid="query-view">
      <Show when={def().title}>
        <h3 class="query-view-title">{def().title}</h3>
      </Show>
      <Show when={def().views.length > 1}>
        <div class="tabs" role="tablist" aria-label="Views">
          <For each={def().views}>
            {(v) => (
              <button
                type="button"
                role="tab"
                aria-selected={current()?.name === v.name}
                onClick={() => setSelected(v.name)}
              >
                {v.title}
              </button>
            )}
          </For>
        </div>
      </Show>
      <Show when={data.error()}>
        <p class="error" role="alert">
          {data.error()}
        </p>
      </Show>
      <Show when={data.total()} fallback={
        <Show when={data.total() === 0} fallback={<Show when={!data.error()}><p class="muted">Loading…</p></Show>}>
          <p class="muted">No notes match.</p>
        </Show>
      }>
        {(total) => (
          <>
            <p class="muted list-count" data-testid="query-count">
              {count(total())}
            </p>
            <Show when={current()}>
              {(view) => (
                <div class="query-view-body" data-view-kind={view().kind}>
                  <Switch fallback={<p class="muted">The {view().kind} view is not available yet.</p>}>
                    <Match when={view().kind === "table"}>
                      <TableView view={view()} data={data} total={total()} />
                    </Match>
                    <Match when={view().kind === "list"}>
                      <ListView data={data} total={total()} />
                    </Match>
                    <Match when={view().kind === "cards"}>
                      <CardsView view={view()} data={data} total={total()} />
                    </Match>
                    <Match when={view().kind === "details"}>
                      <DetailsView view={view()} data={data} total={total()} />
                    </Match>
                    <Match when={view().kind === "map"}>
                      <MapView
                        location={view().location}
                        data={data}
                        total={total()}
                        card={(row) => (
                          <>
                            <NoteLink row={row} />
                            <TemplateBody row={row} template={viewTemplate(typeDef(row.type), view().template, "card")} />
                          </>
                        )}
                      />
                    </Match>
                  </Switch>
                </div>
              )}
            </Show>
          </>
        )}
      </Show>
    </section>
  );
}
