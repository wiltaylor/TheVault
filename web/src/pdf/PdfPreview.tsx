// A PDF's first page as a thumbnail, and the button that opens the viewer over the app from it.

import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { Portal } from "solid-js/web";
import { assetUrl } from "../api";
import { pdfLib } from "./load";
import { CloseIcon, PdfFileIcon } from "./PdfIcons";
import { PdfViewer } from "./PdfViewer";
import "./viewer.css";

/** The first page of the PDF at `url`, `width` CSS pixels wide, drawn once it scrolls into view. */
export function PdfThumb(props: { url: string; width: number; alt?: string }) {
  let box!: HTMLSpanElement;
  const [src, setSrc] = createSignal<string | null>(null);
  const [failed, setFailed] = createSignal(false);
  onMount(() => {
    let alive = true;
    onCleanup(() => {
      alive = false;
    });
    const draw = () => {
      pdfLib()
        .then((lib) => lib.thumbnail(props.url, props.width))
        .then(
          (s) => alive && setSrc(s),
          () => alive && setFailed(true),
        );
    };
    if (typeof IntersectionObserver === "undefined") return draw();
    const seen = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        seen.disconnect();
        draw();
      }
    });
    seen.observe(box);
    onCleanup(() => seen.disconnect());
  });
  return (
    <span
      ref={box}
      class="pdf-thumb"
      classList={{ failed: failed() }}
      data-testid="pdf-thumb"
      style={{ width: `${props.width}px`, "min-height": src() ? undefined : `${Math.round(props.width * 1.29)}px` }}
    >
      <Show when={src()} fallback={<PdfFileIcon size={Math.min(32, Math.round(props.width / 2))} />}>
        {(s) => <img src={s()} alt={props.alt ?? ""} />}
      </Show>
    </span>
  );
}

/** The viewer over the whole app, closed by Escape, its close button or a click beside it. */
export function PdfOverlay(props: { url: string; filename: string; onClose: () => void }) {
  let opener: Element | null = null;
  onMount(() => {
    opener = document.activeElement;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented && !document.fullscreenElement) {
        e.preventDefault();
        props.onClose();
      }
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => {
      document.removeEventListener("keydown", onKey);
      if (opener instanceof HTMLElement) opener.focus({ preventScroll: true });
    });
  });
  return (
    <Portal>
      <div
        class="pdf-overlay"
        role="dialog"
        aria-modal="true"
        aria-label={props.filename}
        data-testid="pdf-overlay"
        onClick={(e) => {
          if (e.target === e.currentTarget) props.onClose();
        }}
      >
        <div class="pdf-overlay-panel">
          <PdfViewer
            url={props.url}
            filename={props.filename}
            autofocus
            extra={
              <>
                <span class="ink-divider" aria-hidden="true" />
                <button type="button" class="ink-action" aria-label="Close" title="Close (Esc)" onClick={() => props.onClose()}>
                  <CloseIcon />
                </button>
              </>
            }
          />
        </div>
      </div>
    </Portal>
  );
}

/**
 * A PDF asset as a first-page thumbnail; clicking it opens the viewer over the app. `named`
 * shows the file name under the thumbnail.
 */
export function PdfThumbButton(props: { id: string; filename: string; width?: number; named?: boolean }) {
  const [open, setOpen] = createSignal(false);
  return (
    <>
      <button
        type="button"
        class="pdf-thumb-button"
        aria-label={`View ${props.filename}`}
        title={`View ${props.filename}`}
        data-node-view-interactive=""
        onClick={() => setOpen(true)}
      >
        <PdfThumb url={assetUrl(props.id)} width={props.width ?? 64} />
        <Show when={props.named}>
          <span class="pdf-thumb-name">{props.filename}</span>
        </Show>
      </button>
      <Show when={open()}>
        <PdfOverlay url={assetUrl(props.id)} filename={props.filename} onClose={() => setOpen(false)} />
      </Show>
    </>
  );
}
