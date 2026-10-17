import { A, useParams } from "@solidjs/router";
import { Match, Show, Switch, createEffect, createResource, createSignal, onCleanup } from "solid-js";
import { type AssetUsage, ApiError, type UsingNote, api, assetUrl, usingNotes } from "../api";
import {
  assetsChanged,
  assetsRevision,
  dateText,
  kindIcon,
  kindLabel,
  knowAsset,
  sizeText,
  usedText,
} from "../assetLibrary";
import { changes } from "../changes";
import { deleteUnusedAsset, undoDelete } from "../components/AssetUsage";
import { TypeIcon } from "../components/Icon";
import { showToast } from "../components/Toasts";
import { UsingNotes } from "../components/UsingNotes";
import { PdfViewer } from "../pdf/PdfViewer";
import { focusPanel, toggleSidebar } from "../sidebars";
import { navigatePane } from "../workspace/store";
import { usePane } from "../workspace/pane";
import { errorText } from "./TypeEditor";
import "./assets.css";

/**
 * One uploaded file, opened as a note is: a large preview, what it is, and Download, Rename and
 * Delete. The right sidebar lists the notes using it.
 */
export function AssetView() {
  const params = useParams();
  const pane = usePane();
  const id = () => (params.id ?? "").toUpperCase();
  const [usage, { refetch }] = createResource(
    () => [id(), changes(), assetsRevision()] as const,
    ([i]) => api.assetUsage(i),
  );

  createEffect(() => {
    const u = usage.latest;
    if (!u) return;
    knowAsset(u.id, u.filename);
    pane?.setTitle({ title: u.filename, icon: kindIcon(u.kind) });
  });
  onCleanup(() => pane?.setTitle(null));

  return (
    <section class="asset-page">
      <Show when={usage.error && !usage.latest}>
        <p class="error" role="alert">
          {usage.error instanceof ApiError && usage.error.status === 404
            ? "There is no such file. It may have been deleted."
            : errorText(usage.error)}
        </p>
        <p>
          <A href="/assets">Back to images and files</A>
        </p>
      </Show>
      <Show when={usage.latest} keyed>
        {(u) => <AssetBody usage={u} pane={pane?.id} refetch={() => void refetch()} />}
      </Show>
    </section>
  );
}

function AssetBody(props: { usage: AssetUsage; pane: string | undefined; refetch: () => void }) {
  const u = () => props.usage;
  const [renaming, setRenaming] = createSignal(false);
  const [name, setName] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  // A delete waiting on the owner agreeing to take the file out of these notes.
  const [pending, setPending] = createSignal<UsingNote[] | null>(null);

  function startRename() {
    setName(u().filename);
    setError(null);
    setRenaming(true);
  }

  async function rename() {
    const next = name().trim();
    if (!next || next === u().filename) {
      setRenaming(false);
      return;
    }
    setBusy(true);
    try {
      await api.renameAsset(u().id, next);
      setRenaming(false);
      assetsChanged();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove(unlink: boolean) {
    setError(null);
    if (u().used_by === 0) {
      await deleteUnusedAsset(u(), props.pane);
      return;
    }
    if (u().trashed.length > 0) {
      setError(
        `${u().filename} cannot be deleted while a note in the trash uses it. Restore that note and take the file out of it first.`,
      );
      return;
    }
    if (u().types.length > 0) {
      setError(`${u().filename} cannot be deleted while a type definition uses it: ${u().types.join(", ")}.`);
      return;
    }
    if (!unlink) {
      setPending(u().notes.map((n) => ({ id: n.id, type: n.type, title: n.title })));
      return;
    }
    setBusy(true);
    try {
      const out = await api.deleteAsset(u().id, true);
      const n = out.notes?.length ?? 0;
      assetsChanged();
      if (props.pane) navigatePane(props.pane, "/assets");
      showToast(`Deleted ${u().filename} and took it out of ${n} ${n === 1 ? "note" : "notes"}.`, {
        label: "Undo",
        run: () => void undoDelete([u().id]),
      });
    } catch (e) {
      const notes = usingNotes(e);
      if (notes.length) setPending(notes);
      setError(errorText(e));
      props.refetch();
    } finally {
      setBusy(false);
    }
  }

  function showUsers() {
    if (typeof window.matchMedia === "function" && window.matchMedia("(max-width: 600px)").matches) toggleSidebar("right", true);
    else focusPanel("properties");
    queueMicrotask(() => document.getElementById("usage-panel")?.focus());
  }

  return (
    <>
      <header class="note-header asset-header">
        <Show
          when={renaming()}
          fallback={
            <h1 class="type-list-title asset-title">
              <TypeIcon name={kindIcon(u().kind)} size={24} />
              <span>{u().filename}</span>
            </h1>
          }
        >
          <form
            class="asset-rename"
            onSubmit={(e) => {
              e.preventDefault();
              void rename();
            }}
          >
            <input
              aria-label="File name"
              value={name()}
              ref={(el) => queueMicrotask(() => el.select())}
              onInput={(e) => setName(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  setRenaming(false);
                }
              }}
            />
            <button type="submit" class="primary" disabled={busy()}>
              Save
            </button>
            <button type="button" onClick={() => setRenaming(false)}>
              Cancel
            </button>
          </form>
        </Show>
        <div class="actions">
          <a class="button" href={assetUrl(u().id)} download={u().filename}>
            <TypeIcon name="download" size={16} />
            <span>Download</span>
          </a>
          <button type="button" onClick={startRename} disabled={renaming()}>
            <TypeIcon name="pencil" size={16} />
            <span>Rename</span>
          </button>
          <button type="button" class="danger" disabled={busy()} onClick={() => void remove(false)}>
            <TypeIcon name="trash-2" size={16} />
            <span>Delete</span>
          </button>
        </div>
      </header>

      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={pending()}>
        {(notes) => (
          <div class="confirm-panel" role="alertdialog" aria-label={`Delete ${u().filename}`}>
            <p>
              {notes().length === 1 ? "This note uses" : `These ${notes().length} notes use`} {u().filename}. Deleting it takes
              it out of {notes().length === 1 ? "that note" : "each of them"}.
            </p>
            <UsingNotes notes={notes()} label="Notes using it" />
            <div class="actions">
              <button type="button" class="danger" disabled={busy()} onClick={() => void remove(true)}>
                Remove from notes and delete
              </button>
              <button type="button" disabled={busy()} onClick={() => setPending(null)}>
                Cancel
              </button>
            </div>
          </div>
        )}
      </Show>

      <div class="asset-preview" data-testid="asset-preview">
        <Switch
          fallback={
            <div class="asset-file-card">
              <TypeIcon name={kindIcon(u().kind)} size={56} />
              <span class="asset-file-card-name">{u().filename}</span>
              <span class="muted">
                {kindLabel(u().filename, u().kind)}, {sizeText(u().size)}
              </span>
              <a class="button primary" href={assetUrl(u().id)} download={u().filename}>
                Download
              </a>
            </div>
          }
        >
          <Match when={u().kind === "image"}>
            <a href={assetUrl(u().id)} target="_blank" rel="noopener" class="asset-preview-image-link" title="Open full size">
              <img class="asset-preview-image" src={assetUrl(u().id)} alt={u().filename} />
            </a>
          </Match>
          <Match when={u().kind === "pdf"}>
            <PdfViewer url={assetUrl(u().id)} filename={u().filename} height={680} />
          </Match>
          <Match when={u().kind === "audio"}>
            <audio class="asset-preview-audio" controls preload="metadata" src={assetUrl(u().id)} />
          </Match>
          <Match when={u().kind === "video"}>
            <video class="asset-preview-video" controls preload="metadata" src={assetUrl(u().id)} />
          </Match>
        </Switch>
      </div>

      <dl class="asset-meta" aria-label="About this file">
        <dt>File name</dt>
        <dd>{u().filename}</dd>
        <dt>Type</dt>
        <dd>
          {kindLabel(u().filename, u().kind)} <span class="muted">({u().content_type.split(";")[0]})</span>
        </dd>
        <dt>Size</dt>
        <dd>
          {sizeText(u().size)} <span class="muted">({u().size.toLocaleString("en-US")} bytes)</span>
        </dd>
        <Show when={u().uploaded_ms}>
          <dt>Uploaded</dt>
          <dd>
            {dateText(u().uploaded_ms)}{" "}
            <span class="muted">
              {new Date(u().uploaded_ms!).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}
            </span>
          </dd>
        </Show>
        <dt>Used by</dt>
        <dd>
          <button type="button" class="link-button" onClick={showUsers}>
            {usedText(u().used_by)}
          </button>
        </dd>
        <dt>Path</dt>
        <dd>
          <code>{u().path}</code>
        </dd>
      </dl>
    </>
  );
}
