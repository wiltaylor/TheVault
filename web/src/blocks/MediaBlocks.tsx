// Image and asset blocks. Both name an uploaded file by its asset ULID; the file itself lives at
// `assets/<ULID>/<filename>` and is uploaded through the assets API (D22).

import { Show, createResource, createSignal } from "solid-js";
import { type Asset, type Block, api, assetUrl } from "../api";
import { text, withItem, withText } from "./fields";

let known: Promise<Map<string, Asset>> | undefined;

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

export function ImageBlock(props: { block: Block; onChange?: (block: Block) => void }) {
  const [error, setError] = createSignal<string | null>(null);
  const asset = () => text(props.block, "asset");
  const width = () => {
    const w = props.block.fields?.width;
    return typeof w === "number" ? w : undefined;
  };
  const set = (item: string, value: string) => props.onChange?.(withText(props.block, item, value));

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
        <img src={assetUrl(asset())} alt={text(props.block, "alt")} width={width()} loading="lazy" />
      </Show>
      <Show
        when={props.onChange}
        fallback={
          <Show when={text(props.block, "caption")}>
            <figcaption>{text(props.block, "caption")}</figcaption>
          </Show>
        }
      >
        <div class="block-fields">
          <input
            aria-label="Alt text"
            placeholder="Alt text"
            value={text(props.block, "alt")}
            onInput={(e) => set("alt", e.currentTarget.value)}
          />
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
            value={width() ?? ""}
            onInput={(e) => {
              const n = Number(e.currentTarget.value);
              props.onChange?.(withItem(props.block, "width", Number.isInteger(n) && n > 0 ? n : undefined));
            }}
          />
          <FilePicker label="Replace image" text="Replace" accept="image/*" onFile={replace} />
        </div>
        <Show when={error()}>
          <p class="error" role="alert">
            {error()}
          </p>
        </Show>
      </Show>
    </figure>
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
