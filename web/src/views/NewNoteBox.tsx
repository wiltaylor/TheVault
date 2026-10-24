// The inline input a query view with `allow_new` adds notes from: type a title, Enter adds the
// note and leaves the box ready for the next, Escape closes it. The note starts with the view's
// fields (`newNote.ts`) and whatever the place it was added at stands for.

import { type JSX, Show, createContext, createSignal, onMount, useContext } from "solid-js";
import type { Json, Note } from "../api";
import { typeDef } from "../session";
import { type NewNoteSpec, createFromView } from "./newNote";

/** The view's new-note spec, or null when the view does not add notes. */
export const NewNoteContext = createContext<() => NewNoteSpec | null>(() => null);

/** The spec of the query view around, or null. */
export function useNewNote(): () => NewNoteSpec | null {
  return useContext(NewNoteContext);
}

/** The lower-case label of a spec's type, as "task". */
export function newLabel(spec: NewNoteSpec): string {
  return (typeDef(spec.type)?.label ?? spec.type).toLowerCase();
}

export function NewNoteBox(props: {
  spec: NewNoteSpec;
  /** Fields the place the note is added at sets over the view's own. */
  extra?: () => Record<string, Json | undefined>;
  /** Close after one note, as a popover on a calendar day does; otherwise stay for the next. */
  once?: boolean;
  onCreated?: (note: Note) => void;
  onClose?: () => void;
  class?: string;
  style?: JSX.CSSProperties;
}) {
  const [text, setText] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  let input!: HTMLInputElement;
  onMount(() => input.focus());

  async function add() {
    const title = text().trim();
    if (!title || busy()) return;
    setBusy(true);
    setError(null);
    try {
      const note = await createFromView(props.spec, title, props.extra?.() ?? {});
      setText("");
      props.onCreated?.(note);
      if (props.once) props.onClose?.();
      else queueMicrotask(() => input.focus());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form
      class={`new-note-box ${props.class ?? ""}`}
      style={props.style}
      data-testid="new-note-box"
      onSubmit={(e) => {
        e.preventDefault();
        void add();
      }}
    >
      <input
        ref={input}
        value={text()}
        placeholder={`New ${newLabel(props.spec)}`}
        aria-label={`New ${newLabel(props.spec)} title`}
        disabled={busy()}
        onInput={(e) => setText(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            setText("");
            props.onClose?.();
          }
        }}
        onBlur={() => {
          if (!text().trim() && !busy()) props.onClose?.();
        }}
      />
      <button type="submit" disabled={busy() || !text().trim()}>
        Add
      </button>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
    </form>
  );
}

/** A "+ New <type>" button that opens a [`NewNoteBox`] in its place. */
export function NewNoteButton(props: {
  spec: NewNoteSpec;
  extra?: () => Record<string, Json | undefined>;
  class?: string;
  /** The button's text; "New <type>" by default. */
  label?: string;
}) {
  const [open, setOpen] = createSignal(false);
  let button: HTMLButtonElement | undefined;
  return (
    <Show
      when={open()}
      fallback={
        <button
          ref={button}
          type="button"
          class={`new-note-button ${props.class ?? ""}`}
          data-testid="new-note-button"
          onClick={() => setOpen(true)}
        >
          + {props.label ?? `New ${newLabel(props.spec)}`}
        </button>
      }
    >
      <NewNoteBox
        spec={props.spec}
        extra={props.extra}
        class={props.class}
        onClose={() => {
          setOpen(false);
          queueMicrotask(() => button?.focus());
        }}
      />
    </Show>
  );
}

/**
 * A [`NewNoteBox`] floating at `at`, in pixels from the top left of the positioned element that
 * holds it: where a calendar day, a point on a timeline or a spot on a map was picked. It adds
 * one note and closes.
 */
export function NewNotePopover(props: {
  spec: NewNoteSpec;
  at: { x: number; y: number };
  extra: () => Record<string, Json | undefined>;
  /** What was picked, said above the box, as "Tue 21 Oct". */
  caption?: string;
  onClose: () => void;
  onCreated?: (note: Note) => void;
}) {
  return (
    <div
      class="new-note-popover"
      role="dialog"
      aria-label={`New ${newLabel(props.spec)}`}
      style={{ left: `${props.at.x}px`, top: `${props.at.y}px` }}
    >
      <Show when={props.caption}>
        <p class="muted new-note-caption">{props.caption}</p>
      </Show>
      <NewNoteBox spec={props.spec} extra={props.extra} once onClose={props.onClose} onCreated={props.onCreated} />
    </div>
  );
}
