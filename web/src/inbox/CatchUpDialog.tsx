// The inbox's one-time catch-up (TV-30): pick a date, see how many emails, messages and calls
// sent or made before it are not processed yet, and mark them all processed. The home server does
// the work in chunks; the apps do not offer it (see crates/api/src/inbox.rs).

import { Show, createEffect, createResource, createSignal } from "solid-js";
import { type CatchUp, api } from "../api";
import { localDateTime } from "../capture/model";
import { errorText } from "../pages/TypeEditor";

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** Today's date in the browser's time zone, as `YYYY-MM-DD`. */
export function today(now: Date = new Date()): string {
  return localDateTime(now).slice(0, 10);
}

/** The instant a `YYYY-MM-DD` date starts in the browser's time zone, as an RFC 3339 UTC string. */
export function startOf(date: string): string | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  if (!m) return null;
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])).toISOString();
}

/** What a dry run found, in words. */
export function summary(c: CatchUp, date: string): string {
  if (c.total === 0) return `Nothing sent or made before ${date} waits to be processed.`;
  const parts = [
    c.matched.email ? plural(c.matched.email, "email", "emails") : null,
    c.matched.message ? plural(c.matched.message, "message", "messages") : null,
    c.matched.call ? plural(c.matched.call, "call", "calls") : null,
  ].filter((p): p is string => p !== null);
  return `${plural(c.total, "item", "items")} sent or made before ${date} will be marked processed: ${parts.join(", ")}.`;
}

export function CatchUpDialog(props: { onCancel: () => void; onDone: (changed: number) => void }) {
  let dialog!: HTMLDivElement;
  const [date, setDate] = createSignal(today());
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [count] = createResource(
    () => startOf(date()),
    (before) => api.markProcessedBefore(before, true),
  );
  // A resource's value throws once it failed, so read it only when it did not.
  const found = (): CatchUp | undefined => (count.error ? undefined : count.latest);
  const counted = () => {
    const f = found();
    return f ? summary(f, date()) : "";
  };
  createEffect(() => dialog?.querySelector<HTMLInputElement>("input[type=date]")?.focus());

  async function confirm() {
    const before = startOf(date());
    if (!before) return;
    setBusy(true);
    setError(null);
    try {
      const done = await api.markProcessedBefore(before, false);
      props.onDone(done.changed);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      class="capture-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && !busy()) props.onCancel();
      }}
    >
      <div
        ref={dialog}
        class="capture-dialog inbox-catch-up"
        role="dialog"
        aria-modal="true"
        aria-labelledby="catch-up-heading"
        onKeyDown={(e) => {
          if (e.key === "Escape" && !busy()) {
            e.preventDefault();
            props.onCancel();
          }
        }}
      >
        <header class="capture-head">
          <h2 id="catch-up-heading">Mark older items processed</h2>
        </header>
        <p>Every email, message and call sent or made before this date is marked processed and leaves the inbox. Captures stay. Nothing is sent to Gmail. No model calls or agent runs are started by this action.</p>
        <label class="inbox-catch-up-date">
          Before
          <input type="date" value={date()} max={today()} onInput={(e) => setDate(e.currentTarget.value)} />
        </label>
        <p class="inbox-catch-up-count" role="status">
          <Show when={!count.loading} fallback="Counting…">
            <Show when={count.error} fallback={counted()}>
              {errorText(count.error)}
            </Show>
          </Show>
        </p>
        <Show when={error()}>
          <p class="error" role="alert">
            {error()}
          </p>
        </Show>
        <div class="actions">
          <button type="button" disabled={busy()} onClick={props.onCancel}>
            Cancel
          </button>
          <button
            type="button"
            class="primary"
            disabled={busy() || count.loading || (found()?.total ?? 0) === 0}
            onClick={() => void confirm()}
          >
            {busy() ? "Marking…" : `Mark ${plural(found()?.total ?? 0, "item", "items")} processed`}
          </button>
        </div>
      </div>
    </div>
  );
}
