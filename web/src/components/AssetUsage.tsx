// What uses an asset, for the right sidebar's Used by panel beside an asset's page, and the
// delete both offer once nothing does.

import { A } from "@solidjs/router";
import { For, Show, createResource } from "solid-js";
import { type AssetUsage, api } from "../api";
import { assetsChanged, assetsRevision, howText, usedText } from "../assetLibrary";
import { changes } from "../changes";
import { errorText } from "../pages/TypeEditor";
import { typeDef } from "../session";
import { navigatePane } from "../workspace/store";
import { TypeIcon } from "./Icon";
import { showToast } from "./Toasts";

/** `n` files, with thousands separated. */
export function fileCount(n: number): string {
  return n === 1 ? "1 file" : `${n.toLocaleString("en-US")} files`;
}

/** Restore `ids` from the asset trash, saying how it went. */
export async function undoDelete(ids: string[], after?: () => void) {
  try {
    const out = await api.restoreAssets(ids);
    const n = out.restored.length;
    showToast(out.failed.length ? `Restored ${fileCount(n)}; ${out.failed.length} could not be.` : `Restored ${fileCount(n)}.`);
  } catch (e) {
    showToast(`Could not restore: ${errorText(e)}`);
  }
  assetsChanged();
  after?.();
}

/**
 * Delete an asset nothing uses, after the browser's confirm, into the asset trash, with Undo in
 * the toast. The pane showing it goes back to the library. False when it was not deleted.
 */
export async function deleteUnusedAsset(usage: AssetUsage, pane: string | undefined): Promise<boolean> {
  if (!confirm(`Delete ${usage.filename}? It can be restored for 30 days.`)) return false;
  try {
    await api.deleteAsset(usage.id);
  } catch (e) {
    showToast(`Could not delete ${usage.filename}: ${errorText(e)}`);
    return false;
  }
  assetsChanged();
  if (pane) navigatePane(pane, "/assets");
  showToast(`Deleted ${usage.filename}.`, {
    label: "Undo",
    run: () => void undoDelete([usage.id]),
  });
  return true;
}

/** Every note, trashed note and type using an asset, each a link. */
export function UsedByList(props: { usage: AssetUsage }) {
  return (
    <ul class="asset-users" aria-label="Notes using this file" data-testid="asset-users">
      <For each={props.usage.notes}>
        {(n) => {
          const t = () => typeDef(n.type);
          return (
            <li>
              <A href={`/notes/${n.id}`} class="asset-user-link" data-testid="asset-user">
                <TypeIcon name={t()?.icon} size={16} />
                <span class="asset-user-title">{n.title || "Untitled"}</span>
              </A>
              <span class="asset-user-how muted">
                {[t()?.label ?? n.type, ...n.how.map((h) => howText(h, t()))].join(" · ")}
                <Show when={n.archived}> · archived</Show>
              </span>
            </li>
          );
        }}
      </For>
      <For each={props.usage.trashed}>
        {(n) => (
          <li class="trashed">
            <A href="/trash" class="asset-user-link" data-testid="asset-user-trashed">
              <TypeIcon name={typeDef(n.type)?.icon} size={16} />
              <span class="asset-user-title">{n.title || "Untitled"}</span>
            </A>
            <span class="asset-user-how muted">{typeDef(n.type)?.label ?? n.type} · in trash</span>
          </li>
        )}
      </For>
      <For each={props.usage.types}>
        {(name) => (
          <li>
            <A href={`/settings/types/${encodeURIComponent(name)}/edit`} class="asset-user-link">
              <TypeIcon name={typeDef(name)?.icon} size={16} />
              <span class="asset-user-title">{typeDef(name)?.label ?? name}</span>
            </A>
            <span class="asset-user-how muted">Type definition</span>
          </li>
        )}
      </For>
    </ul>
  );
}

/** The right sidebar beside an asset's page: what uses it, or a Delete when nothing does. */
export function AssetUsagePanel(props: { id: string; pane: string }) {
  const [usage, { refetch }] = createResource(
    () => [props.id, changes(), assetsRevision()] as const,
    ([id]) => api.assetUsage(id),
  );
  return (
    <aside id="usage-panel" class="side-panel asset-usage-panel" aria-label="Used by" tabindex={-1}>
      <Show when={usage.error}>
        <p class="error">{errorText(usage.error)}</p>
      </Show>
      <Show when={usage.latest}>
        {(u) => (
          <Show
            when={u().used_by > 0}
            fallback={
              <div class="asset-usage-empty">
                <p class="muted">Not used by any note.</p>
                <button
                  type="button"
                  class="danger"
                  onClick={() => void deleteUnusedAsset(u(), props.pane).then((gone) => !gone && refetch())}
                >
                  Delete {u().filename}
                </button>
              </div>
            }
          >
            <p class="muted asset-usage-count">
              {usedText(u().used_by)}
            </p>
            <UsedByList usage={u()} />
          </Show>
        )}
      </Show>
    </aside>
  );
}
