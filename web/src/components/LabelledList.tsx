// A list of labelled values, such as a person's phones: a list whose items are dicts of a value
// key and a `label` key, both text (`[{ number: "+61400000020", label: "mobile" }]`). One chip per
// value with its label before it, each with Copy, Edit and Remove, and a + that adds a value with
// its label. Every change is by index, so duplicate values are kept apart and a label never moves
// to another value.

import { For, Show, createSignal, onCleanup } from "solid-js";
import type { Json } from "../api";
import type { LabelledShape } from "../labelled";
import { copyText } from "../clipboard";
import { chipColour } from "./ChipPicker";

export { type LabelledShape, labelledShape } from "../labelled";

/** The labels offered first, before those already used in the vault. */
export const LABEL_SUGGESTIONS = ["mobile", "home", "work", "main", "other"];

/** A list's values and their labels, index-aligned: `labels` as long as `values`. */
export interface Labelled {
  values: string[];
  labels: string[];
}

const isRecord = (v: Json | undefined): v is Record<string, Json> => !!v && typeof v === "object" && !Array.isArray(v);
const text = (v: Json | undefined): string => (v === undefined || v === null ? "" : typeof v === "string" ? v : String(v));

/** A stored list's values and labels: each item's `key` and `label`; a bare item is a value with no label. */
export function labelled(value: Json | undefined, key: string): Labelled {
  const items = Array.isArray(value) ? value : value === undefined || value === null ? [] : [value];
  return {
    values: items.map((x) => (isRecord(x) ? text(x[key]) : text(x))),
    labels: items.map((x) => (isRecord(x) ? text(x.label) : "")),
  };
}

/** Value `i` and its label set to `value` and `label`. */
export function editAt(l: Labelled, i: number, value: string, label: string): Labelled {
  return {
    values: l.values.map((v, j) => (j === i ? value : v)),
    labels: l.labels.map((x, j) => (j === i ? label : x)),
  };
}

/** `value` with `label` added after the others. */
export function addTo(l: Labelled, value: string, label: string): Labelled {
  return { values: [...l.values, value], labels: [...l.labels, label] };
}

/** Value `i` and its label removed. */
export function removeAt(l: Labelled, i: number): Labelled {
  return { values: l.values.filter((_, j) => j !== i), labels: l.labels.filter((_, j) => j !== i) };
}

/**
 * The list as it is stored: one record per value, with `label` left out where a value has none
 * (unless the shape requires it), and nothing at all for an empty list.
 */
export function labelledValue(l: Labelled, shape: LabelledShape): Json | undefined {
  if (!l.values.length) return undefined;
  return l.values.map((v, i) => {
    const label = l.labels[i] ?? "";
    return label || shape.labelRequired ? { [shape.key]: v, label } : { [shape.key]: v };
  });
}

/** Read-only text: "mobile +61400000020, home +61400000021". */
export function labelledText(l: Labelled): string {
  return l.values.map((v, i) => (l.labels[i] ? `${l.labels[i]} ${v}` : v)).join(", ");
}

/** Lucide's copy icon, drawn inline so a chip never waits on the icon set. */
function CopyIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <rect width="14" height="14" x="8" y="8" rx="2" ry="2" />
      <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />
    </svg>
  );
}

/** Lucide's pencil icon. */
function EditIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z" />
      <path d="m15 5 4 4" />
    </svg>
  );
}

let seq = 0;

export interface LabelledListProps {
  /** The field's label, such as "Phones". */
  label: string;
  values: string[];
  /** One per value; "" for a value with no label. */
  labels: string[];
  /** Labels already used for this field in the vault, offered after the usual ones. */
  knownLabels?: () => Promise<string[]> | string[];
  disabled?: boolean;
  id?: string;
  labelledBy?: string;
  describedBy?: string;
  /** Muted text beside the + while there are no values. */
  emptyText?: string;
  class?: string;
  onChange: (next: Labelled) => void;
}

/** What the popover edits: value `index`, or a new one. */
type Editing = { index: number | null };

/** Labelled values as chips, with Copy, Edit and Remove on each and a + to add one. */
export function LabelledList(props: LabelledListProps) {
  const uid = `labelled-${++seq}`;
  const [editing, setEditing] = createSignal<Editing | null>(null);
  const [copied, setCopied] = createSignal<number | null>(null);
  const [suggestions, setSuggestions] = createSignal<string[]>(LABEL_SUGGESTIONS);
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));
  let root!: HTMLDivElement;
  let plus: HTMLButtonElement | undefined;
  let labelInput: HTMLInputElement | undefined;
  let valueInput: HTMLInputElement | undefined;
  /** What gets focus back when the popover closes. */
  let returnTo: HTMLElement | null = null;

  const current = (): Labelled => ({ values: props.values, labels: props.values.map((_, i) => props.labels[i] ?? "") });
  const shown = (i: number) => (current().labels[i] ? `${current().labels[i]} ${props.values[i]}` : props.values[i]!);

  async function loadSuggestions() {
    let known: string[] = [];
    try {
      known = (await props.knownLabels?.()) ?? [];
    } catch {
      known = [];
    }
    const used = current().labels.filter(Boolean);
    const all = [...LABEL_SUGGESTIONS, ...known, ...used];
    setSuggestions([...new Set(all.map((l) => l.trim()).filter(Boolean))]);
  }

  function open(index: number | null, from: HTMLElement | null) {
    if (props.disabled) return;
    returnTo = from;
    setEditing({ index });
    void loadSuggestions();
    queueMicrotask(() => {
      if (labelInput) labelInput.value = index === null ? "" : (current().labels[index] ?? "");
      if (valueInput) valueInput.value = index === null ? "" : (props.values[index] ?? "");
      valueInput?.focus();
      valueInput?.select();
    });
  }

  function close() {
    setEditing(null);
    const back = returnTo?.isConnected ? returnTo : plus;
    queueMicrotask(() => back?.focus());
  }

  function save() {
    const e = editing();
    if (!e) return;
    const value = (valueInput?.value ?? "").trim();
    const label = (labelInput?.value ?? "").trim();
    if (!value) {
      valueInput?.focus();
      return;
    }
    const next = e.index === null ? addTo(current(), value, label) : editAt(current(), e.index, value, label);
    // The chip that was edited may be replaced; focus goes back to the + or the list.
    returnTo = e.index === null ? plus ?? null : null;
    setEditing(null);
    props.onChange(next);
    queueMicrotask(() => {
      const target = e.index === null ? plus : root.querySelectorAll<HTMLElement>(".labelled-chip .chip-text")[e.index];
      target?.focus();
    });
  }

  function remove(i: number) {
    setEditing(null);
    props.onChange(removeAt(current(), i));
    queueMicrotask(() => plus?.focus());
  }

  async function copy(i: number) {
    const ok = await copyText(props.values[i] ?? "");
    clearTimeout(timer);
    setCopied(ok ? i : null);
    timer = setTimeout(() => setCopied(null), 1500);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      save();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  }

  return (
    <div
      ref={root}
      class={`chip-picker labelled-list${props.class ? ` ${props.class}` : ""}`}
      role="group"
      aria-label={props.labelledBy ? undefined : props.label}
      aria-labelledby={props.labelledBy}
      aria-describedby={props.describedBy || undefined}
    >
      <ul class="chips" aria-label={`${props.label} values`}>
        <For each={props.values}>
          {(v, i) => (
            <li
              class="chip labelled-chip"
              data-value={v}
              data-label={current().labels[i()] || undefined}
              data-colour={current().labels[i()] ? chipColour(current().labels[i()]!) : "gray"}
            >
              <button
                type="button"
                class="chip-text"
                aria-label={shown(i())}
                aria-haspopup={props.disabled ? undefined : "dialog"}
                title={props.disabled ? undefined : "Edit"}
                disabled={props.disabled}
                onClick={(e) => open(i(), e.currentTarget)}
              >
                <Show when={current().labels[i()]}>
                  <span class="chip-label">{current().labels[i()]}</span>
                </Show>
                <span class="chip-value">{v}</span>
              </button>
              <span class="chip-actions">
                <button
                  type="button"
                  class="chip-action"
                  aria-label={copied() === i() ? `Copied ${v}` : `Copy ${shown(i())}`}
                  title={copied() === i() ? "Copied" : "Copy"}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => void copy(i())}
                >
                  <Show when={copied() === i()} fallback={<CopyIcon />}>
                    <span class="chip-copied">Copied</span>
                  </Show>
                </button>
                <Show when={!props.disabled}>
                  <button
                    type="button"
                    class="chip-action"
                    aria-label={`Edit ${shown(i())}`}
                    title="Edit"
                    onClick={(e) => open(i(), e.currentTarget.closest("li")?.querySelector<HTMLElement>(".chip-text") ?? null)}
                  >
                    <EditIcon />
                  </button>
                  <button type="button" class="chip-action chip-remove" aria-label={`Remove ${shown(i())}`} title="Remove" onClick={() => remove(i())}>
                    ×
                  </button>
                </Show>
              </span>
            </li>
          )}
        </For>
        <Show when={props.values.length === 0 && props.disabled}>
          <li class="muted">—</li>
        </Show>
      </ul>
      <Show when={!props.disabled}>
        <button
          type="button"
          ref={plus}
          id={props.id}
          class="chip-add"
          aria-label={`Add to ${props.label}`}
          title={`Add to ${props.label}`}
          aria-expanded={editing()?.index === null}
          onClick={(e) => open(null, e.currentTarget)}
        >
          +
        </button>
        <Show when={props.values.length === 0 && props.emptyText && !editing()}>
          <span class="chip-empty" aria-hidden="true" onClick={() => open(null, plus ?? null)}>
            {props.emptyText}
          </span>
        </Show>
      </Show>
      <span class="visually-hidden" role="status">
        {copied() !== null ? "Copied" : ""}
      </span>
      <Show when={editing()}>
        {(e) => (
          <div
            class="labelled-popover"
            role="dialog"
            aria-label={e().index === null ? `Add to ${props.label}` : `Edit ${shown(e().index!)}`}
            onKeyDown={onKey}
            onFocusOut={(ev) => {
              const to = ev.relatedTarget as Node | null;
              if (to && !ev.currentTarget.contains(to)) close();
            }}
          >
            <label class="labelled-popover-field">
              <span>Label</span>
              <input
                ref={labelInput}
                class="labelled-label-input"
                list={`${uid}-labels`}
                autocomplete="off"
                placeholder="mobile"
                aria-label="Label"
              />
            </label>
            <datalist id={`${uid}-labels`}>
              <For each={suggestions()}>{(s) => <option value={s} />}</For>
            </datalist>
            <label class="labelled-popover-field">
              <span>Value</span>
              <input ref={valueInput} class="labelled-value-input" autocomplete="off" aria-label="Value" />
            </label>
            <div class="labelled-popover-actions">
              <Show when={e().index !== null}>
                <button type="button" onClick={() => void copy(e().index!)}>
                  {copied() === e().index ? "Copied" : "Copy"}
                </button>
                <button type="button" class="danger" onClick={() => remove(e().index!)}>
                  Remove
                </button>
              </Show>
              <span class="labelled-popover-spacer" />
              <button type="button" onClick={close}>
                Cancel
              </button>
              <button type="button" class="primary" onClick={save}>
                Save
              </button>
            </div>
          </div>
        )}
      </Show>
    </div>
  );
}
