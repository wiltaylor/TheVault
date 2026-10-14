// Image and asset blocks. Both name an uploaded file by its asset ULID; the file itself lives at
// `assets/<ULID>/<filename>` and is uploaded through the assets API (D22).

import { For, Show, createResource, createSignal, useContext } from "solid-js";
import { type Asset, type Block, type FieldDef, type Json, api, assetUrl } from "../api";
import { FieldRefs, fillFieldRefs } from "../components/fieldRefs";
import { useControlBinding } from "../controls/binding";
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

/** The image in an image block: fixed, or a template's image from an asset field of the note. */
export function ImageBlock(props: { block: Block; onChange?: (block: Block) => void }) {
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

/** Alt text, caption and width inputs, which every image block has when edited. */
function ImageSettings(props: { block: Block; onChange: (block: Block) => void }) {
  const set = (item: string, value: string) => props.onChange(withText(props.block, item, value));
  return (
    <>
      <input aria-label="Alt text" placeholder="Alt text" value={text(props.block, "alt")} onInput={(e) => set("alt", e.currentTarget.value)} />
      <input
        aria-label="Caption"
        placeholder="Caption"
        value={text(props.block, "caption")}
        onInput={(e) => set("caption", e.currentTarget.value)}
      />
      <input
        type="number"
        min="1"
        step="1"
        aria-label="Width"
        placeholder="Width"
        value={widthOf(props.block) ?? ""}
        onInput={(e) => {
          const n = Number(e.currentTarget.value);
          props.onChange(withItem(props.block, "width", Number.isInteger(n) && n > 0 ? n : undefined));
        }}
      />
    </>
  );
}

function FixedImage(props: { block: Block; onChange?: (block: Block) => void }) {
  const [error, setError] = createSignal<string | null>(null);
  const refs = useContext(FieldRefs);
  const asset = () => text(props.block, "asset");

  async function replace(file: File) {
    setError(null);
    try {
      const a = await upload(file);
      props.onChange?.(withItem(props.block, "asset", a.id));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <figure class="block-image">
      <Show when={asset()}>
        <img src={assetUrl(asset())} alt={fillFieldRefs(text(props.block, "alt"), refs)} width={widthOf(props.block)} loading="lazy" />
      </Show>
      <Show
        when={props.onChange}
        fallback={
          <Show when={text(props.block, "caption")}>
            <figcaption>{fillFieldRefs(text(props.block, "caption"), refs)}</figcaption>
          </Show>
        }
      >
        {(change) => (
          <>
            <div class="block-fields">
              <ImageSettings block={props.block} onChange={change()} />
              <FilePicker label="Replace image" text="Replace" accept="image/*" onFile={replace} />
            </div>
            <Show when={error()}>
              <p class="error" role="alert">
                {error()}
              </p>
            </Show>
          </>
        )}
      </Show>
    </figure>
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
 * editor, it is a placeholder naming the field. Empty on the note page, it offers to add one.
 */
function FieldImage(props: { block: Block; onChange?: (block: Block) => void }) {
  const binding = useControlBinding();
  const refs = useContext(FieldRefs);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
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
  const canAdd = () => !props.onChange && !!binding?.set && !binding.preview && field()?.kind === "asset";

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
        <figure class="block-image field-image editing" data-field={name()}>
          <div class="field-image-placeholder" data-testid="field-image-placeholder">
            <ImageIcon />
            <span>Image from {label()}</span>
          </div>
          <div class="block-fields">
            <select
              aria-label="Image field"
              value={name()}
              onChange={(e) => props.onChange?.(withItem(props.block, "field", e.currentTarget.value))}
            >
              <For each={assetFields()}>{(f) => <option value={f.name}>{f.label}</option>}</For>
              <Show when={!assetFields().some((f) => f.name === name())}>
                <option value={name()}>{name()} (not an asset field)</option>
              </Show>
            </select>
            <ImageSettings block={props.block} onChange={props.onChange!} />
          </div>
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

/** The note page's placeholder for an empty image field: choosing a file uploads it and sets the field. */
function FieldImageAdd(props: { label: string; busy: boolean; onFile: (file: File) => void }) {
  let input!: HTMLInputElement;
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
        data-testid="field-image-add"
        aria-label={`Add image to ${props.label}`}
        title={`Add image to ${props.label}`}
        disabled={props.busy}
        onClick={() => input.click()}
      >
        <ImageIcon />
        <span>{props.busy ? "Uploading…" : "Add image"}</span>
      </button>
    </>
  );
}

export function AssetBlock(props: { block: Block; onChange?: (block: Block) => void }) {
  const asset = () => text(props.block, "asset");
  const [info] = createResource(asset, async (id) => (await assetsById()).get(id) ?? null);
  const name = () => info()?.filename ?? (info.loading ? "…" : asset());

  return (
    <div class="block-asset">
      <a href={assetUrl(asset())} download={info()?.filename ?? ""} class="asset-card">
        {name()}
      </a>
      <Show when={info()}>{(a) => <span class="muted"> {formatSize(a().size)}</span>}</Show>
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
    </div>
  );
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
