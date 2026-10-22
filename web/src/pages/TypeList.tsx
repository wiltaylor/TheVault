import { A, useParams } from "@solidjs/router";
import { For, Match, Show, Switch, createEffect, createMemo, createResource, createSignal, on, onCleanup } from "solid-js";
import { type Row, type TypeDef, api, isArchived } from "../api";
import { changes } from "../changes";
import { ArchiveIcon, SearchIcon, TypeIcon } from "../components/Icon";
import { FieldValue } from "../components/Render";
import { knowArchived, knowTitle } from "../editor/titles";
import { typeDef } from "../session";
import {
  type ListSort,
  type TypePageView,
  type TypePrefs,
  cardTemplate,
  flipSort,
  listColumns,
  listPageSettings,
  loadTypePrefs,
  pageStatus,
  sameSort,
  saveTypePrefs,
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
 * Every note of one type, a page at a time, as cards, a sortable list or each note expanded, with
 * a search box over them. No page loads more than the type's page size, so a big type never loads
 * whole (D33; lists L2).
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

/** Where paging is for one query: the cursor of each page seen so far, and the page shown. */
interface Paging {
  query: string;
  cursors: (string | null)[];
  index: number;
}

export function TypeNotes(props: { type: TypeDef }) {
  const name = () => props.type.name;
  const settings = () => listPageSettings(props.type);
  const [prefs, setPrefs] = createSignal<TypePrefs>(loadTypePrefs(props.type));
  const view = (): TypePageView => prefs().view ?? settings().view;
  const sort = (): ListSort => prefs().sort ?? settings().sort;
  const pageSize = () => settings().pageSize;
  const [draft, setDraft] = createSignal("");
  const [text, setText] = createSignal("");
  const [showArchived, setShowArchived] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));

  // The same page component can move to another type: start that type afresh.
  createEffect(
    on(
      name,
      () => {
        clearTimeout(timer);
        setPrefs(loadTypePrefs(props.type));
        setDraft("");
        setText("");
        setShowArchived(false);
      },
      { defer: true },
    ),
  );

  function remember(next: TypePrefs) {
    const out: TypePrefs = {};
    // A choice equal to the type's own setting is no choice: the type's later changes apply.
    if (next.view && next.view !== settings().view) out.view = next.view;
    if (next.sort && !sameSort(next.sort, settings().sort)) out.sort = next.sort;
    setPrefs(out);
    saveTypePrefs(name(), out);
  }

  const choose = (v: TypePageView) => remember({ ...prefs(), view: v });
  const sortBy = (key: string) => remember({ ...prefs(), sort: flipSort(sort(), key) });
  const reset = () => remember({});
  const customised = () => prefs().view !== undefined || prefs().sort !== undefined;

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

  const query = () => typeQuery(name(), text(), showArchived(), sort());

  // A new query (type, search text, archived toggle or sort) starts again at page 1.
  const [paging, setPaging] = createSignal<Paging>({ query: "", cursors: [null], index: 0 });
  const at = (): Paging => {
    const p = paging();
    return p.query === query() ? p : { query: query(), cursors: [null], index: 0 };
  };

  // What the page shown depends on. Learning the next page's cursor changes `paging` and not
  // this, so it fetches nothing again.
  const source = createMemo(
    () => {
      const p = at();
      return [p.query, p.cursors[p.index] ?? null, p.index, pageSize(), changes()] as const;
    },
    undefined,
    { equals: (a, b) => a.every((x, i) => x === b[i]) },
  );
  const [page] = createResource(
    source,
    async ([q, cursor, index, size]) => {
      const p = await api.query(q, cursor, size);
      // Rows hold every stored field, `archived` among them.
      for (const r of p.rows) {
        knowTitle(r.id, r.title, r.type);
        knowArchived(r.id, isArchived(r.fields));
      }
      // The next page's cursor, so that Next can go there and Previous can come back.
      setPaging((prev) => {
        const base = prev.query === q ? prev : { query: q, cursors: [null], index };
        const cursors = base.cursors.slice(0, index + 1);
        if (p.next) cursors[index + 1] = p.next;
        return { ...base, cursors };
      });
      return { ...p, index };
    },
  );
  const rows = () => page.latest?.rows ?? [];
  const searching = () => text().trim() !== "";
  const plural = () => props.type.plural;
  const hasNext = () => at().cursors[at().index + 1] != null;
  const go = (index: number) => setPaging({ ...at(), index });

  const status = () => {
    const p = page.latest;
    if (!p) return searching() ? "Searching…" : "Loading…";
    return pageStatus(p.index, pageSize(), p.rows.length, p.total, searching());
  };

  const viewButton = (v: TypePageView, icon: string, label: string) => (
    <button type="button" aria-pressed={view() === v} onClick={() => choose(v)}>
      <TypeIcon name={icon} size={16} />
      <span>{label}</span>
    </button>
  );

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
          {viewButton("cards", "layout-grid", "Cards")}
          {viewButton("list", "list", "List")}
          {viewButton("details", "layout-list", "Details")}
        </div>
        <Show when={customised()}>
          <button type="button" class="type-list-reset" onClick={reset} title="Show the view and sort the type sets">
            Reset to type default
          </button>
        </Show>
      </div>

      <div class="type-list-statusbar">
        <p class="muted list-count type-list-status" data-testid="type-list-status" aria-live="polite">
          {status()}
        </p>
        <Show when={at().index > 0 || hasNext()}>
          <nav class="pager type-list-pager" aria-label="Pages">
            <button type="button" onClick={() => go(0)} disabled={at().index === 0}>
              First
            </button>
            <button type="button" onClick={() => go(at().index - 1)} disabled={at().index === 0}>
              Previous
            </button>
            <button type="button" onClick={() => go(at().index + 1)} disabled={!hasNext()}>
              Next
            </button>
          </nav>
        </Show>
      </div>
      <Show when={page.error}>
        <p class="error" role="alert">
          {String(page.error instanceof Error ? page.error.message : page.error)}
        </p>
      </Show>

      <Show when={page.latest}>
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
              <ListTable type={props.type} rows={rows()} sort={sort()} onSort={sortBy} />
            </Match>
            <Match when={view() === "details"}>
              <DetailsList type={props.type} rows={rows()} />
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

/** One note expanded with its type's default template, its title linking to the note (lists L4). */
function Detail(props: { type: TypeDef; row: Row }) {
  // The row holds the note's stored fields; its content fields come with the note.
  const [note] = createResource(
    () => [props.row.id, changes()] as const,
    ([id]) => api.note(id),
  );
  const template = () => props.type.templates.find((t) => t.name === "default");
  return (
    <article class="query-detail type-detail" classList={{ archived: isArchived(props.row.fields) }} data-testid="type-detail">
      <h3>
        <A class="query-title" href={`/notes/${props.row.id}`} data-testid="query-title">
          {props.row.title || "Untitled"}
        </A>
        <Show when={isArchived(props.row.fields)}>
          {" "}
          <span class="archived-chip">Archived</span>
        </Show>
      </h3>
      <TemplateBody row={props.row} template={template()} doc={note.error ? undefined : note.latest?.note} tabTitles />
    </article>
  );
}

function DetailsList(props: { type: TypeDef; rows: Row[] }) {
  return (
    <div class="query-details type-details" data-testid="type-details">
      <For each={props.rows}>{(row) => <Detail type={props.type} row={row} />}</For>
    </div>
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

/**
 * One row per note, with the card's fields and Updated as columns. A header click sorts the
 * whole type by that column on the server, so every page follows it (lists L3).
 */
function ListTable(props: { type: TypeDef; rows: Row[]; sort: ListSort; onSort: (key: string) => void }) {
  const columns = () => listColumns(props.type);
  const heads = () => [
    { key: "title", label: "Title" },
    ...columns().map((f) => ({ key: f.name, label: f.label })),
    { key: "updated", label: "Updated" },
  ];
  const ariaSort = (key: string) => {
    if (props.sort.key !== key) return "none";
    return props.sort.dir === "asc" ? "ascending" : "descending";
  };
  return (
    <div class="type-table-wrap">
      <table class="type-table" data-testid="note-table">
        <thead>
          <tr>
            <For each={heads()}>
              {(h) => (
                <th scope="col" aria-sort={ariaSort(h.key)} classList={{ "type-table-updated": h.key === "updated" }}>
                  <button type="button" class="type-table-sort" onClick={() => props.onSort(h.key)}>
                    {h.label}
                    <span class="type-table-arrow" aria-hidden="true">
                      {props.sort.key === h.key ? (props.sort.dir === "asc" ? "↑" : "↓") : ""}
                    </span>
                  </button>
                </th>
              )}
            </For>
          </tr>
        </thead>
        <tbody>
          <For each={props.rows}>
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
                <For each={columns()}>
                  {(f) => (
                    <td>
                      <FieldValue field={f} value={row.fields[f.name]} fields={row.fields} size="small" />
                    </td>
                  )}
                </For>
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
