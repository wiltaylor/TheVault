import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { type Row, api } from "../api";
import { isUlid } from "./markup";
import type { SolidNodeViewProps } from "./solidNodeView";
import { knowTitle } from "./titles";

const DEBOUNCE_MS = 150;
const RESULTS = 8;

/**
 * The mention picker: a search box inside the document, standing where the mention will go.
 * Enter or a click turns the chosen note into a mention; Escape, or leaving the box, removes it.
 */
export function MentionPicker(props: SolidNodeViewProps) {
  let input!: HTMLInputElement;
  const [query, setQuery] = createSignal("");
  const [results, setResults] = createSignal<Row[]>([]);
  const [active, setActive] = createSignal(0);
  const [error, setError] = createSignal<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let seq = 0;
  let done = false;

  onMount(() => queueMicrotask(() => input.focus()));
  onCleanup(() => clearTimeout(timer));

  function search(q: string) {
    setQuery(q);
    clearTimeout(timer);
    const mine = ++seq;
    if (q.trim() === "") {
      setResults([]);
      return;
    }
    timer = setTimeout(async () => {
      try {
        const page = await api.search(q, null, RESULTS);
        if (mine !== seq) return;
        setResults(page.rows);
        setActive(0);
        setError(null);
      } catch (e) {
        if (mine === seq) setError(e instanceof Error ? e.message : String(e));
      }
    }, DEBOUNCE_MS);
  }

  /** Remove the picker and put the caret where it stood, after `content` when there is one. */
  function finish(content?: { id: string; title: string }) {
    if (done) return;
    done = true;
    const pos = props.getPos();
    const { editor } = props;
    const mention = editor.schema.nodes.mention;
    if (content && mention) {
      knowTitle(content.id, content.title);
      props.replaceWith(mention.create({ id: content.id, label: null }));
    } else {
      props.replaceWith();
    }
    if (pos !== undefined) {
      // Focus now, not on TipTap's next frame: the box is gone, so keys typed before then are lost.
      editor.view.focus();
      editor.commands.setTextSelection(pos + (content ? 1 : 0));
    }
  }

  function choose(i: number) {
    const row = results()[i];
    if (row) finish({ id: row.id, title: row.title });
    else if (isUlid(query().trim())) finish({ id: query().trim(), title: query().trim() });
  }

  function onKeyDown(e: KeyboardEvent) {
    const n = results().length;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (n) setActive((active() + 1) % n);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (n) setActive((active() + n - 1) % n);
    } else if (e.key === "Enter") {
      e.preventDefault();
      choose(active());
    } else if (e.key === "Escape") {
      e.preventDefault();
      finish();
    }
  }

  function onFocusOut(e: FocusEvent) {
    const root = (e.currentTarget as HTMLElement) ?? null;
    if (root && e.relatedTarget instanceof Node && root.contains(e.relatedTarget)) return;
    // Leaving the box abandons the mention. Wait a tick so a click on a result lands first.
    setTimeout(() => {
      if (!done && !root?.contains(document.activeElement)) {
        done = true;
        props.replaceWith();
      }
    }, 0);
  }

  return (
    <span class="mention-picker" data-testid="mention-picker" onFocusOut={onFocusOut}>
      <span class="mention-picker-prefix">[[</span>
      <input
        ref={input}
        type="search"
        aria-label="Find a note to mention"
        role="combobox"
        aria-expanded={results().length > 0}
        aria-controls="mention-results"
        aria-autocomplete="list"
        autocomplete="off"
        value={query()}
        onInput={(e) => search(e.currentTarget.value)}
        onKeyDown={onKeyDown}
      />
      <Show when={results().length > 0 || error()}>
        <span class="mention-results" id="mention-results" role="listbox" data-node-view-interactive>
          <Show when={error()}>
            <span class="error">{error()}</span>
          </Show>
          <For each={results()}>
            {(row, i) => (
              <span
                role="option"
                class="mention-result"
                aria-selected={active() === i()}
                onMouseDown={(e) => {
                  e.preventDefault();
                  choose(i());
                }}
              >
                {row.title}
                <span class="muted"> {row.type}</span>
              </span>
            )}
          </For>
        </span>
      </Show>
    </span>
  );
}
