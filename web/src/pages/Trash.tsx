import { useNavigate } from "@solidjs/router";
import { For, Show, createResource, createSignal } from "solid-js";
import { type TrashEntry, api } from "../api";
import { authorText, when, wclTitle } from "../admin/format";
import { knowTitle } from "../editor/titles";
import { typeDef } from "../session";
import { errorText } from "./TypeEditor";

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
