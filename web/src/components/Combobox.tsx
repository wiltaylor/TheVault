// A text box with a list of suggestions under it (the ARIA combobox pattern). The chip picker's
// add box and a control's reference dropdown are both one of these.

import { For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { TypeIcon } from "./Icon";

/** A suggestion a combobox offers: the value it writes and the text it shows. */
export interface Suggestion {
  value: string;
  text: string;
  /** How deep in a hierarchy it sits, shown as indent. */
  depth?: number;
  /** A type icon's name, shown before the text; `null` shows the default icon. */
  icon?: string | null;
  /** A palette colour, shown as a dot before the text, such as the colour a term's chip takes. */
  colour?: string;
  /** Muted text after the text, such as a term's parent path. */
  hint?: string;
  /** Shown for context, such as a term already picked under which others sit, but not choosable. */
  disabled?: boolean;
  /** Choosing it puts this text in the box and keeps the list open, rather than taking it. */
  fill?: string;
}

/**
 * A text box with a list of suggestions under it, driven from the box with the arrow keys, Enter
 * and Escape. `choose` takes a suggestion, or the typed text when none is highlighted.
 */
export function Combobox(props: {
  id?: string;
  label: string;
  placeholder?: string;
  disabled?: boolean;
  describedBy?: string;
  /** What the box shows when it is not being typed in. */
  shown?: () => string;
  suggest: (text: string) => Promise<Suggestion[]> | Suggestion[];
  choose: (s: Suggestion | null, text: string) => void;
  onBackspaceEmpty?: () => void;
  /** Focus the box as soon as it shows. */
  autoFocus?: boolean;
  /** The box is done with: Escape with no list open, or focus leaving it empty. */
  onDone?: (how: "escape" | "blur") => void;
  /** Shown at the end of the box for the text in it, such as the type a search is limited to. */
  badge?: (text: string) => { icon: string | null; text: string } | null;
}) {
  const [open, setOpen] = createSignal(false);
  const [items, setItems] = createSignal<Suggestion[]>([]);
  const [active, setActive] = createSignal(-1);
  const [text, setText] = createSignal("");
  const listId = `${props.id ?? `combo-${++seq}`}-list`;
  let input!: HTMLInputElement;
  let run = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));
  function refresh(text: string) {
    const mine = ++run;
    clearTimeout(timer);
    timer = setTimeout(async () => {
      try {
        const list = await props.suggest(text);
        if (mine !== run) return;
        setItems(list);
        setActive(-1);
      } catch {
        if (mine === run) setItems([]);
      }
    }, 120);
  }
  function pick(s: Suggestion | null) {
    if (s?.fill !== undefined) {
      input.value = s.fill;
      setText(s.fill);
      input.focus();
      setOpen(true);
      refresh(s.fill);
      return;
    }
    props.choose(s, input.value);
    input.value = props.shown?.() ?? "";
    setText(input.value);
    setOpen(false);
    setItems([]);
  }
  createEffect(() => {
    const t = props.shown?.() ?? "";
    if (document.activeElement !== input) {
      input.value = t;
      setText(t);
    }
  });
  onMount(() => {
    if (props.autoFocus) input.focus();
  });
  return (
    <div class="control-combo" classList={{ "has-badge": !!props.badge?.(text()) }}>
      <input
        ref={input}
        id={props.id}
        role="combobox"
        aria-label={props.label}
        aria-autocomplete="list"
        aria-expanded={open() && items().length > 0}
        aria-controls={listId}
        aria-activedescendant={active() >= 0 ? `${listId}-${active()}` : undefined}
        aria-describedby={props.describedBy || undefined}
        placeholder={props.placeholder}
        disabled={props.disabled}
        autocomplete="off"
        onFocus={(e) => {
          if (props.shown) e.currentTarget.select();
          setOpen(true);
          refresh(props.shown ? "" : e.currentTarget.value);
        }}
        onInput={(e) => {
          setText(e.currentTarget.value);
          setOpen(true);
          refresh(e.currentTarget.value);
        }}
        onBlur={() => {
          setOpen(false);
          input.value = props.shown?.() ?? input.value;
          setText(input.value);
          if (props.onDone && input.value.trim() === "") props.onDone("blur");
        }}
        onKeyDown={(e) => {
          const n = items().length;
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            setOpen(true);
            const step = e.key === "ArrowDown" ? 1 : -1;
            let i = active() < 0 && step < 0 ? n : active();
            for (let tries = 0; tries < n; tries++) {
              i = (i + step + n) % n;
              if (!items()[i]?.disabled) {
                setActive(i);
                break;
              }
            }
          } else if (e.key === "Enter") {
            e.preventDefault();
            pick(open() && active() >= 0 ? (items()[active()] ?? null) : null);
          } else if (e.key === "Escape") {
            // Only a list that is showing takes the Escape; otherwise the box is done with.
            if (open() && items().length > 0) {
              e.preventDefault();
              e.stopPropagation();
              setOpen(false);
            } else if (props.onDone) {
              e.preventDefault();
              e.stopPropagation();
              props.onDone("escape");
            }
          } else if (e.key === "Backspace" && input.value === "" && props.onBackspaceEmpty) {
            props.onBackspaceEmpty();
          }
        }}
      />
      <Show when={props.badge?.(text())}>
        {(b) => (
          <span class="combo-badge" data-testid="combo-badge">
            <TypeIcon name={b().icon} size={12} />
            <span>{b().text}</span>
          </span>
        )}
      </Show>
      <Show when={open() && items().length > 0}>
        <ul class="control-options" role="listbox" id={listId} aria-label={`${props.label} suggestions`} data-node-view-interactive>
          <For each={items()}>
            {(s, i) => (
              <li
                role="option"
                id={`${listId}-${i()}`}
                aria-selected={i() === active()}
                aria-disabled={s.disabled || undefined}
                classList={{ disabled: !!s.disabled }}
                style={{ "padding-left": `${0.6 + (s.depth ?? 0) * 0.9}rem` }}
                onMouseDown={(e) => e.preventDefault()}
                onMouseMove={() => !s.disabled && setActive(i())}
                onClick={() => !s.disabled && pick(s)}
              >
                <Show when={s.icon !== undefined}>
                  <TypeIcon name={s.icon} size={14} />
                </Show>
                <Show when={s.colour}>
                  <span class="combo-dot" data-colour={s.colour} aria-hidden="true" />
                </Show>
                <span class="combo-text">{s.text}</span>
                <Show when={s.hint}>
                  <span class="combo-hint">{s.hint}</span>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}

let seq = 0;
