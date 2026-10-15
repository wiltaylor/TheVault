// Images and PDFs pasted or dropped into the block editor. Each file is uploaded as an asset and
// becomes a block where the caret (or the drop) was: an `image` block for an image, an `asset`
// block, which shows a PDF in its viewer, for a PDF. While it uploads, an `imageUpload`
// placeholder node holds its place, so the image lands there however the document changes in the
// meantime. The placeholder is never saved: `docToBlocks` keeps no node it does not know.
//
// An image block sits between blocks, at the top level or in a column, never inside a paragraph,
// a list or a quote. A caret in the middle of a paragraph splits it; anywhere else deeper, the
// image goes after the list or quote.
//
// Pasted HTML is left to the editor's own paste, except that an `<img>` holding a data URL is
// taken out of it as a file and uploaded the same way. A clipboard that holds image files as well
// as HTML (a browser's "Copy image") pastes the files.

import { Node } from "@tiptap/core";
import type { Node as PMNode } from "@tiptap/pm/model";
import { type EditorState, Plugin, PluginKey, Selection, type Transaction } from "@tiptap/pm/state";
import { ReplaceAroundStep, ReplaceStep, canJoin } from "@tiptap/pm/transform";
import type { EditorView } from "@tiptap/pm/view";
import type { Block } from "../api";
import { fresh } from "./BlockHolder";

export interface ImageUploadOptions {
  /** Upload `file` as an asset; resolves to the asset's id. */
  upload: ((file: File) => Promise<string>) | null;
  /** An upload is starting. */
  onStart?: () => void;
  /** The upload of `file` failed with `error`; its placeholder has gone. */
  onError?: (file: File, error: unknown) => void;
}

/** The file extension each image type the server serves as an image is stored under. */
const EXTENSIONS: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
  "image/avif": "avif",
  "image/svg+xml": "svg",
};

const EXT_RE = /\.(png|jpe?g|gif|webp|avif|svg)$/i;

/** Whether `f` is a PDF, by its type or its name. */
export function isPdf(f: File): boolean {
  return f.type === "application/pdf" || /\.pdf$/i.test(f.name);
}

/**
 * The image and PDF files among `files`, in order, images named as `imageFiles` names them and a
 * PDF named without the extension given one, so the server serves it as a PDF.
 */
export function pastedFiles(files: Iterable<File>): File[] {
  const out: File[] = [];
  for (const f of files) {
    if (isPdf(f)) {
      out.push(/\.pdf$/i.test(f.name) ? f : new File([f], `${f.name || "pasted"}.pdf`, { type: "application/pdf" }));
    } else {
      out.push(...imageFiles([f]));
    }
  }
  return out;
}

/**
 * The image files among `files`, each named so the server serves it as an image: a file named
 * without an image extension (a screenshot's "image", say) gets one from its type.
 */
export function imageFiles(files: Iterable<File>): File[] {
  const out: File[] = [];
  for (const f of files) {
    const ext = EXTENSIONS[f.type];
    if (!ext) continue;
    out.push(EXT_RE.test(f.name) ? f : new File([f], `${f.name.replace(/\.[^.]*$/, "") || "pasted-image"}.${ext}`, { type: f.type }));
  }
  return out;
}

/** The file a `data:` URL of an image holds, or null when it is not one the vault keeps. */
export function dataUrlFile(url: string, name = "pasted-image"): File | null {
  const m = /^data:([^;,]+)((?:;[^;,]*)*),(.*)$/s.exec(url.trim());
  if (!m) return null;
  const type = m[1]!.toLowerCase();
  const ext = EXTENSIONS[type];
  if (!ext) return null;
  try {
    const data = m[3]!;
    const bytes = /;base64/i.test(m[2]!)
      ? Uint8Array.from(atob(data.replace(/\s+/g, "")), (c) => c.charCodeAt(0))
      : new TextEncoder().encode(decodeURIComponent(data));
    return new File([bytes], `${name}.${ext}`, { type });
  } catch {
    return null;
  }
}

// Placeholders by id: `pending` before their upload starts, `uploading` until it ends. A
// placeholder whose id is in neither (brought back by undo, say) is taken out.
const pending = new Map<string, File>();
const uploading = new Set<string>();
let counter = 0;

/** Register `file` for a new placeholder and return its id. */
function register(file: File): string {
  const id = `upload-${Date.now().toString(36)}-${(counter++).toString(36)}`;
  pending.set(id, file);
  return id;
}

/** The deepest ancestor depth of `$pos` that holds blocks an image may sit among: the document or a column. */
function containerDepth(doc: PMNode, pos: number): number {
  const $pos = doc.resolve(pos);
  let d = $pos.depth;
  while (d > 0 && $pos.node(d).type.name !== "column") d--;
  return d;
}

/**
 * Where blocks go for a caret at `pos` in `tr`'s document: between blocks of the document or a
 * column. A caret inside a paragraph or heading splits it there (at its start or end, the blocks
 * go before or after it); deeper, in a list or a quote, they go after that list or quote.
 * Returns the position and whether a textblock was split.
 */
export function blockInsertPos(tr: Transaction, pos: number): { at: number; split: boolean } {
  const $pos = tr.doc.resolve(pos);
  const d = containerDepth(tr.doc, pos);
  if ($pos.depth === d) return { at: pos, split: false };
  if ($pos.depth === d + 1 && $pos.parent.isTextblock) {
    if ($pos.parentOffset === 0) return { at: $pos.before(), split: false };
    if ($pos.parentOffset === $pos.parent.content.size) return { at: $pos.after(), split: false };
    tr.split(pos);
    return { at: pos + 1, split: true };
  }
  return { at: $pos.after(d + 1), split: false };
}

/**
 * The transaction that replaces `from`..`to` with upload placeholders for `files`, in order, at
 * the nearest place a block may go, and leaves the caret after them.
 */
export function placeholdersTr(state: EditorState, from: number, to: number, files: File[]): Transaction {
  const { schema } = state;
  const tr = state.tr;
  if (to > from) tr.delete(from, to);
  const { at, split } = blockInsertPos(tr, tr.mapping.map(from, -1));
  const nodes = files.map((f) => schema.nodes.imageUpload!.create({ id: register(f), split, kind: isPdf(f) ? "asset" : "image" }));
  tr.insert(at, nodes);
  const end = at + nodes.length;
  // At the end of the document or a column, a line to type on after the images.
  const $end = tr.doc.resolve(end);
  if (!$end.nodeAfter) tr.insert(end, schema.nodes.paragraph!.create());
  tr.setSelection(Selection.near(tr.doc.resolve(end)));
  return tr.scrollIntoView();
}

/** Where the placeholder `id` is in `doc`, or null when it has gone. */
function find(doc: PMNode, id: string): { pos: number; node: PMNode } | null {
  let found: { pos: number; node: PMNode } | null = null;
  doc.descendants((node, pos) => {
    if (found) return false;
    if (node.type.name === "imageUpload" && node.attrs.id === id) {
      found = { pos, node };
      return false;
    }
    return !node.isTextblock;
  });
  return found;
}

/** Every placeholder in `doc`. */
function placeholders(doc: PMNode): { pos: number; node: PMNode }[] {
  const out: { pos: number; node: PMNode }[] = [];
  doc.descendants((node, pos) => {
    if (node.type.name === "imageUpload") out.push({ pos, node });
    return !node.isTextblock;
  });
  return out;
}

/**
 * The transaction that turns placeholder `id` into a block of `asset`, or null when it has gone:
 * an `image` block, or an `asset` block for a PDF.
 */
export function finishTr(state: EditorState, id: string, asset: string): Transaction | null {
  const at = find(state.doc, id);
  if (!at) return null;
  const block: Block = { kind: at.node.attrs.kind === "asset" ? "asset" : "image", fields: { asset } };
  const node = state.schema.nodes.vaultBlock!.create({ block: fresh(block) });
  return state.tr.replaceWith(at.pos, at.pos + at.node.nodeSize, node).setMeta("addToHistory", false);
}

/** The transaction that takes placeholder `id` out, joining the textblock its insertion split. */
export function failTr(state: EditorState, id: string): Transaction | null {
  const at = find(state.doc, id);
  if (!at) return null;
  const tr = state.tr.delete(at.pos, at.pos + at.node.nodeSize);
  if (at.node.attrs.split && canJoin(tr.doc, at.pos)) {
    const before = tr.doc.resolve(at.pos).nodeBefore;
    const after = tr.doc.resolve(at.pos).nodeAfter;
    if (before?.isTextblock && after?.isTextblock) tr.join(at.pos);
  }
  return tr.setMeta("addToHistory", false);
}

/** Whether any of `trs` put a placeholder in the document. */
function addsPlaceholder(trs: readonly Transaction[]): boolean {
  for (const tr of trs) {
    for (const step of tr.steps) {
      if (!(step instanceof ReplaceStep || step instanceof ReplaceAroundStep)) continue;
      let found = false;
      step.slice.content.descendants((n) => {
        if (n.type.name === "imageUpload") found = true;
        return !found;
      });
      if (found) return true;
    }
  }
  return false;
}

/**
 * After a change that brought placeholders in: take out any no upload stands behind, and move
 * any that landed inside a paragraph, list or quote (from pasted HTML) to between blocks.
 */
function tidyTr(state: EditorState): Transaction | null {
  const tr = state.tr;
  // Last first, so earlier positions stay put.
  for (const { pos, node } of placeholders(state.doc).reverse()) {
    const id = node.attrs.id as string;
    const live = pending.has(id) || uploading.has(id);
    const $pos = state.doc.resolve(pos);
    const placed = $pos.depth === containerDepth(state.doc, pos);
    if (live && placed) continue;
    const from = tr.mapping.map(pos);
    tr.delete(from, from + node.nodeSize);
    if (!live) continue;
    const { at } = blockInsertPos(tr, from);
    tr.insert(at, node);
  }
  return tr.docChanged ? tr.setMeta("addToHistory", false) : null;
}

/** Start the uploads of the placeholders in `view` that wait for one. */
function startUploads(view: EditorView, options: ImageUploadOptions) {
  const upload = options.upload;
  if (!pending.size || !upload) return;
  for (const { node } of placeholders(view.state.doc)) {
    const id = node.attrs.id as string;
    const file = pending.get(id);
    if (!file) continue;
    pending.delete(id);
    uploading.add(id);
    options.onStart?.();
    upload(file).then(
      (asset) => {
        uploading.delete(id);
        if (view.isDestroyed) return;
        const tr = finishTr(view.state, id, asset);
        if (tr) view.dispatch(tr);
      },
      (e: unknown) => {
        uploading.delete(id);
        if (view.isDestroyed) return;
        const tr = failTr(view.state, id);
        if (tr) view.dispatch(tr);
        options.onError?.(file, e);
      },
    );
  }
}

// The placeholders the last pasted HTML registered, dropped again when the paste takes files instead.
let fromHtml: string[] = [];

/** HTML with every `<img>` holding a data URL turned into a placeholder for its file. */
export function htmlWithPlaceholders(html: string): string {
  fromHtml = [];
  if (!/<img[^>]+src\s*=\s*["']?\s*data:/i.test(html)) return html;
  const tpl = document.createElement("template");
  tpl.innerHTML = html;
  let changed = false;
  for (const img of Array.from(tpl.content.querySelectorAll("img"))) {
    const file = dataUrlFile(img.getAttribute("src") ?? "");
    if (!file) continue;
    const div = document.createElement("div");
    const id = register(file);
    fromHtml.push(id);
    div.setAttribute("data-image-upload", id);
    img.replaceWith(div);
    changed = true;
  }
  return changed ? tpl.innerHTML : html;
}

/**
 * Whether a paste whose clipboard also holds image files pastes those files rather than its
 * text: when it has no text, or its HTML is an image. An office suite puts a picture of copied
 * text beside the text itself, and that pastes as text.
 */
export function pastesFiles(html: string, text: string): boolean {
  return html ? /<img[\s>]/i.test(html) : text.trim() === "";
}

/** The image and PDF files of a paste or a drop, or none. */
function transferFiles(data: DataTransfer | null): File[] {
  if (!data) return [];
  const files = Array.from(data.files ?? []);
  if (!files.length) {
    for (const item of Array.from(data.items ?? [])) {
      const f = item.kind === "file" ? item.getAsFile() : null;
      if (f) files.push(f);
    }
  }
  return pastedFiles(files);
}

/** A placeholder standing where an image being uploaded will go. Only the editor has it. */
export const ImageUpload = Node.create<ImageUploadOptions>({
  name: "imageUpload",
  group: "block",
  atom: true,
  selectable: false,
  draggable: false,
  // Before the other paste and drop handlers: an image paste is not text.
  priority: 1000,
  addOptions() {
    return { upload: null };
  },
  addAttributes() {
    return {
      id: { default: "", parseHTML: (el) => el.getAttribute("data-image-upload") },
      split: { default: false, rendered: false },
      // The block the upload becomes: `image`, or `asset` for a PDF.
      kind: { default: "image", rendered: false },
    };
  },
  parseHTML() {
    // Only a placeholder this page made for pasted HTML; never one copied out of a document.
    return [{ tag: "div[data-image-upload]", getAttrs: (el) => (pending.has(el.getAttribute("data-image-upload") ?? "") ? null : false) }];
  },
  renderHTML({ node }) {
    return ["div", { "data-image-upload": node.attrs.id }];
  },
  addNodeView() {
    return ({ node }) => {
      const dom = document.createElement("div");
      dom.className = "field-image-placeholder image-upload";
      dom.setAttribute("data-testid", "image-upload");
      dom.setAttribute("contenteditable", "false");
      dom.setAttribute("role", "status");
      dom.textContent = node.attrs.kind === "asset" ? "Uploading PDF…" : "Uploading image…";
      return { dom };
    };
  },
  addProseMirrorPlugins() {
    const options = this.options;
    if (!options.upload) return [];
    const insert = (view: EditorView, files: File[], from: number, to: number) => {
      view.dispatch(placeholdersTr(view.state, from, to, files));
      view.focus();
    };
    return [
      new Plugin({
        key: new PluginKey("imageUpload"),
        props: {
          transformPastedHTML: (html) => htmlWithPlaceholders(html),
          handlePaste(view, event) {
            const data = event.clipboardData;
            const files = transferFiles(data);
            if (!files.length || !view.editable) return false;
            // A PDF is never a picture of the text beside it, so it always pastes as a file.
            if (!files.some(isPdf) && !pastesFiles(data?.getData("text/html") ?? "", data?.getData("text/plain") ?? "")) return false;
            for (const id of fromHtml) pending.delete(id);
            fromHtml = [];
            const { from, to } = view.state.selection;
            insert(view, files, from, to);
            return true;
          },
          handleDrop(view, event, _slice, moved) {
            if (moved || !view.editable) return false;
            const files = transferFiles(event.dataTransfer);
            if (!files.length) return false;
            const at = view.posAtCoords({ left: event.clientX, top: event.clientY });
            if (!at) return false;
            event.preventDefault();
            insert(view, files, at.pos, at.pos);
            return true;
          },
        },
        appendTransaction: (trs, _old, state) => (addsPlaceholder(trs) ? tidyTr(state) : null),
        view: (view) => {
          startUploads(view, options);
          return { update: (v) => startUploads(v, options) };
        },
      }),
    ];
  },
});
