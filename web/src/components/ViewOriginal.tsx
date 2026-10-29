// View original (rich-import H6): an imported email's own HTML, as the sender wrote it, in a
// sandboxed frame. The frame's `sandbox` allows only popups that leave the sandbox, so a link
// opens in a tab of its own, and the page runs no script, submits no form and has no origin of
// its own; its policy, set in the page itself, lets in `data:` images (the
// inline images the server put in place) and inline styles, and remote images only once the user
// asks for them, since loading one tells the sender the mail was read.

import { Portal } from "solid-js/web";
import { Show, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { originalHtml } from "../api";
import { errorText } from "../pages/TypeEditor";

/** The policy of the shown page: no requests but `data:` images and inline styles, and with
 * `remote` set, `https:` images too. */
export function originalPolicy(remote: boolean): string {
  const img = remote ? "data: https:" : "data:";
  return `default-src 'none'; img-src ${img}; style-src 'unsafe-inline'; font-src data:; form-action 'none'; base-uri 'none'`;
}

/** `html` as the frame's document: the policy and a no-referrer rule first, in a head of their
 * own, so they hold before anything in the page loads. */
export function originalDocument(html: string, remote: boolean): string {
  const head = `<meta http-equiv="Content-Security-Policy" content="${originalPolicy(remote)}"><meta name="referrer" content="no-referrer"><base target="_blank">`;
  return `<!DOCTYPE html><html><head>${head}</head><body>${html}</body></html>`;
}

/** Whether `html` names an image to load from the network. */
export function hasRemoteImages(html: string): boolean {
  return /<img[^>]+src\s*=\s*["']?\s*(https?:)?\/\//i.test(html) || /url\(\s*["']?https?:/i.test(html);
}

/** The View original button of note `id`, and the dialog it opens. */
export function ViewOriginalButton(props: { id: string }) {
  const [open, setOpen] = createSignal(false);
  return (
    <>
      <button type="button" class="outbox-note-action" onClick={() => setOpen(true)}>
        View original
      </button>
      <Show when={open()}>
        <OriginalDialog id={props.id} onClose={() => setOpen(false)} />
      </Show>
    </>
  );
}

function OriginalDialog(props: { id: string; onClose: () => void }) {
  const [html] = createResource(() => props.id, originalHtml);
  const [remote, setRemote] = createSignal(false);
  let dialog!: HTMLDivElement;
  let close!: HTMLButtonElement;
  const returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  onMount(() => {
    close.focus();
    onCleanup(() => setTimeout(() => returnTo?.isConnected && returnTo.focus()));
  });
  return (
    <Portal>
      <div
        class="capture-backdrop"
        onMouseDown={(e) => {
          if (e.target === e.currentTarget) props.onClose();
        }}
      >
        <div
          ref={dialog}
          class="capture-dialog original-dialog"
          role="dialog"
          aria-modal="true"
          aria-labelledby="original-heading"
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              props.onClose();
            }
          }}
        >
          <header class="capture-head">
            <h2 id="original-heading">Original</h2>
            <Show when={!remote() && hasRemoteImages(html() ?? "")}>
              <button type="button" class="original-remote" onClick={() => setRemote(true)}>
                Load remote images
              </button>
            </Show>
            <button ref={close} type="button" class="capture-x" aria-label="Close" title="Close (Esc)" onClick={() => props.onClose()}>
              ×
            </button>
          </header>
          <Show when={html.error}>
            <p class="error" role="alert">
              {errorText(html.error)}
            </p>
          </Show>
          <Show when={html()}>
            {(text) => (
              <iframe
                class="original-frame"
                title="The original email"
                sandbox="allow-popups allow-popups-to-escape-sandbox"
                referrerpolicy="no-referrer"
                srcdoc={originalDocument(text(), remote())}
                data-testid="original-frame"
              />
            )}
          </Show>
        </div>
      </div>
    </Portal>
  );
}
