// Editors for every field kind a note page edits in place (D3, D7). Content, ink and canvas fields
// have editors of their own; a computed field is never given one.

import { For, Match, Show, Switch, createResource, createSignal, onCleanup } from "solid-js";
import { type FieldDef, type Json, type Row, type Term, api } from "../api";
import { knowTitle, noteTitle } from "../editor/titles";
import { MapCanvas } from "../map/MapCanvas";
import { geoPoint, imagePoint, mapKind } from "../map/model";
import { taxonomyDef } from "../session";

export interface FieldInputProps {
  field: FieldDef;
  value: Json | undefined;
  onChange: (value: Json | undefined) => void;
  label: string;
  id: string;
  /** Shown in an empty text-like input. */
  placeholder?: string;
}

const SCALAR_INPUT: Record<string, string> = {
  int: "number",
  float: "number",
  date: "date",
  datetime: "datetime-local",
  url: "url",
};

const DATETIME = /^(\d{4}-\d\d-\d\dT\d\d:\d\d(?::\d\d)?)(?:\.\d+)?(Z|[+-]\d\d:\d\d)$/;

/** The local part of a stored datetime, as a datetime-local input shows it. */
export function datetimeInput(v: Json | undefined): string {
  return typeof v === "string" ? (DATETIME.exec(v)?.[1] ?? "") : "";
}

/**
 * A datetime-local input's text as a stored datetime: seconds always written, and the offset the
 * stored value had, else the browser's offset at that time.
 */
export function datetimeValue(raw: string, previous: Json | undefined): string {
  const local = raw.length === 16 ? `${raw}:00` : raw;
  const kept = typeof previous === "string" ? DATETIME.exec(previous)?.[2] : undefined;
  if (kept) return `${local}${kept}`;
  const minutes = -new Date(local).getTimezoneOffset();
  if (minutes === 0 || Number.isNaN(minutes)) return `${local}Z`;
  const abs = Math.abs(minutes);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${local}${minutes < 0 ? "-" : "+"}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}`;
}

/** An input's text as a scalar of `kind`. Empty text means no value. */
export function scalarValue(kind: string, raw: string, previous?: Json): Json | undefined {
  if (raw === "") return undefined;
  if (kind === "int" || kind === "float") {
    const n = Number(raw);
    if (!Number.isFinite(n)) return raw;
    return kind === "int" ? Math.trunc(n) : n;
  }
  if (kind === "datetime") return datetimeValue(raw, previous);
  return raw;
}

function scalarText(kind: string, v: Json | undefined): string {
  if (v === undefined || v === null) return "";
  if (kind === "datetime") return datetimeInput(v);
  return typeof v === "object" ? JSON.stringify(v) : String(v);
}

/** One scalar of `kind`: string, int, float, bool, date, datetime or url. */
export function ScalarInput(props: {
  kind: string;
  value: Json | undefined;
  onChange: (v: Json | undefined) => void;
  label: string;
  id?: string;
  required?: boolean;
  placeholder?: string;
}) {
  return (
    <Show
      when={props.kind === "bool"}
      fallback={
        <input
          id={props.id}
          type={SCALAR_INPUT[props.kind] ?? "text"}
          step={props.kind === "float" ? "any" : props.kind === "datetime" ? "1" : undefined}
          value={scalarText(props.kind, props.value)}
          required={props.required}
          placeholder={props.placeholder}
          aria-label={props.label}
          onInput={(e) => props.onChange(scalarValue(props.kind, e.currentTarget.value, props.value))}
        />
      }
    >
      <input
        id={props.id}
        type="checkbox"
        checked={props.value === true}
        aria-label={props.label}
        onChange={(e) => props.onChange(e.currentTarget.checked)}
      />
    </Show>
  );
}

const asList = (v: Json | undefined): Json[] => (Array.isArray(v) ? v : v === undefined || v === null ? [] : [v]);
const asStrings = (v: Json | undefined): string[] => asList(v).filter((x): x is string => typeof x === "string");

/** A set of choices from `options`: one in a select, or several as checkboxes. */
function Choice(props: {
  options: { value: string; text: string; depth?: number }[];
  multiple: boolean;
  required: boolean;
  value: Json | undefined;
  onChange: (v: Json | undefined) => void;
  label: string;
  id: string;
}) {
  return (
    <Show
      when={props.multiple}
      fallback={
        <select
          id={props.id}
          aria-label={props.label}
          onChange={(e) => props.onChange(e.currentTarget.value === "" ? undefined : e.currentTarget.value)}
        >
          <Show when={!props.required || props.value === undefined}>
            <option value="" selected={props.value === undefined}>
              —
            </option>
          </Show>
          <For each={props.options}>
            {(o) => (
              <option value={o.value} selected={props.value === o.value}>
                {"\u00a0\u00a0".repeat(o.depth ?? 0) + o.text}
              </option>
            )}
          </For>
        </select>
      }
    >
      <div id={props.id} role="group" aria-label={props.label} class="choice-group">
        <For each={props.options}>
          {(o) => (
            <label class="choice" style={{ "padding-left": `${(o.depth ?? 0) * 1.25}rem` }}>
              <input
                type="checkbox"
                aria-label={o.value}
                checked={asStrings(props.value).includes(o.value)}
                onChange={(e) => {
                  const now = asStrings(props.value).filter((x) => x !== o.value);
                  if (e.currentTarget.checked) {
                    // Keep the options' order, which is display order.
                    const want = new Set([...now, o.value]);
                    props.onChange(props.options.map((x) => x.value).filter((x) => want.has(x)));
                  } else {
                    props.onChange(now.length ? now : undefined);
                  }
                }}
              />{" "}
              {o.text}
            </label>
          )}
        </For>
      </div>
    </Show>
  );
}

/** Every term of a taxonomy, parents first, with its depth. */
export function termOptions(terms: Term[], depth = 0): { value: string; text: string; depth: number }[] {
  return terms.flatMap((t) => [{ value: t.path, text: t.name, depth }, ...termOptions(t.children, depth + 1)]);
}

const SEARCH_RESULTS = 20;
const DEBOUNCE_MS = 150;

/** A query-language string literal. */
function queryString(s: string): string {
  return JSON.stringify(s);
}

/** Notes of the field's target type, found by title text, to link to. */
function ReferencePicker(props: FieldInputProps) {
  const [q, setQ] = createSignal("");
  const [results, setResults] = createSignal<Row[]>([]);
  const [open, setOpen] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let seq = 0;
  onCleanup(() => clearTimeout(timer));

  const ids = () => asStrings(props.value);
  const target = () => props.field.target ?? "*";

  function find(text: string) {
    setQ(text);
    setOpen(true);
    clearTimeout(timer);
    const mine = ++seq;
    timer = setTimeout(async () => {
      try {
        const t = text.trim();
        let page;
        if (target() === "*") {
          page = t ? await api.search(t, null, SEARCH_RESULTS) : { rows: [] as Row[] };
        } else {
          const search = t ? ` search ${queryString(t)}` : "";
          page = await api.query(`from ${target()}${search} sort title asc select title`, null, SEARCH_RESULTS);
        }
        if (mine !== seq) return;
        for (const r of page.rows) knowTitle(r.id, r.title);
        setResults(page.rows);
        setError(null);
      } catch (e) {
        if (mine === seq) setError(e instanceof Error ? e.message : String(e));
      }
    }, DEBOUNCE_MS);
  }

  function choose(r: Row) {
    if (props.field.multiple) {
      if (!ids().includes(r.id)) props.onChange([...ids(), r.id]);
    } else {
      props.onChange(r.id);
      setOpen(false);
    }
    setQ("");
  }

  function remove(id: string) {
    const rest = ids().filter((x) => x !== id);
    props.onChange(props.field.multiple && rest.length ? rest : undefined);
  }

  return (
    <div class="reference-picker" role="group" aria-label={props.label} id={props.id}>
      <ul class="chips">
        <For each={ids()}>
          {(id) => (
            <li class="chip">
              <a href={`/notes/${id}`}>{noteTitle(id) ?? id}</a>
              <button type="button" aria-label={`Remove ${noteTitle(id) ?? id}`} onClick={() => remove(id)}>
                ×
              </button>
            </li>
          )}
        </For>
      </ul>
      <input
        type="search"
        placeholder={target() === "*" ? "Search notes" : `Find a ${target()}`}
        aria-label={`Find ${props.label}`}
        value={q()}
        onFocus={() => find(q())}
        onInput={(e) => find(e.currentTarget.value)}
        onKeyDown={(e) => e.key === "Escape" && setOpen(false)}
      />
      <Show when={error()}>
        <p class="error">{error()}</p>
      </Show>
      <Show when={open() && results().length > 0}>
        <ul class="reference-results" role="listbox" aria-label={`${props.label} results`}>
          <For each={results().filter((r) => !ids().includes(r.id))}>
            {(r) => (
              <li role="option" aria-selected="false" onMouseDown={(e) => e.preventDefault()} onClick={() => choose(r)}>
                {r.title || "Untitled"}
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}

/** An ordered list of scalars. */
function ListInput(props: FieldInputProps & { kind: string }) {
  // Blank rows are kept while editing and left out of the value.
  const [rows, setRows] = createSignal<(Json | undefined)[]>(asList(props.value));
  function update(next: (Json | undefined)[]) {
    setRows(next);
    const values = next.filter((v): v is Json => v !== undefined);
    props.onChange(values.length ? values : undefined);
  }
  return (
    <div class="list-input" role="group" aria-label={props.label} id={props.id}>
      <For each={rows()}>
        {(v, i) => (
          <div class="list-row">
            <ScalarInput
              kind={props.kind}
              value={v}
              label={`${props.label} item ${i() + 1}`}
              onChange={(x) => update(rows().map((old, j) => (j === i() ? x : old)))}
            />
            <button
              type="button"
              aria-label={`Remove ${props.label} item ${i() + 1}`}
              onClick={() => update(rows().filter((_, j) => j !== i()))}
            >
              ×
            </button>
          </div>
        )}
      </For>
      <button type="button" onClick={() => setRows([...rows(), props.kind === "bool" ? false : undefined])}>
        Add item
      </button>
    </div>
  );
}

/** Named values of one kind. Keys are identifiers. */
function DictInput(props: FieldInputProps & { kind: string }) {
  const start = props.value && typeof props.value === "object" && !Array.isArray(props.value) ? props.value : {};
  const [rows, setRows] = createSignal<{ key: string; value: Json | undefined }[]>(
    Object.entries(start).map(([key, value]) => ({ key, value })),
  );
  function update(next: { key: string; value: Json | undefined }[]) {
    setRows(next);
    const out: Record<string, Json> = {};
    for (const r of next) if (r.key && r.value !== undefined) out[r.key] = r.value;
    props.onChange(Object.keys(out).length ? out : undefined);
  }
  const set = (i: number, patch: Partial<{ key: string; value: Json | undefined }>) =>
    update(rows().map((r, j) => (j === i ? { ...r, ...patch } : r)));
  return (
    <div class="dict-input" role="group" aria-label={props.label} id={props.id}>
      <For each={rows()}>
        {(r, i) => (
          <div class="list-row">
            <input
              aria-label={`${props.label} key ${i() + 1}`}
              placeholder="key"
              pattern="[A-Za-z_][A-Za-z0-9_]*"
              value={r.key}
              onInput={(e) => set(i(), { key: e.currentTarget.value })}
            />
            <ScalarInput
              kind={props.kind}
              value={r.value}
              label={`${props.label} value ${i() + 1}`}
              onChange={(value) => set(i(), { value })}
            />
            <button
              type="button"
              aria-label={`Remove ${props.label} entry ${i() + 1}`}
              onClick={() => update(rows().filter((_, j) => j !== i()))}
            >
              ×
            </button>
          </div>
        )}
      </For>
      <button
        type="button"
        onClick={() => setRows([...rows(), { key: "", value: props.kind === "bool" ? false : undefined }])}
      >
        Add entry
      </button>
    </div>
  );
}

/** A record of two named parts, each a scalar of `kind`. Either part empty means no value. */
function PairInput(props: FieldInputProps & { kind: string; parts: [string, string]; names: [string, string] }) {
  const rec = () =>
    props.value && typeof props.value === "object" && !Array.isArray(props.value) ? props.value : {};
  // A half-filled pair is kept here until both parts have a value.
  const [draft, setDraft] = createSignal<Record<string, Json | undefined>>({ ...rec() });
  function set(part: string, v: Json | undefined) {
    const next = { ...draft(), [part]: v };
    setDraft(next);
    const [a, b] = props.parts;
    const va = next[a];
    const vb = next[b];
    props.onChange(va === undefined || vb === undefined ? undefined : { [a]: va, [b]: vb });
  }
  return (
    <div class="pair-input" role="group" aria-label={props.label} id={props.id}>
      <For each={[0, 1] as const}>
        {(n) => (
          <label>
            <span>{props.names[n]}</span>
            <ScalarInput
              kind={props.kind}
              value={draft()[props.parts[n]]}
              label={props.names[n]}
              onChange={(v) => set(props.parts[n], v)}
            />
          </label>
        )}
      </For>
    </div>
  );
}

/**
 * A location: picked by clicking a map, by dragging its pin, or typed. A geo field picks on a
 * real-world map, an image-map field on its image (D3, D29).
 */
function LocationInput(props: FieldInputProps) {
  const map = mapKind(props.field);
  const image = map.kind === "image";
  const parts: [string, string] = image ? ["x", "y"] : ["lat", "lng"];
  const names: [string, string] = image ? ["X", "Y"] : ["Latitude", "Longitude"];
  const rec = (v: Json | undefined) => (v && typeof v === "object" && !Array.isArray(v) ? v : {});
  // A half-typed point is kept here until both parts have a value.
  const [draft, setDraft] = createSignal<Record<string, Json | undefined>>({ ...rec(props.value) });
  function pick(v: Json) {
    setDraft({ ...rec(v) });
    props.onChange(v);
  }
  function set(part: string, v: Json | undefined) {
    const next = { ...draft(), [part]: v };
    setDraft(next);
    const [a, b] = parts;
    props.onChange(next[a] === undefined || next[b] === undefined ? undefined : { [a]: next[a]!, [b]: next[b]! });
  }
  const pins = () => {
    const d = draft() as Json;
    const point = image ? imagePoint(d) : geoPoint(d);
    return point ? [{ id: "value", title: props.label, point }] : [];
  };
  return (
    <div class="location-input" role="group" aria-label={props.label} id={props.id}>
      <MapCanvas
        map={map}
        pins={pins()}
        label={`${props.label} map`}
        class="location-map"
        draggable
        onPick={pick}
        onMove={(_, v) => pick(v)}
      />
      <div class="pair-input">
        <For each={[0, 1] as const}>
          {(n) => (
            <label>
              <span>{names[n]}</span>
              <ScalarInput
                kind="float"
                value={draft()[parts[n]]}
                label={names[n]}
                onChange={(v) => set(parts[n], v)}
              />
            </label>
          )}
        </For>
        <button
          type="button"
          onClick={() => {
            setDraft({});
            props.onChange(undefined);
          }}
        >
          Clear
        </button>
      </div>
    </div>
  );
}

function AssetInput(props: FieldInputProps) {
  const [assets] = createResource(() => api.assets());
  const options = () => {
    const list = (assets() ?? []).map((a) => ({ value: a.id, text: a.filename }));
    // Keep a stored id the listing no longer has, so it is not silently dropped.
    for (const id of asStrings(props.value)) {
      if (!list.some((o) => o.value === id)) list.push({ value: id, text: id });
    }
    return list;
  };
  return (
    <Choice
      options={options()}
      multiple={props.field.multiple}
      required={props.field.required}
      value={props.value}
      onChange={props.onChange}
      label={props.label}
      id={props.id}
    />
  );
}

/** The editor for one field of a note, chosen by the field's kind. */
export function FieldInput(props: FieldInputProps) {
  const f = () => props.field;
  return (
    <Switch
      fallback={
        <ScalarInput
          kind={f().kind}
          value={props.value}
          onChange={props.onChange}
          label={props.label}
          id={props.id}
          required={f().required}
          placeholder={props.placeholder}
        />
      }
    >
      <Match when={f().kind === "select"}>
        <Choice
          options={(f().options ?? []).map((o) => ({ value: o, text: o }))}
          multiple={f().multiple}
          required={f().required}
          value={props.value}
          onChange={props.onChange}
          label={props.label}
          id={props.id}
        />
      </Match>
      <Match when={f().kind === "taxonomy"}>
        <Choice
          options={termOptions(taxonomyDef(f().taxonomy ?? "")?.terms ?? [])}
          multiple={f().multiple}
          required={f().required}
          value={props.value}
          onChange={props.onChange}
          label={props.label}
          id={props.id}
        />
      </Match>
      <Match when={f().kind === "reference"}>
        <ReferencePicker {...props} />
      </Match>
      <Match when={f().kind === "asset"}>
        <AssetInput {...props} />
      </Match>
      <Match when={f().kind === "list"}>
        <ListInput {...props} kind={f().items ?? "string"} />
      </Match>
      <Match when={f().kind === "dict"}>
        <DictInput {...props} kind={f().values ?? "string"} />
      </Match>
      <Match when={f().kind === "date_range"}>
        <PairInput {...props} kind={f().of ?? "date"} parts={["start", "end"]} names={["Start", "End"]} />
      </Match>
      <Match when={f().kind === "location"}>
        <LocationInput {...props} />
      </Match>
    </Switch>
  );
}

/** Kinds a note page edits with [`FieldInput`]. Content has the block editor; ink and canvas their own. */
const INPUT_KINDS = new Set([
  "string",
  "int",
  "float",
  "bool",
  "date",
  "datetime",
  "date_range",
  "url",
  "select",
  "taxonomy",
  "reference",
  "list",
  "dict",
  "location",
  "asset",
]);

export function hasInput(f: FieldDef): boolean {
  return !f.computed && INPUT_KINDS.has(f.kind);
}
