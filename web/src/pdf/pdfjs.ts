// pdf.js, loaded on demand: only `load.ts` imports this module, dynamically, so pdf.js and its
// worker are a chunk of their own that a page with no PDF never fetches.
//
// Documents are shared: the viewer and every thumbnail of one asset use one loaded document,
// counted by `acquire` and `release`, and destroyed a while after the last user lets it go.

// The legacy build: the modern one calls JavaScript newer than many phones' browsers have
// (`Map.prototype.getOrInsertComputed`), and the legacy one carries the polyfills.
import { GlobalWorkerOptions, PDFWorker, TextLayer, getDocument } from "pdfjs-dist/legacy/build/pdf.mjs";
import type { PDFDocumentLoadingTask, PDFDocumentProxy, PDFPageProxy, RenderTask } from "pdfjs-dist";
import workerUrl from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";
import "./pdf.css";

GlobalWorkerOptions.workerSrc = workerUrl;

export { TextLayer };
export type { PDFDocumentProxy, PDFPageProxy, RenderTask };

interface Held {
  task: PDFDocumentLoadingTask;
  refs: number;
  timer?: ReturnType<typeof setTimeout>;
}

const held = new Map<string, Held>();

// One worker for every document: without it, pdf.js starts a worker per document, and a page of
// thumbnails would start dozens.
let worker: PDFWorker | undefined;

/** How long a document no one uses stays loaded, in case it is shown again. */
const KEEP_MS = 30_000;

/**
 * The document at `url`, loaded once however many ask. Fetches only the byte ranges it needs
 * when the server serves ranges (crates/api/src/assets.rs). Pair every call with `release`.
 */
export function acquire(url: string): Promise<PDFDocumentProxy> {
  let h = held.get(url);
  if (!h) {
    worker ??= new PDFWorker();
    const task = getDocument({ url, worker, disableAutoFetch: true, rangeChunkSize: 128 * 1024 });
    h = { task, refs: 0 };
    held.set(url, h);
    // A document that fails to load is not kept, so the next try fetches it again.
    task.promise.catch(() => {
      if (held.get(url)?.task === task) held.delete(url);
    });
  }
  if (h.timer) clearTimeout(h.timer);
  h.timer = undefined;
  h.refs++;
  return h.task.promise;
}

/** Let go of a document `acquire` gave. */
export function release(url: string) {
  const h = held.get(url);
  if (!h) return;
  h.refs = Math.max(0, h.refs - 1);
  if (h.refs > 0) return;
  h.timer = setTimeout(() => {
    if (held.get(url) !== h || h.refs > 0) return;
    held.delete(url);
    void h.task.destroy();
  }, KEEP_MS);
}

const thumbs = new Map<string, Promise<string>>();

/**
 * The first page of the PDF at `url` as an image `cssWidth` pixels wide, as a data URL. Kept
 * for the page's life, so a list of thumbnails draws each once.
 */
export function thumbnail(url: string, cssWidth: number): Promise<string> {
  const key = `${cssWidth} ${url}`;
  let t = thumbs.get(key);
  if (!t) {
    t = (async () => {
      const doc = await acquire(url);
      try {
        const page = await doc.getPage(1);
        const base = page.getViewport({ scale: 1 });
        const ratio = Math.min(2, globalThis.devicePixelRatio || 1);
        const viewport = page.getViewport({ scale: (cssWidth * ratio) / base.width });
        const canvas = document.createElement("canvas");
        canvas.width = Math.max(1, Math.floor(viewport.width));
        canvas.height = Math.max(1, Math.floor(viewport.height));
        await page.render({ canvas, viewport }).promise;
        page.cleanup();
        return canvas.toDataURL("image/png");
      } finally {
        release(url);
      }
    })();
    thumbs.set(key, t);
    t.catch(() => thumbs.delete(key));
  }
  return t;
}
