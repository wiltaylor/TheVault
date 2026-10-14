// The right sidebar's History panel: the versions of the active pane's note, newest first, each
// with when, who and what changed. Choosing one shows it in the pane, read-only (preview.ts);
// Arrow Up and Down step through them.

import { For, Show, createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { type Author, type Note, type NoteDoc, type Version, api } from "../api";
import { authorText, when } from "../admin/format";
import { errorText } from "../pages/TypeEditor";
import { type NoteSource, rightIsDrawer, setRightDrawer } from "../sidebars";
import { previewOf, setPreview } from "./preview";
import { restoreVersion } from "./restore";
import { changeSummary, relativeTime, summaryText } from "./summary";

/** How many versions the list shows at first, and how many more each "Show older" adds. */
export const PAGE = 25;

type Stored = Note & { author: Author; at: number };

/** The panel for the note `source` offers in `pane`. */
export function HistoryPanel(props: { pane: string; source: NoteSource }) {
  const noteId = props.source.noteId();
  const [versions, setVersions] = createSignal<Version[] | null>(null);
  const [docs, setDocs] = createSignal<Record<string, Stored>>({});
  const [limit, setLimit] = createSignal(PAGE);
  const [error, setError] = createSignal<string | null>(null);
  const [now, setNow] = createSignal(Date.now());
  const tick = setInterval(() => setNow(Date.now()), 30_000);
  onCleanup(() => clearInterval(tick));

  let list: HTMLOListElement | undefined;

  /** The version shown in the pane: the previewed one, else the current one. */
  const selected = () => previewOf(props.pane)?.version ?? versions()?.find((v) => v.current)?.version;

  // The list loads again whenever the note saves (each save has a new version token).
  let loads = 0;
  createEffect(
    on(
      () => props.source.version(),
      async () => {
        const mine = ++loads;
        try {
          const list = [...(await api.history(noteId))].reverse();
          // Versions already listed keep their rows, so focus and selection stay put.
          if (mine === loads) setVersions((old) => list.map((v) => old?.find((o) => o.version === v.version && o.current === v.current) ?? v));
        } catch (e) {
          if (mine === loads) setError(errorText(e));
        }
      },
    ),
  );

  const visible = createMemo(() => (versions() ?? []).slice(0, limit()));

  // Each shown version and the one before it are fetched once, for their summaries.
  const fetching = new Set<string>();
  createEffect(() => {
    const all = versions() ?? [];
    const want = all.slice(0, limit() + 1).filter((v) => !(v.version in docs()) && !fetching.has(v.version));
    for (const v of want) {
      fetching.add(v.version);
      api
        .version(noteId, v.version)
        .then((d) => setDocs((all) => ({ ...all, [v.version]: d })))
        .catch(() => {})
        .finally(() => fetching.delete(v.version));
    }
  });

  /** What version `i` of the list changed over the one before it. */
  function summary(i: number): string | null {
    const all = versions() ?? [];
    const d = docs()[all[i]!.version];
    if (!d) return null;
    const older = all[i + 1];
    if (!older) return isCreation(d.note) ? "created" : "earliest kept version";
    const prev = docs()[older.version];
    return prev ? summaryText(changeSummary(prev.note, d.note)) : null;
  }

  // Choosing again before the last one loaded: only the last choice shows.
  let choosing = 0;
  async function choose(v: Version) {
    setError(null);
    const mine = ++choosing;
    if (v.current) {
      setPreview(props.pane, null);
      return;
    }
    // Unsaved edits are saved first, so the live note is in the history and nothing is lost.
    if (!previewOf(props.pane) && !(await props.source.flush())) {
      setError("Your latest edits could not be saved; resolve them before viewing an older version.");
      return;
    }
    try {
      const d = docs()[v.version] ?? (await api.version(noteId, v.version));
      if (mine !== choosing) return;
      setDocs((all) => ({ ...all, [v.version]: d }));
      setPreview(props.pane, { noteId, version: v.version, note: d, author: d.author ?? v.author, at: d.at ?? v.at });
      // On a phone the drawer covers the pane: close it to show the version.
      if (rightIsDrawer()) setRightDrawer(false);
    } catch (e) {
      if (mine === choosing) setError(errorText(e));
    }
  }

  function step(i: number, delta: number) {
    const all = versions() ?? [];
    const j = i + delta;
    if (j < 0 || j >= all.length) return;
    if (j >= limit()) setLimit(limit() + PAGE);
    const v = all[j]!;
    queueMicrotask(() => list?.querySelector<HTMLButtonElement>(`li[data-version="${v.version}"] .history-item`)?.focus());
    void choose(v);
  }

  async function restore(v: Version) {
    setError(null);
    const err = await restoreVersion(props.pane, { noteId, version: v.version, at: v.at });
    if (err) setError(err);
  }

  return (
    <div class="history-panel">
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={versions()} fallback={<p class="muted">Loading…</p>}>
        <ol class="history-list" aria-label="Versions" ref={list}>
          <For each={visible()}>
            {(v, i) => {
              const isSelected = () => selected() === v.version;
              return (
                <li classList={{ selected: isSelected(), current: v.current }} data-version={v.version}>
                  <button
                    type="button"
                    class="history-item"
                    aria-pressed={isSelected()}
                    tabindex={isSelected() || (!selected() && i() === 0) ? 0 : -1}
                    onClick={() => void choose(v)}
                    onKeyDown={(e) => {
                      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                        e.preventDefault();
                        step(i(), e.key === "ArrowDown" ? 1 : -1);
                      }
                    }}
                  >
                    <span class="history-when">
                      <span>
                        <span class="history-ago">{relativeTime(v.at, now())}</span>{" "}
                        <span class="history-author">by {authorText(v.author)}</span>
                      </span>
                      <Show when={v.current}>
                        <span class="badge history-current">current</span>
                      </Show>
                    </span>
                    <span class="history-meta">
                      <time datetime={new Date(v.at).toISOString()}>{when(v.at)}</time>
                    </span>
                    <span class="history-summary" data-testid="change-summary">
                      {summary(i()) ?? "…"}
                    </span>
                  </button>
                  <Show when={isSelected() && !v.current}>
                    <div class="history-actions">
                      <button type="button" class="primary" onClick={() => void restore(v)}>
                        Restore
                      </button>
                      <button type="button" onClick={() => setPreview(props.pane, null)}>
                        Back to current
                      </button>
                    </div>
                  </Show>
                </li>
              );
            }}
          </For>
        </ol>
        <Show when={(versions()?.length ?? 0) > limit()}>
          <button type="button" class="history-more" onClick={() => setLimit(limit() + PAGE)}>
            Show older versions ({(versions()?.length ?? 0) - limit()} more)
          </button>
        </Show>
      </Show>
    </div>
  );
}

/** Whether `doc` is a note as first written: the server stamps `created` and `updated` alike then. */
function isCreation(doc: NoteDoc): boolean {
  const { created, updated } = doc.fields;
  return created === undefined || created === updated;
}
