import { Show, createEffect, createSignal, createUniqueId, onCleanup } from "solid-js";
import { api } from "../api";

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** How long typing pauses before the query is checked. */
const CHECK_DELAY_MS = 400;

/**
 * A query, edited as text in the query language. The text is checked with the server shortly
 * after typing stops, and a syntax error is shown under the box without changing the text.
 */
export function QueryEditor(props: { value: string; onChange: (text: string) => void }) {
  const [error, setError] = createSignal<string | null>(null);
  const id = createUniqueId();
  let seq = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  createEffect(() => {
    const text = props.value;
    const n = ++seq;
    clearTimeout(timer);
    if (text.trim() === "") {
      setError(null);
      return;
    }
    timer = setTimeout(() => {
      api.parseQuery(text).then(
        () => n === seq && setError(null),
        (e) => n === seq && setError(message(e)),
      );
    }, CHECK_DELAY_MS);
  });
  onCleanup(() => clearTimeout(timer));

  return (
    <div class="query-editor">
      <label class="query-editor-label" for={id}>
        Query
      </label>
      <textarea
        id={id}
        aria-invalid={error() !== null}
        rows={3}
        spellcheck={false}
        placeholder={'from task where status != "done" sort due'}
        value={props.value}
        onInput={(e) => props.onChange(e.currentTarget.value)}
      />
      <Show when={error()}>
        {(e) => (
          <p class="query-editor-error" role="alert">
            {e()}
          </p>
        )}
      </Show>
    </div>
  );
}
