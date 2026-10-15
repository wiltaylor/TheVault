// A PDF viewer drawn with pdf.js, which every browser runs, the phone's included: native PDF
// embedding shows nothing on Android Chrome.
//
// Pages are canvases with a text layer over each, so text selects and copies. Only the pages near
// the viewport are drawn (model.ts, `pagesNear`); the rest are empty boxes of the right size, so
// the scroll bar is true from the start. The controls float over the top of the viewer, as the
// ink and canvas controls do. Pages keep their own colours in dark mode; only the chrome darkens.

import { For, type JSX, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount, untrack } from "solid-js";
import type { PDFDocumentProxy, PDFPageProxy, PdfLib, RenderTask } from "./types";
import { pdfLib } from "./load";
import {
  type Match,
  type PageSize,
  VIEWER_HEIGHT,
  clampHeight,
  currentPage,
  findMatches,
  fitScale,
  layout,
  normalise,
  pageLabel,
  pagesNear,
  scrollFor,
  stepMatch,
  zoomStep,
} from "./model";
import {
  ChevronDownIcon,
  ChevronUpIcon,
  CloseIcon,
  DownloadIcon,
  ExternalIcon,
  FitWidthIcon,
  MaximiseIcon,
  MinimiseIcon,
  SearchIcon,
  ZoomInIcon,
  ZoomOutIcon,
} from "./PdfIcons";
import "./viewer.css";

export interface PdfViewerProps {
  /** Where the PDF's bytes are fetched. Read once: key the viewer on it to show another. */
  url: string;
  filename: string;
  /** The viewer's height in CSS pixels. Without it the viewer fills its parent. */
  height?: number;
  /** Set to give the viewer a drag handle on its bottom edge that sets its height. */
  onHeight?: (height: number) => void;
  /** More buttons for the end of the controls, such as close. */
  extra?: JSX.Element;
  /** Focus the viewer once it opens, so the keyboard scrolls it at once. */
  autofocus?: boolean;
}

/** The largest canvas drawn, in device pixels: browsers refuse larger ones, phones sooner. */
const MAX_CANVAS_PIXELS = 4096 * 4096;

export function PdfViewer(props: PdfViewerProps) {
  const url = untrack(() => props.url);
  let root!: HTMLDivElement;
  let scroller!: HTMLDivElement;
  let controls!: HTMLDivElement;
  let findInput: HTMLInputElement | undefined;

  const [lib, setLib] = createSignal<PdfLib | null>(null);
  const [doc, setDoc] = createSignal<PDFDocumentProxy | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [count, setCount] = createSignal(0);
  const [sizes, setSizes] = createSignal<(PageSize | undefined)[]>([]);
  const [scale, setScale] = createSignal(1);
  const [fit, setFit] = createSignal(true);
  const [view, setView] = createSignal({ top: 0, width: 0, height: 0 });
  const [full, setFull] = createSignal(false);
  // Find: the box open or not, the text, every page's text once fetched, and the match on show.
  const [finding, setFinding] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [texts, setTexts] = createSignal<string[] | null>(null);
  const [active, setActive] = createSignal(-1);
  const [heightPreview, setHeightPreview] = createSignal<number | null>(null);

  let disposed = false;
  // The library holding the document, once acquired, to release it when the viewer goes.
  let held: PdfLib | null = null;
  onMount(() => {
    void (async () => {
      try {
        const l = await pdfLib();
        if (disposed) return;
        setLib(l);
        const d = await l.acquire(url);
        if (disposed) {
          l.release(url);
          return;
        }
        held = l;
        const first = await d.getPage(1);
        if (disposed) return;
        const v = first.getViewport({ scale: 1 });
        setSizes([{ width: v.width, height: v.height }]);
        setCount(d.numPages);
        setDoc(d);
      } catch (e) {
        if (!disposed) setError(e instanceof Error ? e.message : String(e));
      }
    })();
    const sizer =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(() => setView((v) => ({ ...v, width: scroller.clientWidth, height: scroller.clientHeight })));
    sizer?.observe(scroller);
    const controlsSizer =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => setControlsHeight(controls.offsetHeight + 6));
    controlsSizer?.observe(controls);
    setControlsHeight(controls.offsetHeight + 6);
    setView({ top: scroller.scrollTop, width: scroller.clientWidth, height: scroller.clientHeight });
    const onFull = () => setFull(document.fullscreenElement === root || root.classList.contains("pseudo-full"));
    document.addEventListener("fullscreenchange", onFull);
    if (props.autofocus) scroller.focus({ preventScroll: true });
    onCleanup(() => {
      disposed = true;
      sizer?.disconnect();
      controlsSizer?.disconnect();
      document.removeEventListener("fullscreenchange", onFull);
    });
  });
  onCleanup(() => {
    disposed = true;
    held?.release(url);
    held = null;
  });

  // The controls' height: the first page starts below them, and a page gone to sits below them.
  const [controlsHeight, setControlsHeight] = createSignal(40);
  const pageLayout = createMemo(() => layout(sizes(), count(), scale(), controlsHeight() + 8));
  const near = createMemo(() => pagesNear(pageLayout(), view().top, view().height || 600, 1));
  const shownPages = createMemo(() => {
    const r = near();
    if (!r || !doc()) return [];
    const out: number[] = [];
    for (let i = r.first; i <= r.last; i++) out.push(i);
    return out;
  });
  const current = createMemo(() => currentPage(pageLayout(), view().top, view().height));
  const contentWidth = createMemo(() => Math.max(view().width, Math.max(0, ...pageLayout().widths) + 24));

  // Fit to width while fitting is on: on open, and whenever the viewer's width changes.
  createEffect(() => {
    const first = sizes()[0];
    const width = view().width;
    if (!fit() || !first || !width) return;
    const next = fitScale(first.width, width);
    if (Math.abs(next - untrack(scale)) > 0.001) zoomTo(next, true);
  });

  function onScroll() {
    setView((v) => ({ ...v, top: scroller.scrollTop }));
  }

  /** Change the scale, keeping the spot at the top of the view where it is. */
  function zoomTo(next: number, keepFit = false) {
    if (!keepFit) setFit(false);
    const before = pageLayout();
    const p = current();
    const top = before.tops[p] ?? 0;
    const h = before.heights[p] || 1;
    const frac = (scroller.scrollTop - top) / h;
    const fracX = scroller.scrollWidth > scroller.clientWidth ? (scroller.scrollLeft + scroller.clientWidth / 2) / scroller.scrollWidth : 0.5;
    setScale(next);
    const after = pageLayout();
    scroller.scrollTop = (after.tops[p] ?? 0) + frac * (after.heights[p] ?? 0);
    scroller.scrollLeft = fracX * scroller.scrollWidth - scroller.clientWidth / 2;
    onScroll();
  }

  function goTo(index: number) {
    scroller.scrollTop = scrollFor(pageLayout(), index);
    onScroll();
  }

  function onSize(index: number, size: PageSize) {
    const known = sizes()[index];
    if (known && known.width === size.width && known.height === size.height) return;
    setSizes((s) => {
      const next = s.slice();
      next[index] = size;
      return next;
    });
  }

  // ---- fullscreen ---------------------------------------------------------------------------

  async function toggleFull() {
    if (document.fullscreenElement === root) {
      await document.exitFullscreen().catch(() => {});
      return;
    }
    if (root.classList.contains("pseudo-full")) {
      root.classList.remove("pseudo-full");
      setFull(false);
      return;
    }
    try {
      if (!root.requestFullscreen) throw new Error("no fullscreen");
      await root.requestFullscreen();
    } catch {
      // No element fullscreen (iOS Safari): cover the window instead.
      root.classList.add("pseudo-full");
      setFull(true);
    }
  }

  // ---- find ---------------------------------------------------------------------------------

  const matches = createMemo<Match[]>(() => {
    const t = texts();
    return t ? findMatches(t, query()) : [];
  });

  async function loadTexts() {
    const d = doc();
    if (!d || texts()) return;
    const out: string[] = [];
    for (let i = 1; i <= d.numPages; i++) {
      const page = await d.getPage(i);
      const content = await page.getTextContent();
      out.push(content.items.map((it) => ("str" in it ? it.str + (it.hasEOL ? "\n" : " ") : "")).join(""));
    }
    if (!disposed) setTexts(out);
  }

  function openFind() {
    setFinding(true);
    void loadTexts();
    queueMicrotask(() => findInput?.select());
  }

  function closeFind() {
    setFinding(false);
    setQuery("");
    setActive(-1);
    scroller.focus({ preventScroll: true });
  }

  function step(dir: 1 | -1) {
    const n = stepMatch(matches().length, active(), dir);
    setActive(n);
    const m = matches()[n];
    if (m && m.page !== current()) goTo(m.page);
  }

  // A new query shows its first match.
  createEffect(
    on([query, texts], () => {
      setActive(-1);
      if (matches().length) step(1);
    }),
  );

  function onKey(e: KeyboardEvent) {
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === "f") {
      e.preventDefault();
      e.stopPropagation();
      openFind();
    } else if (e.key === "Escape" && finding()) {
      e.preventDefault();
      e.stopPropagation();
      closeFind();
    } else if (mod && (e.key === "=" || e.key === "+")) {
      e.preventDefault();
      zoomTo(zoomStep(scale(), 1));
    } else if (mod && e.key === "-") {
      e.preventDefault();
      zoomTo(zoomStep(scale(), -1));
    }
  }

  // ---- the height handle --------------------------------------------------------------------

  const height = () => heightPreview() ?? props.height;
  let heightDrag: { pointer: number; sy: number; from: number } | null = null;

  function onHeightDown(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    try {
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    } catch {
      // A synthetic pointer still sends its events here.
    }
    const from = props.height ?? VIEWER_HEIGHT.initial;
    heightDrag = { pointer: e.pointerId, sy: e.clientY, from };
    setHeightPreview(from);
  }

  function onHeightMove(e: PointerEvent) {
    const d = heightDrag;
    if (!d || e.pointerId !== d.pointer) return;
    setHeightPreview(clampHeight(d.from + e.clientY - d.sy));
  }

  function onHeightUp(e: PointerEvent) {
    const d = heightDrag;
    if (!d || e.pointerId !== d.pointer) return;
    heightDrag = null;
    const h = clampHeight(d.from + e.clientY - d.sy);
    setHeightPreview(null);
    if (h !== props.height) props.onHeight?.(h);
  }

  function onHeightKey(e: KeyboardEvent) {
    const now = props.height ?? VIEWER_HEIGHT.initial;
    const stepBy = e.shiftKey ? 160 : 40;
    let next: number | null = null;
    if (e.key === "ArrowDown") next = now + stepBy;
    else if (e.key === "ArrowUp") next = now - stepBy;
    else if (e.key === "Home") next = VIEWER_HEIGHT.min;
    else if (e.key === "End") next = VIEWER_HEIGHT.max;
    if (next === null) return;
    e.preventDefault();
    if (clampHeight(next) !== now) props.onHeight?.(clampHeight(next));
  }

  const label = () => `PDF ${props.filename}`;

  return (
    <div
      ref={root}
      class="pdf-viewer"
      classList={{ fill: props.height === undefined, full: full() }}
      role="region"
      aria-label={label()}
      data-testid="pdf-viewer"
      data-node-view-interactive=""
      onKeyDown={onKey}
    >
      <div class="pdf-stage" style={height() !== undefined && !full() ? { height: `${height()}px` } : undefined}>
        <div ref={controls} class="pdf-controls" data-testid="pdf-controls">
          <div class="ink-cluster" role="toolbar" aria-label="Pages">
            <button
              type="button"
              class="ink-action"
              aria-label="Previous page"
              title="Previous page"
              disabled={current() <= 0}
              onClick={() => goTo(current() - 1)}
            >
              <ChevronUpIcon />
            </button>
            <span class="pdf-page-label" data-testid="pdf-page" aria-live="polite">
              {pageLabel(current(), count())}
            </span>
            <button
              type="button"
              class="ink-action"
              aria-label="Next page"
              title="Next page"
              disabled={current() >= count() - 1}
              onClick={() => goTo(current() + 1)}
            >
              <ChevronDownIcon />
            </button>
          </div>
          <div class="ink-cluster" role="toolbar" aria-label="View">
            <button type="button" class="ink-action" aria-label="Zoom out" title="Zoom out" onClick={() => zoomTo(zoomStep(scale(), -1))}>
              <ZoomOutIcon />
            </button>
            <span class="pdf-zoom-label" data-testid="pdf-zoom" aria-label="Zoom">
              {Math.round(scale() * 100)}%
            </span>
            <button type="button" class="ink-action" aria-label="Zoom in" title="Zoom in" onClick={() => zoomTo(zoomStep(scale(), 1))}>
              <ZoomInIcon />
            </button>
            <button
              type="button"
              class="ink-action"
              aria-label="Fit width"
              title="Fit width"
              aria-pressed={fit()}
              onClick={() => setFit(true)}
            >
              <FitWidthIcon />
            </button>
          </div>
          <div class="ink-cluster pdf-actions" role="toolbar" aria-label="Actions">
            <button
              type="button"
              class="ink-action"
              aria-label="Find in PDF"
              title="Find in PDF (Ctrl+F)"
              aria-pressed={finding()}
              disabled={!doc()}
              onClick={() => (finding() ? closeFind() : openFind())}
            >
              <SearchIcon />
            </button>
            <a class="ink-action" href={url} download={props.filename} aria-label="Download" title="Download">
              <DownloadIcon />
            </a>
            <a class="ink-action" href={url} target="_blank" rel="noopener" aria-label="Open in new tab" title="Open in new tab">
              <ExternalIcon />
            </a>
            <button
              type="button"
              class="ink-action"
              aria-label={full() ? "Exit fullscreen" : "Fullscreen"}
              title={full() ? "Exit fullscreen" : "Fullscreen"}
              aria-pressed={full()}
              onClick={() => void toggleFull()}
            >
              <Show when={full()} fallback={<MaximiseIcon />}>
                <MinimiseIcon />
              </Show>
            </button>
            {props.extra}
          </div>
          <Show when={finding()}>
            <div class="pdf-find-row">
              <div class="ink-cluster pdf-find" role="search" aria-label="Find in PDF">
                <input
                  ref={findInput}
                  type="search"
                  aria-label="Find text"
                  placeholder="Find"
                  value={query()}
                  onInput={(e) => setQuery(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      step(e.shiftKey ? -1 : 1);
                    }
                  }}
                />
                <span class="pdf-find-count" data-testid="pdf-find-count">
                  {!query().trim() ? "" : texts() === null ? "…" : matches().length ? `${active() + 1} of ${matches().length}` : "No matches"}
                </span>
                <button type="button" class="ink-action" aria-label="Previous match" title="Previous match" disabled={!matches().length} onClick={() => step(-1)}>
                  <ChevronUpIcon />
                </button>
                <button type="button" class="ink-action" aria-label="Next match" title="Next match" disabled={!matches().length} onClick={() => step(1)}>
                  <ChevronDownIcon />
                </button>
                <button type="button" class="ink-action" aria-label="Close find" title="Close find" onClick={closeFind}>
                  <CloseIcon />
                </button>
              </div>
            </div>
          </Show>
        </div>
        <div ref={scroller} class="pdf-scroll" tabIndex={0} aria-label={`${props.filename}, pages`} onScroll={onScroll}>
          <Show when={error()}>
            <p class="pdf-message" role="alert">
              Could not show this PDF.{" "}
              <a href={url} download={props.filename}>
                Download it
              </a>
            </p>
          </Show>
          <Show when={!doc() && !error()}>
            <p class="pdf-message" role="status">
              Loading PDF…
            </p>
          </Show>
          <div class="pdf-pages" style={{ height: `${pageLayout().total}px`, width: `${contentWidth()}px` }}>
            <For each={shownPages()}>
              {(i) => (
                <PdfPage
                  lib={lib()!}
                  doc={doc()!}
                  index={i}
                  scale={scale()}
                  top={pageLayout().tops[i] ?? 0}
                  left={Math.max(12, (contentWidth() - (pageLayout().widths[i] ?? 0)) / 2)}
                  width={pageLayout().widths[i] ?? 0}
                  height={pageLayout().heights[i] ?? 0}
                  query={finding() ? query() : ""}
                  onSize={onSize}
                />
              )}
            </For>
          </div>
        </div>
      </div>
      <Show when={props.onHeight && !full()}>
        <div
          class="canvas-height-handle pdf-height-handle"
          role="slider"
          tabIndex={0}
          aria-label="Viewer height"
          aria-orientation="vertical"
          aria-valuemin={VIEWER_HEIGHT.min}
          aria-valuemax={VIEWER_HEIGHT.max}
          aria-valuenow={height() ?? VIEWER_HEIGHT.initial}
          title="Drag to set the viewer's height"
          onPointerDown={onHeightDown}
          onPointerMove={onHeightMove}
          onPointerUp={onHeightUp}
          onPointerCancel={onHeightUp}
          onKeyDown={onHeightKey}
        >
          <span class="canvas-height-grip" />
        </div>
      </Show>
    </div>
  );
}

/** One drawn page: its canvas and its text layer, drawn again when the scale changes. */
function PdfPage(props: {
  lib: PdfLib;
  doc: PDFDocumentProxy;
  index: number;
  scale: number;
  top: number;
  left: number;
  width: number;
  height: number;
  query: string;
  onSize: (index: number, size: PageSize) => void;
}) {
  let box!: HTMLDivElement;
  let textBox!: HTMLDivElement;
  let canvas: HTMLCanvasElement | null = null;
  let task: RenderTask | null = null;
  let textLayer: InstanceType<PdfLib["TextLayer"]> | null = null;
  let page: PDFPageProxy | null = null;
  let alive = true;
  const [drawn, setDrawn] = createSignal(false);
  const [textDrawn, setTextDrawn] = createSignal(0);

  onCleanup(() => {
    alive = false;
    task?.cancel();
    textLayer?.cancel();
    page?.cleanup();
  });

  createEffect(
    on(
      () => props.scale,
      (scale) => void draw(scale),
    ),
  );

  async function draw(scale: number) {
    task?.cancel();
    textLayer?.cancel();
    try {
      page ??= await props.doc.getPage(props.index + 1);
      if (!alive || scale !== props.scale) return;
      const base = page.getViewport({ scale: 1 });
      props.onSize(props.index, { width: base.width, height: base.height });
      const viewport = page.getViewport({ scale });
      let ratio = Math.min(3, globalThis.devicePixelRatio || 1);
      const area = viewport.width * viewport.height * ratio * ratio;
      if (area > MAX_CANVAS_PIXELS) ratio *= Math.sqrt(MAX_CANVAS_PIXELS / area);
      // Draw into a new canvas and swap it in when done, so the old drawing shows meanwhile.
      const next = document.createElement("canvas");
      next.width = Math.max(1, Math.floor(viewport.width * ratio));
      next.height = Math.max(1, Math.floor(viewport.height * ratio));
      next.className = "pdf-canvas";
      next.setAttribute("aria-hidden", "true");
      task = page.render({ canvas: next, viewport, transform: ratio === 1 ? undefined : [ratio, 0, 0, ratio, 0, 0] });
      await task.promise;
      if (!alive || scale !== props.scale) return;
      if (canvas) canvas.replaceWith(next);
      else box.prepend(next);
      canvas = next;
      setDrawn(true);

      textBox.replaceChildren();
      textBox.style.setProperty("--total-scale-factor", String(scale));
      textLayer = new props.lib.TextLayer({ textContentSource: page.streamTextContent(), container: textBox, viewport });
      await textLayer.render();
      if (alive && scale === props.scale) setTextDrawn((n) => n + 1);
    } catch (e) {
      // A cancelled draw is the next one starting; anything else leaves the page blank.
      if (!(e instanceof Error && e.name === "RenderingCancelledException")) console.warn("pdf page", props.index + 1, e);
    }
  }

  // Mark the text runs holding the find text.
  createEffect(() => {
    textDrawn();
    const q = normalise(props.query);
    for (const span of Array.from(textBox.querySelectorAll("span"))) {
      const hit = q !== "" && normalise(span.textContent ?? "").includes(q);
      span.classList.toggle("pdf-hit", hit);
    }
  });

  return (
    <div
      ref={box}
      class="pdf-page"
      classList={{ drawn: drawn() }}
      data-page={props.index + 1}
      style={{ top: `${props.top}px`, left: `${props.left}px`, width: `${props.width}px`, height: `${props.height}px` }}
    >
      <div ref={textBox} class="textLayer" />
    </div>
  );
}
