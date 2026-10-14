import { A, useNavigate, useParams } from "@solidjs/router";
import { For, Show, createEffect, createResource, createSignal, on } from "solid-js";
import { ApiError, type Note, type TrashEntry, type Version, api } from "../api";
import { authorText, when, wclTitle } from "../admin/format";
import { formatValue } from "../components/Render";
import { knowTitle } from "../editor/titles";
import { typeDef } from "../session";
import { errorText } from "./TypeEditor";

/**
 * A note's versions, newest first (D13). Choosing one shows the file as it was, and restoring it
 * writes that content as a new version over the current one.
 */
export function NoteHistory() {
  const params = useParams();
  const navigate = useNavigate();
  const [note, setNote] = createSignal<Note | null>(null);
  const [versions, setVersions] = createSignal<Version[]>([]);
  const [chosen, setChosen] = createSignal<(Note & { author: unknown; at: number }) | null>(null);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  async function load(id: string) {
    const [n, v] = await Promise.all([api.note(id), api.history(id)]);
    setNote(n);
    setVersions([...v].reverse());
  }

  createEffect(
    on(
      () => params.id,
      async (id) => {
        setLoadError(null);
        setChosen(null);
        try {
          await load(id ?? "");
        } catch (e) {
          setLoadError(e instanceof ApiError && e.status === 404 ? "This note does not exist." : errorText(e));
        }
      },
    ),
  );

  async function choose(v: Version) {
    setError(null);
    try {
      setChosen(await api.version(params.id!, v.version));
    } catch (e) {
      setError(errorText(e));
    }
  }

  async function restore() {
    const c = chosen();
    const n = note();
    if (!c || !n) return;
    setBusy(true);
    setError(null);
    try {
      const saved = await api.restoreVersion(n.id, c.version, n.version);
      const title = saved.note.fields.title;
      if (typeof title === "string") knowTitle(saved.id, title, saved.type);
      navigate(`/notes/${n.id}`);
    } catch (e) {
      setError(errorText(e));
      if (e instanceof ApiError && e.status === 409) await load(n.id);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section class="def-editor history">
      <Show when={loadError()}>
        <p class="error" role="alert">
          {loadError()}
        </p>
      </Show>
      <Show when={note()}>
        {(n) => (
          <>
            <header class="note-header">
              <div>
                <p class="note-kind">History</p>
                <h1>
                  <A href={`/notes/${n().id}`}>{formatValue(n().note.fields.title) || "Untitled"}</A>
                </h1>
              </div>
            </header>
            <ol class="version-list" aria-label="Versions">
              <For each={versions()}>
                {(v) => (
                  <li classList={{ chosen: chosen()?.version === v.version }}>
                    <button type="button" class="link" onClick={() => void choose(v)}>
                      {when(v.at)}
                    </button>{" "}
                    <span class="muted">by {authorText(v.author)}</span>
                    <Show when={v.current}>
                      <span class="badge">current</span>
                    </Show>
                  </li>
                )}
              </For>
            </ol>
            <Show when={error()}>
              <p class="error" role="alert">
                {error()}
              </p>
            </Show>
            <Show when={chosen()}>
              {(c) => (
                <section class="preview" aria-label="Chosen version">
                  <h2>Version of {when(c().at)}</h2>
                  <pre class="note-wcl">{c().wcl}</pre>
                  <Show
                    when={c().version !== n().version}
                    fallback={<p class="muted">This is the current version.</p>}
                  >
                    <button type="button" class="primary" disabled={busy()} onClick={() => void restore()}>
                      Restore this version
                    </button>
                  </Show>
                </section>
              )}
            </Show>
          </>
        )}
      </Show>
    </section>
  );
}

/** Deleted notes, newest first, each restorable to where it was (D13). */
export function Trash() {
  const navigate = useNavigate();
  const [entries, { refetch }] = createResource(api.trash);
  const [error, setError] = createSignal<string | null>(null);

  async function restore(t: TrashEntry) {
    setError(null);
    try {
      const saved = await api.restoreFromTrash(t.id);
      const title = saved.note.fields.title;
      if (typeof title === "string") knowTitle(saved.id, title, saved.type);
      navigate(`/notes/${saved.id}`);
    } catch (e) {
      setError(errorText(e));
      await refetch();
    }
  }

  const sorted = () => [...(entries() ?? [])].sort((a, b) => b.deleted_at - a.deleted_at);

  return (
    <section class="list-page">
      <header class="note-header">
        <h1>Trash</h1>
      </header>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={entries()}>
        <Show when={sorted().length > 0} fallback={<p class="muted">The trash is empty.</p>}>
          <table class="admin-table" aria-label="Deleted notes">
            <thead>
              <tr>
                <th>Note</th>
                <th>Type</th>
                <th>Deleted</th>
                <th />
              </tr>
            </thead>
            <tbody>
              <For each={sorted()}>
                {(t) => (
                  <tr data-note={t.id}>
                    <td>{wclTitle(t.wcl) ?? t.id}</td>
                    <td>{typeDef(t.type)?.label ?? t.type}</td>
                    <td>
                      {when(t.deleted_at)} <span class="muted">by {authorText(t.deleted_by)}</span>
                    </td>
                    <td>
                      <button type="button" onClick={() => void restore(t)}>
                        Restore
                      </button>
                    </td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </Show>
      </Show>
    </section>
  );
}
