// The model behind the assets library (`/assets`) and an asset's page (`/assets/:id`): its view,
// remembered as a type's list view is; how sizes, kinds, dates and uses read; and the names of
// assets seen, so a tab can be titled before its page loads.

import { createRoot, createSignal } from "solid-js";
import { createStore } from "solid-js/store";
import type { AssetHow, AssetKind, FieldDef, TypeDef } from "./api";
import { type TypeListView, loadTypeView, saveTypeView } from "./typeList";

/** The key the library's view is kept under among the types' views. No type name has a colon. */
export const ASSETS_VIEW = ":assets";

/** The most assets one page of the library loads; more come with "Show more". */
export const ASSET_PAGE = 120;

export function loadAssetsView(): TypeListView {
  return loadTypeView(ASSETS_VIEW);
}

export function saveAssetsView(view: TypeListView) {
  saveTypeView(ASSETS_VIEW, view);
}

/** A size in bytes, in the largest unit that keeps it at least 1. */
export function sizeText(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/** The file name's extension, upper case, or "" when it has none. */
export function extension(filename: string): string {
  const dot = filename.lastIndexOf(".");
  return dot > 0 && dot < filename.length - 1 ? filename.slice(dot + 1).toUpperCase() : "";
}

const KIND_WORD: Record<AssetKind, string> = { image: "image", pdf: "PDF", audio: "audio", video: "video", other: "file" };

/** "PNG image", "PDF", "MP3 audio", "ZIP file", or "File" for a name with no extension. */
export function kindLabel(filename: string, kind: AssetKind): string {
  if (kind === "pdf") return "PDF";
  const ext = extension(filename);
  return ext ? `${ext} ${KIND_WORD[kind]}` : KIND_WORD[kind][0]!.toUpperCase() + KIND_WORD[kind].slice(1);
}

/** The icon of an asset of `kind`, by Lucide name. */
export function kindIcon(kind: AssetKind | undefined): string {
  switch (kind) {
    case "image":
      return "image";
    case "pdf":
      return "file-text";
    case "audio":
      return "file-audio";
    case "video":
      return "file-video";
    default:
      return "file";
  }
}

/** The kind of a file, by its name, as the server works it out. */
export function kindOf(filename: string): AssetKind {
  const ext = extension(filename).toLowerCase();
  if (["png", "jpg", "jpeg", "gif", "webp", "avif", "svg"].includes(ext)) return "image";
  if (ext === "pdf") return "pdf";
  if (["mp3", "wav"].includes(ext)) return "audio";
  if (["mp4", "webm"].includes(ext)) return "video";
  return "other";
}

/** A day, as the type lists show one, or "" for none. */
export function dateText(ms: number | null | undefined): string {
  if (!ms) return "";
  return new Date(ms).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

/** "Not used", "Used by 1 note", "Used by 3 notes". */
export function usedText(n: number): string {
  if (n === 0) return "Not used";
  return `Used by ${n} ${n === 1 ? "note" : "notes"}`;
}

function fieldLabel(type: TypeDef | undefined, name: string): string {
  const f: FieldDef | undefined = type?.fields.find((x) => x.name === name);
  return f?.label ?? name;
}

/** How a note uses an asset, for the Used by panel: "Cover", "Image in Body", "File in Body". */
export function howText(how: AssetHow, type: TypeDef | undefined): string {
  const field = fieldLabel(type, how.field);
  switch (how.via) {
    case "field":
      return field;
    case "image":
      return `Image in ${field}`;
    case "asset":
      return `File in ${field}`;
    default:
      return `Card in ${field}`;
  }
}

/** The asset id an address shows, for `/assets/<ULID>`. */
export function assetIdOf(url: string): string | null {
  const m = /^\/assets\/([0-9A-HJKMNP-TV-Z]{26})\/?$/i.exec(url.split(/[?#]/)[0]!);
  return m ? m[1]!.toUpperCase() : null;
}

/** The file names of the assets this page has seen, so a tab names its asset at once. */
const known = createRoot(() => createStore<Record<string, string>>({}));

export function knowAsset(id: string, filename: string) {
  if (known[0][id] !== filename) known[1](id, filename);
}

/** Reactive: asset `id`'s file name, if it has been seen. */
export function assetName(id: string): string | undefined {
  return known[0][id];
}

const revision = createRoot(() => createSignal(0));

/** Reactive: bumped whenever this client deletes, restores or renames an asset. */
export function assetsRevision(): number {
  return revision[0]();
}

export function assetsChanged() {
  revision[1]((n) => n + 1);
}
