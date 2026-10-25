import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { agentCallHref, newUlid } from "../agentCalls";
import { AGENT_ICON, agentLabel, agents, callableAgents } from "../agents";
import type { AgentSummary, Row, TypeDef } from "../api";
import { TypeIcon } from "../components/Icon";
import { readPickerQuery, recentTypes, typeOptions, typeUsed } from "../placeholders";
import { typeDef, types } from "../session";
import { filterText, readTypeFilter, searchNotes } from "../typeFilter";
import { useThisNote } from "../views/thisNote";
import { isUlid } from "./markup";
import type { Node as PMNode } from "@tiptap/pm/model";
import type { SolidNodeViewProps } from "./solidNodeView";
import { knowTitle, noteType } from "./titles";

const DEBOUNCE_MS = 150;
const RESULTS = 8;

/** The type filter and title a search was made for, to tell whether its results are the text's. */
const searchKey = (type: string | null, title: string) => `${type ?? ""}/${title}`;

/**
 * One choice in the picker: an agent to call, a note to mention, a type for a new placeholder,
 * or "Choose type…".
 */
type Choice =
  | { kind: "filter"; type: TypeDef }
  | { kind: "agent"; agent: AgentSummary }
  | { kind: "note"; row: Row }
  | { kind: "new"; type: TypeDef }
  | { kind: "types" };

/**
 * The mention picker: a search box inside the document, standing where the mention will go.
 * Opened by typing `@`, it lists first "Ask <Label>" for each agent the note's type may call
 * (agents G16) whose name or label starts with the text. Choosing one turns the box into the
 * request box, "Ask <Label>…" (agents G19): Enter with a request commits the call as one link,
 * `[@<Label> <request>](agent:<name>?call=<ULID>)`, which the note's next save carries whole;
 * Shift+Enter adds a line; Escape, Enter with nothing written, or leaving the box leaves
 * nothing. Then it lists the notes the text finds, then "New <type>" for a placeholder link titled with the text: the type of the note
 * being edited and the recently used types first, and "Choose type…" for the rest, which `type:`
 * in the text also narrows. Enter or a click takes the choice; Escape, Backspace in the empty
 * box, or leaving the box gives up. Given up, a picker opened by typing `@` leaves the `@` and
 * the text typed after it; one opened by `[[` leaves nothing.
 *
 * `<type>/<text>` looks for the text among one type's notes only, and `<type>/` lists its most
 * recently changed; the type shows beside the box, and "New" offers only that type. While the
 * text could still be the start of a type, those types are offered first, as "filter by type",
 * and choosing one types `<type>/`. Deleting the `/` takes the filter away again.
 */
export function MentionPicker(props: SolidNodeViewProps) {
  let input!: HTMLInputElement;
  let requestBox: HTMLTextAreaElement | undefined;
  // The agent the request box asks, once "Ask <Label>" is chosen.
  const [asking, setAsking] = createSignal<AgentSummary | null>(null);
  const trigger = (): string => (props.node().attrs.trigger === "@" ? "@" : "[[");
  const host = useThisNote();
  const [query, setQuery] = createSignal("");
  const [results, setResults] = createSignal<Row[]>([]);
  // The type filter and title the shown results were found for, as `searchKey` makes it. "New
  // <type>" waits for them, so the list does not jump under the pointer and Enter on a note found
  // never picks "New" instead.
  const [searched, setSearched] = createSignal<string | null>(null);
  const [active, setActive] = createSignal(0);
  const [error, setError] = createSignal<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let seq = 0;
  let done = false;

  onMount(() => queueMicrotask(() => input.focus()));
  onCleanup(() => clearTimeout(timer));

  const filter = createMemo(() => readTypeFilter(query(), types()));
  const parsed = createMemo(() => readPickerQuery(filter().text));
  const found = () => searched() === searchKey(filter().type?.name ?? null, parsed().title);
  const newTypes = createMemo((): TypeDef[] => {
    const q = parsed();
    if (q.title === "" || !found()) return [];
    const only = filter().type;
    if (only) return [only];
    const current = host ? noteType(host) : undefined;
    return typeOptions(types(), current, recentTypes(), q.typeFilter);
  });
  const callable = createMemo((): AgentSummary[] => {
    if (trigger() !== "@" || filter().type) return [];
    const q = parsed().title;
    if (q.includes(" ")) return [];
    return callableAgents(agents(), host ? noteType(host) : undefined, q);
  });
  const choices = createMemo((): Choice[] => {
    const out: Choice[] = filter().suggestions.map((type) => ({ kind: "filter", type }));
    for (const agent of callable()) out.push({ kind: "agent", agent });
    for (const row of results()) out.push({ kind: "note", row });
    for (const type of newTypes()) out.push({ kind: "new", type });
    if (parsed().title !== "" && parsed().typeFilter === null && filter().type === null && types().length > newTypes().length) out.push({ kind: "types" });
    return out;
  });
  // What the text found nothing for, or why its `/` sets no type filter.
  const hint = createMemo((): string | null => {
    const f = filter();
    if (f.problem) return f.problem;
    if (f.type && parsed().title === "" && found() && results().length === 0) return `No ${f.type.label.toLowerCase()} yet`;
    return null;
  });
  // The choice highlighted first: past the types offered as filters, when anything follows them,
  // so Enter still takes the best note found.
  const firstActive = () => {
    const n = filter().suggestions.length;
    return choices().length > n ? n : 0;
  };

  function search(text: string) {
    setQuery(text);
    clearTimeout(timer);
    const mine = ++seq;
    const f = readTypeFilter(text, types());
    const type = f.type?.name ?? null;
    const q = readPickerQuery(f.text).title;
    const key = searchKey(type, q);
    if (q === "" && type === null) {
      setResults([]);
      setSearched(null);
      setActive(firstActive());
      return;
    }
    setActive(firstActive());
    if (key === searched()) return;
    timer = setTimeout(async () => {
      try {
        const rows = await searchNotes(type, q, RESULTS);
        if (mine !== seq) return;
        setResults(rows);
        setSearched(key);
        setActive(firstActive());
        setError(null);
      } catch (e) {
        if (mine !== seq) return;
        setError(e instanceof Error ? e.message : String(e));
        setResults([]);
        setSearched(key);
      }
    }, DEBOUNCE_MS);
  }

  /**
   * Remove the picker and put the caret where it stood, after what replaced it: a mention, a
   * placeholder link, or for an abandoned `@` picker the text typed.
   */
  function finish(
    content?:
      | { note: { id: string; title: string } }
      | { placeholder: { type: string; title: string } }
      | { call: { agent: AgentSummary; request: string } }
      | { nothing: true },
  ) {
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
    } else if (content && "call" in content && editor.schema.marks.link) {
      // One link, the call, then a space outside it (agents G19).
      const { agent, request } = content.call;
      const link = editor.schema.marks.link.create({ href: agentCallHref(agent.name, newUlid()) });
      const lines = `@${agentLabel(agent)} ${request}`.split("\n");
      const nodes: PMNode[] = [];
      lines.forEach((line, i) => {
        if (i > 0 && editor.schema.nodes.hardBreak) nodes.push(editor.schema.nodes.hardBreak.create(null, null, [link]));
        if (line) nodes.push(editor.schema.text(line, [link]));
      });
      nodes.push(editor.schema.text(" "));
      props.replaceWith(nodes);
      size = nodes.reduce((n, node) => n + node.nodeSize, 0);
    } else if (content && "nothing" in content) {
      props.replaceWith();
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
    if (c?.kind === "filter") {
      // The type is the filter now; what is typed next is searched for among its notes.
      const text = filterText(c.type);
      input.value = text;
      search(text);
      input.focus();
    } else if (c?.kind === "agent") {
      setAsking(c.agent);
      queueMicrotask(() => requestBox?.focus());
    }
    else if (c?.kind === "note") finish({ note: { id: c.row.id, title: c.row.title } });
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

  function onRequestKeyDown(e: KeyboardEvent) {
    const agent = asking();
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      const request = (requestBox?.value ?? "").trim();
      finish(agent && request ? { call: { agent, request } } : { nothing: true });
    } else if (e.key === "Escape") {
      e.preventDefault();
      finish({ nothing: true });
    }
  }

  function onFocusOut(e: FocusEvent) {
    const root = (e.currentTarget as HTMLElement) ?? null;
    if (root && e.relatedTarget instanceof Node && root.contains(e.relatedTarget)) return;
    // Leaving the box abandons the mention. Wait a tick so a click on a result lands first.
    setTimeout(() => {
      if (!done && !root?.contains(document.activeElement)) {
        if (asking()) {
          // A request not committed is not sent.
          done = true;
          props.replaceWith();
        } else if (trigger() === "@") {
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
      <Show when={asking()}>
        {(a) => (
          <span class="mention-request" data-testid="mention-request" data-agent={a().name} data-node-view-interactive>
            <label class="mention-request-label">
              <TypeIcon name={a().icon || AGENT_ICON} size={14} />
              <span>Ask {agentLabel(a())}…</span>
              <textarea
                ref={requestBox}
                rows={2}
                aria-label={`Ask ${agentLabel(a())}…`}
                placeholder={`What should ${agentLabel(a())} do?`}
                onKeyDown={onRequestKeyDown}
              />
            </label>
            <span class="muted mention-hint">Enter to send, Shift+Enter for a new line, Escape to cancel</span>
          </span>
        )}
      </Show>
      <Show when={!asking()}>
        <span class="mention-picker-prefix">{trigger()}</span>
        <Show when={filter().type}>
          {(t) => (
            <span class="mention-type-filter" data-testid="mention-type-filter" data-type={t().name} title={`Only ${t().plural || t().label}`}>
              <TypeIcon name={t().icon} size={12} />
              <span>{t().label}</span>
            </span>
          )}
        </Show>
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
      </Show>
      <Show when={!asking() && (choices().length > 0 || error() || hint())}>
        <span class="mention-results" id="mention-results" role="listbox" data-node-view-interactive>
          <Show when={error()}>
            <span class="error">{error()}</span>
          </Show>
          <Show when={hint()}>
            <span class="muted mention-filter-hint" data-testid="mention-filter-hint">
              {hint()}
            </span>
          </Show>
          <For each={choices()}>
            {(c, i) => (
              <span
                role="option"
                class="mention-result"
                classList={{
                  "mention-filter": c.kind === "filter",
                  "mention-agent": c.kind === "agent",
                  "mention-new": c.kind === "new" || c.kind === "types",
                  "mention-new-first": c.kind !== "note" && c.kind !== "agent" && choices()[i() - 1]?.kind === "note",
                }}
                aria-selected={active() === i()}
                data-testid={
                  c.kind === "filter" ? "mention-filter" : c.kind === "agent" ? "mention-agent" : c.kind === "note" ? "mention-result" : c.kind === "new" ? "mention-new" : "mention-choose-type"
                }
                data-type={c.kind === "new" || c.kind === "filter" ? c.type.name : undefined}
                data-agent={c.kind === "agent" ? c.agent.name : undefined}
                onMouseDown={(e) => {
                  e.preventDefault();
                  choose(i());
                }}
                onMouseEnter={() => setActive(i())}
              >
                <Show when={c.kind === "filter" && c}>
                  {(f) => (
                    <>
                      <TypeIcon name={f().type.icon} size={14} />
                      <span class="mention-result-title">{f().type.label}</span>
                      <span class="muted mention-hint">filter by type</span>
                    </>
                  )}
                </Show>
                <Show when={c.kind === "agent" && c}>
                  {(a) => (
                    <>
                      <TypeIcon name={a().agent.icon || AGENT_ICON} size={14} />
                      <span class="mention-result-title">Ask {agentLabel(a().agent)}</span>
                      <span class="muted"> @{a().agent.name}</span>
                    </>
                  )}
                </Show>
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
