// Editors for every field kind a note page edits in place (D3, D7). Content, ink and canvas fields
// have editors of their own; a computed field is never given one.

import { For, Match, Show, Switch, createEffect, createMemo, createSignal, on } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { type FieldDef, type Json, type Row, type Term, api } from "../api";
import { knowTitle, noteTitle } from "../editor/titles";
import { MapCanvas } from "../map/MapCanvas";
import {
  LOCATION_MAP_HEIGHT,
  geoPoint,
  imagePoint,
  locationMapHeight,
  mapKind,
  setLocationMapHeight,
} from "../map/model";
import { canonicalJson } from "../notes";
import { taxonomyDef, typeDef, types } from "../session";
import { filterText, readTypeFilter, searchNotes } from "../typeFilter";
import { AssetInput } from "./AssetField";
import { ChipPicker, matches } from "./ChipPicker";
import type { Suggestion } from "./Combobox";
import { ImageField } from "./ImageField";
import { LabelledList, type LabelledShape, labelled, labelledShape, labelledValue } from "./LabelledList";
import { LinkListInput, linkShape } from "./LinkList";
import { optionChoices, optionText } from "../options";
import { termPicker } from "./termPicker";
import { useControlBinding } from "../controls/binding";
import { RuleEditor } from "../series/RuleEditor";
import { isRuleField } from "../series/rrule";

export interface FieldInputProps {
  field: FieldDef;
  value: Json | undefined;
  onChange: (value: Json | undefined) => void;
  label: string;
  id: string;
  /** Shown in an empty text-like input. */
  placeholder?: string;
  /** Offer no new terms, even for a field on an open taxonomy, as for a field's default. */
  fixedTerms?: boolean;
  /** The note the field belongs to, for suggestions drawn from other notes of its type. */
  related?: RelatedFields;
}

/** The note a field belongs to. */
export interface RelatedFields {
  /** The note's type name, for suggestions drawn from other notes of the type. */
  type?: string;
}

/**
 * The labels notes of `type` give the values of field `name`, a list of labelled values (each
 * item's `label`), most used first; fetched once per type and field while the page is open.
 */
const knownLabelCache = new Map<string, Promise<string[]>>();
export function knownLabels(type: string | undefined, name: string): Promise<string[]> {
  if (!type) return Promise.resolve([]);
  const key = `${type}\u0000${name}`;
  let p = knownLabelCache.get(key);
  if (!p) {
    p = api
      .query(`from ${type} select ${name}`, null, 500)
      .then((page) => {
        const counts = new Map<string, number>();
        for (const r of page.rows) {
          const v = r.fields[name];
          for (const item of Array.isArray(v) ? v : []) {
            const l = item && typeof item === "object" && !Array.isArray(item) ? item.label : undefined;
            if (typeof l === "string" && l.trim()) counts.set(l.trim(), (counts.get(l.trim()) ?? 0) + 1);
          }
        }
        return [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([l]) => l);
      })
      .catch(() => {
        knownLabelCache.delete(key);
        return [];
      });
    knownLabelCache.set(key, p);
  }
  return p;
}

/** A list of labelled values: one chip per value with its label. */
function LabelledInput(props: FieldInputProps & { shape: LabelledShape }) {
  const l = () => labelled(props.value, props.shape.key);
  return (
    <LabelledList
      id={props.id}
      label={props.label}
      values={l().values}
      labels={l().labels}
      emptyText={props.placeholder}
      knownLabels={() => knownLabels(props.related?.type, props.field.name)}
      onChange={(next) => props.onChange(labelledValue(next, props.shape))}
    />
  );
}

const SCALAR_INPUT: Record<string, string> = {
  int: "number",
  float: "number",
  date: "date",
  datetime: "datetime-local",
  time: "time",
  url: "url",
};

const KEY_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

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

/** A stored time of day, 24-hour: `HH:MM` or `HH:MM:SS`. */
export const TIME = /^([01]\d|2[0-3]):[0-5]\d(:[0-5]\d)?$/;

/**
 * A time input's text as a stored time: `HH:MM`, or `HH:MM:SS` when it has seconds. A fraction of
 * a second some browsers add is dropped.
 */
export function timeValue(raw: string): string {
  const m = /^(\d\d:\d\d(?::\d\d)?)(?:\.\d+)?$/.exec(raw);
  return m ? m[1]! : raw;
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
  if (kind === "time") return timeValue(raw);
  return raw;
}

function scalarText(kind: string, v: Json | undefined): string {
  if (v === undefined || v === null) return "";
  if (kind === "datetime") return datetimeInput(v);
  return typeof v === "object" ? JSON.stringify(v) : String(v);
}

/**
 * The step of a scalar's input: any number for a float, and seconds for a datetime, and for a
 * time that holds seconds, so a time without them stays `HH:MM`.
 */
function scalarStep(kind: string, v: Json | undefined): string | undefined {
  if (kind === "float") return "any";
  if (kind === "datetime") return "1";
  if (kind === "time" && typeof v === "string" && v.length === 8) return "1";
  return undefined;
}

/** One scalar of `kind`: string, int, float, bool, date, datetime, time or url. */
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
          step={scalarStep(props.kind, props.value)}
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

/** One choice from `options`, in a select. */
function Choice(props: {
  options: { value: string; text: string; depth?: number }[];
  required: boolean;
  value: Json | undefined;
  onChange: (v: Json | undefined) => void;
  label: string;
  id: string;
}) {
  return (
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
  );
}

/**
 * A field's values as chips with a + to add to them: several, or one when the field is not
 * `multiple`. An empty list is no value.
 */
function Picker(
  props: FieldInputProps & {
    suggest: (text: string) => Promise<Suggestion[]> | Suggestion[];
    freeText?: boolean;
    chipText?: (v: string) => string;
    chipHref?: (v: string) => string | undefined;
    chipColour?: (v: string) => string | undefined;
    /** What the add box shows before anything is typed. */
    boxPlaceholder?: string;
    badge?: (text: string) => { icon: string | null; text: string } | null;
    create?: (text: string) => { value: string } | { error: string };
  },
) {
  const single = () => !props.field.multiple && props.field.kind !== "list";
  const [problem, setProblem] = createSignal<string | null>(null);
  return (
    <>
      <ChipPicker
        id={props.id}
        label={props.label}
        values={asStrings(props.value)}
        single={single()}
        suggest={props.suggest}
        freeText={props.freeText}
        create={props.create}
        chipText={props.chipText}
        chipHref={props.chipHref}
        chipColour={props.chipColour}
        placeholder={props.boxPlaceholder}
        badge={props.badge}
        emptyText={props.placeholder}
        describedBy={problem() ? `${props.id}-problem` : undefined}
        onReject={props.create ? (t, reason) => setProblem(reason ?? `${t} is not one of the choices.`) : undefined}
        onChange={(values) => {
          setProblem(null);
          props.onChange(values.length === 0 ? undefined : single() ? values[0] : values);
        }}
      />
      <Show when={problem()}>
        <p class="field-problem error" id={`${props.id}-problem`} role="alert">
          {problem()}
        </p>
      </Show>
    </>
  );
}

/** Options as suggestions, matched by their text. */
const optionSuggestions = (options: { value: string; text: string }[], text: string): Suggestion[] =>
  options.filter((o) => matches(text, o.text, o.value));

/** Every term of a taxonomy, parents first, with its depth. */
export function termOptions(terms: Term[], depth = 0): { value: string; text: string; depth: number }[] {
  return terms.flatMap((t) => [{ value: t.path, text: t.name, depth }, ...termOptions(t.children, depth + 1)]);
}

const SEARCH_RESULTS = 20;

/** A query-language string literal. */
function queryString(s: string): string {
  return JSON.stringify(s);
}

/**
 * Up to 20 notes of type `target` (`*` for any type) whose text matches `text`, by title; with no
 * text, the first notes of the type by title (none for any type). For any type, `<type>/<text>`
 * looks among one type's notes (see `readTypeFilter`). Their titles are remembered.
 */
export async function findNotes(target: string, text: string): Promise<Row[]> {
  const t = text.trim();
  let page;
  if (target === "*") {
    const f = readTypeFilter(text, types());
    page = { rows: await searchNotes(f.type?.name ?? null, f.text, SEARCH_RESULTS) };
  } else {
    const search = t ? ` search ${queryString(t)}` : "";
    page = await api.query(`from ${target}${search} sort title asc select title`, null, SEARCH_RESULTS);
  }
  for (const r of page.rows) knowTitle(r.id, r.title);
  return page.rows;
}

/** Marks a suggestion that is a type to filter by, or a hint; no note id starts with it. */
const NOT_A_NOTE = "\u0000";

/**
 * Suggestions for a search of every type's notes: while a type is being typed, the types it
 * starts, which fill in `<type>/`; then why the text sets no filter, if it tried to; then the
 * notes found, as `row` shows them.
 */
export async function suggestNotes(text: string, row: (r: Row) => Suggestion): Promise<Suggestion[]> {
  const f = readTypeFilter(text, types());
  const out: Suggestion[] = f.suggestions.map((t) => ({ value: `${NOT_A_NOTE}type:${t.name}`, text: t.label, icon: t.icon, hint: "filter by type", fill: filterText(t) }));
  if (f.problem) out.push({ value: `${NOT_A_NOTE}problem`, text: f.problem, disabled: true });
  const rows = await findNotes("*", text);
  out.push(...rows.map(row));
  if (f.type && rows.length === 0) out.push({ value: `${NOT_A_NOTE}none`, text: f.text ? `No ${f.type.label.toLowerCase()} matches “${f.text}”` : `No ${f.type.label.toLowerCase()} yet`, disabled: true });
  return out;
}

/** The type a search of every type's notes is limited to, as a box's badge. */
export function noteBadge(text: string): { icon: string | null; text: string } | null {
  const t = readTypeFilter(text, types()).type;
  return t ? { icon: t.icon, text: t.label } : null;
}

/** Notes of the field's target type, found by title text, to link to. */
function ReferencePicker(props: FieldInputProps) {
  const target = () => props.field.target ?? "*";
  return (
    <Picker
      {...props}
      boxPlaceholder={target() === "*" ? "Search notes" : `Find a ${(typeDef(target())?.label ?? target()).toLowerCase()}`}
      suggest={async (text) => {
        const row = (r: Row): Suggestion => ({ value: r.id, text: r.title || "Untitled" });
        return target() === "*" ? suggestNotes(text, row) : (await findNotes(target(), text)).map(row);
      }}
      badge={target() === "*" ? noteBadge : undefined}
      chipText={(id) => noteTitle(id) ?? id}
      chipHref={(id) => `/notes/${id}`}
    />
  );
}

/** The definition of a list's items: its `item`, or a bare one of its `items` kind from an older server. */
export function itemDef(f: FieldDef): FieldDef {
  return f.item ?? { name: f.name, label: f.label, description: null, kind: f.items ?? "string", required: false, computed: false, multiple: false, compound: false };
}

/** What a dict's other entries are, an older server's kind name read as a bare definition of it. */
export function dictValues(f: FieldDef): FieldDef | "any" | null {
  const v = f.values as FieldDef | string | null | undefined;
  if (v === "any") return "any";
  if (typeof v === "string") return { name: f.name, label: f.label, description: null, kind: v, required: false, computed: false, multiple: false, compound: false };
  return v ?? null;
}

const isRecord = (v: Json | undefined): v is Record<string, Json> => !!v && typeof v === "object" && !Array.isArray(v);
const same = (a: Json | undefined, b: Json | undefined) => canonicalJson(a) === canonicalJson(b);

/** A new row's value: an unticked box for a yes/no, else nothing yet. */
const blank = (kind: string): Json | undefined => (kind === "bool" ? false : undefined);

let nextRow = 1;

/**
 * An ordered list, each item edited with [`FieldInput`] for the list's item definition: a list of
 * date ranges, of locations, of lists and of dicts as well as of scalars. Rows move up and down.
 */
function ListInput(props: FieldInputProps) {
  const item = () => itemDef(props.field);
  // Each row has an id, so a row keeps its editor while its value changes and as it moves. Blank
  // rows are kept while editing and left out of the value.
  const [rows, setRows] = createSignal<number[]>([]);
  const [values, setValues] = createSignal<Record<number, Json | undefined>>({});
  function load(v: Json | undefined) {
    const list = asList(v);
    const ids = list.map(() => nextRow++);
    setValues(Object.fromEntries(ids.map((id, i) => [id, list[i]])));
    setRows(ids);
  }
  load(props.value);
  // What this editor last wrote. A different value came from elsewhere (a control bound to the
  // same field), and the rows start again from it.
  let written: Json | undefined = props.value;
  createEffect(
    on(
      () => props.value,
      (v) => {
        if (!same(v, written)) load(v);
        written = v;
      },
      { defer: true },
    ),
  );
  function write(ids: number[], vals: Record<number, Json | undefined>) {
    setValues(vals);
    setRows(ids);
    const out = ids.map((id) => vals[id]).filter((v): v is Json => v !== undefined);
    written = out.length ? out : undefined;
    props.onChange(written);
  }
  function move(i: number, by: number) {
    const ids = [...rows()];
    const j = i + by;
    if (j < 0 || j >= ids.length) return;
    [ids[i], ids[j]] = [ids[j]!, ids[i]!];
    write(ids, values());
  }
  return (
    <div class="list-input" role="group" aria-label={props.label} id={props.id}>
      <For each={rows()}>
        {(row, i) => {
          const label = () => `${props.label} item ${i() + 1}`;
          return (
            <div class="list-row" data-testid="list-item">
              <div class="list-item-value">
                <FieldInput
                  field={item()}
                  value={values()[row]}
                  onChange={(v) => write(rows(), { ...values(), [row]: v })}
                  label={label()}
                  id={`${props.id}-item-${row}`}
                  fixedTerms={props.fixedTerms}
                />
              </div>
              <button type="button" aria-label={`Move ${label()} up`} disabled={i() === 0} onClick={() => move(i(), -1)}>
                ↑
              </button>
              <button
                type="button"
                aria-label={`Move ${label()} down`}
                disabled={i() === rows().length - 1}
                onClick={() => move(i(), 1)}
              >
                ↓
              </button>
              <button type="button" aria-label={`Remove ${label()}`} onClick={() => write(rows().filter((r) => r !== row), values())}>
                ×
              </button>
            </div>
          );
        }}
      </For>
      <button
        type="button"
        onClick={() => {
          const id = nextRow++;
          setValues({ ...values(), [id]: blank(item().kind) });
          setRows([...rows(), id]);
        }}
      >
        Add item
      </button>
    </div>
  );
}

/** The kinds an entry of a dict with `values = "any"` may be chosen as, and what the picker calls them. */
export const ANY_KINDS = [
  ["string", "Text"],
  ["int", "Whole number"],
  ["float", "Number"],
  ["bool", "Yes/no"],
  ["date", "Date"],
  ["datetime", "Date and time"],
  ["time", "Time"],
  ["url", "URL"],
  ["list", "List of text"],
] as const;

const DATE = /^\d{4}-\d\d-\d\d$/;

/**
 * The kind an entry of any kind is edited as, read from its value; `other` for a record or a list
 * of other than text, which is shown as it is.
 */
export function anyKind(v: Json | undefined): string {
  if (v === undefined || v === null) return "string";
  if (typeof v === "boolean") return "bool";
  if (typeof v === "number") return Number.isInteger(v) ? "int" : "float";
  if (typeof v === "string") {
    if (DATE.test(v)) return "date";
    if (DATETIME.test(v)) return "datetime";
    if (TIME.test(v)) return "time";
    if (/^https?:\/\//.test(v)) return "url";
    return "string";
  }
  if (Array.isArray(v) && v.every((x) => typeof x === "string")) return "list";
  return "other";
}

/** A bare definition of `kind`, as an entry of any kind is edited. */
function anyDef(name: string, kind: string): FieldDef {
  const d: FieldDef = { name, label: name, description: null, kind, required: false, computed: false, multiple: false, compound: false };
  return kind === "list" ? { ...d, items: "string", item: { ...d, kind: "string" } } : d;
}

/** An entry of a dict no key declares, as its editor holds it. */
interface FreeEntry {
  key: string;
  /** For a dict of any kind: what the entry is edited as. */
  kind: string;
}

/**
 * A dict (types T8): each declared key on a row of its own, edited with [`FieldInput`] for the
 * key's definition, then the entries no key declares, each with its key name and, when the dict
 * takes entries of any kind, a picker of the entry's kind. The value keeps the declared keys
 * first, in their order, then the other entries in theirs.
 */
function DictInput(props: FieldInputProps) {
  const keys = () => props.field.keys ?? [];
  const values = () => dictValues(props.field);
  const declared = () => new Set(keys().map((k) => k.name));
  const [own, setOwn] = createSignal<Record<string, Json | undefined>>({});
  const [rows, setRows] = createSignal<number[]>([]);
  const [free, setFree] = createStore<Record<number, FreeEntry>>({});
  const [freeValues, setFreeValues] = createSignal<Record<number, Json | undefined>>({});
  function load(v: Json | undefined) {
    const rec = isRecord(v) ? v : {};
    const names = declared();
    setOwn(Object.fromEntries(Object.entries(rec).filter(([k]) => names.has(k))));
    const others = Object.entries(rec).filter(([k]) => !names.has(k));
    const ids = others.map(() => nextRow++);
    setFree(reconcile(Object.fromEntries(ids.map((id, i) => [id, { key: others[i]![0], kind: anyKind(others[i]![1]) }]))));
    setFreeValues(Object.fromEntries(ids.map((id, i) => [id, others[i]![1]])));
    setRows(ids);
  }
  load(props.value);
  let written: Json | undefined = props.value;
  createEffect(
    on(
      () => props.value,
      (v) => {
        if (!same(v, written)) load(v);
        written = v;
      },
      { defer: true },
    ),
  );
  /** Whether a free entry's key is one the value can hold: a key name no key and no earlier entry takes. */
  const keyProblem = (id: number): string | null => {
    const key = free[id]?.key ?? "";
    if (key === "") return null;
    if (!KEY_NAME.test(key)) return "Use letters, digits and _, not starting with a digit.";
    if (declared().has(key)) return `${key} is a key of its own above.`;
    if (rows().some((r) => r < id && free[r]?.key === key)) return `${key} is already an entry.`;
    return null;
  };
  function write() {
    const out: Record<string, Json> = {};
    const mine = own();
    for (const k of keys()) {
      const v = mine[k.name];
      if (v !== undefined) out[k.name] = v;
    }
    const vals = freeValues();
    for (const id of rows()) {
      const key = free[id]?.key ?? "";
      const v = vals[id];
      if (key && !keyProblem(id) && v !== undefined) out[key] = v;
    }
    written = Object.keys(out).length ? out : undefined;
    props.onChange(written);
  }
  const entryDef = (id: number): FieldDef | null => {
    const v = values();
    if (v === "any" || v === null) {
      const kind = free[id]?.kind ?? "string";
      return kind === "other" ? null : anyDef(props.field.name, kind);
    }
    return v;
  };
  function addEntry() {
    const id = nextRow++;
    const v = values();
    const kind = v === "any" || v === null ? "string" : v.kind;
    setFree(id, { key: "", kind });
    setFreeValues({ ...freeValues(), [id]: blank(kind) });
    setRows([...rows(), id]);
  }
  return (
    <div class="dict-input" role="group" aria-label={props.label} id={props.id}>
      <For each={keys()}>
        {(k) => {
          const id = `${props.id}-key-${k.name}`;
          const label = () => k.label || k.name;
          return (
            <div class="list-row dict-key-entry" data-testid={`dict-key-${k.name}`}>
              <label class="dict-entry-label" for={id}>
                {label()}
                <Show when={k.required}>
                  <span class="muted" title="Required">
                    {" "}
                    (required)
                  </span>
                </Show>
              </label>
              <div class="list-item-value">
                <FieldInput
                  field={k}
                  value={own()[k.name]}
                  onChange={(v) => {
                    setOwn({ ...own(), [k.name]: v });
                    write();
                  }}
                  label={label()}
                  id={id}
                  fixedTerms={props.fixedTerms}
                />
              </div>
            </div>
          );
        }}
      </For>
      <For each={rows()}>
        {(row, i) => {
          const n = () => i() + 1;
          // A new editor only when the entry's kind changes, not as its key is typed.
          const def = createMemo(() => entryDef(row));
          return (
            <div class="list-row dict-free-entry" classList={{ "with-kind": values() === "any" }} data-testid="dict-entry">
              <input
                aria-label={`${props.label} key ${n()}`}
                placeholder="key"
                pattern="[A-Za-z_][A-Za-z0-9_]*"
                value={free[row]?.key ?? ""}
                aria-invalid={keyProblem(row) ? "true" : undefined}
                title={keyProblem(row) ?? undefined}
                onInput={(e) => {
                  setFree(row, "key", e.currentTarget.value);
                  write();
                }}
              />
              <Show when={values() === "any"}>
                <select
                  aria-label={`${props.label} kind ${n()}`}
                  onChange={(e) => {
                    const kind = e.currentTarget.value;
                    setFree(row, "kind", kind);
                    setFreeValues({ ...freeValues(), [row]: blank(kind) });
                    write();
                  }}
                >
                  <For each={ANY_KINDS}>
                    {([kind, text]) => (
                      <option value={kind} selected={free[row]?.kind === kind}>
                        {text}
                      </option>
                    )}
                  </For>
                  <Show when={free[row]?.kind === "other"}>
                    <option value="other" selected>
                      As stored
                    </option>
                  </Show>
                </select>
              </Show>
              <div class="list-item-value">
                <Show when={def()} keyed fallback={<code>{JSON.stringify(freeValues()[row] ?? null)}</code>}>
                  {(d) => (
                    <FieldInput
                      field={d}
                      value={freeValues()[row]}
                      onChange={(v) => {
                        setFreeValues({ ...freeValues(), [row]: v });
                        write();
                      }}
                      label={`${props.label} value ${n()}`}
                      id={`${props.id}-entry-${row}`}
                      fixedTerms={props.fixedTerms}
                    />
                  )}
                </Show>
              </div>
              <button
                type="button"
                aria-label={`Remove ${props.label} entry ${n()}`}
                onClick={() => {
                  setRows(rows().filter((r) => r !== row));
                  write();
                }}
              >
                ×
              </button>
              <Show when={keyProblem(row)}>
                <p class="field-problem error" role="alert">
                  {keyProblem(row)}
                </p>
              </Show>
            </div>
          );
        }}
      </For>
      <Show when={values() !== null}>
        <button type="button" onClick={addEntry}>
          Add entry
        </button>
      </Show>
    </div>
  );
}

/** Whether a `date_or_datetime` range holds dates: its ends are ten-character dates, or it is empty. */
export function rangeIsAllDay(range: Record<string, Json | undefined>): boolean {
  const end = [range.start, range.end].find((v) => typeof v === "string");
  return typeof end !== "string" || end.length === 10;
}

/**
 * A `date_or_datetime` range's ends moved to dates (`allDay`) or to datetimes (google-sync D15).
 * A datetime keeps its local date. A date becomes a time in the browser's zone: the start at the
 * start of its day, the end at 23:59 on its day. An end already in the form is kept.
 */
export function switchAllDay(range: Record<string, Json | undefined>, allDay: boolean): Record<string, Json | undefined> {
  const out: Record<string, Json | undefined> = { ...range };
  for (const part of ["start", "end"] as const) {
    const v = range[part];
    if (typeof v !== "string") continue;
    if (allDay) {
      if (v.length !== 10) out[part] = datetimeInput(v).slice(0, 10) || undefined;
    } else if (/^\d{4}-\d\d-\d\d$/.test(v)) {
      out[part] = datetimeValue(`${v}T${part === "start" ? "00:00" : "23:59"}`, undefined);
    }
  }
  return out;
}

/**
 * A record of two named parts, each a scalar of `kind`. Either part empty means no value. With
 * `allDayToggle`, for a `date_or_datetime` range, an All day box picks dates or datetimes for both.
 */
function PairInput(
  props: FieldInputProps & { kind: string; parts: [string, string]; names: [string, string]; allDayToggle?: boolean },
) {
  const rec = () =>
    props.value && typeof props.value === "object" && !Array.isArray(props.value) ? props.value : {};
  // A half-filled pair is kept here until both parts have a value.
  const [draft, setDraft] = createSignal<Record<string, Json | undefined>>({ ...rec() });
  const [allDay, setAllDay] = createSignal(rangeIsAllDay(rec()));
  const kind = () => (props.allDayToggle ? (allDay() ? "date" : "datetime") : props.kind);
  // A value written elsewhere (a calendar control on the same field) replaces the draft.
  let written: Json | undefined = props.value;
  createEffect(
    on(
      () => props.value,
      (v) => {
        if (JSON.stringify(v ?? null) !== JSON.stringify(written ?? null)) {
          // The inputs change type before they take the new text, which the old type would refuse.
          if (v !== undefined && v !== null) setAllDay(rangeIsAllDay(rec()));
          setDraft({ ...rec() });
        }
        written = v;
      },
      { defer: true },
    ),
  );
  function write(next: Record<string, Json | undefined>) {
    setDraft(next);
    const [a, b] = props.parts;
    const va = next[a];
    const vb = next[b];
    written = va === undefined || vb === undefined ? undefined : { [a]: va, [b]: vb };
    props.onChange(written);
  }
  const set = (part: string, v: Json | undefined) => write({ ...draft(), [part]: v });
  function toggle(on: boolean) {
    setAllDay(on);
    const next = switchAllDay(draft(), on);
    // Nothing to convert: no value is written.
    if (JSON.stringify(next) === JSON.stringify(draft())) setDraft(next);
    else write(next);
  }
  return (
    <div class="pair-input" role="group" aria-label={props.label} id={props.id}>
      <Show when={props.allDayToggle}>
        <label class="all-day">
          <input type="checkbox" checked={allDay()} onChange={(e) => toggle(e.currentTarget.checked)} />
          <span>All day</span>
        </label>
      </Show>
      <For each={[0, 1] as const}>
        {(n) => (
          <label>
            <span>{props.names[n]}</span>
            <ScalarInput
              kind={kind()}
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
 * real-world map, an image-map field on its image (D3, D29). The handle on the map's bottom edge
 * sets its height, which this browser keeps per type and field (docs/vault-format.md, "location").
 */
function LocationInput(props: FieldInputProps) {
  const map = mapKind(props.field);
  const image = map.kind === "image";
  const parts: [string, string] = image ? ["x", "y"] : ["lat", "lng"];
  const names: [string, string] = image ? ["X", "Y"] : ["Latitude", "Longitude"];
  const rec = (v: Json | undefined) => (v && typeof v === "object" && !Array.isArray(v) ? v : {});
  // A half-typed point is kept here until both parts have a value.
  const [draft, setDraft] = createSignal<Record<string, Json | undefined>>({ ...rec(props.value) });
  // A value written elsewhere (the same field on the page and in the properties sidebar, or a
  // control bound to it) replaces the draft.
  let written: Json | undefined = props.value;
  createEffect(
    on(
      () => props.value,
      (v) => {
        if (!same(v, written)) setDraft({ ...rec(v) });
        written = v;
      },
      { defer: true },
    ),
  );
  function pick(v: Json) {
    setDraft({ ...rec(v) });
    written = v;
    props.onChange(v);
  }
  function set(part: string, v: Json | undefined) {
    const next = { ...draft(), [part]: v };
    setDraft(next);
    const [a, b] = parts;
    written = next[a] === undefined || next[b] === undefined ? undefined : { [a]: next[a]!, [b]: next[b]! };
    props.onChange(written);
  }
  const heightKey = () => [props.related?.type ?? "", props.field.name] as const;
  const [height, setHeight] = createSignal(locationMapHeight(...heightKey()));
  function resize(h: number) {
    setLocationMapHeight(...heightKey(), h);
    setHeight(h);
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
        height={height()}
        range={LOCATION_MAP_HEIGHT}
        onHeight={resize}
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
            written = undefined;
            props.onChange(undefined);
          }}
        >
          Clear
        </button>
      </div>
    </div>
  );
}

/** Several terms of a taxonomy as chips. An open taxonomy takes new terms typed into the box. */
function TermsPicker(props: FieldInputProps) {
  const terms = termPicker(() => props.field.taxonomy ?? "", props.fixedTerms);
  return (
    <Picker
      {...props}
      suggest={terms.suggest}
      chipColour={terms.chipColour}
      create={terms.create()}
      onChange={(v) => props.onChange(terms.picked(v))}
    />
  );
}

/** The editor for one field of a note, chosen by the field's kind. */
export function FieldInput(props: FieldInputProps) {
  const f = () => props.field;
  const binding = useControlBinding();
  // An event series' rule gets the rule builder (event-series E6).
  const rule = () => isRuleField(f(), props.related?.type ?? binding?.type()?.name);
  return (
    <Switch
      fallback={
        <ScalarInput
          kind={f().kind}
          // A checkbox of a note from before the field shows the field's default.
          value={f().kind === "bool" && (props.value === undefined || props.value === null) ? f().default : props.value}
          onChange={props.onChange}
          label={props.label}
          id={props.id}
          required={f().required}
          placeholder={props.placeholder}
        />
      }
    >
      <Match when={f().kind === "select" && f().multiple}>
        <Picker {...props} suggest={(text) => optionSuggestions(optionChoices(f()), text)} chipText={(v) => optionText(f(), v)} />
      </Match>
      <Match when={f().kind === "select"}>
        <Choice
          options={optionChoices(f())}
          required={f().required}
          value={props.value}
          onChange={props.onChange}
          label={props.label}
          id={props.id}
        />
      </Match>
      <Match when={f().kind === "taxonomy" && f().multiple}>
        <TermsPicker {...props} />
      </Match>
      <Match when={f().kind === "taxonomy"}>
        <Choice
          options={termOptions(taxonomyDef(f().taxonomy ?? "")?.terms ?? [])}
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
      <Match when={f().kind === "asset" && f().accept === "image" && !f().multiple}>
        <ImageField
          id={props.id}
          label={props.label}
          value={typeof props.value === "string" ? props.value : undefined}
          onChange={(id) => props.onChange(id)}
        />
      </Match>
      <Match when={f().kind === "asset"}>
        <AssetInput {...props} />
      </Match>
      <Match when={rule()}>
        <RuleEditor value={props.value} onChange={props.onChange} label={props.label} id={props.id} />
      </Match>
      <Match when={labelledShape(f())}>{(shape) => <LabelledInput {...props} shape={shape()} />}</Match>
      <Match when={linkShape(f())}>
        {(shape) => <LinkListInput shape={shape()} value={props.value} onChange={props.onChange} label={props.label} id={props.id} />}
      </Match>
      <Match when={f().kind === "list" && (f().items ?? "string") === "string"}>
        <Picker {...props} freeText suggest={() => []} />
      </Match>
      <Match when={f().kind === "list"}>
        <ListInput {...props} />
      </Match>
      <Match when={f().kind === "dict"}>
        <DictInput {...props} />
      </Match>
      <Match when={f().kind === "date_range"}>
        <PairInput
          {...props}
          kind={f().of ?? "date"}
          allDayToggle={f().of === "date_or_datetime"}
          parts={["start", "end"]}
          names={["Start", "End"]}
        />
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
  "time",
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
