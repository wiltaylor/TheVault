import { For, Show, createResource, createSignal } from "solid-js";
import { type Asset, ApiError, type UsingNote, api, assetUrl, usingNotes } from "../api";
import { UsingNotes } from "../components/UsingNotes";
import { errorText } from "./TypeEditor";

/** A size in bytes, in the largest unit that keeps it at least 1. */
function sizeText(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * Every uploaded file (D22). Deleting one that notes name asks first, listing those notes, and
 * then takes it out of each of them in the same commit.
 */
export function Assets() {
  const [assets, { refetch }] = createResource(api.assets);
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  // An asset whose delete waits on the owner agreeing to take it out of these notes.
  const [pending, setPending] = createSignal<{ asset: Asset; notes: UsingNote[] } | null>(null);

  async function remove(asset: Asset, unlink: boolean) {
    if (!unlink && !confirm(`Delete ${asset.filename}?`)) return;
    setBusy(true);
    setError(null);
    setStatus(null);
    try {
      const out = await api.deleteAsset(asset.id, unlink);
      setPending(null);
      const n = out.notes?.length ?? 0;
      setStatus(
        n ? `Deleted ${asset.filename} and took it out of ${n} ${n === 1 ? "note" : "notes"}.` : `Deleted ${asset.filename}.`,
      );
      await refetch();
    } catch (e) {
      const notes = usingNotes(e);
      const byType = e instanceof ApiError && Array.isArray(e.body.by) && e.body.by.some((b) => String(b).startsWith("types/"));
      if (e instanceof ApiError && e.body.error === "asset_in_use" && !byType && notes.length > 0) {
        setPending({ asset, notes });
      } else if (byType) {
        setPending(null);
        setError(`${asset.filename} is named by a type definition, so it cannot be deleted: ${(e as ApiError).body.by}.`);
      } else {
        setPending(null);
        setError(errorText(e));
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <section class="list-page">
      <header class="note-header">
        <h1>Assets</h1>
      </header>
      <p class="muted">Files uploaded into image and file blocks, asset fields and canvases.</p>
      <Show when={pending()}>
        {(p) => (
          <div class="confirm-panel" role="alertdialog" aria-label={`Delete ${p().asset.filename}`}>
            <p>
              {p().notes.length === 1 ? "This note uses" : `These ${p().notes.length} notes use`} {p().asset.filename}.
              Deleting it takes it out of {p().notes.length === 1 ? "that note" : "each of them"}.
            </p>
            <UsingNotes notes={p().notes} label="Notes using it" />
            <div class="actions">
              <button type="button" class="danger" disabled={busy()} onClick={() => void remove(p().asset, true)}>
                Remove from notes and delete
              </button>
              <button type="button" disabled={busy()} onClick={() => setPending(null)}>
                Cancel
              </button>
            </div>
          </div>
        )}
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={status()}>
        <p role="status">{status()}</p>
      </Show>
      <Show when={assets()}>
        {(list) => (
          <Show when={list().length > 0} fallback={<p class="muted">No assets yet.</p>}>
            <table class="admin-table" aria-label="Assets">
              <thead>
                <tr>
                  <th>File</th>
                  <th>Size</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                <For each={list()}>
                  {(a) => (
                    <tr data-asset={a.id}>
                      <td>
                        <a href={assetUrl(a.id)} target="_blank" rel="noopener">
                          {a.filename}
                        </a>
                      </td>
                      <td>{sizeText(a.size)}</td>
                      <td class="actions">
                        <button
                          type="button"
                          class="danger"
                          aria-label={`Delete ${a.filename}`}
                          disabled={busy()}
                          onClick={() => void remove(a, false)}
                        >
                          Delete
                        </button>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Show>
        )}
      </Show>
    </section>
  );
}
