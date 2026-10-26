// A list of links, such as a project's documentation: a list whose items are dicts of a `name`
// and a `url` (`[{ name: "Docs", url: "https://example.com/docs" }]`). Read-only, each link opens
// in a new tab, shown by its name or else its URL's host and path. Edited, one row per link with a
// Name box and a URL box, moved up and down; a URL pasted into a row that has none fills the URL,
// and the name with the URL's host when it is blank.

import { For, Show, createEffect, createSignal, on } from "solid-js";
import type { Json } from "../api";
import { type Link, type LinkShape, SAFE_LINK, hostOf, isUrl, linkText, linksOf, linksValue } from "../links";
import { canonicalJson } from "../notes";

export { type Link, type LinkShape, linkShape, linksOf, linksText, linksValue } from "../links";

/** Lucide's external-link icon, drawn inline so a row never waits on the icon set. */
function OpenIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M15 3h6v6" />
      <path d="M10 14 21 3" />
      <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
    </svg>
  );
}

/** One link read-only: a link to another site in a new tab, or its text when the URL may not be opened. */
export function LinkAnchor(props: { link: Link }) {
  return (
    <Show when={SAFE_LINK.test(props.link.url)} fallback={<span class="link-text">{linkText(props.link)}</span>}>
      <a class="field-link" href={props.link.url} target="_blank" rel="noopener noreferrer" title={props.link.url}>
        {linkText(props.link)}
      </a>
    </Show>
  );
}

/** A list of links read-only, inline and apart by commas, as tables, cards and the page show them. */
export function LinkAnchors(props: { value: Json | undefined }) {
  return (
    <span class="link-list-view">
      <For each={linksOf(props.value)}>
        {(l, i) => (
          <>
            {i() > 0 ? ", " : ""}
            <LinkAnchor link={l} />
          </>
        )}
      </For>
    </span>
  );
}

/** Why a row is not written yet, or null when it is a link the list stores. */
export function rowProblem(l: Link, shape: LinkShape): string | null {
  const url = l.url.trim();
  if (!url) return l.name.trim() ? "Enter the link's URL." : null;
  if (!isUrl(url)) return `“${url}” is not an absolute URL, such as https://example.com.`;
  if (shape.nameRequired && !l.name.trim()) return "Enter the link's name.";
  return null;
}

/** The rows that are links, trimmed, as the list stores them; blank rows and rows with a problem are left out. */
export function rowsValue(rows: Link[], shape: LinkShape): Json | undefined {
  const ok = rows
    .map((l) => ({ name: l.name.trim(), url: l.url.trim() }))
    .filter((l) => l.url && rowProblem(l, shape) === null);
  return linksValue(ok, shape);
}

/** Row `l` after `text` is pasted into it: the URL and, when the name is blank, the URL's host. Null when the paste is ordinary text. */
export function pasteInto(l: Link, text: string): Link | null {
  const t = text.trim();
  if (l.url.trim() || !isUrl(t)) return null;
  return { url: t, name: l.name.trim() ? l.name : hostOf(t) };
}

let nextRow = 1;
const same = (a: Json | undefined, b: Json | undefined) => canonicalJson(a) === canonicalJson(b);

export interface LinkListInputProps {
  shape: LinkShape;
  value: Json | undefined;
  onChange: (value: Json | undefined) => void;
  /** The field's label, such as "Links". */
  label: string;
  id: string;
}

/** The editor of a list of links: a row per link with Name and URL, moved up and down, and Add link. */
export function LinkListInput(props: LinkListInputProps) {
  // Each row has an id, so a row keeps its boxes while it changes and as it moves. A row with no
  // URL yet, or one that is not a URL, is kept while editing and left out of the value.
  const [rows, setRows] = createSignal<number[]>([]);
  const [links, setLinks] = createSignal<Record<number, Link>>({});
  // Rows whose URL box has been left, which then show what is wrong with them.
  const [touched, setTouched] = createSignal<Set<number>>(new Set());
  let root!: HTMLDivElement;
  function load(v: Json | undefined) {
    const list = linksOf(v);
    const ids = list.map(() => nextRow++);
    setLinks(Object.fromEntries(ids.map((id, i) => [id, list[i]!])));
    setRows(ids);
    setTouched(new Set<number>());
  }
  load(props.value);
  // What this editor last wrote. A different value came from elsewhere (the Properties panel, a
  // script), and the rows start again from it.
  let written: Json | undefined = props.value;
  createEffect(
    on(
      () => props.value,
      (v) => {
        if (!same(v, written)) load(v);
        written = v;
      },
      { defer: true },
    ),
  );
  function write(ids: number[], next: Record<number, Link>) {
    setLinks(next);
    setRows(ids);
    const out = rowsValue(
      ids.map((id) => next[id]!),
      props.shape,
    );
    if (same(out, written)) return;
    written = out;
    props.onChange(out);
  }
  const edit = (row: number, l: Link) => write(rows(), { ...links(), [row]: l });
  function move(i: number, by: number) {
    const ids = [...rows()];
    const j = i + by;
    if (j < 0 || j >= ids.length) return;
    [ids[i], ids[j]] = [ids[j]!, ids[i]!];
    write(ids, links());
  }
  function add() {
    const id = nextRow++;
    setLinks({ ...links(), [id]: { name: "", url: "" } });
    setRows([...rows(), id]);
    queueMicrotask(() => root.querySelector<HTMLInputElement>(`[data-row="${id}"] .link-name-input`)?.focus());
  }
  function paste(row: number, e: ClipboardEvent) {
    const next = pasteInto(links()[row]!, e.clipboardData?.getData("text/plain") ?? "");
    if (!next) return;
    e.preventDefault();
    edit(row, next);
  }
  return (
    <div ref={root} class="link-list-input" role="group" aria-label={props.label} id={props.id}>
      <For each={rows()}>
        {(row, i) => {
          const l = () => links()[row] ?? { name: "", url: "" };
          const who = () => (l().url.trim() || l().name.trim() ? linkText(l()) : `link ${i() + 1}`);
          const problem = () => (touched().has(row) ? rowProblem(l(), props.shape) : null);
          const problemId = `${props.id}-link-${row}-problem`;
          return (
            <div class="link-row" data-testid="link-row" data-row={row}>
              <input
                class="link-name-input"
                aria-label={`Name of ${props.label} link ${i() + 1}`}
                placeholder={props.shape.nameRequired ? "Name" : "Name (optional)"}
                autocomplete="off"
                value={l().name}
                required={props.shape.nameRequired}
                onInput={(e) => edit(row, { ...l(), name: e.currentTarget.value })}
                onPaste={(e) => paste(row, e)}
              />
              <input
                class="link-url-input"
                type="url"
                aria-label={`URL of ${props.label} link ${i() + 1}`}
                placeholder="https://"
                autocomplete="off"
                value={l().url}
                required
                aria-invalid={problem() ? true : undefined}
                aria-describedby={problem() ? problemId : undefined}
                onInput={(e) => edit(row, { ...l(), url: e.currentTarget.value })}
                onPaste={(e) => paste(row, e)}
                onBlur={() => setTouched(new Set([...touched(), row]))}
              />
              <Show
                when={isUrl(l().url.trim()) && SAFE_LINK.test(l().url.trim())}
                fallback={<span class="link-open" aria-hidden="true" />}
              >
                <a
                  class="link-open"
                  href={l().url.trim()}
                  target="_blank"
                  rel="noopener noreferrer"
                  aria-label={`Open ${who()}`}
                  title={l().url.trim()}
                >
                  <OpenIcon />
                </a>
              </Show>
              <button type="button" aria-label={`Move ${who()} up`} disabled={i() === 0} onClick={() => move(i(), -1)}>
                ↑
              </button>
              <button type="button" aria-label={`Move ${who()} down`} disabled={i() === rows().length - 1} onClick={() => move(i(), 1)}>
                ↓
              </button>
              <button
                type="button"
                aria-label={`Remove ${who()}`}
                onClick={() =>
                  write(
                    rows().filter((r) => r !== row),
                    links(),
                  )
                }
              >
                ×
              </button>
              <Show when={problem()}>
                <p class="field-problem error" id={problemId} role="alert">
                  {problem()}
                </p>
              </Show>
            </div>
          );
        }}
      </For>
      <button type="button" class="link-add" onClick={add}>
        Add link
      </button>
    </div>
  );
}
