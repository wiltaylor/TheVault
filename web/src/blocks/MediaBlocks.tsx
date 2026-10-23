// Image and asset blocks. Both name an uploaded file by its asset ULID; the file itself lives at
// `assets/<ULID>/<filename>` and is uploaded through the assets API (D22).

import { For, Show, createContext, createResource, createSignal, onCleanup, onMount, useContext } from "solid-js";
import { Portal } from "solid-js/web";
import { type Asset, type Block, type FieldDef, type Json, api, assetUrl } from "../api";
import { FieldRefs, fillFieldRefs } from "../components/fieldRefs";
import { useControlBinding } from "../controls/binding";
import { templatePlaces } from "../notes";
import { VIEWER_HEIGHT, isPdfFile } from "../pdf/model";
import { CardIcon } from "../pdf/PdfIcons";
import { PdfThumbButton } from "../pdf/PdfPreview";
import { PdfViewer } from "../pdf/PdfViewer";
import { text, withItem, withText } from "./fields";

let known: Promise<Map<string, Asset>> | undefined;
// Asset ids the listing lacked, for which it has been fetched again already.
const refetched = new Set<string>();

/** Every asset by id, fetched once a page and kept up to date by `upload`. */
function assetsById(): Promise<Map<string, Asset>> {
  known ??= api
    .assets()
    .then((list) => new Map(list.map((a) => [a.id, a])))
    .catch((e) => {
      known = undefined;
      throw e;
    });
  return known;
}

/** Upload `file` as a new asset. */
export async function upload(file: File): Promise<Asset> {
  const asset = await api.uploadAsset(file);
  if (known) (await known.catch(() => new Map<string, Asset>())).set(asset.id, asset);
  return asset;
}

/** A hidden file input and the button that opens it; `onFile` gets the chosen file. */
export function FilePicker(props: { label: string; text: string; accept?: string; onFile: (file: File) => void }) {
  let input!: HTMLInputElement;
  return (
    <>
      <input
        ref={input}
        type="file"
        hidden
        accept={props.accept}
        onChange={(e) => {
          const file = e.currentTarget.files?.[0];
          e.currentTarget.value = "";
          if (file) props.onFile(file);
        }}
      />
      <button type="button" aria-label={props.label} title={props.label} onMouseDown={(e) => e.preventDefault()} onClick={() => input.click()}>
        {props.text}
      </button>
    </>
  );
}

/** File name endings served as images (crates/api/src/assets.rs, `content_type`). */
const IMAGE_EXTENSIONS = new Set(["png", "jpg", "jpeg", "gif", "webp", "avif", "svg"]);

/** Whether an asset named `filename` is an image, by its extension. */
export function isImageFile(filename: string): boolean {
  const dot = filename.lastIndexOf(".");
  return dot > 0 && IMAGE_EXTENSIONS.has(filename.slice(dot + 1).toLowerCase());
}

/** The asset ids a field value holds: one, a list, or none. */
function assetIds(v: Json | undefined): string[] {
  const list = Array.isArray(v) ? v : v === undefined || v === null ? [] : [v];
  return list.filter((x): x is string => typeof x === "string" && x !== "");
}

/** What an image block gets: `onChange` when it is edited, and the editor's selection and removal. */
export interface ImageBlockProps {
  block: Block;
  onChange?: (block: Block) => void;
  /** Whether the editor has the block selected, which shows its Edit and Remove buttons. */
  selected?: boolean;
  /** Delete the block from the content. */
  onRemove?: () => void;
}

/** The image in an image block: fixed, or a template's image from an asset field of the note. */
export function ImageBlock(props: ImageBlockProps) {
  return (
    <Show when={typeof props.block.fields?.field === "string"} fallback={<FixedImage {...props} />}>
      <FieldImage {...props} />
    </Show>
  );
}

const widthOf = (block: Block): number | undefined => {
  const w = block.fields?.width;
  return typeof w === "number" ? w : undefined;
};

const keepSelection = (e: MouseEvent) => e.preventDefault();

function PencilIcon() {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z" />
    </svg>
  );
}

function BinIcon() {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2M10 11v6M14 11v6" />
    </svg>
  );
}

/**
 * The Edit and Remove buttons over an edited image's top-right corner, shown while the block is
 * selected or holds the keyboard focus (styles.css). A press keeps the editor's selection.
 */
function ImageTools(props: { onEdit: () => void; onRemove?: () => void }) {
  return (
    <div class="block-image-tools" role="toolbar" aria-label="Image">
      <button type="button" aria-label="Edit image" title="Edit image" aria-haspopup="dialog" onMouseDown={keepSelection} onClick={() => props.onEdit()}>
        <PencilIcon />
      </button>
      <Show when={props.onRemove}>
        {(remove) => (
          <button type="button" aria-label="Remove image" title="Remove image" onMouseDown={keepSelection} onClick={() => remove()()}>
            <BinIcon />
          </button>
        )}
      </Show>
    </div>
  );
}

const FOCUSABLE = 'button:not([disabled]), input:not([type="hidden"]):not([hidden]), select:not([disabled]), a[href], [tabindex]:not([tabindex="-1"])';

/**
 * The image's properties in a modal dialog: alt text, caption, width and the file (a fixed image)
 * or the asset field (a template's image from a field, when `fields` lists the type's asset
 * fields). Save applies every change as one block; Cancel, Escape and a click outside discard
 * them. Focus returns to what opened it.
 */
function ImageDialog(props: { block: Block; fields?: FieldDef[]; onSave: (block: Block) => void; onClose: () => void }) {
  const fromField = () => props.fields !== undefined;
  const [alt, setAlt] = createSignal(text(props.block, "alt"));
  const [caption, setCaption] = createSignal(text(props.block, "caption"));
  const [width, setWidth] = createSignal(String(widthOf(props.block) ?? ""));
  const [asset, setAsset] = createSignal(text(props.block, "asset"));
  const [field, setField] = createSignal(text(props.block, "field"));
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  let dialog!: HTMLDivElement;
  let first!: HTMLInputElement;

  async function replace(file: File) {
    setError(null);
    setBusy(true);
    try {
      const a = await upload(file);
      setAsset(a.id);
    } catch (e) {
      setError(`Could not upload ${file.name}: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  function save(e: Event) {
    e.preventDefault();
    if (busy()) return;
    const n = Number(width().trim());
    let next = withText(props.block, "alt", alt());
    next = withText(next, "caption", caption());
    next = withItem(next, "width", width().trim() !== "" && Number.isInteger(n) && n > 0 ? n : undefined);
    next = fromField() ? withItem(next, "field", field()) : withText(next, "asset", asset());
    if (JSON.stringify(next) !== JSON.stringify(props.block)) props.onSave(next);
    props.onClose();
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      props.onClose();
      return;
    }
    if (e.key === "Tab") {
      const items = [...dialog.querySelectorAll<HTMLElement>(FOCUSABLE)];
      if (!items.length) return;
      const head = items[0]!;
      const tail = items[items.length - 1]!;
      if (e.shiftKey && document.activeElement === head) {
        e.preventDefault();
        tail.focus();
      } else if (!e.shiftKey && document.activeElement === tail) {
        e.preventDefault();
        head.focus();
      }
    }
  }

  onMount(() => {
    first.focus();
    // Focus that wanders out of the dialog (a click on the page behind it) comes back.
    const keep = (e: FocusEvent) => {
      if (e.target instanceof Node && !dialog.contains(e.target)) first.focus();
    };
    document.addEventListener("focusin", keep);
    onCleanup(() => {
      document.removeEventListener("focusin", keep);
      setTimeout(() => {
        if (returnTo?.isConnected) returnTo.focus();
      });
    });
  });

  return (
    <Portal>
      <div
        class="capture-backdrop"
        onMouseDown={(e) => {
          if (e.target === e.currentTarget) props.onClose();
        }}
      >
        <div ref={dialog} class="capture-dialog image-dialog" role="dialog" aria-modal="true" aria-labelledby="image-dialog-heading" onKeyDown={onKeyDown}>
          <header class="capture-head">
            <h2 id="image-dialog-heading">Image</h2>
            <button type="button" class="capture-x" aria-label="Close" title="Close (Esc)" onClick={() => props.onClose()}>
              ×
            </button>
          </header>
          <form class="image-dialog-form" onSubmit={save}>
            <Show when={!fromField() && asset()}>
              <img class="image-dialog-preview" src={assetUrl(asset())} alt="" />
            </Show>
            <Show when={fromField()}>
              <label>
                Image field
                <select value={field()} onChange={(e) => setField(e.currentTarget.value)}>
                  <For each={props.fields}>{(f) => <option value={f.name}>{f.label}</option>}</For>
                  <Show when={!props.fields?.some((f) => f.name === field())}>
                    <option value={field()}>{field()} (not an asset field)</option>
                  </Show>
                </select>
              </label>
            </Show>
            <label>
              Alt text
              <input ref={first} value={alt()} onInput={(e) => setAlt(e.currentTarget.value)} />
            </label>
            <label>
              Caption
              <input value={caption()} onInput={(e) => setCaption(e.currentTarget.value)} />
            </label>
            <label>
              Width
              <input
                type="number"
                min="1"
                step="1"
                placeholder="Content width"
                value={width()}
                onInput={(e) => setWidth(e.currentTarget.value)}
              />
            </label>
            <Show when={!fromField()}>
              <div class="image-dialog-replace">
                <FilePicker label="Replace image" text={busy() ? "Uploading…" : "Replace image…"} accept="image/*" onFile={(file) => void replace(file)} />
              </div>
              <Show when={error()}>
                <p class="error" role="alert">
                  {error()}
                </p>
              </Show>
            </Show>
            <div class="actions">
              <button type="button" onClick={() => props.onClose()}>
                Cancel
              </button>
              <button type="submit" class="primary" disabled={busy()}>
                Save
              </button>
            </div>
          </form>
        </div>
      </div>
    </Portal>
  );
}

/**
 * A fixed image. Edited, it is the image alone with its caption; selected, it shows Edit (the
 * properties dialog) and Remove over its corner. Without a file yet, it offers to upload one.
 */
function FixedImage(props: ImageBlockProps) {
  const [error, setError] = createSignal<string | null>(null);
  const [editing, setEditing] = createSignal(false);
  const refs = useContext(FieldRefs);
  const asset = () => text(props.block, "asset");
  const caption = () => (
    <Show when={text(props.block, "caption")}>
      <figcaption>{fillFieldRefs(text(props.block, "caption"), refs)}</figcaption>
    </Show>
  );
  const image = () => <img src={assetUrl(asset())} alt={fillFieldRefs(text(props.block, "alt"), refs)} width={widthOf(props.block)} loading="lazy" />;

  async function choose(file: File) {
    setError(null);
    try {
      const a = await upload(file);
      props.onChange?.(withItem(props.block, "asset", a.id));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <Show
      when={props.onChange}
      fallback={
        <figure class="block-image">
          <Show when={asset()}>{image()}</Show>
          {caption()}
        </figure>
      }
    >
      {(change) => (
        <figure class="block-image editing" classList={{ selected: !!props.selected }}>
          <div class="block-image-frame">
            <Show
              when={asset()}
              fallback={
                <div class="field-image-placeholder">
                  <ImageIcon />
                  <FilePicker label="Upload image" text="Upload image" accept="image/*" onFile={(file) => void choose(file)} />
                </div>
              }
            >
              {image()}
            </Show>
            <ImageTools onEdit={() => setEditing(true)} onRemove={props.onRemove} />
          </div>
          {caption()}
          <Show when={error()}>
            <p class="error" role="alert">
              {error()}
            </p>
          </Show>
          <Show when={editing()}>
            <ImageDialog block={props.block} onSave={change()} onClose={() => setEditing(false)} />
          </Show>
        </figure>
      )}
    </Show>
  );
}

export function ImageIcon() {
  return (
    <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <rect width="18" height="18" x="3" y="3" rx="2" ry="2" />
      <circle cx="9" cy="9" r="2" />
      <path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21" />
    </svg>
  );
}

/**
 * A template's image from an asset field (docs/vault-format.md, "Images from fields"): every
 * image asset in that field of the note shown, in a row that wraps. Edited in the template
 * editor, it is a placeholder naming the field. Empty on the note page, it offers to add one in a
 * slim row, unless the template places the field another way too (a slot or a control, which has
 * its own upload): then it shows nothing.
 */
function FieldImage(props: ImageBlockProps) {
  const binding = useControlBinding();
  const refs = useContext(FieldRefs);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [editing, setEditing] = createSignal(false);
  const name = () => text(props.block, "field");
  const field = (): FieldDef | undefined => binding?.type()?.fields.find((f) => f.name === name());
  const label = () => field()?.label ?? name();
  const ids = () => {
    const f = field();
    return f && binding ? assetIds(binding.value(f)) : [];
  };
  // The field's assets that are images. Ids the listing lacks are not shown.
  const [images] = createResource(
    () => ids().join(" ") || null,
    async (list) => {
      const wanted = list.split(" ");
      let byId = await assetsById();
      // An asset uploaded elsewhere since the listing was fetched: fetch it again, once per id.
      const unknown = wanted.filter((id) => !byId.has(id) && !refetched.has(id));
      if (unknown.length) {
        for (const id of unknown) refetched.add(id);
        known = undefined;
        byId = await assetsById();
      }
      return wanted.filter((id) => isImageFile(byId.get(id)?.filename ?? ""));
    },
  );
  const shown = () => (ids().length ? (images.latest ?? []) : []);
  const placedElsewhere = () => {
    const t = binding?.type();
    return !!t && templatePlaces(t, name());
  };
  const canAdd = () => !props.onChange && !!binding?.set && !binding.preview && field()?.kind === "asset" && !placedElsewhere();

  async function add(file: File) {
    const f = field();
    const set = binding?.set;
    if (!f || !set) return;
    setError(null);
    setBusy(true);
    try {
      const a = await upload(file);
      set(f, f.multiple ? [...ids(), a.id] : a.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const caption = () => fillFieldRefs(text(props.block, "caption"), refs);
  const assetFields = () => (binding?.type()?.fields ?? []).filter((f) => f.kind === "asset");

  return (
    <Show
      when={!props.onChange}
      fallback={
        <figure class="block-image field-image editing" classList={{ selected: !!props.selected }} data-field={name()}>
          <div class="block-image-frame">
            <div class="field-image-placeholder" data-testid="field-image-placeholder">
              <ImageIcon />
              <span>Image from {label()}</span>
            </div>
            <ImageTools onEdit={() => setEditing(true)} onRemove={props.onRemove} />
          </div>
          <Show when={caption()}>
            <figcaption>{caption()}</figcaption>
          </Show>
          <Show when={editing()}>
            <ImageDialog block={props.block} fields={assetFields()} onSave={(b) => props.onChange?.(b)} onClose={() => setEditing(false)} />
          </Show>
        </figure>
      }
    >
      <Show
        when={shown().length > 0}
        fallback={
          <Show when={canAdd()}>
            <figure class="block-image field-image empty" data-field={name()}>
              <FieldImageAdd label={label()} busy={busy()} onFile={(file) => void add(file)} />
              <Show when={error()}>
                <p class="error" role="alert">
                  {error()}
                </p>
              </Show>
            </figure>
          </Show>
        }
      >
        <figure class="block-image field-image" data-field={name()} classList={{ several: shown().length > 1 }}>
          <div class="field-images">
            <For each={shown()}>
              {(id) => <img src={assetUrl(id)} alt={fillFieldRefs(text(props.block, "alt"), refs)} width={widthOf(props.block)} loading="lazy" />}
            </For>
          </div>
          <Show when={caption()}>
            <figcaption>{caption()}</figcaption>
          </Show>
        </figure>
      </Show>
    </Show>
  );
}

/** Whether a drag carries files. */
const dragsFiles = (e: DragEvent) => !!e.dataTransfer && [...e.dataTransfer.types].includes("Files");

/**
 * The note page's placeholder for an empty image field: a slim row whose button chooses a file,
 * and which grows into a drop target while a file is dragged over it. Either uploads the file and
 * sets the field.
 */
function FieldImageAdd(props: { label: string; busy: boolean; onFile: (file: File) => void }) {
  let input!: HTMLInputElement;
  const [over, setOver] = createSignal(false);
  const enter = (e: DragEvent) => {
    if (!dragsFiles(e) || props.busy) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    setOver(true);
  };
  return (
    <>
      <input
        ref={input}
        type="file"
        hidden
        accept="image/*"
        onChange={(e) => {
          const file = e.currentTarget.files?.[0];
          e.currentTarget.value = "";
          if (file) props.onFile(file);
        }}
      />
      <button
        type="button"
        class="field-image-placeholder add"
        classList={{ over: over() }}
        data-testid="field-image-add"
        aria-label={`Add image to ${props.label}`}
        title={`Add image to ${props.label}`}
        disabled={props.busy}
        onClick={() => input.click()}
        onDragEnter={enter}
        onDragOver={enter}
        onDragLeave={(e) => {
          if (!(e.relatedTarget instanceof Node && e.currentTarget.contains(e.relatedTarget))) setOver(false);
        }}
        onDrop={(e) => {
          if (!dragsFiles(e)) return;
          e.preventDefault();
          e.stopPropagation();
          setOver(false);
          const files = [...(e.dataTransfer?.files ?? [])];
          const file = files.find((f) => f.type.startsWith("image/") || isImageFile(f.name));
          if (file && !props.busy) props.onFile(file);
        }}
      >
        <ImageIcon />
        <span>{props.busy ? "Uploading…" : over() ? "Drop image to add it" : "Add image"}</span>
      </button>
    </>
  );
}

/**
 * Set by a surface too small for a PDF viewer, such as a canvas card: there an `asset` block of a
 * PDF shows its file card with a thumbnail that opens the viewer over the app.
 */
export const AssetCards = createContext(false);

/**
 * An `asset` block: the file's download card. A PDF shows in an inline viewer instead, unless the
 * block's `card` item is set; the viewer's height is its `height` item. Changes to either are
 * kept in the block when it is editable, and only on the page otherwise.
 */
export function AssetBlock(props: { block: Block; onChange?: (block: Block) => void }) {
  const asset = () => text(props.block, "asset");
  const [info] = createResource(asset, async (id) => (await assetsById()).get(id) ?? null);
  const name = () => info()?.filename ?? (info.loading ? "…" : asset());
  const compact = useContext(AssetCards);
  const pdf = () => isPdfFile(info()?.filename ?? "");
  const [localCard, setLocalCard] = createSignal<boolean | null>(null);
  const [localHeight, setLocalHeight] = createSignal<number | null>(null);
  const card = () => compact || (localCard() ?? props.block.fields?.card === true);
  const height = () => {
    const h = props.block.fields?.height;
    return localHeight() ?? (typeof h === "number" ? h : VIEWER_HEIGHT.initial);
  };

  function setCard(on: boolean) {
    setLocalCard(on);
    props.onChange?.(withItem(props.block, "card", on ? true : undefined));
  }

  function setHeight(h: number) {
    setLocalHeight(h);
    props.onChange?.(withItem(props.block, "height", h === VIEWER_HEIGHT.initial ? undefined : h));
  }

  const caption = () => (
    <Show
      when={props.onChange}
      fallback={
        <Show when={text(props.block, "caption")}>
          <p class="asset-caption">{text(props.block, "caption")}</p>
        </Show>
      }
    >
      <div class="block-fields">
        <input
          aria-label="Caption"
          placeholder="Caption"
          value={text(props.block, "caption")}
          onInput={(e) => props.onChange?.(withText(props.block, "caption", e.currentTarget.value))}
        />
      </div>
    </Show>
  );

  return (
    <Show
      when={pdf() && !card()}
      fallback={
        <div class="block-asset">
          <div class="asset-card-row">
            <Show when={pdf() && info()}>{(a) => <PdfThumbButton id={a().id} filename={a().filename} width={48} />}</Show>
            <span>
              <a href={assetUrl(asset())} download={info()?.filename ?? ""} class="asset-card">
                {name()}
              </a>
              <Show when={info()}>{(a) => <span class="muted"> {formatSize(a().size)}</span>}</Show>
            </span>
            <Show when={pdf() && !compact}>
              <button type="button" aria-label="Show the PDF inline" data-node-view-interactive="" onClick={() => setCard(false)}>
                Show PDF
              </button>
            </Show>
          </div>
          {caption()}
        </div>
      }
    >
      <div class="block-asset block-pdf">
        <Show when={info()} keyed>
          {(a) => (
            <PdfViewer
              url={assetUrl(a.id)}
              filename={a.filename}
              height={height()}
              onHeight={setHeight}
              extra={
                <>
                  <span class="ink-divider" aria-hidden="true" />
                  <button type="button" class="ink-action" aria-label="Show as file card" title="Show as file card" onClick={() => setCard(true)}>
                    <CardIcon />
                  </button>
                </>
              }
            />
          )}
        </Show>
        {caption()}
      </div>
    </Show>
  );
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
