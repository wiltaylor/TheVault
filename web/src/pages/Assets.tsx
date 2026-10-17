import { A } from "@solidjs/router";
import { For, Match, Show, Switch, createEffect, createResource, createSignal, on, onCleanup } from "solid-js";
import {
  type AssetCleanup,
  type AssetFilter,
  type AssetPage,
  type AssetSummary,
  type TrashedAsset,
  api,
  assetUrl,
} from "../api";
import {
  ASSET_PAGE,
  assetsChanged,
  assetsRevision,
  dateText,
  kindIcon,
  kindOf,
  kindLabel,
  knowAsset,
  loadAssetsView,
  saveAssetsView,
  sizeText,
  usedText,
} from "../assetLibrary";
import { changes } from "../changes";
import { SearchIcon, TypeIcon } from "../components/Icon";
import { fileCount, undoDelete } from "../components/AssetUsage";
import { showToast } from "../components/Toasts";
import { PdfThumb } from "../pdf/PdfPreview";
import { type ListSort, type TypeListView, nextSort } from "../typeList";
import { errorText } from "./TypeEditor";
import "./assets.css";

/** How long typing waits before the search runs. */
const TYPING_MS = 250;

const FILTERS: { id: AssetFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "image", label: "Images" },
  { id: "pdf", label: "PDFs" },
  { id: "other", label: "Other" },
  { id: "unused", label: "Unused" },
];


/**
 * Every uploaded file (D22), as cards or a sortable list like a type's notes, with a search over
 * file names and PDF text, filters by kind and for what nothing uses, and a cleanup that deletes
 * the unused files into the asset trash, from which they can be restored for 30 days.
 */
export function Assets() {
  const [view, setView] = createSignal<TypeListView>(loadAssetsView());
  const [filter, setFilter] = createSignal<AssetFilter>("all");
  const [draft, setDraft] = createSignal("");
  const [text, setText] = createSignal("");
  const [sort, setSort] = createSignal<ListSort | null>(null);
  const [extra, setExtra] = createSignal<AssetSummary[]>([]);
  const [next, setNext] = createSignal<string | null>(null);
  const [loadingMore, setLoadingMore] = createSignal(false);
  const [cleanup, setCleanup] = createSignal<AssetCleanup | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [showTrash, setShowTrash] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));

  function choose(v: TypeListView) {
    setView(v);
    saveAssetsView(v);
  }

  function type(value: string) {
    setDraft(value);
    clearTimeout(timer);
    timer = setTimeout(() => setText(draft()), TYPING_MS);
  }

  const query = () => {
    const s = sort();
    return { filter: filter(), q: text(), sort: s?.key ?? "uploaded", dir: s?.dir, limit: ASSET_PAGE };
  };

  const [page] = createResource(
    () => [query(), changes(), assetsRevision()] as const,
    async ([q]): Promise<AssetPage> => {
      const p = await api.assetLibrary(q);
      for (const a of p.assets) knowAsset(a.id, a.filename);
      return p;
    },
  );
  // A new first page starts the list again.
  createEffect(
    on(
      () => page.latest,
      (p) => {
        setExtra([]);
        setNext(p?.next ?? null);
      },
    ),
  );
  const rows = () => [...(page.latest?.assets ?? []), ...extra()];

  async function more() {
    const cursor = next();
    if (!cursor) return;
    setLoadingMore(true);
    try {
      const p = await api.assetLibrary({ ...query(), cursor });
      for (const a of p.assets) knowAsset(a.id, a.filename);
      setExtra((list) => [...list, ...p.assets]);
      setNext(p.next);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setLoadingMore(false);
    }
  }

  const [trash, { refetch: refetchTrash }] = createResource(
    () => [showTrash(), assetsRevision()] as const,
    ([shown]) => (shown ? api.assetTrash() : Promise.resolve([] as TrashedAsset[])),
  );

  const unused = () => page.latest?.unused;
  const searching = () => text().trim() !== "";

  const status = () => {
    const p = page.latest;
    if (!p) return "Loading…";
    const shown = rows().length;
    if (searching()) return p.total === 1 ? "1 match" : `${p.total.toLocaleString("en-US")} matches`;
    if (p.total > shown) return `${fileCount(p.total)}, showing ${shown.toLocaleString("en-US")}`;
    return fileCount(p.total);
  };

  async function askCleanup() {
    setBusy(true);
    setError(null);
    try {
      setCleanup(await api.cleanupAssets(true));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function runCleanup(plan: AssetCleanup) {
    setBusy(true);
    setError(null);
    try {
      const done = await api.cleanupAssets(
        false,
        plan.assets.map((a) => a.id),
      );
      setCleanup(null);
      const ids = done.assets.map((a) => a.id);
      assetsChanged();
      showToast(`Deleted ${fileCount(done.count)} (${sizeText(done.size)}). Restorable for ${done.purge_days} days.`, {
        label: "Undo",
        run: () => void undoDelete(ids, () => assetsChanged()),
      });
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function restore(t: TrashedAsset) {
    await undoDelete([t.id], () => assetsChanged());
    await refetchTrash();
  }

  return (
    <section class="list-page type-list assets-page">
      <header class="note-header">
        <h1 class="type-list-title">
          <TypeIcon name="images" size={24} />
          Images and files
        </h1>
        <div class="actions">
          <Show when={(unused()?.count ?? 0) > 0}>
            <button
              type="button"
              class="assets-cleanup"
              disabled={busy()}
              title="Delete every file no note uses"
              onClick={() => void askCleanup()}
            >
              <TypeIcon name="sparkles" size={16} />
              <span>
                Clean up {unused()!.count === 1 ? "1 unused file" : `${unused()!.count.toLocaleString("en-US")} unused files`} (
                {sizeText(unused()!.size)})
              </span>
            </button>
          </Show>
          <button type="button" aria-pressed={showTrash()} onClick={() => setShowTrash(!showTrash())}>
            <TypeIcon name="history" size={16} />
            <span>Recently deleted</span>
          </button>
        </div>
      </header>

      <Show when={showTrash()}>
        <section class="assets-trash" aria-label="Recently deleted files">
          <h2>Recently deleted</h2>
          <p class="muted">Deleted files stay here for 30 days, then go for good.</p>
          <Show when={(trash.latest ?? []).length > 0} fallback={<p class="muted">Nothing deleted lately.</p>}>
            <ul>
              <For each={trash.latest}>
                {(t) => (
                  <li>
                    <TypeIcon name={kindIcon(kindOf(t.filename))} size={16} />
                    <span class="assets-trash-name">{t.filename}</span>
                    <span class="muted">
                      {sizeText(t.size)}, deleted {dateText(t.deleted_ms)}
                    </span>
                    <button type="button" onClick={() => void restore(t)} aria-label={`Restore ${t.filename}`}>
                      Restore
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </section>
      </Show>

      <div class="type-list-toolbar">
        <div class="search-bar type-list-search" role="search">
          <SearchIcon />
          <input
            type="search"
            placeholder="Search file names and PDF text"
            aria-label="Search images and files"
            value={draft()}
            onInput={(e) => type(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape" && draft()) {
                e.preventDefault();
                clearTimeout(timer);
                setDraft("");
                setText("");
              } else if (e.key === "Enter") {
                clearTimeout(timer);
                setText(draft());
              }
            }}
          />
        </div>
        <div class="segmented assets-filters" role="group" aria-label="Show">
          <For each={FILTERS}>
            {(f) => (
              <button type="button" aria-pressed={filter() === f.id} onClick={() => setFilter(f.id)}>
                <span>{f.label}</span>
                <Show when={page.latest}>
                  {(p) => <span class="assets-filter-count">{p().counts[f.id].toLocaleString("en-US")}</span>}
                </Show>
              </button>
            )}
          </For>
        </div>
        <div class="segmented type-list-views" role="group" aria-label="View">
          <button type="button" aria-pressed={view() === "cards"} onClick={() => choose("cards")}>
            <TypeIcon name="layout-grid" size={16} />
            <span>Cards</span>
          </button>
          <button type="button" aria-pressed={view() === "list"} onClick={() => choose("list")}>
            <TypeIcon name="list" size={16} />
            <span>List</span>
          </button>
        </div>
      </div>

      <p class="muted list-count type-list-status" data-testid="assets-status" aria-live="polite">
        {status()}
      </p>
      <Show when={error() ?? (page.error ? errorText(page.error) : null)}>
        {(e) => (
          <p class="error" role="alert">
            {e()}
          </p>
        )}
      </Show>

      <Show when={page.latest}>
        <Show
          when={rows().length > 0}
          fallback={
            <p class="muted type-list-empty">
              {searching() ? "No files match." : filter() === "unused" ? "Every file is used by a note." : "No files yet."}
            </p>
          }
        >
          <Switch>
            <Match when={view() === "cards"}>
              <AssetCards rows={rows()} />
            </Match>
            <Match when={view() === "list"}>
              <AssetTable rows={rows()} sort={sort()} onSort={(key) => setSort(nextSort(sort(), key))} />
            </Match>
          </Switch>
          <Show when={next()}>
            <button type="button" class="assets-more" disabled={loadingMore()} onClick={() => void more()}>
              {loadingMore() ? "Loading…" : "Show more"}
            </button>
          </Show>
        </Show>
      </Show>

      <Show when={cleanup()}>
        {(plan) => (
          <CleanupDialog plan={plan()} busy={busy()} onCancel={() => setCleanup(null)} onConfirm={() => void runCleanup(plan())} />
        )}
      </Show>
    </section>
  );
}

/** A file's picture: the image itself, a PDF's first page, or its kind's icon and extension. */
export function AssetThumb(props: { asset: AssetSummary; width: number }) {
  return (
    <Switch
      fallback={
        <span class="asset-thumb-icon">
          <TypeIcon name={kindIcon(props.asset.kind)} size={36} />
          <span>{kindLabel(props.asset.filename, props.asset.kind)}</span>
        </span>
      }
    >
      <Match when={props.asset.kind === "image"}>
        <img class="asset-thumb-image" src={assetUrl(props.asset.id)} alt="" loading="lazy" decoding="async" />
      </Match>
      <Match when={props.asset.kind === "pdf"}>
        <PdfThumb url={assetUrl(props.asset.id)} width={props.width} />
      </Match>
    </Switch>
  );
}

function UsedChip(props: { asset: AssetSummary }) {
  return (
    <span class="asset-used" classList={{ unused: props.asset.used_by === 0 }} data-testid="asset-used">
      {usedText(props.asset.used_by)}
    </span>
  );
}

/** One card per file; the whole card opens the file's page. */
function AssetCards(props: { rows: AssetSummary[] }) {
  return (
    <ul class="asset-grid" aria-label="Images and files" data-testid="asset-cards">
      <For each={props.rows}>
        {(a) => (
          <li class="query-card type-note-card asset-card" data-asset={a.id} data-testid="asset-card">
            <div class="asset-card-thumb">
              <AssetThumb asset={a} width={150} />
            </div>
            <A class="type-note-link asset-card-name" href={`/assets/${a.id}`}>
              {a.filename}
            </A>
            <span class="asset-card-meta muted">
              {kindLabel(a.filename, a.kind)}, {sizeText(a.size)}
              <Show when={a.uploaded_ms}>, {dateText(a.uploaded_ms)}</Show>
            </span>
            <UsedChip asset={a} />
          </li>
        )}
      </For>
    </ul>
  );
}

/**
 * A click anywhere on a row clicks its name link, keeping the button and modifier keys, so the
 * workspace opens the file here, or in a background tab for a middle or Ctrl+click.
 */
function forward(e: MouseEvent) {
  const row = e.currentTarget as HTMLElement;
  if (e.defaultPrevented || (e.target as Element).closest("a, button")) return;
  if (getSelection()?.toString()) return;
  row.querySelector("a")?.dispatchEvent(new MouseEvent(e.type, e));
}

const HEADS = [
  { key: "name", label: "Name" },
  { key: "kind", label: "Type" },
  { key: "size", label: "Size" },
  { key: "uploaded", label: "Uploaded" },
  { key: "used", label: "Used by" },
];

/** One row per file, sorted on the server by any column. */
function AssetTable(props: { rows: AssetSummary[]; sort: ListSort | null; onSort: (key: string) => void }) {
  const ariaSort = (key: string) => {
    const s = props.sort;
    if (s?.key !== key) return "none";
    return s.dir === "asc" ? "ascending" : "descending";
  };
  return (
    <div class="type-table-wrap">
      <table class="type-table asset-table" data-testid="asset-table" aria-label="Images and files">
        <thead>
          <tr>
            <For each={HEADS}>
              {(h) => (
                <th scope="col" aria-sort={ariaSort(h.key)}>
                  <button type="button" class="type-table-sort" onClick={() => props.onSort(h.key)}>
                    {h.label}
                    <span class="type-table-arrow" aria-hidden="true">
                      {props.sort?.key === h.key ? (props.sort.dir === "asc" ? "↑" : "↓") : ""}
                    </span>
                  </button>
                </th>
              )}
            </For>
          </tr>
        </thead>
        <tbody>
          <For each={props.rows}>
            {(a) => (
              <tr data-testid="asset-row" data-asset={a.id} onClick={forward} onAuxClick={forward}>
                <td class="type-table-title">
                  <A class="type-table-link" href={`/assets/${a.id}`}>
                    <TypeIcon name={kindIcon(a.kind)} size={16} />
                    <span>{a.filename}</span>
                  </A>
                </td>
                <td>{kindLabel(a.filename, a.kind)}</td>
                <td class="asset-table-num">{sizeText(a.size)}</td>
                <td>{dateText(a.uploaded_ms)}</td>
                <td>
                  <UsedChip asset={a} />
                </td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  );
}

/** The confirm before a cleanup: how many files, how much space, and which. */
function CleanupDialog(props: { plan: AssetCleanup; busy: boolean; onCancel: () => void; onConfirm: () => void }) {
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
        class="capture-dialog assets-cleanup-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="cleanup-heading"
        onKeyDown={onKey}
      >
        <header class="capture-head">
          <h2 id="cleanup-heading">Delete unused files</h2>
        </header>
        <Show
          when={props.plan.count > 0}
          fallback={<p>Every file is used by a note, a note in the trash or a type, so there is nothing to clean up.</p>}
        >
          <p data-testid="cleanup-summary">
            Delete {fileCount(props.plan.count)} that no note uses, {sizeText(props.plan.size)} in all? They can be
            restored from Recently deleted for {props.plan.purge_days} days.
          </p>
          <ul class="assets-cleanup-list" aria-label="Files to delete">
            <For each={props.plan.assets.slice(0, 50)}>
              {(a) => (
                <li>
                  <span>{a.filename}</span>
                  <span class="muted">{sizeText(a.size)}</span>
                </li>
              )}
            </For>
            <Show when={props.plan.count > 50}>
              <li class="muted">and {(props.plan.count - 50).toLocaleString("en-US")} more</li>
            </Show>
          </ul>
        </Show>
        <div class="actions">
          <button type="button" onClick={props.onCancel}>
            Cancel
          </button>
          <Show when={props.plan.count > 0}>
            <button type="button" class="danger" disabled={props.busy} onClick={props.onConfirm}>
              Delete {fileCount(props.plan.count)}
            </button>
          </Show>
        </div>
      </div>
    </div>
  );
}
