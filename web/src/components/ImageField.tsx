// An asset field that takes only images (`accept = "image"`), such as a person's photo: a square
// tile showing the image, or a placeholder when the field is empty. The tile opens a dialog that
// uploads an image, by the file picker or by dropping one, or picks one already in the vault.

import { For, Show, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { type AssetSummary, api, assetUrl } from "../api";
import { FilePicker, ImageIcon, isImageFile, upload } from "../blocks/MediaBlocks";

/** The placeholder's text: "Add photo" for a field labelled Photo, else "Add image". */
export function addImageText(label: string): string {
  return label.trim().toLowerCase() === "photo" ? "Add photo" : "Add image";
}

/** An image asset's thumbnail, for cards, tables and read-only places. */
export function ImageThumb(props: { id: string; alt: string; size?: "small" | "card" }) {
  return (
    <img
      class="image-thumb"
      classList={{ "image-thumb-small": props.size === "small" }}
      src={assetUrl(props.id)}
      alt={props.alt}
      loading="lazy"
      data-testid="image-thumb"
    />
  );
}

export interface ImageFieldProps {
  label: string;
  id?: string;
  value: string | undefined;
  disabled?: boolean;
  onChange: (id: string | undefined) => void;
}

/** The image tile, with Remove and Open full size once it holds an image. */
export function ImageField(props: ImageFieldProps) {
  const [open, setOpen] = createSignal(false);
  let tile!: HTMLButtonElement;
  function close() {
    setOpen(false);
    queueMicrotask(() => tile?.focus());
  }
  return (
    <div class="image-field" data-testid="image-field">
      <button
        ref={tile}
        type="button"
        id={props.id}
        class="image-tile"
        classList={{ empty: !props.value }}
        disabled={props.disabled}
        aria-label={props.value ? `Change ${props.label}` : addImageText(props.label)}
        title={props.value ? `Change ${props.label}` : addImageText(props.label)}
        aria-haspopup="dialog"
        onClick={() => setOpen(true)}
      >
        <Show
          when={props.value}
          fallback={
            <span class="image-tile-placeholder">
              <ImageIcon />
              <span>{addImageText(props.label)}</span>
            </span>
          }
        >
          {(id) => <img src={assetUrl(id())} alt={props.label} />}
        </Show>
      </button>
      <Show when={props.value && !props.disabled}>
        <div class="image-tile-actions">
          <a href={assetUrl(props.value!)} target="_blank" rel="noopener">
            Open full size
          </a>
          <button type="button" onClick={() => props.onChange(undefined)}>
            Remove
          </button>
        </div>
      </Show>
      <Show when={open()}>
        <ImagePickerDialog
          label={props.label}
          current={props.value}
          onPick={(id) => {
            props.onChange(id);
            close();
          }}
          onClose={close}
        />
      </Show>
    </div>
  );
}

/** The dialog: Upload (or drop) an image, or choose one of the vault's images. */
export function ImagePickerDialog(props: {
  label: string;
  current?: string;
  onPick: (id: string) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = createSignal("");
  const [images] = createResource(query, async (q): Promise<AssetSummary[]> => {
    const page = await api.assetLibrary({ filter: "image", q, sort: "uploaded", dir: "desc", limit: 60 });
    return page.assets.filter((a) => isImageFile(a.filename));
  });
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [dragging, setDragging] = createSignal(false);
  let dialog!: HTMLDivElement;
  let search!: HTMLInputElement;
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));

  onMount(() => search.focus());

  async function uploadFile(file: File) {
    setError(null);
    if (!isImageFile(file.name) && !file.type.startsWith("image/")) {
      setError(`${file.name} is not an image.`);
      return;
    }
    setBusy(true);
    try {
      const a = await upload(file);
      props.onPick(a.id);
    } catch (e) {
      setError(`Could not upload ${file.name}: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      class="capture-backdrop image-picker-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) props.onClose();
      }}
    >
      <div
        ref={dialog}
        class="capture-dialog image-picker"
        classList={{ dragging: dragging() }}
        role="dialog"
        aria-modal="true"
        aria-label={`Choose ${props.label}`}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            props.onClose();
          }
        }}
        onDragOver={(e) => {
          if ([...(e.dataTransfer?.types ?? [])].includes("Files")) {
            e.preventDefault();
            setDragging(true);
          }
        }}
        onDragLeave={(e) => {
          if (!dialog.contains(e.relatedTarget as Node | null)) setDragging(false);
        }}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          const file = e.dataTransfer?.files?.[0];
          if (file) void uploadFile(file);
        }}
      >
        <header class="capture-head">
          <h2>Choose {props.label.toLowerCase()}</h2>
          <button type="button" class="capture-x" aria-label="Close" title="Close (Esc)" onClick={props.onClose}>
            ×
          </button>
        </header>
        <div class="image-picker-upload">
          <FilePicker
            label={`Upload an image to ${props.label}`}
            text={busy() ? "Uploading…" : "Upload"}
            accept="image/*"
            onFile={(file) => void uploadFile(file)}
          />
          <span class="muted">or drop an image here</span>
        </div>
        <Show when={error()}>
          {(e) => (
            <p class="error" role="alert">
              {e()}
            </p>
          )}
        </Show>
        <input
          ref={search}
          type="search"
          class="image-picker-search"
          aria-label="Search images"
          placeholder="Search images"
          onInput={(e) => {
            const v = e.currentTarget.value;
            clearTimeout(timer);
            timer = setTimeout(() => setQuery(v), 150);
          }}
        />
        <Show when={!images.loading || images()} fallback={<p class="muted">Loading…</p>}>
          <Show when={(images() ?? []).length > 0} fallback={<p class="muted">{query() ? "No images match." : "No images yet."}</p>}>
            <ul class="image-picker-grid" aria-label="Images">
              <For each={images()}>
                {(a) => (
                  <li>
                    <button
                      type="button"
                      class="image-picker-item"
                      classList={{ current: a.id === props.current }}
                      aria-label={a.filename}
                      aria-pressed={a.id === props.current}
                      title={a.filename}
                      onClick={() => props.onPick(a.id)}
                    >
                      <img src={assetUrl(a.id)} alt="" loading="lazy" />
                      <span class="image-picker-name">{a.filename}</span>
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </Show>
      </div>
    </div>
  );
}
