// The one way to pick several values, or one from many: the values as chips with no box round
// them, and a round + after them that opens a combobox to add another. A chip's × removes it,
// and Backspace in the empty box removes the last one.

import { For, Show, createSignal } from "solid-js";
import type { Term, TypeDef } from "../api";
import { termColour } from "../defs/taxonomyModel";
import { Combobox, type Suggestion } from "./Combobox";
import { TypeIcon } from "./Icon";

export type { Suggestion } from "./Combobox";

export interface ChipPickerProps {
  /** What is being picked; the + and its box are "Add to <label>". */
  label: string;
  values: string[];
  /** A value's chip text; the value itself by default. */
  chipText?: (value: string) => string;
  /** A type icon for a value's chip; `undefined` shows none. */
  chipIcon?: (value: string) => string | null | undefined;
  /** A palette colour for a value's chip, such as its term's; `undefined` uses the hashed colour. */
  chipColour?: (value: string) => string | undefined;
  /** Where a value's chip links to, if anywhere. */
  chipHref?: (value: string) => string | undefined;
  /** Suggestions for the typed text. Values already picked are left out of what it returns. */
  suggest: (text: string) => Promise<Suggestion[]> | Suggestion[];
  /** Typed text with no suggestion picked becomes a value; otherwise it must match a suggestion. */
  freeText?: boolean;
  /** At most one value: the + hides while it is set, and its chip's × clears it. */
  single?: boolean;
  disabled?: boolean;
  onChange: (values: string[]) => void;
  /**
   * Typed text that matches no suggestion may become a new value, offered as an "Add “text”"
   * suggestion after the others: the value it adds, or why it cannot be one.
   */
  create?: (text: string) => { value: string } | { error: string };
  /** Typed text that is not free text and matches no suggestion, with why when `create` said. */
  onReject?: (text: string, reason?: string) => void;
  /** The id of the + and of the box that replaces it, so a label can point at them. */
  id?: string;
  /** Label the group by this element rather than by `label`. */
  labelledBy?: string;
  describedBy?: string;
  placeholder?: string;
  /** Muted text beside the + while there are no values. */
  emptyText?: string;
  class?: string;
}

/** Whether a pointer is pressed anywhere, so a box can wait for the press to end before closing. */
let pointerDown = false;
if (typeof window !== "undefined") {
  window.addEventListener("pointerdown", () => (pointerDown = true), { capture: true });
  for (const end of ["pointerup", "pointercancel"]) window.addEventListener(end, () => (pointerDown = false), { capture: true });
}

/** Marks the suggestion that adds the typed text as a new value. No real value starts with it. */
const ADD = "\u0000add:";

const CHIP_COLOURS = ["blue", "green", "purple", "orange", "pink", "yellow", "red", "brown"] as const;

/** A palette colour for a value, the same every time the value is shown. Gray is left out, as too dull. */
export function chipColour(value: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < value.length; i++) h = Math.imul(h ^ value.charCodeAt(i), 0x01000193);
  h = Math.imul(h ^ (h >>> 16), 0x45d9f3b);
  h ^= h >>> 16;
  return CHIP_COLOURS[(h >>> 0) % CHIP_COLOURS.length] ?? "blue";
}

/** A chip list with a + to add to it. */
export function ChipPicker(props: ChipPickerProps) {
  const text = (v: string) => props.chipText?.(v) ?? v;
  const [adding, setAdding] = createSignal(false);
  let root!: HTMLDivElement;
  let plus: HTMLButtonElement | undefined;
  const full = () => props.single === true && props.values.length > 0;

  function add(v: string) {
    const t = v.trim();
    if (t === "" || props.values.includes(t)) return;
    props.onChange(props.single ? [t] : [...props.values, t]);
    if (props.single) {
      setAdding(false);
      queueMicrotask(() => root.querySelector<HTMLElement>(".chip button, .chip a")?.focus());
    }
  }
  function remove(v: string) {
    props.onChange(props.values.filter((x) => x !== v));
  }
  /** Whether a value or its text is `typed`, without case. */
  const same = (typed: string, value: string, shown: string) => {
    const lower = typed.trim().toLowerCase();
    return value.toLowerCase() === lower || shown.toLowerCase() === lower;
  };
  async function suggest(q: string): Promise<Suggestion[]> {
    const have = new Set(props.values);
    const list = await props.suggest(q);
    // A value already picked is not offered again. In a hierarchy it still shows, for the
    // terms under it, when any follow it.
    const out = list.flatMap((s, i) => {
      if (!have.has(s.value)) return [s];
      const next = list[i + 1];
      const parent = s.depth !== undefined && next?.depth !== undefined && next.depth > s.depth;
      return parent ? [{ ...s, disabled: true }] : [];
    });
    // Text that names nothing offered or picked can be added, last.
    const t = q.trim();
    if (props.create && t && !list.some((s) => same(t, s.value, s.text)) && !props.values.some((v) => same(t, v, text(v)))) {
      // It shows the colour its chip will have, once it is a value.
      const made = props.create(t);
      const value = "value" in made ? made.value : undefined;
      out.push({ value: ADD + t, text: `Add “${t}”`, colour: value && (props.chipColour?.(value) ?? chipColour(value)) });
    }
    return out;
  }
  function create(t: string) {
    const made = props.create?.(t);
    if (!made) return props.onReject?.(t);
    if ("error" in made) return props.onReject?.(t, made.error);
    add(made.value);
  }
  async function choose(s: Suggestion | null, typed: string) {
    if (s?.value.startsWith(ADD)) return create(s.value.slice(ADD.length));
    if (s) return add(s.value);
    const t = typed.trim();
    if (!t) return;
    if (props.freeText) return add(t);
    // Only a suggestion's value or text, matched without case, is taken.
    if (props.values.some((v) => same(t, v, text(v)))) return;
    let hit: Suggestion | undefined;
    try {
      hit = (await suggest(t)).find((x) => !x.disabled && !x.value.startsWith(ADD) && same(t, x.value, x.text));
    } catch {
      hit = undefined;
    }
    if (hit) add(hit.value);
    else if (props.create) create(t);
    else props.onReject?.(t);
  }
  function done(how: "escape" | "blur") {
    if (how === "escape") {
      setAdding(false);
      queueMicrotask(() => plus?.focus());
      return;
    }
    // Focus left the empty box. A press that took it is let finish first, so the + taking the
    // box's place does not move what is being clicked out from under the pointer.
    const close = () => {
      if (!root.querySelector("[role=combobox]")?.matches(":focus")) setAdding(false);
    };
    if (!pointerDown) return close();
    const ended = () => {
      window.removeEventListener("pointerup", ended, true);
      window.removeEventListener("pointercancel", ended, true);
      setTimeout(close);
    };
    window.addEventListener("pointerup", ended, true);
    window.addEventListener("pointercancel", ended, true);
  }

  return (
    <div
      ref={root}
      class={`chip-picker${props.class ? ` ${props.class}` : ""}`}
      role="group"
      aria-label={props.labelledBy ? undefined : props.label}
      aria-labelledby={props.labelledBy}
      aria-describedby={props.describedBy || undefined}
    >
      <ul class="chips" aria-label={`${props.label} values`}>
        <For each={props.values}>
          {(v) => (
            <li class="chip" data-value={v} data-colour={props.chipColour?.(v) ?? chipColour(v)}>
              <Show when={props.chipIcon && props.chipIcon(v) !== undefined}>
                <TypeIcon name={props.chipIcon!(v)} size={14} />
              </Show>
              <Show when={props.chipHref?.(v)} fallback={<span class="chip-text">{text(v)}</span>}>
                {(href) => (
                  <a class="chip-text" href={href()}>
                    {text(v)}
                  </a>
                )}
              </Show>
              <Show when={!props.disabled}>
                <button type="button" aria-label={`Remove ${text(v)}`} onClick={() => remove(v)}>
                  ×
                </button>
              </Show>
            </li>
          )}
        </For>
        <Show when={props.values.length === 0 && props.disabled}>
          <li class="muted">—</li>
        </Show>
      </ul>
      <Show when={!props.disabled && !full()}>
        <Show
          when={adding()}
          fallback={
            <>
              <button
                type="button"
                ref={plus}
                id={props.id}
                class="chip-add"
                aria-label={`Add to ${props.label}`}
                title={`Add to ${props.label}`}
                onClick={() => setAdding(true)}
              >
                +
              </button>
              <Show when={props.values.length === 0 && props.emptyText}>
                <span class="chip-empty" aria-hidden="true" onClick={() => setAdding(true)}>
                  {props.emptyText}
                </span>
              </Show>
            </>
          }
        >
          <Combobox
            id={props.id}
            label={`Add to ${props.label}`}
            placeholder={props.placeholder ?? "Add…"}
            disabled={props.disabled}
            suggest={suggest}
            choose={(s, t) => void choose(s, t)}
            autoFocus
            onDone={done}
            onBackspaceEmpty={() => {
              const last = props.values.at(-1);
              if (last !== undefined) remove(last);
            }}
          />
        </Show>
      </Show>
    </div>
  );
}

/** Text matches when it is empty or found, without case, in any of `parts`. */
export function matches(text: string, ...parts: (string | null | undefined)[]): boolean {
  const t = text.trim().toLowerCase();
  return t === "" || parts.some((p) => p?.toLowerCase().includes(t));
}

/**
 * A taxonomy's terms as suggestions: parents first, indented by depth, matched by name or path.
 * While filtering, a nested term says which parent it is under. Each carries the colour its chip
 * will have: the term's own or inherited colour, else the hashed one.
 */
export function termSuggestions(terms: Term[], text: string): Suggestion[] {
  const out: Suggestion[] = [];
  const walk = (list: Term[], depth: number) => {
    for (const t of list) {
      if (matches(text, t.name, t.path)) {
        const parent = t.path.includes("/") ? t.path.slice(0, t.path.lastIndexOf("/")) : undefined;
        out.push({
          value: t.path,
          text: t.name,
          depth,
          hint: text.trim() && parent ? parent : undefined,
          colour: termChipColour(terms, t.path),
        });
      }
      walk(t.children, depth + 1);
    }
  };
  walk(terms, 0);
  return out;
}

/**
 * The colour of a term's chip: its own or inherited colour, else the hashed colour of its path.
 * A term not saved yet inherits from its nearest saved ancestor.
 */
export function termChipColour(terms: Term[], path: string): string {
  for (let p = path; ; p = p.slice(0, p.lastIndexOf("/"))) {
    const colour = termColour(terms, p);
    if (colour) return colour;
    if (!p.includes("/")) return chipColour(path);
  }
}

/**
 * The term path typed text names, for an open taxonomy: an existing term's, matched name by name
 * without case, or a new term's under an existing parent. `Plant/Solar` adds Solar under Plant.
 */
export function newTermPath(terms: Term[], text: string): { value: string } | { error: string } {
  const names = text.split("/").map((n) => n.trim());
  if (names.some((n) => !n)) return { error: "A term name cannot be empty." };
  let level = terms;
  const path: string[] = [];
  for (const [i, name] of names.entries()) {
    const lower = name.toLowerCase();
    const t = level.find((x) => x.name === name) ?? level.find((x) => x.name.toLowerCase() === lower);
    if (!t) {
      if (i === names.length - 1) return { value: [...path, name].join("/") };
      const parent = [...path, name].join("/");
      return { error: `There is no term “${parent}” to add “${names.at(-1)}” under.` };
    }
    path.push(t.name);
    level = t.children;
  }
  return { value: path.join("/") };
}

/** Every type as a suggestion with its icon, matched by name, label or plural. */
export function typeSuggestions(types: TypeDef[], text: string): Suggestion[] {
  return types
    .filter((t) => matches(text, t.name, t.label, t.plural))
    .map((t) => ({ value: t.name, text: t.label, icon: t.icon }));
}
