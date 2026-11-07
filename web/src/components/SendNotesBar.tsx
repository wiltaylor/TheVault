// Send notes from a list (TV-77 D7, TV-79): the Inbox and Search rows can be selected, one by one
// or all on the page at once, and the bar sends the selected notes to an agent through the Send to
// agent dialog, in one request and one thread (TV-77 Q1 B). The bar says how many notes one send
// takes before it is sent, and a send of more is not offered. The selection is cleared once sent.

import { type Accessor, Show, createEffect, createMemo, createSignal } from "solid-js";
import { MAX_NOTES, SendToAgentDialog } from "./SendToAgent";

export interface Selection {
  /** The notes of the page that are selected, in the page's order. */
  selected: Accessor<string[]>;
  has: (id: string) => boolean;
  toggle: (id: string, on: boolean) => void;
  /** Select every note on the page, or none. */
  all: (on: boolean) => void;
  clear: () => void;
}

/** The selected notes of a list whose page shows `page`. A note no longer shown is not sent. */
export function createSelection(page: Accessor<readonly string[]>): Selection {
  const [picked, setPicked] = createSignal<ReadonlySet<string>>(new Set());
  const selected = createMemo(() => page().filter((id) => picked().has(id)));
  return {
    selected,
    has: (id) => picked().has(id),
    toggle: (id, on) =>
      setPicked((s) => {
        const next = new Set(s);
        if (on) next.add(id);
        else next.delete(id);
        return next;
      }),
    all: (on) => setPicked(on ? new Set(page()) : new Set<string>()),
    clear: () => setPicked(new Set<string>()),
  };
}

const notesText = (n: number) => (n === 1 ? "1 note" : `${n} notes`);

/** A row's checkbox. */
export function SelectBox(props: { selection: Selection; id: string; title: string }) {
  return (
    <input
      type="checkbox"
      class="select-note"
      aria-label={`Select ${props.title || "Untitled"}`}
      checked={props.selection.has(props.id)}
      onChange={(e) => props.selection.toggle(props.id, e.currentTarget.checked)}
    />
  );
}

/** "Select all on this page", and with notes selected, how many and "Send N notes to agent". */
export function SendNotesBar(props: { selection: Selection; page: Accessor<readonly string[]> }) {
  const count = () => props.selection.selected().length;
  const all = () => props.page().length > 0 && count() === props.page().length;
  const over = () => count() > MAX_NOTES;
  // The notes the open dialog sends, kept as they were when it opened.
  const [sending, setSending] = createSignal<string[] | null>(null);
  let box!: HTMLInputElement;
  createEffect(() => {
    box.indeterminate = count() > 0 && !all();
  });
  return (
    <div class="send-notes-bar" role="toolbar" aria-label="Selected notes" data-testid="send-notes-bar">
      <label class="send-notes-all">
        <input
          ref={box}
          type="checkbox"
          checked={all()}
          disabled={props.page().length === 0}
          onChange={(e) => props.selection.all(e.currentTarget.checked)}
        />
        Select all on this page
      </label>
      <Show when={count() > 0}>
        <span class="send-notes-count" aria-live="polite">
          {count()} selected
        </span>
        <button type="button" class="link" onClick={() => props.selection.clear()}>
          Clear
        </button>
        <span class="muted send-notes-cap" classList={{ error: over() }} role={over() ? "alert" : undefined}>
          {over() ? `One send takes at most ${MAX_NOTES} notes: select fewer.` : `Up to ${MAX_NOTES} notes at once`}
        </span>
        <button type="button" class="primary send-notes-send" disabled={over()} onClick={() => setSending([...props.selection.selected()])}>
          Send {notesText(count())} to agent
        </button>
      </Show>
      <Show when={sending()}>
        {(notes) => <SendToAgentDialog notes={notes()} onClose={() => setSending(null)} onSent={() => props.selection.clear()} />}
      </Show>
    </div>
  );
}
