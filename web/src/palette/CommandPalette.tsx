// The command palette (Ctrl/Cmd+K): find a note by title, or run an action. `>` lists actions
// only; `#type` or `type:type` lists notes of one type. Enter opens the choice in the active pane;
// Ctrl/Cmd+Enter, Ctrl/Cmd+click or a middle-click opens a note or page in a background tab.

import { For, type JSX, Match, Show, Switch, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { type Row, api } from "../api";
import { activeNote } from "../archive";
import {
  ArchiveIcon,
  CalendarIcon,
  CaptureIcon,
  GearIcon,
  InboxIcon,
  SearchIcon,
  TrashIcon,
  TypeIcon,
  UnarchiveIcon,
} from "../components/Icon";
import { inAndroidApp } from "../deviceImport";
import { knowTitle, noteArchived } from "../editor/titles";
import { typeDef, types } from "../session";
import { theme } from "../theme";
import { openInBackground } from "../workspace/events";
import * as M from "../workspace/model";
import { scrollPaneTop } from "../workspace/pane";
import { navigatePane, workspace } from "../workspace/store";
import { agents, agentsReadOnly } from "../agents";
import { type Action, type Glyph, agentActions, generalActions, noteActions, typeActions } from "./actions";
import {
  type NoteHit,
  type RankedNote,
  notesQuery,
  parseInput,
  rankActions,
  rankNotes,
  resolveType,
  segments,
  withoutArchived,
} from "./model";
import { recents, recordRecent } from "./recents";
import { closePalette } from "./store";
import "./palette.css";

/** How long typing waits before the notes are searched. */
const DEBOUNCE_MS = 120;
/** How many notes the search asks for, to rank again here. */
const FETCH = 25;

type Item =
  | { kind: "note"; key: string; section: string; note: RankedNote }
  | { kind: "search"; key: string; section: string; text: string }
  | { kind: "action"; key: string; section: string; action: Action; positions: number[] };

interface Section {
  title: string;
  items: Item[];
}

const PHONE = "(max-width: 600px)";

function shortDate(v: unknown): string {
  if (typeof v !== "string") return "";
  const d = new Date(v);
  if (Number.isNaN(d.getTime())) return "";
  const now = new Date();
  const opts: Intl.DateTimeFormatOptions =
    d.getFullYear() === now.getFullYear() ? { month: "short", day: "numeric" } : { year: "numeric", month: "short", day: "numeric" };
  return d.toLocaleDateString(undefined, opts);
}

function hitOf(row: Row): NoteHit {
  const updated = row.fields.updated;
  return { id: row.id, title: row.title, type: row.type, updated: typeof updated === "string" ? updated : null };
}

function Highlighted(props: { text: string; positions: readonly number[] }) {
  return (
    <For each={segments(props.text, props.positions)}>{(s) => (s.hit ? <mark>{s.text}</mark> : <>{s.text}</>)}</For>
  );
}

export function CommandPalette() {
  const returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const phone = typeof window.matchMedia === "function" && window.matchMedia(PHONE).matches;
  const [input, setInput] = createSignal("");
  const [found, setFound] = createSignal<{ query: string; rows: NoteHit[] } | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [active, setActive] = createSignal(0);
  let field!: HTMLInputElement;
  let dialog!: HTMLDivElement;
  let list!: HTMLDivElement;

  const parsed = createMemo(() => parseInput(input()));
  const filterType = createMemo(() => (parsed().mode === "type" ? resolveType(types(), parsed().typeWord) : null));
  const query = createMemo(() => {
    const p = parsed();
    if (p.mode === "actions") return null;
    if (p.mode === "type") return filterType() ? notesQuery(p.text, filterType()!.name) : null;
    return notesQuery(p.text, null);
  });
  const actions = createMemo(() => [
    ...noteActions(activeNote()),
    ...generalActions({ phone, dark: theme() === "dark", androidApp: inAndroidApp() }),
    ...typeActions(types()),
    ...agentActions(agents(), agentsReadOnly()),
  ]);

  // The notes search: debounced, and a newer query cancels the one in flight.
  let timer: ReturnType<typeof setTimeout> | undefined;
  let inflight: AbortController | undefined;
  onCleanup(() => {
    clearTimeout(timer);
    inflight?.abort();
  });
  createEffect(
    on(query, (q) => {
      clearTimeout(timer);
      inflight?.abort();
      inflight = undefined;
      if (q === null) return;
      timer = setTimeout(async () => {
        const ctl = new AbortController();
        inflight = ctl;
        try {
          const page = await api.query(q, null, FETCH, undefined, ctl.signal);
          if (ctl.signal.aborted) return;
          for (const r of page.rows) knowTitle(r.id, r.title, r.type);
          setFound({ query: q, rows: page.rows.map(hitOf) });
          setError(null);
        } catch (e) {
          if (ctl.signal.aborted) return;
          setFound({ query: q, rows: [] });
          setError(e instanceof Error ? e.message : String(e));
        }
      }, DEBOUNCE_MS);
    }),
  );

  const sections = createMemo((): Section[] => {
    const p = parsed();
    // Recent notes the user has since archived are left out, as the notes search leaves them out.
    const recent = withoutArchived(
      recents().filter((r) => r.title),
      noteArchived,
    );
    const out: Section[] = [];
    const noteItems = (notes: RankedNote[], section: string): Item[] =>
      notes.map((note) => ({ kind: "note", key: `note:${note.id}`, section, note }));
    const actionItems = (limit: number, all = false): Item[] =>
      rankActions(actions(), p.text, limit, all).map((r) => ({
        kind: "action",
        key: `action:${r.action.id}`,
        section: "Actions",
        action: r.action,
        positions: r.positions,
      }));

    if (p.mode === "actions") {
      const items = actionItems(60, true);
      if (items.length) out.push({ title: "Actions", items });
      return out;
    }

    // The rows found for the query as it stands; while a newer one runs, those of the last.
    const f = found();
    const rows = f && f.query === query() ? f.rows : (f?.rows ?? []);

    if (p.mode === "type") {
      const t = filterType();
      if (!t) return out;
      const pool = recent.filter((r) => r.type === t.name);
      const notes = p.text.trim() ? rankNotes(p.text, rows, pool, 20) : rankNotes("", rows, [], 20);
      const title = t.plural || t.label;
      if (notes.length) out.push({ title, items: noteItems(notes, title) });
      const typed = actions().filter((a) => a.id === `new:${t.name}` || a.id === `list:${t.name}`);
      out.push({
        title: "Actions",
        items: typed.map((action) => ({ kind: "action", key: `action:${action.id}`, section: "Actions", action, positions: [] })),
      });
      return out;
    }

    if (!p.text.trim()) {
      const notes = recent.slice(0, 8).map((r) => ({ ...r, recent: true, positions: [] }));
      if (notes.length) out.push({ title: "Recent notes", items: noteItems(notes, "Recent notes") });
      out.push({ title: "Actions", items: actionItems(40) });
      return out;
    }

    const notes = rankNotes(p.text, rows, recent, 8);
    const noteSection: Section = { title: "Notes", items: noteItems(notes, "Notes") };
    noteSection.items.push({ kind: "search", key: "search", section: "Notes", text: p.text.trim() });
    const ranked = rankActions(actions(), p.text, 6);
    const actionSection: Section = {
      title: "Actions",
      items: ranked.map((r) => ({ kind: "action", key: `action:${r.action.id}`, section: "Actions", action: r.action, positions: r.positions })),
    };
    // An action whose words the text starts comes first: "inbox" means Go to Inbox more often
    // than a note that mentions an inbox.
    const actionsFirst = ranked[0]?.strong && !notes.some((n) => n.title.toLowerCase() === p.text.trim().toLowerCase());
    if (actionsFirst) out.push(actionSection, noteSection);
    else out.push(noteSection, actionSection);
    return out.filter((s) => s.items.length);
  });

  const items = createMemo(() => sections().flatMap((s) => s.items));
  const searching = () => {
    const q = query();
    return q !== null && found()?.query !== q;
  };

  // When results arrive, the highlight stays on the item the user moved it to, else goes back
  // to the top.
  let activeKey: string | null = null;
  let moved = false;
  const move = (i: number) => {
    moved = true;
    setActive(i);
  };
  createEffect(
    on(items, (list) => {
      const i = moved && activeKey ? list.findIndex((x) => x.key === activeKey) : -1;
      setActive(i >= 0 ? i : 0);
    }),
  );
  createEffect(() => {
    activeKey = items()[active()]?.key ?? null;
  });
  createEffect(
    on(active, (i) => {
      const el = list?.querySelector<HTMLElement>(`[data-index="${i}"]`);
      el?.scrollIntoView?.({ block: "nearest" });
    }),
  );

  const optionId = (i: number) => `palette-option-${i}`;

  // A browser sends a pointer move when the list changes under a pointer that has not moved;
  // only a real move takes the highlight from the keyboard. The first move only sets the mark.
  let pointerAt: string | null = null;
  function pointerMoved(e: PointerEvent): boolean {
    const at = `${e.screenX},${e.screenY}`;
    const moved = pointerAt !== null && pointerAt !== at;
    pointerAt = at;
    return moved;
  }

  function go(href: string, background: boolean) {
    if (background) {
      openInBackground(href);
      return;
    }
    const pane = M.activePane(workspace());
    navigatePane(pane, href);
    scrollPaneTop(pane);
  }

  function choose(item: Item | undefined, background: boolean) {
    if (!item) return;
    // Close first, so focus is back where it was before the action moves it.
    closePalette();
    switch (item.kind) {
      case "note":
        recordRecent(item.note.id, item.note.title, item.note.type);
        go(`/notes/${item.note.id}`, background);
        break;
      case "search":
        go(`/search?q=${encodeURIComponent(item.text)}`, background);
        break;
      case "action":
        if (item.action.href) go(item.action.href, background);
        else item.action.run?.();
        break;
    }
  }

  function onKeyDown(e: KeyboardEvent) {
    const n = items().length;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closePalette();
    } else if (e.key === "ArrowDown" || (e.key === "n" && e.ctrlKey && !e.metaKey)) {
      e.preventDefault();
      if (n) move((active() + 1) % n);
    } else if (e.key === "ArrowUp" || (e.key === "p" && e.ctrlKey && !e.metaKey)) {
      e.preventDefault();
      if (n) move((active() - 1 + n) % n);
    } else if (e.key === "PageDown" || e.key === "PageUp") {
      e.preventDefault();
      if (n) move(Math.max(0, Math.min(n - 1, active() + (e.key === "PageDown" ? 8 : -8))));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (e.isComposing) return;
      choose(items()[active()], e.ctrlKey || e.metaKey);
    } else if (e.key === "Tab") {
      // The box is the only stop: the list is driven from it.
      e.preventDefault();
    }
  }

  onMount(() => {
    field.focus();
    // Focus that wanders out of the dialog comes back to the box.
    const keep = (e: FocusEvent) => {
      if (e.target instanceof Node && !dialog.contains(e.target)) field.focus();
    };
    document.addEventListener("focusin", keep);
    onCleanup(() => {
      document.removeEventListener("focusin", keep);
      if (returnTo?.isConnected) returnTo.focus({ preventScroll: true });
    });
  });

  const placeholder = () =>
    phone ? "Search notes or run a command" : "Search notes, or type > for commands, # for a type";

  return (
    <div
      class="palette-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) closePalette();
      }}
    >
      <div ref={dialog} class="palette" role="dialog" aria-modal="true" aria-label="Command palette" onKeyDown={onKeyDown}>
        <div class="palette-bar">
          <SearchIcon />
          <input
            ref={field}
            class="palette-input"
            type="text"
            role="combobox"
            aria-label="Search or run a command"
            aria-expanded="true"
            aria-controls="palette-list"
            aria-autocomplete="list"
            aria-activedescendant={items().length ? optionId(active()) : undefined}
            autocomplete="off"
            autocapitalize="off"
            spellcheck={false}
            placeholder={placeholder()}
            value={input()}
            onInput={(e) => {
              moved = false;
              setInput(e.currentTarget.value);
            }}
          />
          <Show when={phone}>
            <button type="button" class="palette-cancel" onClick={() => closePalette()}>
              Cancel
            </button>
          </Show>
        </div>
        <Show when={parsed().mode !== "all"}>
          <p class="palette-mode muted">
            <Switch>
              <Match when={parsed().mode === "actions"}>Commands only</Match>
              <Match when={filterType()}>{(t) => <>Notes of type {t().label}</>}</Match>
              <Match when={parsed().typeWord}>No type matches “{parsed().typeWord}”</Match>
              <Match when={true}>Type a type’s name after #</Match>
            </Switch>
          </p>
        </Show>
        <div ref={list} id="palette-list" class="palette-list" role="listbox" aria-label="Results">
          <For each={sections()}>
            {(section) => {
              const head = `palette-group-${section.title.replace(/\W+/g, "-").toLowerCase()}`;
              return (
                <div role="group" class="palette-group" aria-labelledby={head}>
                  <div class="palette-group-title" id={head} role="presentation">
                    {section.title}
                  </div>
                  <For each={section.items}>
                    {(item) => {
                      const i = () => items().indexOf(item);
                      return (
                        <div
                          id={optionId(i())}
                          data-index={i()}
                          role="option"
                          class="palette-option"
                          classList={{ active: active() === i() }}
                          aria-selected={active() === i()}
                          onPointerMove={(e) => {
                            if (pointerMoved(e) && active() !== i()) move(i());
                          }}
                          onMouseDown={(e) => e.preventDefault()}
                          onClick={(e) => choose(item, e.ctrlKey || e.metaKey)}
                          onAuxClick={(e) => {
                            if (e.button === 1) {
                              e.preventDefault();
                              choose(item, true);
                            }
                          }}
                        >
                          <OptionBody item={item} />
                        </div>
                      );
                    }}
                  </For>
                </div>
              );
            }}
          </For>
          <Show when={!items().length}>
            <p class="palette-empty muted" role="presentation">
              {searching() ? "Searching…" : parsed().mode === "type" && !filterType() ? "No matching type." : "Nothing matches."}
            </p>
          </Show>
        </div>
        <Show when={error()}>
          <p class="palette-error error" role="alert">
            {error()}
          </p>
        </Show>
        <footer class="palette-foot muted" aria-hidden="true">
          <span>
            <kbd>↑</kbd>
            <kbd>↓</kbd> move
          </span>
          <span>
            <kbd>Enter</kbd> open
          </span>
          <span>
            <kbd>Ctrl+Enter</kbd> background tab
          </span>
          <span>
            <kbd>Esc</kbd> close
          </span>
          <span class="palette-foot-status" aria-live="polite">
            {searching() ? "Searching…" : ""}
          </span>
        </footer>
      </div>
    </div>
  );
}

function OptionBody(props: { item: Item }): JSX.Element {
  return (
    <Switch>
      <Match when={props.item.kind === "note" && props.item}>
        {(it) => {
          const n = () => (it() as Extract<Item, { kind: "note" }>).note;
          const t = () => typeDef(n().type);
          return (
            <>
              <span class="palette-icon">
                <TypeIcon name={t()?.icon ?? null} size={18} />
              </span>
              <span class="palette-label">
                <Highlighted text={n().title || "Untitled"} positions={n().positions} />
              </span>
              <span class="palette-meta">
                <span class="palette-type">{t()?.label ?? n().type}</span>
                <Show when={shortDate(n().updated)}>{(d) => <span class="palette-date">{d()}</span>}</Show>
              </span>
            </>
          );
        }}
      </Match>
      <Match when={props.item.kind === "search" && props.item}>
        {(it) => (
          <>
            <span class="palette-icon">
              <SearchIcon />
            </span>
            <span class="palette-label">
              Search all notes for “{(it() as Extract<Item, { kind: "search" }>).text}”
            </span>
          </>
        )}
      </Match>
      <Match when={props.item.kind === "action" && props.item}>
        {(it) => {
          const a = () => (it() as Extract<Item, { kind: "action" }>).action;
          return (
            <>
              <span class="palette-icon">
                <ActionGlyph icon={a().icon} />
              </span>
              <span class="palette-label">
                <Highlighted text={a().label} positions={(it() as Extract<Item, { kind: "action" }>).positions} />
              </span>
              <span class="palette-meta">
                <Show when={a().shortcut}>{(k) => <kbd class="palette-key">{k()}</kbd>}</Show>
                <span class="palette-section">{a().section}</span>
              </span>
            </>
          );
        }}
      </Match>
    </Switch>
  );
}

function Svg(props: { children: JSX.Element }) {
  return (
    <svg
      class="icon"
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {props.children}
    </svg>
  );
}

const GLYPHS: Partial<Record<Glyph, () => JSX.Element>> = {
  capture: CaptureIcon,
  inbox: InboxIcon,
  calendar: CalendarIcon,
  search: SearchIcon,
  trash: TrashIcon,
  archive: ArchiveIcon,
  unarchive: UnarchiveIcon,
  gear: GearIcon,
  files: () => (
    <Svg>
      <path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7z" />
      <path d="M14 2v5h6" />
    </Svg>
  ),
  tab: () => (
    <Svg>
      <rect x="3" y="5" width="18" height="15" rx="2" />
      <path d="M3 9h18M12 12v5M9.5 14.5h5" />
    </Svg>
  ),
  close: () => (
    <Svg>
      <path d="M18 6 6 18M6 6l12 12" />
    </Svg>
  ),
  "split-right": () => (
    <Svg>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M12 4v16" />
    </Svg>
  ),
  "split-down": () => (
    <Svg>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M3 12h18" />
    </Svg>
  ),
  "sidebar-left": () => (
    <Svg>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M9 4v16" />
    </Svg>
  ),
  "sidebar-right": () => (
    <Svg>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M15 4v16" />
    </Svg>
  ),
  theme: () => (
    <Svg>
      <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9z" />
    </Svg>
  ),
  ai: () => (
    <Svg>
      <path d="M12 3l1.8 4.7 4.7 1.8-4.7 1.8L12 16l-1.8-4.7-4.7-1.8 4.7-1.8z" />
      <path d="M19 15l.8 2.2 2.2.8-2.2.8L19 21l-.8-2.2-2.2-.8 2.2-.8z" />
    </Svg>
  ),
};

function ActionGlyph(props: { icon: Action["icon"] }) {
  return (
    <Show when={"glyph" in props.icon && props.icon.glyph} fallback={<TypeIcon name={"type" in props.icon ? props.icon.type : null} size={18} />}>
      {(g) => GLYPHS[g()]?.() ?? null}
    </Show>
  );
}
