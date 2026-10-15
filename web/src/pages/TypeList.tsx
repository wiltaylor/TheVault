import { A, useParams } from "@solidjs/router";
import { For, Match, Show, Switch, createEffect, createResource, createSignal, on, onCleanup } from "solid-js";
import { type Row, type TypeDef, api, isArchived } from "../api";
import { changes } from "../changes";
import { ArchiveIcon, SearchIcon, TypeIcon } from "../components/Icon";
import { formatValue } from "../components/Render";
import { knowArchived, knowTitle } from "../editor/titles";
import { typeDef } from "../session";
import {
  type ListSort,
  MAX_LISTED,
  type TypeListView,
  cardTemplate,
  listColumns,
  loadTypeView,
  nextSort,
  noteCount,
  saveTypeView,
  sortRows,
  typeQuery,
} from "../typeList";
import { TemplateBody } from "../views/parts";

/** How long typing waits before the search runs. */
const TYPING_MS = 250;

function formatDate(v: unknown): string {
  if (typeof v !== "string") return "";
  const d = new Date(v);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

/**
 * Every note of one type, as cards or a sortable list, with a search box over them. A type with
 * more than MAX_LISTED notes lists none until a search narrows it, so a big type never loads
 * whole (D33).
 */
export function TypeList() {
  const params = useParams();
  const name = () => params.type ?? "";
  return (
    <section class="list-page type-list">
      <Show when={typeDef(name())} fallback={<p class="error">There is no type {name()}.</p>}>
        {(t) => <TypeNotes type={t()} />}
      </Show>
    </section>
  );
}

function TypeNotes(props: { type: TypeDef }) {
  const name = () => props.type.name;
  const [view, setView] = createSignal<TypeListView>(loadTypeView(name()));
  const [draft, setDraft] = createSignal("");
  const [text, setText] = createSignal("");
  const [showArchived, setShowArchived] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));

  // The same page component can move to another type: start that type afresh.
  createEffect(
    on(
      name,
      (n) => {
        clearTimeout(timer);
        setView(loadTypeView(n));
        setDraft("");
        setText("");
        setShowArchived(false);
      },
      { defer: true },
    ),
  );

  function choose(v: TypeListView) {
    setView(v);
    saveTypeView(name(), v);
  }

  function type(value: string) {
    setDraft(value);
    clearTimeout(timer);
    timer = setTimeout(() => setText(draft()), TYPING_MS);
  }

  function clear() {
    clearTimeout(timer);
    setDraft("");
    setText("");
  }

  // How many notes the type has: one row asked for, the total read off the page.
  const [count] = createResource(
    () => [name(), showArchived(), changes()] as const,
    async ([n, archived]) => (await api.query(`from ${n}${archived ? " include archived" : ""}`, null, 1)).total,
  );
  const big = () => (count.latest ?? 0) > MAX_LISTED;

  const [page] = createResource(
    () => {
      if (count.latest === undefined) return false;
      // A big type loads nothing until there is something to search for.
      if (big() && !text().trim()) return false;
      return [typeQuery(name(), text(), showArchived()), changes()] as const;
    },
    async ([query]) => {
      const p = await api.query(query, null, MAX_LISTED);
      // Rows hold every stored field, `archived` among them.
      for (const r of p.rows) {
        knowTitle(r.id, r.title, r.type);
        knowArchived(r.id, isArchived(r.fields));
      }
      return p;
    },
  );
  const rows = () => (big() && !text().trim() ? [] : (page.latest?.rows ?? []));
  const searching = () => text().trim() !== "";
  const plural = () => props.type.plural;

  const status = () => {
    const total = count.latest;
    if (total === undefined) return "Loading…";
    if (!searching()) return big() ? `${noteCount(total)}. Search to find one.` : noteCount(total);
    const p = page.latest;
    if (!p) return "Searching…";
    if (p.total > p.rows.length) return `Showing the first ${p.rows.length} of ${p.total.toLocaleString("en-US")} matches. Search for more words to narrow it.`;
    return p.total === 1 ? "1 match" : `${p.total.toLocaleString("en-US")} matches`;
  };

  return (
    <>
      <header class="note-header">
        <h1 class="type-list-title">
          <TypeIcon name={props.type.icon} size={24} />
          {plural()}
        </h1>
        <div class="actions">
          <A href={`/notes/new?type=${encodeURIComponent(name())}`} class="button primary">
            New {props.type.label.toLowerCase()}
          </A>
        </div>
      </header>

      <div class="type-list-toolbar">
        <div class="search-bar type-list-search" role="search">
          <SearchIcon />
          <input
            type="search"
            placeholder={`Search ${plural().toLowerCase()}`}
            aria-label={`Search ${plural().toLowerCase()}`}
            value={draft()}
            onInput={(e) => type(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape" && draft()) {
                e.preventDefault();
                clear();
              } else if (e.key === "Enter") {
                clearTimeout(timer);
                setText(draft());
              }
            }}
          />
        </div>
        <button
          type="button"
          class="type-list-archived"
          aria-pressed={showArchived()}
          title={showArchived() ? "Hide archived notes" : "Show archived notes"}
          onClick={() => setShowArchived(!showArchived())}
        >
          <ArchiveIcon />
          <span>Show archived</span>
        </button>
        <div class="segmented type-list-views" role="group" aria-label="View">
          <button type="button" aria-pressed={view() === "cards"} onClick={() => choose("cards")}>
            <TypeIcon name="layout-grid" size={16} />
            <span>Cards</span>
          </button>
          <button type="button" aria-pressed={view() === "list"} onClick={() => choose("list")}>
            <TypeIcon name="list" size={16} />
            <span>List</span>
          </button>
        </div>
      </div>

      <p class="muted list-count type-list-status" data-testid="type-list-status" aria-live="polite">
        {status()}
      </p>
      <Show when={page.error}>
        <p class="error" role="alert">
          {String(page.error instanceof Error ? page.error.message : page.error)}
        </p>
      </Show>

      <Show when={!(big() && !searching()) && page.latest}>
        <Show
          when={rows().length > 0}
          fallback={
            <p class="muted type-list-empty">
              {searching() ? "No notes match." : `No ${plural().toLowerCase()} yet.`}
            </p>
          }
        >
          <Switch>
            <Match when={view() === "cards"}>
              <CardsGrid type={props.type} rows={rows()} />
            </Match>
            <Match when={view() === "list"}>
              <ListTable type={props.type} rows={rows()} />
            </Match>
          </Switch>
        </Show>
      </Show>
    </>
  );
}

/** One card per note, drawn with the type's `card` template; the whole card opens the note. */
function CardsGrid(props: { type: TypeDef; rows: Row[] }) {
  const template = () => cardTemplate(props.type);
  return (
    <ul class="type-note-grid" aria-label={props.type.plural} data-testid="type-cards">
      <For each={props.rows}>
        {(row) => (
          <li class="query-card type-note-card" classList={{ archived: isArchived(row.fields) }} data-testid="note-card">
            <A class="query-title type-note-link" href={`/notes/${row.id}`} data-testid="query-title">
              {row.title || "Untitled"}
            </A>
            <Show when={isArchived(row.fields)}>
              <span class="archived-chip">Archived</span>
            </Show>
            <Show when={template()}>{(t) => <TemplateBody row={row} template={t()} />}</Show>
          </li>
        )}
      </For>
    </ul>
  );
}

/**
 * A click anywhere on a row clicks its title link, keeping the button and modifier keys, so the
 * workspace opens the note in this pane, or in a background tab for a middle or Ctrl+click.
 */
function forward(e: MouseEvent) {
  const row = e.currentTarget as HTMLElement;
  if (e.defaultPrevented || (e.target as Element).closest("a")) return;
  if (getSelection()?.toString()) return;
  row.querySelector("a")?.dispatchEvent(new MouseEvent(e.type, e));
}

/** One row per note, with the card's fields and Updated as columns, sorted by any of them. */
function ListTable(props: { type: TypeDef; rows: Row[] }) {
  const [sort, setSort] = createSignal<ListSort | null>(null);
  const columns = () => listColumns(props.type);
  const heads = () => [
    { key: "title", label: "Title" },
    ...columns().map((f) => ({ key: f.name, label: f.label })),
    { key: "updated", label: "Updated" },
  ];
  const sorted = () => sortRows(props.rows, sort());
  const ariaSort = (key: string) => {
    const s = sort();
    if (s?.key !== key) return "none";
    return s.dir === "asc" ? "ascending" : "descending";
  };
  return (
    <div class="type-table-wrap">
      <table class="type-table" data-testid="note-table">
        <thead>
          <tr>
            <For each={heads()}>
              {(h) => (
                <th scope="col" aria-sort={ariaSort(h.key)} classList={{ "type-table-updated": h.key === "updated" }}>
                  <button type="button" class="type-table-sort" onClick={() => setSort(nextSort(sort(), h.key))}>
                    {h.label}
                    <span class="type-table-arrow" aria-hidden="true">
                      {sort()?.key === h.key ? (sort()!.dir === "asc" ? "↑" : "↓") : ""}
                    </span>
                  </button>
                </th>
              )}
            </For>
          </tr>
        </thead>
        <tbody>
          <For each={sorted()}>
            {(row) => (
              <tr data-testid="note-row" onClick={forward} onAuxClick={forward}>
                <td class="type-table-title">
                  <A class="type-table-link" href={`/notes/${row.id}`}>
                    <TypeIcon name={props.type.icon} size={16} />
                    <span>{row.title || "Untitled"}</span>
                  </A>
                  <Show when={isArchived(row.fields)}>
                    <span class="archived-chip">Archived</span>
                  </Show>
                </td>
                <For each={columns()}>{(f) => <td>{formatValue(row.fields[f.name])}</td>}</For>
                <td class="type-table-updated">
                  <time datetime={typeof row.fields.updated === "string" ? row.fields.updated : undefined}>
                    {formatDate(row.fields.updated)}
                  </time>
                </td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  );
}
