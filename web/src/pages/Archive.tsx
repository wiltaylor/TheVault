// The Archive page: every archived note, most recently changed first, with a search box and a
// type filter. Each can be taken out of the archive or moved to the trash. Archived notes are
// left out of every other listing, so this is where to find them.

import { A } from "@solidjs/router";
import { For, Show, createEffect, createSignal, on, onCleanup } from "solid-js";
import { type Row, api } from "../api";
import { archiveQuery } from "../archive";
import { changes } from "../changes";
import { ArchiveIcon, SearchIcon, TypeIcon } from "../components/Icon";
import { createPagedRows } from "../components/pagedRows";
import { showToast } from "../components/Toasts";
import { knowArchived, knowTitle } from "../editor/titles";
import { typeDef, types } from "../session";
import { errorText } from "./TypeEditor";

const PAGE_SIZE = 100;
/** How long typing waits before the search runs. */
const TYPING_MS = 250;

function formatDate(v: unknown): string {
  if (typeof v !== "string") return "";
  const d = new Date(v);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

export function Archive() {
  const [draft, setDraft] = createSignal("");
  const [text, setText] = createSignal("");
  const [type, setType] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  /** Rows acted on, hidden until the list reloads without them. */
  const [gone, setGone] = createSignal<Set<string>>(new Set());
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));

  const query = () => archiveQuery(text(), type());
  const paged = createPagedRows(
    async (cursor, size) => {
      const page = await api.query(query(), cursor, size);
      for (const r of page.rows) {
        knowTitle(r.id, r.title, r.type);
        knowArchived(r.id, true);
      }
      return page;
    },
    query,
    PAGE_SIZE,
    changes,
  );
  const rows = () => paged.rows().filter((r) => !gone().has(r.id));
  // A reload reads the archive as it is now, so nothing needs hiding any more.
  createEffect(on(paged.rows, () => setGone(new Set<string>()), { defer: true }));

  /** Run `work`, an undo from a toast, showing what goes wrong. */
  async function undo(work: () => Promise<unknown>) {
    setError(null);
    try {
      await work();
    } catch (e) {
      setError(errorText(e));
    }
  }

  function typed(value: string) {
    setDraft(value);
    clearTimeout(timer);
    timer = setTimeout(() => setText(draft()), TYPING_MS);
  }

  async function act(r: Row, work: () => Promise<void>) {
    setError(null);
    setGone(new Set([...gone(), r.id]));
    try {
      await work();
    } catch (e) {
      const back = new Set(gone());
      back.delete(r.id);
      setGone(back);
      setError(errorText(e));
    }
  }

  const unarchive = (r: Row) =>
    act(r, async () => {
      const n = await api.note(r.id);
      const saved = await api.unarchiveNote(r.id, n.version);
      knowArchived(r.id, false);
      showToast(`“${r.title || "Untitled"}” taken out of the archive`, {
        label: "Undo",
        run: () =>
          void undo(async () => {
            await api.archiveNote(r.id, saved.version);
            knowArchived(r.id, true);
          }),
      });
    });

  const remove = (r: Row) => {
    if (!confirm("Move this note to the trash?")) return;
    void act(r, async () => {
      const n = await api.note(r.id);
      await api.deleteNote(r.id, n.version);
      showToast(`“${r.title || "Untitled"}” moved to the trash`, {
        label: "Undo",
        run: () => void undo(() => api.restoreFromTrash(r.id)),
      });
    });
  };

  const status = () => {
    const total = paged.total();
    if (total === null) return "Loading…";
    if (text().trim()) return total === 1 ? "1 match" : `${total} matches`;
    return total === 1 ? "1 archived note" : `${total} archived notes`;
  };

  return (
    <section class="list-page archive-page" aria-label="Archive">
      <header class="note-header">
        <h1 class="type-list-title">
          <ArchiveIcon />
          Archive
        </h1>
      </header>
      <p class="muted archive-intro">
        Archived notes are left out of search, lists and queries. They stay readable and editable, and links to them
        still work.
      </p>

      <div class="type-list-toolbar">
        <div class="search-bar type-list-search" role="search">
          <SearchIcon />
          <input
            type="search"
            placeholder="Search archived notes"
            aria-label="Search archived notes"
            value={draft()}
            onInput={(e) => typed(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                clearTimeout(timer);
                setText(draft());
              }
            }}
          />
        </div>
        <select
          class="archive-type"
          aria-label="Type"
          onChange={(e) => setType(e.currentTarget.value || null)}
        >
          <option value="" selected={type() === null}>
            Every type
          </option>
          <For each={types()}>
            {(t) => (
              <option value={t.name} selected={type() === t.name}>
                {t.plural || t.label}
              </option>
            )}
          </For>
        </select>
      </div>

      <p class="muted list-count" data-testid="archive-status" aria-live="polite">
        {status()}
      </p>
      <Show when={error() ?? paged.error()}>
        {(e) => (
          <p class="error" role="alert">
            {e()}
          </p>
        )}
      </Show>

      <Show when={paged.total() !== null}>
        <Show
          when={rows().length > 0}
          fallback={<p class="muted archive-empty">{text().trim() ? "No archived notes match." : "Nothing is archived."}</p>}
        >
          <ul class="archive-list" aria-label="Archived notes" data-testid="archive-list">
            <For each={rows()}>
              {(r) => (
                <li class="archive-row" data-note={r.id} data-testid="archive-row">
                  <A class="archive-link" href={`/notes/${r.id}`}>
                    <TypeIcon name={typeDef(r.type)?.icon} size={16} />
                    <span class="note-title">{r.title || "Untitled"}</span>
                  </A>
                  <span class="note-type muted">{typeDef(r.type)?.label ?? r.type}</span>
                  <time class="note-date muted" datetime={typeof r.fields.updated === "string" ? r.fields.updated : undefined}>
                    {formatDate(r.fields.updated)}
                  </time>
                  <span class="archive-actions">
                    <button type="button" onClick={() => void unarchive(r)}>
                      Unarchive
                    </button>
                    <button type="button" class="danger" onClick={() => remove(r)}>
                      Delete
                    </button>
                  </span>
                </li>
              )}
            </For>
          </ul>
          <Show when={paged.rows().length < (paged.total() ?? 0)}>
            <button type="button" class="link archive-more" onClick={() => void paged.loadUntil(paged.rows().length + PAGE_SIZE - 1)}>
              Show more
            </button>
          </Show>
        </Show>
      </Show>
    </section>
  );
}
