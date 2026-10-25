// A `control` block on the page: a form control showing one field of the note and writing it
// through the note's own edit path (docs/vault-format.md, "control"). Every kind is keyboard
// accessible and labelled; a computed field, or a control with no note to edit, is read-only.

import { For, type JSX, Match, Show, Switch, createEffect, createMemo, createSignal, on, untrack } from "solid-js";
import { type Block, type FieldDef, type Json, type Row, api } from "../api";
import { ChipPicker, matches } from "../components/ChipPicker";
import { termPicker } from "../components/termPicker";
import { Combobox, type Suggestion } from "../components/Combobox";
import { TIME, datetimeInput, datetimeValue, findNotes, knownLabels, termOptions, timeValue } from "../components/FieldInput";
import { LabelledList, type LabelledShape, labelled, labelledShape, labelledValue } from "../components/LabelledList";
import { optionChoices, optionText } from "../options";
import { noteTitle } from "../editor/titles";
import { TITLE_FIELD } from "../notes";
import { taxonomyDef, typeDef } from "../session";
import { useControlBinding } from "./binding";
import {
  CONTROL_INFO,
  type ControlSpec,
  addMonths,
  calendarPick,
  chipsOf,
  clampNumber,
  controlFits,
  controlOf,
  dayState,
  fieldKindText,
  isoDate,
  monthGrid,
  parseNumber,
  rangeOf,
  ratingKey,
  ratingMax,
  ratingValue,
  refusal,
  sameDayMonths,
  addDays,
  stepNumber,
  timeOf,
  withTime,
} from "./model";

let seq = 0;

/** What every control kind's input is given. */
interface Part {
  spec: ControlSpec;
  field: FieldDef;
  value: () => Json | undefined;
  /** Write a new value; refused (and explained) when it would empty a required field. */
  commit: (v: Json | undefined) => void;
  /** Say why a typed value is not written, or clear that with null. */
  complain: (message: string | null) => void;
  disabled: boolean;
  id: string;
  labelId: string;
  describedBy: string;
  label: string;
  placeholder: string | undefined;
}

/** The kinds whose control is a group of several inputs, labelled by its heading. */
const GROUPS = new Set(["toggle", "radio", "chips", "calendar", "rating"]);

/** A `control` block, bound to the note around it. */
export function ControlBlock(props: { block: Block }) {
  const binding = useControlBinding();
  const spec = () => controlOf(props.block);
  const field = (): FieldDef | undefined => {
    const s = spec();
    if (!s) return undefined;
    if (s.field === "title") return TITLE_FIELD;
    return binding?.type()?.fields.find((f) => f.name === s.field);
  };
  return (
    <Show
      when={spec()}
      fallback={
        <div class="control control-broken" data-testid="control">
          <p class="muted">A control that names no field or kind.</p>
        </div>
      }
    >
      {(s) => (
        <Show
          when={binding && field()}
          fallback={
            <ControlShell spec={s()} label={s().label ?? s().field} note={binding ? `The type has no field ${s().field}.` : "Not on a note: there is no note for this control to edit."}>
              {(p) => <KindInput {...p} field={{ ...TITLE_FIELD, name: s().field, label: s().field, required: false }} kindOk={false} />}
            </ControlShell>
          }
        >
          {(f) => <BoundControl spec={s()} field={f()} />}
        </Show>
      )}
    </Show>
  );
}

function BoundControl(props: { spec: ControlSpec; field: FieldDef }) {
  const binding = useControlBinding()!;
  const fits = () => controlFits(props.spec.kind, props.field);
  const reason = (): string | null => {
    if (!fits()) return `A ${CONTROL_INFO[props.spec.kind].label.toLowerCase()} does not fit ${props.field.label}, a ${fieldKindText(props.field)} field.`;
    if (props.field.computed) return "Computed: only the type's handlers write this field.";
    if (!binding.set) return binding.reason ?? "Read-only here.";
    return null;
  };
  return (
    <ControlShell
      spec={props.spec}
      label={props.spec.label ?? props.field.label}
      field={props.field}
      note={reason()}
      value={() => binding.value(props.field)}
      write={binding.set && fits() && !props.field.computed ? (v) => binding.set!(props.field, v) : undefined}
    >
      {(p) => <KindInput {...p} kindOk={fits()} />}
    </ControlShell>
  );
}

/**
 * The frame every control shares: its label, the input, why it is read-only and why a change was
 * not written. `write` absent makes it read-only.
 */
function ControlShell(props: {
  spec: ControlSpec;
  label: string;
  field?: FieldDef;
  note: string | null;
  value?: () => Json | undefined;
  write?: (v: Json | undefined) => void;
  children: (p: Omit<Part, "field"> & { field: FieldDef }) => JSX.Element;
}) {
  const n = ++seq;
  const id = `control-${n}`;
  const labelId = `${id}-label`;
  const noteId = `${id}-note`;
  const errorId = `${id}-error`;
  const [error, setError] = createSignal<string | null>(null);
  const group = GROUPS.has(props.spec.kind);
  const value = () => props.value?.();
  const commit = (v: Json | undefined) => {
    const write = props.write;
    if (!write || !props.field) return;
    const refused = refusal(props.field, props.label, v);
    setError(refused);
    if (refused) return;
    write(v);
  };
  // One object for the life of the control, so its input is never made again: what changes is
  // read through getters.
  const part = {
    get spec() {
      return props.spec;
    },
    get field() {
      return props.field ?? TITLE_FIELD;
    },
    value,
    commit,
    complain: setError,
    get disabled() {
      return !props.write;
    },
    id,
    labelId,
    // An id that names no element is ignored, so both are always listed.
    describedBy: `${noteId} ${errorId}`,
    get label() {
      return props.label;
    },
    get placeholder() {
      return props.spec.placeholder ?? undefined;
    },
  };
  return (
    <div
      class={`control control-kind-${props.spec.kind}`}
      classList={{ "control-readonly": !props.write }}
      data-testid="control"
      data-kind={props.spec.kind}
      data-field={props.spec.field}
    >
      <div class="control-label">
        <Show when={group} fallback={<label id={labelId} for={id}>{props.label}</label>}>
          <span id={labelId}>{props.label}</span>
        </Show>
        <Show when={props.field?.required && props.write}>
          <span class="control-required" title="Required">
            required
          </span>
        </Show>
        <Show when={props.field?.computed}>
          <span class="badge" title="Only handlers write this field">
            computed
          </span>
        </Show>
      </div>
      <div class="control-input" data-node-view-interactive>{untrack(() => props.children(part))}</div>
      <Show when={props.note}>
        <p class="control-note" id={noteId}>
          {props.note}
        </p>
      </Show>
      <Show when={error()}>
        <p class="control-error" id={errorId} role="alert">
          {error()}
        </p>
      </Show>
    </div>
  );
}

function KindInput(props: Part & { kindOk: boolean }) {
  const kind = () => props.spec.kind;
  const f = () => props.field;
  return (
    <Switch>
      <Match when={!props.kindOk}>
        <input id={props.id} disabled aria-describedby={props.describedBy || undefined} value={display(props.value())} />
      </Match>
      <Match when={kind() === "text"}>
        <TextInput {...props} />
      </Match>
      <Match when={kind() === "textarea"}>
        <TextInput {...props} multiline />
      </Match>
      <Match when={kind() === "number"}>
        <NumberInput {...props} />
      </Match>
      <Match when={kind() === "slider"}>
        <SliderInput {...props} />
      </Match>
      <Match when={kind() === "toggle"}>
        <button
          type="button"
          role="switch"
          class="control-switch"
          id={props.id}
          aria-checked={props.value() === true}
          aria-labelledby={props.labelId}
          aria-describedby={props.describedBy || undefined}
          disabled={props.disabled}
          onClick={() => props.commit(!(props.value() === true))}
        >
          <span class="control-switch-thumb" aria-hidden="true" />
          <span class="control-switch-text">{props.value() === true ? "On" : "Off"}</span>
        </button>
      </Match>
      <Match when={kind() === "checkbox"}>
        <input
          type="checkbox"
          id={props.id}
          checked={props.value() === true}
          disabled={props.disabled}
          aria-describedby={props.describedBy || undefined}
          onChange={(e) => props.commit(e.currentTarget.checked)}
        />
      </Match>
      <Match when={kind() === "dropdown" && f().kind === "reference"}>
        <ReferenceDropdown {...props} />
      </Match>
      <Match when={kind() === "dropdown"}>
        <SelectInput {...props} />
      </Match>
      <Match when={kind() === "radio"}>
        <RadioInput {...props} />
      </Match>
      <Match when={kind() === "chips" && labelledShape(f())}>{(shape) => <LabelledChips {...props} shape={shape()} />}</Match>
      <Match when={kind() === "chips"}>
        <ChipsInput {...props} />
      </Match>
      <Match when={kind() === "calendar"}>
        <CalendarInput {...props} />
      </Match>
      <Match when={kind() === "datetime"}>
        <input
          type="datetime-local"
          step="1"
          id={props.id}
          value={datetimeInput(props.value())}
          disabled={props.disabled}
          aria-describedby={props.describedBy || undefined}
          onChange={(e) => props.commit(e.currentTarget.value === "" ? undefined : datetimeValue(e.currentTarget.value, props.value()))}
        />
      </Match>
      <Match when={kind() === "time"}>
        <TimeInput {...props} />
      </Match>
      <Match when={kind() === "rating"}>
        <RatingInput {...props} />
      </Match>
    </Switch>
  );
}

/** A value as plain text, for a control that cannot show it any other way. */
function display(v: Json | undefined): string {
  if (v === undefined || v === null) return "";
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}

/**
 * Keep `el` showing `value` as `format` writes it, except while the person is typing in it and
 * what they typed already reads as that value: `1.` stays as typed while the field holds 1.
 */
function follow(el: HTMLInputElement | HTMLTextAreaElement, value: () => Json | undefined, format: (v: Json | undefined) => string, same: (raw: string, v: Json | undefined) => boolean) {
  createEffect(
    on(value, (v) => {
      if (document.activeElement === el && same(el.value, v)) return;
      el.value = format(v);
    }),
  );
}

function TextInput(props: Part & { multiline?: boolean }) {
  const f = () => props.field;
  const numeric = () => f().kind === "int" || f().kind === "float";
  const int = () => f().kind === "int";
  const format = (v: Json | undefined) => (v === undefined || v === null ? "" : typeof v === "object" ? JSON.stringify(v) : String(v));
  const same = (raw: string, v: Json | undefined) => (numeric() ? parseNumber(raw, int()) === (v ?? undefined) : raw === (v ?? ""));
  function onInput(raw: string) {
    if (numeric()) {
      const n = parseNumber(raw, int());
      if (n === null) {
        props.complain(int() ? "Enter a whole number." : "Enter a number.");
        return;
      }
      props.commit(n);
    } else {
      props.commit(raw === "" ? undefined : raw);
    }
  }
  // Leaving the box shows the field's value again, whatever was refused while typing.
  const onBlur = (el: HTMLInputElement | HTMLTextAreaElement) => {
    el.value = format(props.value());
  };
  return (
    <Show
      when={props.multiline}
      fallback={
        <input
          ref={(el) => follow(el, props.value, format, same)}
          id={props.id}
          type={f().kind === "url" ? "url" : "text"}
          inputmode={numeric() ? (int() ? "numeric" : "decimal") : undefined}
          placeholder={props.placeholder}
          required={f().required}
          disabled={props.disabled}
          aria-describedby={props.describedBy || undefined}
          autocomplete="off"
          onInput={(e) => onInput(e.currentTarget.value)}
          onBlur={(e) => onBlur(e.currentTarget)}
        />
      }
    >
      <textarea
        ref={(el) => follow(el, props.value, format, same)}
        id={props.id}
        rows={3}
        placeholder={props.placeholder}
        required={f().required}
        disabled={props.disabled}
        aria-describedby={props.describedBy || undefined}
        onInput={(e) => onInput(e.currentTarget.value)}
        onBlur={(e) => onBlur(e.currentTarget)}
      />
    </Show>
  );
}

function NumberInput(props: Part) {
  const int = () => props.field.kind === "int";
  const s = () => props.spec;
  const format = (v: Json | undefined) => (typeof v === "number" ? String(v) : "");
  const same = (raw: string, v: Json | undefined) => parseNumber(raw, int()) === (v ?? undefined);
  const atMin = () => typeof props.value() === "number" && s().min !== undefined && (props.value() as number) <= s().min!;
  const atMax = () => typeof props.value() === "number" && s().max !== undefined && (props.value() as number) >= s().max!;
  const step = (dir: 1 | -1) => props.commit(stepNumber(props.value(), dir, s(), int()));
  return (
    <div class="control-stepper">
      <button type="button" aria-label={`Decrease ${props.label}`} disabled={props.disabled || atMin()} onClick={() => step(-1)}>
        −
      </button>
      <input
        ref={(el) => follow(el, props.value, format, same)}
        id={props.id}
        type="number"
        min={s().min}
        max={s().max}
        step={s().step ?? (int() ? 1 : "any")}
        placeholder={props.placeholder}
        required={props.field.required}
        disabled={props.disabled}
        aria-describedby={props.describedBy || undefined}
        onInput={(e) => {
          const raw = e.currentTarget.value;
          const n = parseNumber(raw, int());
          if (n === null) return props.complain(int() ? "Enter a whole number." : "Enter a number.");
          if (n !== undefined && clampNumber(n, s().min, s().max) !== n) {
            return props.complain(`Enter a value from ${s().min ?? "any"} to ${s().max ?? "any"}.`);
          }
          props.commit(n);
        }}
        onBlur={(e) => (e.currentTarget.value = format(props.value()))}
      />
      <button type="button" aria-label={`Increase ${props.label}`} disabled={props.disabled || atMax()} onClick={() => step(1)}>
        +
      </button>
    </div>
  );
}

function SliderInput(props: Part) {
  const s = () => props.spec;
  const min = () => s().min ?? 0;
  const max = () => s().max ?? 100;
  const has = () => typeof props.value() === "number";
  const now = () => (has() ? (props.value() as number) : min());
  return (
    <div class="control-slider" classList={{ empty: !has() }}>
      <input
        type="range"
        id={props.id}
        min={min()}
        max={max()}
        step={s().step ?? 1}
        value={now()}
        aria-valuemin={min()}
        aria-valuemax={max()}
        aria-valuenow={has() ? now() : undefined}
        aria-valuetext={has() ? String(now()) : "No value"}
        disabled={props.disabled}
        aria-describedby={props.describedBy || undefined}
        style={{ "--fill": `${((now() - min()) / (max() - min() || 1)) * 100}%` }}
        onInput={(e) => {
          const n = Number(e.currentTarget.value);
          props.commit(props.field.kind === "int" ? Math.round(n) : n);
        }}
      />
      {/* Not an <output>: that is a live status region, and the slider speaks its own value. */}
      <span class="control-slider-value" aria-hidden="true">
        {has() ? String(now()) : "—"}
      </span>
    </div>
  );
}

function SelectInput(props: Part) {
  const options = createMemo(() => {
    const f = props.field;
    if (f.kind === "taxonomy") return termOptions(taxonomyDef(f.taxonomy ?? "")?.terms ?? []);
    return optionChoices(f).map((o) => ({ ...o, depth: 0 }));
  });
  return (
    <select
      id={props.id}
      disabled={props.disabled}
      aria-describedby={props.describedBy || undefined}
      required={props.field.required}
      onChange={(e) => {
        const el = e.currentTarget;
        props.commit(el.value === "" ? undefined : el.value);
        // A refused choice shows the field's value again.
        el.value = typeof props.value() === "string" ? (props.value() as string) : "";
      }}
    >
      <option value="" selected={props.value() === undefined}>
        {props.placeholder ?? "—"}
      </option>
      <For each={options()}>
        {(o) => (
          <option value={o.value} selected={props.value() === o.value}>
            {"\u00a0\u00a0\u00a0".repeat(o.depth) + (o.depth ? "› " : "") + o.text}
          </option>
        )}
      </For>
    </select>
  );
}

function RadioInput(props: Part) {
  return (
    <div class="control-radios" role="radiogroup" aria-labelledby={props.labelId} aria-describedby={props.describedBy || undefined} id={props.id}>
      <For each={props.field.options ?? []}>
        {(o) => (
          <label class="control-radio">
            <input
              type="radio"
              name={props.id}
              value={o}
              checked={props.value() === o}
              disabled={props.disabled}
              onChange={(e) => e.currentTarget.checked && props.commit(o)}
            />
            <span>{optionText(props.field, o)}</span>
          </label>
        )}
      </For>
    </div>
  );
}

const titleOf = (id: string) => noteTitle(id) ?? id;

function ReferenceDropdown(props: Part) {
  const id = () => (typeof props.value() === "string" ? (props.value() as string) : "");
  return (
    <div class="control-reference">
      <Combobox
        id={props.id}
        label={props.label}
        placeholder={props.placeholder ?? (props.field.target && props.field.target !== "*" ? `Find a ${(typeDef(props.field.target)?.label ?? props.field.target).toLowerCase()}` : "Search notes")}
        disabled={props.disabled}
        describedBy={props.describedBy}
        shown={() => (id() ? titleOf(id()) : "")}
        suggest={async (text) => (await findNotes(props.field.target ?? "*", text)).map((r: Row) => ({ value: r.id, text: r.title || "Untitled" }))}
        choose={(s) => {
          if (s) props.commit(s.value);
        }}
      />
      <Show when={id()}>
        <a class="control-open" href={`/notes/${id()}`} aria-label={`Open ${titleOf(id())}`} title="Open the note">
          ↗
        </a>
        <Show when={!props.disabled}>
          <button type="button" class="control-clear" aria-label={`Clear ${props.label}`} onClick={() => props.commit(undefined)}>
            ×
          </button>
        </Show>
      </Show>
    </div>
  );
}

/** Chips for a list of labelled values: each value with its label. */
function LabelledChips(props: Part & { shape: LabelledShape }) {
  const binding = useControlBinding();
  const l = () => labelled(props.value(), props.shape.key);
  return (
    <LabelledList
      class="control-chips"
      id={props.id}
      label={props.label}
      labelledBy={props.labelId}
      describedBy={props.describedBy}
      values={l().values}
      labels={l().labels}
      disabled={props.disabled}
      knownLabels={() => knownLabels(binding?.type()?.name, props.field.name)}
      onChange={(next) => props.commit(labelledValue(next, props.shape))}
    />
  );
}

function ChipsInput(props: Part) {
  const f = () => props.field;
  const text = (v: string) => {
    if (f().kind === "reference") return titleOf(v);
    if (f().kind === "select") return optionText(f(), v);
    return v;
  };
  // Values others hold in a list field, offered as suggestions; fetched once.
  let known: Promise<string[]> | null = null;
  const binding = useControlBinding();
  function listValues(): Promise<string[]> {
    const type = binding?.type()?.name;
    if (!type) return Promise.resolve([]);
    known ??= api
      .query(`from ${type} select ${f().name}`, null, 200)
      .then((page) => [...new Set(page.rows.flatMap((r) => chipsOf(r.fields[f().name])))].sort())
      .catch(() => []);
    return known;
  }
  const terms = termPicker(() => f().taxonomy ?? "");
  async function suggest(q: string): Promise<Suggestion[]> {
    switch (f().kind) {
      case "select":
        return optionChoices(f()).filter((o) => matches(q, o.text, o.value));
      case "taxonomy":
        return terms.suggest(q);
      case "reference":
        return (await findNotes(f().target ?? "*", q)).map((r) => ({ value: r.id, text: r.title || "Untitled" }));
      default:
        return (await listValues())
          .filter((v) => matches(q, v))
          .slice(0, 20)
          .map((v) => ({ value: v, text: v }));
    }
  }
  return (
    <ChipPicker
      class="control-chips"
      id={props.id}
      label={props.label}
      labelledBy={props.labelId}
      describedBy={props.describedBy}
      values={chipsOf(props.value())}
      chipText={text}
      chipColour={f().kind === "taxonomy" ? terms.chipColour : undefined}
      suggest={suggest}
      freeText={f().kind === "list"}
      create={f().kind === "taxonomy" ? terms.create() : undefined}
      disabled={props.disabled}
      placeholder={props.placeholder ?? "Add…"}
      onChange={(values) => props.commit(values.length ? (f().kind === "taxonomy" ? terms.picked(values) : values) : undefined)}
      onReject={(t, reason) =>
        props.complain(reason ?? (f().kind === "reference" ? "Choose a note from the list." : `${t} is not one of the choices.`))
      }
    />
  );
}

const MONTH = new Intl.DateTimeFormat(undefined, { month: "long", year: "numeric" });
const LONG = new Intl.DateTimeFormat(undefined, { weekday: "long", day: "numeric", month: "long", year: "numeric" });
const SHORT = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", year: "numeric" });
const WEEKDAYS = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
const WEEKDAY_NAMES = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

const dateOf = (iso: string) => {
  const [y, m, d] = iso.split("-").map(Number) as [number, number, number];
  return new Date(y, m - 1, d);
};

function CalendarInput(props: Part) {
  const range = () => props.field.kind === "date_range";
  const today = isoDate(new Date());
  const startOf = (v: Json | undefined) => (typeof v === "string" ? v : (rangeOf(v)?.start ?? today));
  // The day that has the keyboard focus, which also picks the month shown.
  const [focus, setFocus] = createSignal(/^\d{4}-\d\d-\d\d$/.test(startOf(props.value())) ? startOf(props.value()) : today);
  const [pending, setPending] = createSignal<string | null>(null);
  const month = () => addMonths(focus(), 0);
  const weeks = createMemo(() => monthGrid(month()));
  let grid!: HTMLTableElement;
  // A value written elsewhere moves the calendar to it.
  createEffect(
    on(
      props.value,
      (v) => {
        if (!grid?.contains(document.activeElement)) setFocus(startOf(v));
      },
      { defer: true },
    ),
  );
  function moveTo(day: string) {
    setFocus(day);
    queueMicrotask(() => grid?.querySelector<HTMLButtonElement>(`button[data-day="${day}"]`)?.focus());
  }
  function pick(day: string) {
    setFocus(day);
    const r = calendarPick(range(), pending(), day);
    setPending(r.pending);
    if (r.write) props.commit(r.value);
  }
  function onKey(e: KeyboardEvent) {
    const d = focus();
    const to: Record<string, () => string> = {
      ArrowLeft: () => addDays(d, -1),
      ArrowRight: () => addDays(d, 1),
      ArrowUp: () => addDays(d, -7),
      ArrowDown: () => addDays(d, 7),
      Home: () => addDays(d, -((dateOf(d).getDay() + 6) % 7)),
      End: () => addDays(d, 6 - ((dateOf(d).getDay() + 6) % 7)),
      PageUp: () => sameDayMonths(d, e.shiftKey ? -12 : -1),
      PageDown: () => sameDayMonths(d, e.shiftKey ? 12 : 1),
    };
    const next = to[e.key];
    if (next) {
      e.preventDefault();
      moveTo(next());
    } else if (e.key === "Escape" && pending()) {
      e.preventDefault();
      e.stopPropagation();
      setPending(null);
    }
  }
  const status = () => {
    if (pending()) return `From ${SHORT.format(dateOf(pending()!))}: pick the end date.`;
    const v = props.value();
    if (typeof v === "string") return SHORT.format(dateOf(v));
    const r = rangeOf(v);
    if (r) return `${SHORT.format(dateOf(r.start))} to ${SHORT.format(dateOf(r.end))}`;
    return range() ? "No dates: pick the start, then the end." : "No date";
  };
  return (
    <div class="control-calendar" role="group" aria-labelledby={props.labelId} aria-describedby={props.describedBy || undefined} id={props.id}>
      <div class="calendar-head">
        <button type="button" aria-label="Previous month" disabled={props.disabled} onClick={() => setFocus(sameDayMonths(focus(), -1))}>
          ‹
        </button>
        <span class="calendar-month" id={`${props.id}-month`} aria-live="polite">
          {MONTH.format(dateOf(month()))}
        </span>
        <button type="button" aria-label="Next month" disabled={props.disabled} onClick={() => setFocus(sameDayMonths(focus(), 1))}>
          ›
        </button>
      </div>
      <table ref={grid} class="calendar-grid" role="grid" aria-labelledby={`${props.id}-month`} onKeyDown={onKey}>
        <thead>
          <tr>
            <For each={WEEKDAYS}>
              {(d, i) => (
                <th scope="col" abbr={WEEKDAY_NAMES[i()]}>
                  {d}
                </th>
              )}
            </For>
          </tr>
        </thead>
        <tbody>
          <For each={weeks()}>
            {(week) => (
              <tr>
                <For each={week}>
                  {(day) => {
                    const state = () => dayState(props.value(), pending(), day);
                    return (
                      <td role="gridcell" aria-selected={state() !== null && state() !== "inside"}>
                        <button
                          type="button"
                          data-day={day}
                          class="calendar-day"
                          classList={{
                            outside: day.slice(0, 7) !== month().slice(0, 7),
                            today: day === today,
                            [`day-${state()}`]: state() !== null,
                          }}
                          tabindex={day === focus() ? 0 : -1}
                          aria-label={LONG.format(dateOf(day))}
                          aria-pressed={state() !== null && state() !== "inside"}
                          disabled={props.disabled}
                          onClick={() => pick(day)}
                        >
                          {Number(day.slice(8))}
                        </button>
                      </td>
                    );
                  }}
                </For>
              </tr>
            )}
          </For>
        </tbody>
      </table>
      <div class="calendar-foot">
        <span class="calendar-status" aria-live="polite">
          {status()}
        </span>
        <Show when={!props.disabled && (props.value() !== undefined || pending())}>
          <button
            type="button"
            class="calendar-clear"
            onClick={() => {
              setPending(null);
              props.commit(undefined);
            }}
          >
            Clear
          </button>
        </Show>
      </div>
    </div>
  );
}

/**
 * A time box. On a `time` field it holds the time of day as stored, `HH:MM` or `HH:MM:SS`, with
 * a seconds box only when the time has seconds; on a datetime it edits the time of the date.
 */
function TimeInput(props: Part) {
  const clock = () => props.field.kind === "time";
  const stored = () => {
    const v = props.value();
    return typeof v === "string" && TIME.test(v) ? v : "";
  };
  const date = () => {
    const v = props.value();
    return typeof v === "string" && /^\d{4}-\d\d-\d\d/.test(v) ? SHORT.format(dateOf(v.slice(0, 10))) : "today";
  };
  return (
    <div class="control-time">
      <input
        type="time"
        id={props.id}
        value={clock() ? stored() : timeOf(props.value())}
        step={clock() && stored().length === 8 ? "1" : undefined}
        disabled={props.disabled}
        aria-describedby={props.describedBy || undefined}
        required={props.field.required}
        onChange={(e) => {
          const raw = e.currentTarget.value;
          if (clock()) {
            props.commit(raw === "" ? undefined : timeValue(raw));
            e.currentTarget.value = stored();
            return;
          }
          props.commit(raw === "" ? undefined : withTime(props.value(), raw));
          e.currentTarget.value = timeOf(props.value());
        }}
      />
      <Show when={!clock()}>
        <span class="control-time-date muted">on {date()}</span>
      </Show>
    </div>
  );
}

function RatingInput(props: Part) {
  const max = () => ratingMax(props.spec);
  const current = () => (typeof props.value() === "number" ? (props.value() as number) : 0);
  const [hover, setHover] = createSignal(0);
  let group!: HTMLDivElement;
  const stars = () => Array.from({ length: max() }, (_, i) => i + 1);
  function set(n: number | undefined) {
    props.commit(n);
    if (n !== undefined) queueMicrotask(() => group?.querySelector<HTMLButtonElement>(`button[data-star="${n}"]`)?.focus());
  }
  return (
    <div
      ref={group}
      class="control-rating"
      role="radiogroup"
      aria-labelledby={props.labelId}
      aria-describedby={props.describedBy || undefined}
      id={props.id}
      onMouseLeave={() => setHover(0)}
      onKeyDown={(e) => {
        const n = ratingKey(props.value(), e.key, max());
        if (n !== null) {
          e.preventDefault();
          set(n);
        }
      }}
    >
      <For each={stars()}>
        {(n) => (
          <button
            type="button"
            role="radio"
            data-star={n}
            class="control-star"
            classList={{ on: n <= (hover() || current()), preview: hover() > 0 }}
            aria-checked={current() === n}
            aria-label={`${n} star${n === 1 ? "" : "s"}`}
            tabindex={(current() || 1) === n ? 0 : -1}
            disabled={props.disabled}
            onMouseEnter={() => !props.disabled && setHover(n)}
            onClick={() => set(ratingValue(props.value(), n))}
          >
            ★
          </button>
        )}
      </For>
      <span class="control-rating-text">{current() ? `${current()} of ${max()}` : "Not rated"}</span>
    </div>
  );
}
