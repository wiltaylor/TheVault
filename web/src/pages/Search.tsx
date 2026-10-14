// The Search page: full text, type, date and property filters, and sorting, over one paged
// query (PRD Appendix B). The whole search lives in the page's URL, so it survives a reload, sits
// in the pane's history and opens in a split or a new tab as it is.

import { A, useLocation, useNavigate } from "@solidjs/router";
import { For, Index, Match, Show, Switch, createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { type FieldDef, type Row, type TypeDef, api } from "../api";
import { copyText } from "../blocks/CodeBlock";
import { ChipPicker, typeSuggestions } from "../components/ChipPicker";
import { FieldInput, termOptions } from "../components/FieldInput";
import { SearchIcon, TypeIcon } from "../components/Icon";
import { createPagedRows } from "../components/pagedRows";
import { VirtualList } from "../components/VirtualList";
import { knowTitle, noteTitle } from "../editor/titles";
import {
  BUILTIN_DATES,
  type Filter,
  type Op,
  type SearchState,
  type SortKey,
  arity,
  completeOnly,
  decodeSearch,
  describe,
  effectiveSort,
  encodeSearch,
  fieldGroups,
  fieldOf,
  filterCount,
  filterable,
  isDateField,
  newFilter,
  opsFor,
  toQuery,
  withOp,
} from "../search/searchModel";
import { taxonomyDef, typeDef, types } from "../session";

const PAGE_SIZE = 100;
const ROW_HEIGHT = 48;
/** How long typing waits before the search runs and the address changes. */
const TYPING_MS = 250;

const SORTS: [SortKey, string][] = [
  ["relevance", "Relevance"],
  ["updated", "Updated"],
  ["created", "Created"],
  ["title", "Title"],
];

const stripQ = (s: string) => (s.startsWith("?") ? s.slice(1) : s);

function formatDate(v: unknown): string {
  if (typeof v !== "string") return "";
  const d = new Date(v);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

export function Search() {
  const location = useLocation();
  const navigate = useNavigate();

  // What the address holds: the search that runs. The draft is what the controls show; typing
  // changes it at once and the address a moment later.
  const committed = createMemo(() => decodeSearch(location.search));
  const [draft, setDraft] = createSignal<SearchState>(committed());
  const [filtersOpen, setFiltersOpen] = createSignal(false);
  let written = stripQ(location.search);
  /** The control the last write came from while typing, so one burst is one history entry. */
  let typing: string | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));

  // Back, forward or a link moved the address: the controls follow it.
  createEffect(
    on(
      () => stripQ(location.search),
      (qs) => {
        if (qs === written) return;
        written = qs;
        typing = null;
        clearTimeout(timer);
        setDraft(decodeSearch(qs));
      },
      { defer: true },
    ),
  );

  function write(s: SearchState, key: string | null) {
    const qs = encodeSearch(completeOnly(s, types()));
    const replace = key !== null && key === typing;
    typing = key;
    if (qs === stripQ(location.search)) return;
    written = qs;
    navigate(qs ? `/search?${qs}` : "/search", { replace, scroll: false });
  }

  /** Change the search. `key` names the control being typed in, which waits for a pause. */
  function update(next: SearchState, key: string | null = null) {
    setDraft(next);
    clearTimeout(timer);
    if (key === null) write(next, null);
    else timer = setTimeout(() => write(draft(), key), TYPING_MS);
  }

  const setFilter = (i: number, f: Filter | null, key: string | null = null) => {
    const filters = draft().filters.slice();
    if (f === null) filters.splice(i, 1);
    else filters[i] = f;
    update({ ...draft(), filters }, key);
  };

  const queryText = createMemo(() => toQuery(committed(), types()));
  const sort = createMemo(() => effectiveSort(draft()));

  const { rows, total, error, loadUntil } = createPagedRows(
    async (cursor, size) => {
      const page = await api.query(`${queryText()} select updated`, cursor, size);
      for (const r of page.rows) knowTitle(r.id, r.title, r.type);
      return page;
    },
    queryText,
    PAGE_SIZE,
  );

  const [copied, setCopied] = createSignal<string | null>(null);
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(copiedTimer));
  async function copyQuery() {
    const ok = await copyText(queryText());
    setCopied(ok ? "Copied" : "Could not copy");
    clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => setCopied(null), 2000);
  }

  function addFilter(section: Filter["section"], value: string) {
    if (!value) return;
    const [owner, name] = value.split(".") as [string, string];
    const s = draft();
    const def = fieldOf(types(), owner || null, s.types, name);
    if (!def) return;
    // With types chosen, remember the first that has the field, for when they are cleared.
    const from = owner || s.types.find((t) => typeDef(t)?.fields.some((f) => f.name === name)) || null;
    update({ ...s, filters: [...s.filters, newFilter(section, def.name === "created" || def.name === "updated" ? null : from, def)] });
    setFiltersOpen(true);
  }

  const count = () => filterCount(draft());

  return (
    <section class="search-page" aria-label="Search">
      <h1>Search</h1>
      <div class="search-bar" role="search">
        <SearchIcon />
        <input
          type="search"
          placeholder="Search titles, fields and content"
          aria-label="Search notes"
          value={draft().text}
          onInput={(e) => update({ ...draft(), text: e.currentTarget.value }, "text")}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              clearTimeout(timer);
              write(draft(), null);
            }
          }}
        />
        <button
          type="button"
          class="search-filters-toggle"
          aria-expanded={filtersOpen()}
          aria-controls="search-filters"
          onClick={() => setFiltersOpen(!filtersOpen())}
        >
          {count() ? `Filters (${count()})` : "Filters"}
        </button>
      </div>

      <div id="search-filters" class="search-filters" classList={{ open: filtersOpen() }}>
        <div class="filter-row filter-types" role="group" aria-label="Type filter" data-filter="type">
          <span class="filter-field" id="search-types-label">
            Type
          </span>
          <ChipPicker
            label="Types"
            labelledBy="search-types-label"
            values={draft().types}
            chipText={(name) => typeDef(name)?.label ?? name}
            chipIcon={(name) => typeDef(name)?.icon ?? null}
            suggest={(text) => typeSuggestions(types(), text)}
            placeholder="Find a type"
            emptyText="Any"
            onChange={(next) => update({ ...draft(), types: next })}
          />
          <Show when={draft().types.length}>
            <button
              type="button"
              class="filter-remove"
              aria-label="Remove type filter"
              title="Remove filter"
              onClick={() => update({ ...draft(), types: [] })}
            >
              ×
            </button>
          </Show>
        </div>

        <Index each={draft().filters}>
          {(f, i) => <FilterRow filter={f()} chosen={draft().types} onChange={(next, key) => setFilter(i, next, key)} />}
        </Index>

        <div class="search-add">
          <AddSelect label="Add date filter" text="Date" onPick={(v) => addFilter("date", v)}>
            <For each={BUILTIN_DATES}>{(d) => <option value={`.${d.name}`}>{d.label}</option>}</For>
            <FieldOptions groups={fieldGroups(types(), draft().types, (f) => isDateField(f) && !f.computed)} />
          </AddSelect>
          <AddSelect label="Add property filter" text="Property" onPick={(v) => addFilter("prop", v)}>
            <FieldOptions groups={fieldGroups(types(), draft().types, filterable)} />
          </AddSelect>
          <Show when={count() > 0}>
            <button type="button" class="link search-clear" onClick={() => update({ ...draft(), types: [], filters: [] })}>
              Clear filters
            </button>
          </Show>
        </div>
      </div>

      <div class="search-toolbar">
        <span class="muted search-count" aria-live="polite">
          <Show when={total() !== null} fallback="Searching…">
            {total() === 1 ? "1 note" : `${total()} notes`}
          </Show>
        </span>
        <span class="search-sort">
          <select
            aria-label="Sort by"
            value={sort().key}
            onChange={(e) => update({ ...draft(), sort: e.currentTarget.value as SortKey, dir: null })}
          >
            <For each={SORTS}>
              {([key, label]) => (
                <option value={key} disabled={key === "relevance" && !draft().text.trim()} selected={sort().key === key}>
                  {label}
                </option>
              )}
            </For>
          </select>
          <Show when={sort().key !== "relevance"}>
            <button
              type="button"
              class="search-dir"
              aria-label={sort().dir === "asc" ? "Ascending, sort descending" : "Descending, sort ascending"}
              title={sort().dir === "asc" ? "Ascending" : "Descending"}
              onClick={() => update({ ...draft(), sort: sort().key, dir: sort().dir === "asc" ? "desc" : "asc" })}
            >
              {sort().dir === "asc" ? "↑" : "↓"}
            </button>
          </Show>
        </span>
        <button type="button" class="search-copy" onClick={() => void copyQuery()} title={queryText()}>
          {copied() ?? "Copy as query"}
        </button>
      </div>

      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <div class="search-results">
        <Show when={total() !== 0} fallback={<p class="muted search-empty">No notes match.</p>}>
          <VirtualList
            count={total() ?? 0}
            rowHeight={ROW_HEIGHT}
            data-testid="note-list"
            aria-label="Search results"
            onRange={(_, end) => {
              if (end > rows().length - PAGE_SIZE / 4) void loadUntil(end);
            }}
          >
            {(i) => (
              <Show when={rows()[i]} fallback={<span class="note-row placeholder">…</span>}>
                {(row) => <ResultRow row={row()} />}
              </Show>
            )}
          </VirtualList>
        </Show>
      </div>
    </section>
  );
}

function ResultRow(props: { row: Row }) {
  const t = () => typeDef(props.row.type);
  const updated = () => props.row.fields.updated;
  return (
    <A class="note-row search-row" href={`/notes/${props.row.id}`} data-testid="note-row">
      <span class="search-row-icon">
        <TypeIcon name={t()?.icon} size={16} />
      </span>
      <span class="note-title">{props.row.title || "Untitled"}</span>
      <span class="search-row-meta">
        <span class="note-type">{t()?.label ?? props.row.type}</span>
        <time class="note-date" datetime={typeof updated() === "string" ? (updated() as string) : undefined}>
          {formatDate(updated())}
        </time>
      </span>
    </A>
  );
}

/** A select that reads as a "+ text" button, and adds what is picked. */
function AddSelect(props: { label: string; text: string; onPick: (value: string) => void; children: unknown }) {
  return (
    <select
      class="search-add-select"
      aria-label={props.label}
      onChange={(e) => {
        const v = e.currentTarget.value;
        e.currentTarget.value = "";
        props.onPick(v);
      }}
    >
      <option value="" selected>
        + {props.text}
      </option>
      {props.children as never}
    </select>
  );
}

/** Options of `type.field` values: one flat list, or a group per type. */
function FieldOptions(props: { groups: { type: TypeDef | null; fields: FieldDef[] }[] }) {
  return (
    <For each={props.groups}>
      {(g) => (
        <Show
          when={g.type}
          fallback={<For each={g.fields}>{(f) => <option value={`.${f.name}`}>{f.label}</option>}</For>}
        >
          {(t) => (
            <optgroup label={t().label}>
              <For each={g.fields}>{(f) => <option value={`${t().name}.${f.name}`}>{f.label}</option>}</For>
            </optgroup>
          )}
        </Show>
      )}
    </For>
  );
}

/** One field condition: the field, an operator for its kind, and the value inputs it needs. */
function FilterRow(props: { filter: Filter; chosen: string[]; onChange: (f: Filter | null, key?: string | null) => void }) {
  const def = createMemo(() => fieldOf(types(), props.filter.owner, props.chosen, props.filter.field));
  const f = () => props.filter;
  const owner = () => (props.chosen.length ? null : f().owner);
  const label = () => def()?.label ?? f().field;
  const key = (part: number) => `${f().section}:${f().owner}:${f().field}:${part}`;
  const set = (i: number, v: string, typing = false) => {
    const values = f().values.slice();
    while (values.length < i) values.push("");
    values[i] = v;
    props.onChange({ ...f(), values }, typing ? key(i) : null);
  };
  const id = () => `filter-${f().section}-${f().owner ?? ""}-${f().field}`;

  return (
    <div
      class="filter-row"
      classList={{ stale: !def() }}
      role="group"
      aria-label={`${label()} filter`}
      data-filter={f().section}
      data-field={f().field}
      title={def() ? describe(def(), f(), (v) => valueText(def()!, v)) : undefined}
    >
      <span class="filter-field">
        <Show when={owner()}>
          {(o) => (
            <span class="filter-owner">
              <TypeIcon name={typeDef(o())?.icon} size={14} />
              {typeDef(o())?.label ?? o()}
              <span aria-hidden="true">›</span>
            </span>
          )}
        </Show>
        {label()}
      </span>
      <Show
        when={def()}
        fallback={<span class="muted filter-stale">Not a field of the chosen types</span>}
      >
        {(d) => (
          <>
            <select
              class="filter-op"
              aria-label="Operator"
              onChange={(e) => props.onChange(withOp(f(), e.currentTarget.value as Op))}
            >
              <For each={opsFor(d())}>
                {([op, text]) => (
                  <option value={op} selected={op === f().op}>
                    {text}
                  </option>
                )}
              </For>
            </select>
            <ValueInputs
              def={d()}
              filter={f()}
              set={set}
              setValues={(values) => props.onChange({ ...f(), values })}
              id={id()}
            />
          </>
        )}
      </Show>
      <button
        type="button"
        class="filter-remove"
        aria-label={`Remove ${label()} filter`}
        title="Remove filter"
        onClick={() => props.onChange(null)}
      >
        ×
      </button>
    </div>
  );
}

/** A value as the chip text shows it: a note's title rather than its id. */
function valueText(def: FieldDef, v: string): string {
  if (def.kind === "reference") return noteTitle(v) ?? v;
  return v;
}

function ValueInputs(props: {
  def: FieldDef;
  filter: Filter;
  set: (i: number, v: string, typing?: boolean) => void;
  setValues: (values: string[]) => void;
  id: string;
}) {
  const op = () => props.filter.op;
  const v = (i: number) => props.filter.values[i] ?? "";
  const kind = () => props.def.kind;
  const dateOp = () => ["on", "before", "after", "between"].includes(op());
  const single = (): FieldDef => ({ ...props.def, multiple: false, required: false });
  return (
    <Show when={arity(op()) > 0}>
      <Switch
        fallback={
          <input
            class="filter-value"
            type={kind() === "int" || kind() === "float" || (kind() === "list" && /int|float/.test(props.def.items ?? "")) ? "number" : "text"}
            step="any"
            aria-label="Value"
            value={v(0)}
            onInput={(e) => props.set(0, e.currentTarget.value, true)}
          />
        }
      >
        <Match when={op() === "last"}>
          <span class="filter-days">
            <input
              class="filter-value filter-number"
              type="number"
              min="1"
              aria-label="Days"
              value={v(0)}
              onInput={(e) => props.set(0, e.currentTarget.value, true)}
            />
            days
          </span>
        </Match>
        <Match when={dateOp()}>
          <input
            class="filter-value"
            type="date"
            aria-label={op() === "between" ? "From" : "Date"}
            value={v(0)}
            onChange={(e) => props.set(0, e.currentTarget.value)}
          />
          <Show when={op() === "between"}>
            <span class="muted">and</span>
            <input
              class="filter-value"
              type="date"
              aria-label="To"
              value={v(1)}
              onChange={(e) => props.set(1, e.currentTarget.value)}
            />
          </Show>
        </Match>
        <Match when={op() === "any_of"}>
          <div class="filter-choices" role="group" aria-label="Values">
            <For each={props.def.options ?? []}>
              {(o) => (
                <label class="choice">
                  <input
                    type="checkbox"
                    checked={props.filter.values.includes(o)}
                    onChange={(e) => {
                      const now = props.filter.values.filter((x) => x !== o && x !== "");
                      const want = new Set(e.currentTarget.checked ? [...now, o] : now);
                      props.setValues((props.def.options ?? []).filter((x) => want.has(x)));
                    }}
                  />
                  {o}
                </label>
              )}
            </For>
          </div>
        </Match>
        <Match when={kind() === "select"}>
          <select class="filter-value" aria-label="Value" onChange={(e) => props.set(0, e.currentTarget.value)}>
            <option value="" selected={v(0) === ""}>
              —
            </option>
            <For each={props.def.options ?? []}>
              {(o) => (
                <option value={o} selected={v(0) === o}>
                  {o}
                </option>
              )}
            </For>
          </select>
        </Match>
        <Match when={kind() === "taxonomy"}>
          <select class="filter-value" aria-label="Value" onChange={(e) => props.set(0, e.currentTarget.value)}>
            <option value="" selected={v(0) === ""}>
              —
            </option>
            <For each={termOptions(taxonomyDef(props.def.taxonomy ?? "")?.terms ?? [])}>
              {(o) => (
                <option value={o.value} selected={v(0) === o.value}>
                  {"  ".repeat(o.depth) + o.text}
                </option>
              )}
            </For>
          </select>
        </Match>
        <Match when={kind() === "reference"}>
          <div class="filter-note">
            <FieldInput
              field={single()}
              value={v(0) || undefined}
              onChange={(x) => props.set(0, typeof x === "string" ? x : "")}
              label="Note"
              id={`${props.id}-note`}
            />
          </div>
        </Match>
      </Switch>
    </Show>
  );
}
