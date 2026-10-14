import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { type Row, type TypeDef, api } from "../api";
import { TypeIcon } from "../components/Icon";
import { readPickerQuery, recentTypes, typeOptions, typeUsed } from "../placeholders";
import { typeDef, types } from "../session";
import { useThisNote } from "../views/thisNote";
import { isUlid } from "./markup";
import type { SolidNodeViewProps } from "./solidNodeView";
import { knowTitle, noteType } from "./titles";

const DEBOUNCE_MS = 150;
const RESULTS = 8;

/** One choice in the picker: a note to mention, a type for a new placeholder, or "Choose type…". */
type Choice = { kind: "note"; row: Row } | { kind: "new"; type: TypeDef } | { kind: "types" };

/**
 * The mention picker: a search box inside the document, standing where the mention will go.
 * It lists the notes the text finds, then "New <type>" for a placeholder link titled with the
 * text: the type of the note being edited and the recently used types first, and "Choose type…"
 * for the rest, which `type:` in the text also narrows. Enter or a click takes the choice;
 * Escape, Backspace in the empty box, or leaving the box gives up. Given up, a picker opened by
 * typing `@` leaves the `@` and the text typed after it; one opened by `[[` leaves nothing.
 */
export function MentionPicker(props: SolidNodeViewProps) {
  let input!: HTMLInputElement;
  const trigger = (): string => (props.node().attrs.trigger === "@" ? "@" : "[[");
  const host = useThisNote();
  const [query, setQuery] = createSignal("");
  const [results, setResults] = createSignal<Row[]>([]);
  // The title the shown results were found for. "New <type>" waits for them, so the list does not
  // jump under the pointer and Enter on a note found never picks "New" instead.
  const [searched, setSearched] = createSignal<string | null>(null);
  const [active, setActive] = createSignal(0);
  const [error, setError] = createSignal<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let seq = 0;
  let done = false;

  onMount(() => queueMicrotask(() => input.focus()));
  onCleanup(() => clearTimeout(timer));

  const parsed = createMemo(() => readPickerQuery(query()));
  const newTypes = createMemo((): TypeDef[] => {
    const q = parsed();
    if (q.title === "" || searched() !== q.title) return [];
    const current = host ? noteType(host) : undefined;
    return typeOptions(types(), current, recentTypes(), q.typeFilter);
  });
  const choices = createMemo((): Choice[] => {
    const out: Choice[] = results().map((row) => ({ kind: "note", row }));
    for (const type of newTypes()) out.push({ kind: "new", type });
    if (parsed().title !== "" && parsed().typeFilter === null && types().length > newTypes().length) out.push({ kind: "types" });
    return out;
  });

  function search(text: string) {
    setQuery(text);
    setActive(0);
    clearTimeout(timer);
    const mine = ++seq;
    const q = readPickerQuery(text).title;
    if (q === "") {
      setResults([]);
      setSearched(null);
      return;
    }
    if (q === searched()) return;
    timer = setTimeout(async () => {
      try {
        const page = await api.search(q, null, RESULTS);
        if (mine !== seq) return;
        setResults(page.rows);
        setSearched(q);
        setActive(0);
        setError(null);
      } catch (e) {
        if (mine !== seq) return;
        setError(e instanceof Error ? e.message : String(e));
        setResults([]);
        setSearched(q);
      }
    }, DEBOUNCE_MS);
  }

  /**
   * Remove the picker and put the caret where it stood, after what replaced it: a mention, a
   * placeholder link, or for an abandoned `@` picker the text typed.
   */
  function finish(content?: { note: { id: string; title: string } } | { placeholder: { type: string; title: string } }) {
    if (done) return;
    done = true;
    const pos = props.getPos();
    const { editor } = props;
    const { mention, placeholderLink } = editor.schema.nodes;
    let size = 0;
    if (content && "note" in content && mention) {
      knowTitle(content.note.id, content.note.title);
      props.replaceWith(mention.create({ id: content.note.id, label: null }));
      size = 1;
    } else if (content && "placeholder" in content && placeholderLink) {
      typeUsed(content.placeholder.type);
      props.replaceWith(placeholderLink.create({ noteType: content.placeholder.type, title: content.placeholder.title }));
      size = 1;
    } else if (trigger() === "@") {
      const text = `@${query()}`;
      props.replaceWith(editor.schema.text(text));
      size = text.length;
    } else {
      props.replaceWith();
    }
    if (pos !== undefined) {
      // Focus now, not on TipTap's next frame: the box is gone, so keys typed before then are lost.
      editor.view.focus();
      editor.commands.setTextSelection(pos + size);
    }
  }

  function choose(i: number) {
    const c = choices()[i];
    const title = parsed().title;
    if (c?.kind === "note") finish({ note: { id: c.row.id, title: c.row.title } });
    else if (c?.kind === "new") finish({ placeholder: { type: c.type.name, title } });
    else if (c?.kind === "types") {
      // Narrow the types by typing after `type:`; the title stays as typed.
      const text = `${query().trimEnd()} type:`;
      setQuery(text);
      input.value = text;
      input.focus();
      setActive(results().length);
    } else if (isUlid(query().trim())) finish({ note: { id: query().trim(), title: query().trim() } });
  }

  function onKeyDown(e: KeyboardEvent) {
    const n = choices().length;
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
    } else if (e.key === "Backspace" && input.value === "") {
      e.preventDefault();
      // `@` and nothing after it: the `@` goes too, as Backspace would have taken it.
      if (trigger() === "@") {
        done = true;
        const pos = props.getPos();
        props.replaceWith(props.editor.schema.text("@"));
        props.editor.view.focus();
        if (pos !== undefined) props.editor.commands.setTextSelection(pos + 1);
      } else {
        finish();
      }
    }
  }

  function onFocusOut(e: FocusEvent) {
    const root = (e.currentTarget as HTMLElement) ?? null;
    if (root && e.relatedTarget instanceof Node && root.contains(e.relatedTarget)) return;
    // Leaving the box abandons the mention. Wait a tick so a click on a result lands first.
    setTimeout(() => {
      if (!done && !root?.contains(document.activeElement)) {
        if (trigger() === "@") {
          done = true;
          props.replaceWith(props.editor.schema.text(`@${query()}`));
        } else {
          done = true;
          props.replaceWith();
        }
      }
    }, 0);
  }

  const label = (name: string) => typeDef(name)?.label ?? name;

  return (
    <span class="mention-picker" data-testid="mention-picker" data-trigger={trigger()} onFocusOut={onFocusOut}>
      <span class="mention-picker-prefix">{trigger()}</span>
      <input
        ref={input}
        type="search"
        aria-label="Find a note to mention, or name a new one"
        role="combobox"
        aria-expanded={choices().length > 0}
        aria-controls="mention-results"
        aria-autocomplete="list"
        autocomplete="off"
        value={query()}
        onInput={(e) => search(e.currentTarget.value)}
        onKeyDown={onKeyDown}
      />
      <Show when={choices().length > 0 || error()}>
        <span class="mention-results" id="mention-results" role="listbox" data-node-view-interactive>
          <Show when={error()}>
            <span class="error">{error()}</span>
          </Show>
          <For each={choices()}>
            {(c, i) => (
              <span
                role="option"
                class="mention-result"
                classList={{ "mention-new": c.kind !== "note", "mention-new-first": c.kind !== "note" && choices()[i() - 1]?.kind === "note" }}
                aria-selected={active() === i()}
                data-testid={c.kind === "note" ? "mention-result" : c.kind === "new" ? "mention-new" : "mention-choose-type"}
                data-type={c.kind === "new" ? c.type.name : undefined}
                onMouseDown={(e) => {
                  e.preventDefault();
                  choose(i());
                }}
                onMouseEnter={() => setActive(i())}
              >
                <Show when={c.kind === "note" && c}>
                  {(n) => (
                    <>
                      <TypeIcon name={typeDef(n().row.type)?.icon} size={14} />
                      <span class="mention-result-title">{n().row.title}</span>
                      <span class="muted"> {label(n().row.type)}</span>
                    </>
                  )}
                </Show>
                <Show when={c.kind === "new" && c}>
                  {(n) => (
                    <>
                      <TypeIcon name={n().type.icon} size={14} />
                      <span class="mention-result-title">
                        New {n().type.label}: “{parsed().title}”
                      </span>
                    </>
                  )}
                </Show>
                <Show when={c.kind === "types"}>
                  <span class="mention-result-title muted">Choose type… </span>
                  <span class="muted mention-hint">
                    or type <kbd>type:</kbd>
                  </span>
                </Show>
              </span>
            )}
          </For>
        </span>
      </Show>
    </span>
  );
}
