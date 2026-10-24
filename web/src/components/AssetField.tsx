// An asset field that takes any file (every asset field but a single image, which has the picture
// tile of ImageField.tsx): the attached files as a list, each with a remove button, and an Add +
// menu that uploads files or chooses ones already in the vault. Files dropped on the field upload
// too. Without `onChange` the list is shown read-only.

import { For, Show, createMemo, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { type Asset, type AssetSummary, type FieldDef, type Json, api, assetUrl } from "../api";
import { sizeText } from "../assetLibrary";
import { isImageFile, upload } from "../blocks/MediaBlocks";
import { isPdfFile } from "../pdf/model";
import { CardIcon } from "../pdf/PdfIcons";
import { PdfThumb, PdfThumbButton } from "../pdf/PdfPreview";

export interface AssetInputProps {
  field: FieldDef;
  value: Json | undefined;
  /** Absent for a read-only list: no remove buttons and no Add. */
  onChange?: (value: Json | undefined) => void;
  label: string;
  id?: string;
}

/** The asset ids a field value holds, in order. */
function assetIds(v: Json | undefined): string[] {
  const list = Array.isArray(v) ? v : v === undefined || v === null ? [] : [v];
  return list.filter((x): x is string => typeof x === "string" && x !== "");
}

/** The files of an asset field, and Add + to attach more. */
export function AssetInput(props: AssetInputProps) {
  const multiple = () => !!props.field.multiple;
  const editable = () => !!props.onChange;
  const [listing] = createResource(() => api.assets().catch(() => [] as Asset[]));
  // Files uploaded or chosen here, known before the listing is read again.
  const [added, setAdded] = createSignal<Map<string, Asset>>(new Map());
  const byId = createMemo(() => {
    const m = new Map((listing() ?? []).map((a) => [a.id, a] as const));
    for (const [id, a] of added()) m.set(id, a);
    return m;
  });
  const ids = () => {
    const all = assetIds(props.value);
    return multiple() ? all : all.slice(0, 1);
  };
  const [menu, setMenu] = createSignal(false);
  const [choosing, setChoosing] = createSignal(false);
  const [progress, setProgress] = createSignal<string | null>(null);
  const [errors, setErrors] = createSignal<string[]>([]);
  const [dragging, setDragging] = createSignal(false);
  let addButton: HTMLButtonElement | undefined;
  let fileInput: HTMLInputElement | undefined;
  let box!: HTMLDivElement;

  function know(assets: Asset[]) {
    const next = new Map(added());
    for (const a of assets) next.set(a.id, a);
    setAdded(next);
  }

  /** Attach `newIds`: appended to a multiple field, replacing a single one's file. */
  function attach(newIds: string[]) {
    if (!props.onChange || newIds.length === 0) return;
    if (!multiple()) {
      props.onChange(newIds[newIds.length - 1]);
      return;
    }
    const current = assetIds(props.value);
    props.onChange([...current, ...newIds.filter((id) => !current.includes(id))]);
  }

  function remove(id: string) {
    if (!props.onChange) return;
    if (!multiple()) return props.onChange(undefined);
    const rest = assetIds(props.value).filter((x) => x !== id);
    props.onChange(rest.length === 0 ? undefined : rest);
  }

  async function uploadFiles(files: File[]) {
    if (!editable() || files.length === 0) return;
    const wanted = multiple() ? files : files.slice(0, 1);
    setErrors([]);
    const done: Asset[] = [];
    const failed: string[] = [];
    for (const [i, file] of wanted.entries()) {
      setProgress(wanted.length > 1 ? `Uploading ${file.name} (${i + 1} of ${wanted.length})…` : `Uploading ${file.name}…`);
      try {
        done.push(await upload(file));
      } catch (e) {
        failed.push(`Could not upload ${file.name}: ${e instanceof Error ? e.message : String(e)}`);
      }
    }
    setProgress(null);
    setErrors(failed);
    know(done);
    attach(done.map((a) => a.id));
  }

  function closeMenu(refocus: boolean) {
    setMenu(false);
    if (refocus) addButton?.focus();
  }

  return (
    <div
      ref={box}
      class="asset-input"
      classList={{ dragging: dragging(), "read-only": !editable() }}
      data-testid="asset-input"
      onDragOver={(e) => {
        if (editable() && [...(e.dataTransfer?.types ?? [])].includes("Files")) {
          e.preventDefault();
          setDragging(true);
        }
      }}
      onDragLeave={(e) => {
        if (!box.contains(e.relatedTarget as Node | null)) setDragging(false);
      }}
      onDrop={(e) => {
        if (!editable()) return;
        e.preventDefault();
        setDragging(false);
        void uploadFiles([...(e.dataTransfer?.files ?? [])]);
      }}
    >
      <Show
        when={ids().length > 0}
        fallback={
          <Show when={!editable()}>
            <span class="muted">—</span>
          </Show>
        }
      >
        <ul class="asset-list" aria-label={props.label}>
          <For each={ids()}>
            {(id) => {
              const a = () => byId().get(id);
              return (
                <li class="asset-row" classList={{ missing: !listing.loading && !a() }} data-asset={id}>
                  <AssetRowIcon id={id} asset={a()} />
                  <Show when={a()} fallback={<span class="asset-row-name" title={id}>{id}</span>}>
                    {(info) => (
                      <>
                        <a class="asset-row-name" href={`/assets/${id}`} title={info().filename}>
                          {info().filename}
                        </a>
                        <span class="asset-row-size muted">{sizeText(info().size)}</span>
                      </>
                    )}
                  </Show>
                  <Show when={editable()}>
                    <button
                      type="button"
                      class="asset-row-remove"
                      aria-label={`Remove ${a()?.filename ?? id}`}
                      title={`Remove ${a()?.filename ?? id} (the file stays in the vault)`}
                      onClick={() => remove(id)}
                    >
                      ×
                    </button>
                  </Show>
                </li>
              );
            }}
          </For>
        </ul>
      </Show>
      <Show when={editable()}>
        <div class="asset-add">
          <input
            ref={fileInput}
            type="file"
            hidden
            multiple={multiple()}
            data-testid="asset-file-input"
            onChange={(e) => {
              const files = [...(e.currentTarget.files ?? [])];
              e.currentTarget.value = "";
              void uploadFiles(files);
            }}
          />
          <button
            ref={addButton}
            type="button"
            id={props.id}
            class="asset-add-button"
            aria-label={`Add an attachment to ${props.label}`}
            title={`Add an attachment to ${props.label}, or drop files here`}
            aria-haspopup="menu"
            aria-expanded={menu()}
            disabled={progress() !== null}
            onClick={() => setMenu(!menu())}
          >
            Add +
          </button>
          <Show when={menu()}>
            <AddMenu
              label={props.label}
              replaces={!multiple() && ids().length > 0 ? (byId().get(ids()[0]!)?.filename ?? ids()[0]) : undefined}
              anchor={() => addButton}
              onClose={closeMenu}
              onUpload={() => {
                closeMenu(true);
                fileInput?.click();
              }}
              onChoose={() => {
                closeMenu(false);
                setChoosing(true);
              }}
            />
          </Show>
        </div>
        <Show when={progress()}>
          {(p) => (
            <p class="asset-progress muted" role="status">
              {p()}
            </p>
          )}
        </Show>
        <For each={errors()}>
          {(e) => (
            <p class="error" role="alert">
              {e}
            </p>
          )}
        </For>
        <Show when={choosing()}>
          <AssetChooser
            label={props.label}
            multiple={multiple()}
            attached={ids()}
            onPick={(picked) => {
              setChoosing(false);
              know(picked);
              attach(picked.map((a) => a.id));
              queueMicrotask(() => addButton?.focus());
            }}
            onClose={() => {
              setChoosing(false);
              queueMicrotask(() => addButton?.focus());
            }}
          />
        </Show>
      </Show>
    </div>
  );
}

/**
 * A file's icon by kind: an image's thumbnail, a PDF's first page (a button opening the viewer,
 * unless `inert`), else a file.
 */
function AssetRowIcon(props: { id: string; asset: Asset | undefined; inert?: boolean }) {
  const name = () => props.asset?.filename ?? "";
  return (
    <span class="asset-row-icon">
      <Show
        when={props.asset && isImageFile(name())}
        fallback={
          <Show when={props.asset && isPdfFile(name())} fallback={<CardIcon />}>
            <Show when={!props.inert} fallback={<PdfThumb url={assetUrl(props.id)} width={32} />}>
              <PdfThumbButton id={props.id} filename={name()} width={32} />
            </Show>
          </Show>
        }
      >
        <img src={assetUrl(props.id)} alt="" loading="lazy" />
      </Show>
    </span>
  );
}

/** Add +'s menu: upload, or choose from the vault. Escape, Tab and a click outside close it. */
function AddMenu(props: {
  label: string;
  /** The file a single field holds, which either choice replaces. */
  replaces?: string;
  anchor: () => HTMLButtonElement | undefined;
  onClose: (refocus: boolean) => void;
  onUpload: () => void;
  onChoose: () => void;
}) {
  let el!: HTMLDivElement;
  const items = () => [...el.querySelectorAll<HTMLElement>("[role=menuitem]")];
  onMount(() => {
    items()[0]?.focus();
    const outside = (e: PointerEvent) => {
      if (!el.contains(e.target as Node) && !props.anchor()?.contains(e.target as Node)) props.onClose(false);
    };
    document.addEventListener("pointerdown", outside, true);
    onCleanup(() => document.removeEventListener("pointerdown", outside, true));
  });
  function onKey(e: KeyboardEvent) {
    const list = items();
    const at = list.indexOf(document.activeElement as HTMLElement);
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      props.onClose(true);
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      list[(at + (e.key === "ArrowDown" ? 1 : -1) + list.length) % list.length]?.focus();
    } else if (e.key === "Tab") {
      props.onClose(false);
    }
  }
  return (
    <div
      ref={el}
      class="pin-menu asset-add-menu"
      role="menu"
      aria-label={`Add an attachment to ${props.label}`}
      onKeyDown={onKey}
      onPointerDown={(e) => e.stopPropagation()}
    >
      <Show when={props.replaces}>
        <p class="asset-add-replaces muted">Replace {props.replaces}</p>
      </Show>
      <button type="button" role="menuitem" onClick={() => props.onUpload()}>
        Upload a file…
      </button>
      <button type="button" role="menuitem" onClick={() => props.onChoose()}>
        Choose from the vault…
      </button>
    </div>
  );
}

/**
 * The vault's files, newest first, with a search box. A single field takes the one clicked; a
 * multiple field takes every file ticked, with Add.
 */
function AssetChooser(props: {
  label: string;
  multiple: boolean;
  attached: string[];
  onPick: (assets: AssetSummary[]) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = createSignal("");
  const [found] = createResource(query, async (q) => (await api.assetLibrary({ q, sort: "uploaded", dir: "desc", limit: 100 })).assets);
  const [picked, setPicked] = createSignal<AssetSummary[]>([]);
  let search!: HTMLInputElement;
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));
  onMount(() => search.focus());

  const isPicked = (id: string) => picked().some((a) => a.id === id);
  function toggle(a: AssetSummary) {
    setPicked(isPicked(a.id) ? picked().filter((p) => p.id !== a.id) : [...picked(), a]);
  }

  return (
    <div
      class="capture-backdrop asset-chooser-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) props.onClose();
      }}
    >
      <div
        class="capture-dialog asset-chooser"
        role="dialog"
        aria-modal="true"
        aria-label={`Choose from the vault for ${props.label}`}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            props.onClose();
          }
        }}
      >
        <header class="capture-head">
          <h2>Choose from the vault</h2>
          <button type="button" class="capture-x" aria-label="Close" title="Close (Esc)" onClick={props.onClose}>
            ×
          </button>
        </header>
        <input
          ref={search}
          type="search"
          class="asset-chooser-search"
          aria-label="Search files"
          placeholder="Search files"
          onInput={(e) => {
            const v = e.currentTarget.value;
            clearTimeout(timer);
            timer = setTimeout(() => setQuery(v), 150);
          }}
        />
        <Show when={!found.error} fallback={<p class="error" role="alert">Could not read the vault's files.</p>}>
          <Show when={!found.loading || found()} fallback={<p class="muted">Loading…</p>}>
            <Show when={(found() ?? []).length > 0} fallback={<p class="muted">{query() ? "No files match." : "No files yet."}</p>}>
              <ul class="asset-chooser-list" aria-label="Files">
                <For each={found()}>
                  {(a) => {
                    const attached = () => props.attached.includes(a.id);
                    return (
                      <li>
                        <button
                          type="button"
                          class="asset-chooser-item"
                          classList={{ picked: isPicked(a.id) }}
                          aria-label={a.filename}
                          aria-pressed={props.multiple ? isPicked(a.id) || attached() : attached()}
                          disabled={props.multiple && attached()}
                          title={attached() ? `${a.filename} (attached)` : a.filename}
                          onClick={() => (props.multiple ? toggle(a) : props.onPick([a]))}
                        >
                          <AssetRowIcon id={a.id} asset={a} inert />
                          <span class="asset-row-name">{a.filename}</span>
                          <span class="asset-row-size muted">{attached() ? "attached" : sizeText(a.size)}</span>
                        </button>
                      </li>
                    );
                  }}
                </For>
              </ul>
            </Show>
          </Show>
        </Show>
        <Show when={props.multiple}>
          <footer class="asset-chooser-foot">
            <button type="button" onClick={props.onClose}>
              Cancel
            </button>
            <button type="button" class="primary" disabled={picked().length === 0} onClick={() => props.onPick(picked())}>
              {picked().length > 1 ? `Add ${picked().length} files` : "Add"}
            </button>
          </footer>
        </Show>
      </div>
    </div>
  );
}
