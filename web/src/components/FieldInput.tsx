// Editors for every field kind a note page edits in place (D3, D7). Content, ink and canvas fields
// have editors of their own; a computed field is never given one.

import { For, Match, Show, Switch, createEffect, createResource, createSignal, on } from "solid-js";
import { type Asset, type FieldDef, type Json, type Row, type Term, api } from "../api";
import { FilePicker, upload } from "../blocks/MediaBlocks";
import { knowTitle, noteTitle } from "../editor/titles";
import { MapCanvas } from "../map/MapCanvas";
import { isPdfFile } from "../pdf/model";
import { PdfThumbButton } from "../pdf/PdfPreview";
import { geoPoint, imagePoint, mapKind } from "../map/model";
import { taxonomyDef, typeDef } from "../session";
import { ChipPicker, matches } from "./ChipPicker";
import type { Suggestion } from "./Combobox";
import { ImageField } from "./ImageField";
import { LabelledList, labelled, labelledWrite } from "./LabelledList";
import { termPicker } from "./termPicker";

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
  /**
   * The note's other fields, for an editor that writes more than one field in one edit: a list
   * with `labels` writes its values and their labels together. Without it, such a list is edited
   * as a plain list.
   */
  related?: RelatedFields;
}

/** The fields of the note a field belongs to. */
export interface RelatedFields {
  /** The note's type name, for suggestions drawn from other notes of the type. */
  type?: string;
  value: (name: string) => Json | undefined;
  /** Write several fields in one edit, `undefined` removing one. */
  set: (values: Record<string, Json | undefined>) => void;
}

/**
 * The labels notes of `type` hold in field `name`, most used first; fetched once per type and
 * field while the page is open.
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
          for (const l of Array.isArray(v) ? v : []) {
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

/** A list of strings with `labels`: one chip per value with its label, both lists written together. */
function LabelledInput(props: FieldInputProps & { related: RelatedFields; labelsField: string }) {
  const l = () => labelled(props.value, props.related.value(props.labelsField));
  return (
    <LabelledList
      id={props.id}
      label={props.label}
      values={l().values}
      labels={l().labels}
      emptyText={props.placeholder}
      knownLabels={() => knownLabels(props.related.type, props.labelsField)}
      onChange={(next) => props.related.set(labelledWrite(next, props.field.name, props.labelsField))}
    />
  );
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
 * text, the first notes of the type by title (none for any type). Their titles are remembered.
 */
export async function findNotes(target: string, text: string): Promise<Row[]> {
  const t = text.trim();
  let page;
  if (target === "*") {
    page = t ? await api.search(t, null, SEARCH_RESULTS) : { rows: [] as Row[] };
  } else {
    const search = t ? ` search ${queryString(t)}` : "";
    page = await api.query(`from ${target}${search} sort title asc select title`, null, SEARCH_RESULTS);
  }
  for (const r of page.rows) knowTitle(r.id, r.title);
  return page.rows;
}

/** Notes of the field's target type, found by title text, to link to. */
function ReferencePicker(props: FieldInputProps) {
  const target = () => props.field.target ?? "*";
  return (
    <Picker
      {...props}
      boxPlaceholder={target() === "*" ? "Search notes" : `Find a ${(typeDef(target())?.label ?? target()).toLowerCase()}`}
      suggest={async (text) => (await findNotes(target(), text)).map((r) => ({ value: r.id, text: r.title || "Untitled" }))}
      chipText={(id) => noteTitle(id) ?? id}
      chipHref={(id) => `/notes/${id}`}
    />
  );
}

/** An ordered list of scalars. */
function ListInput(props: FieldInputProps & { kind: string }) {
  // Blank rows are kept while editing and left out of the value.
  const [rows, setRows] = createSignal<(Json | undefined)[]>(asList(props.value));
  // What this editor last wrote. A different value came from elsewhere (a control bound to the
  // same field), and the rows start again from it.
  let written: Json | undefined = props.value;
  createEffect(
    on(
      () => props.value,
      (v) => {
        if (JSON.stringify(v ?? null) !== JSON.stringify(written ?? null)) setRows(asList(v));
        written = v;
      },
      { defer: true },
    ),
  );
  function update(next: (Json | undefined)[]) {
    setRows(next);
    const values = next.filter((v): v is Json => v !== undefined);
    written = values.length ? values : undefined;
    props.onChange(written);
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

/** An asset field: pick an uploaded file, or upload a new one into the field. */
function AssetInput(props: FieldInputProps) {
  const [assets, { refetch }] = createResource(() => api.assets());
  const [uploading, setUploading] = createSignal(false);
  const [uploadError, setUploadError] = createSignal<string | null>(null);
  const options = () => {
    const list = (assets() ?? []).map((a) => ({ value: a.id, text: a.filename }));
    // Keep a stored id the listing no longer has, so it is not silently dropped.
    for (const id of asStrings(props.value)) {
      if (!list.some((o) => o.value === id)) list.push({ value: id, text: id });
    }
    return list;
  };
  // The field's PDFs, each shown as a thumbnail that opens the viewer.
  const pdfs = () => {
    const byId = new Map((assets() ?? []).map((a) => [a.id, a]));
    return asStrings(props.value)
      .map((id) => byId.get(id))
      .filter((a): a is Asset => !!a && isPdfFile(a.filename));
  };
  async function uploadInto(file: File) {
    setUploading(true);
    setUploadError(null);
    try {
      const a = await upload(file);
      await refetch();
      props.onChange(props.field.multiple ? [...asStrings(props.value), a.id] : a.id);
    } catch (e) {
      setUploadError(`Could not upload ${file.name}: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setUploading(false);
    }
  }
  return (
    <div class="asset-input">
      <Show
        when={!props.field.multiple}
        fallback={
          <Picker
            {...props}
            suggest={(text) => optionSuggestions(options(), text)}
            chipText={(id) => options().find((o) => o.value === id)?.text ?? id}
          />
        }
      >
        <Choice
          options={options()}
          required={props.field.required}
          value={props.value}
          onChange={props.onChange}
          label={props.label}
          id={props.id}
        />
      </Show>
      <FilePicker
        label={`Upload a file to ${props.label}`}
        text={uploading() ? "Uploading…" : "Upload"}
        onFile={(file) => void uploadInto(file)}
      />
      <Show when={pdfs().length > 0}>
        <div class="pdf-thumbs" data-testid="asset-pdf-thumbs">
          <For each={pdfs()}>{(a) => <PdfThumbButton id={a.id} filename={a.filename} width={56} named />}</For>
        </div>
      </Show>
      <Show when={uploadError()}>
        {(e) => (
          <p class="error" role="alert">
            {e()}
          </p>
        )}
      </Show>
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
      <Match when={f().kind === "select" && f().multiple}>
        <Picker {...props} suggest={(text) => optionSuggestions((f().options ?? []).map((o) => ({ value: o, text: o })), text)} />
      </Match>
      <Match when={f().kind === "select"}>
        <Choice
          options={(f().options ?? []).map((o) => ({ value: o, text: o }))}
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
      <Match when={f().kind === "list" && (f().items ?? "string") === "string" && f().labels && props.related}>
        <LabelledInput {...props} related={props.related!} labelsField={f().labels!} />
      </Match>
      <Match when={f().kind === "list" && (f().items ?? "string") === "string"}>
        <Picker {...props} freeText suggest={() => []} />
      </Match>
      <Match when={f().kind === "list"}>
        <ListInput {...props} kind={f().items ?? "string"} />
      </Match>
      <Match when={f().kind === "dict"}>
        <DictInput {...props} kind={f().values ?? "string"} />
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
