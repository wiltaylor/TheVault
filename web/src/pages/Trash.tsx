import { useNavigate } from "@solidjs/router";
import { For, Show, createEffect, createResource, createSignal } from "solid-js";
import { type TrashEntry, api } from "../api";
import { authorText, when, wclTitle } from "../admin/format";
import { knowTitle } from "../editor/titles";
import { appInfo } from "../inApp";
import { typeDef } from "../session";
import { errorText } from "./TypeEditor";

/** What the confirm dialog is asking: to empty the trash, or to purge one note. */
type Purge = { all: true; count: number } | { all: false; entry: TrashEntry };

const titleOf = (t: TrashEntry) => wclTitle(t.wcl) ?? t.id;

const noteCount = (n: number) => (n === 1 ? "1 note" : `${n.toLocaleString("en-US")} notes`);

/**
 * Deleted notes, newest first, each restorable to where it was (D13). The signed-in user can also
 * empty the trash or delete one note forever; inside the apps, whose embedded server cannot tell
 * the user from its device token, that is left to the web app (ai D21).
 */
export function Trash() {
  const navigate = useNavigate();
  const [entries, { refetch }] = createResource(api.trash);
  const [app] = createResource(appInfo);
  const [error, setError] = createSignal<string | null>(null);
  const [purge, setPurge] = createSignal<Purge | null>(null);
  const [busy, setBusy] = createSignal(false);

  /** Purging is offered only in a browser, where the caller is the signed-in session. */
  const canPurge = () => app.state === "ready" && app() === null;

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

  async function confirmPurge(p: Purge) {
    setError(null);
    setBusy(true);
    try {
      if (p.all) await api.emptyTrash();
      else await api.purgeFromTrash(p.entry.id);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
      setPurge(null);
      await refetch();
    }
  }

  const sorted = () => [...(entries() ?? [])].sort((a, b) => b.deleted_at - a.deleted_at);

  return (
    <section class="list-page">
      <header class="note-header">
        <h1>Trash</h1>
        <Show when={canPurge() && sorted().length > 0}>
          <button type="button" class="danger" onClick={() => setPurge({ all: true, count: sorted().length })}>
            Empty trash
          </button>
        </Show>
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
                    <td>{titleOf(t)}</td>
                    <td>{typeDef(t.type)?.label ?? t.type}</td>
                    <td>
                      {when(t.deleted_at)} <span class="muted">by {authorText(t.deleted_by)}</span>
                    </td>
                    <td>
                      <button type="button" onClick={() => void restore(t)}>
                        Restore
                      </button>
                      <Show when={canPurge()}>
                        {" "}
                        <button
                          type="button"
                          class="danger"
                          aria-label={`Delete ${titleOf(t)} forever`}
                          onClick={() => setPurge({ all: false, entry: t })}
                        >
                          Delete forever
                        </button>
                      </Show>
                    </td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </Show>
      </Show>
      <Show when={purge()}>
        {(p) => (
          <PurgeDialog
            purge={p()}
            busy={busy()}
            onCancel={() => setPurge(null)}
            onConfirm={() => void confirmPurge(p())}
          />
        )}
      </Show>
    </section>
  );
}

/** The confirm before purging: emptying names how many notes go, one note names its title. */
function PurgeDialog(props: {
  purge: Purge;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  let dialog!: HTMLDivElement;
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      props.onCancel();
    }
  };
  createEffect(() => dialog?.querySelector<HTMLButtonElement>("button.danger")?.focus());
  return (
    <div
      class="capture-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) props.onCancel();
      }}
    >
      <div
        ref={dialog}
        class="capture-dialog trash-purge-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="purge-heading"
        aria-describedby="purge-summary"
        onKeyDown={onKey}
      >
        <header class="capture-head">
          <h2 id="purge-heading">{props.purge.all ? "Empty trash" : "Delete forever"}</h2>
        </header>
        <p id="purge-summary">
          {props.purge.all
            ? `Delete ${noteCount(props.purge.count)} in the trash forever, with their history?`
            : `Delete “${titleOf(props.purge.entry)}” forever, with its history?`}{" "}
          This cannot be undone. Files only these notes used move to Recently deleted in Images and files.
        </p>
        <div class="actions">
          <button type="button" onClick={props.onCancel}>
            Cancel
          </button>
          <button type="button" class="danger" disabled={props.busy} onClick={props.onConfirm}>
            {props.purge.all ? "Empty trash" : "Delete forever"}
          </button>
        </div>
      </div>
    </div>
  );
}
