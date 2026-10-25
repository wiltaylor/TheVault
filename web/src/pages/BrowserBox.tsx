// The browser box's desktop inside the app (browser B4): noVNC, which the server proxies at
// /browser/ for a password login only, framed to fill the pane, so the desktop can sit in a pane
// beside an agent's conversation while it drives the browser. Same origin, so the frame needs no
// extra permission; the apps cannot reach /browser at all and say so.

import { Show, createResource, createSignal } from "solid-js";
import { type BrowserStatus, api } from "../api";
import { appInfo } from "../inApp";
import { BROWSER_DESKTOP, BROWSER_SETUP_DOC } from "./Integrations";

export function BrowserBox() {
  const [app] = createResource(appInfo);
  const [status, { refetch }] = createResource<BrowserStatus>(() => api.browserStatus());
  // A new key remounts the frame: Reconnect after the box restarted or the session dropped.
  const [key, setKey] = createSignal(0);
  let frame: HTMLIFrameElement | undefined;

  function reconnect() {
    void refetch();
    setKey((k) => k + 1);
  }

  return (
    <section class="browser-box" aria-label="Browser box">
      <header class="browser-box-bar">
        <h1>Browser box</h1>
        <Show when={status()?.reachable}>
          <span class="muted">
            playwright-cli {status()?.playwright_cli ?? ""}
          </span>
        </Show>
        <span class="browser-box-actions">
          <button type="button" onClick={reconnect}>
            Reconnect
          </button>
          <button type="button" onClick={() => void frame?.requestFullscreen?.()}>
            Full screen
          </button>
          <a class="button" href={BROWSER_DESKTOP} target="_blank" rel="noopener noreferrer">
            Open in a new tab
          </a>
        </span>
      </header>
      <Show
        when={app.state === "ready" && !app()}
        fallback={
          <Show when={app.state === "ready"}>
            <p class="muted">The browser box's desktop opens in the web app.</p>
          </Show>
        }
      >
        <Show when={status.error}>
          <p class="error" role="alert">
            {String(status.error instanceof Error ? status.error.message : status.error)}
          </p>
        </Show>
        <Show when={status()}>
          {(s) => (
            <Show
              when={s().configured}
              fallback={
                <p>
                  Not set up.{" "}
                  <a href={BROWSER_SETUP_DOC} target="_blank" rel="noopener noreferrer">
                    How to turn on the browser box
                  </a>
                </p>
              }
            >
              <Show
                when={s().reachable}
                fallback={
                  <p class="error" role="alert">
                    The browser box does not answer{s().error ? `: ${s().error}` : "."}
                  </p>
                }
              >
                <Show when={key() + 1} keyed>
                  {(_attempt) => (
                    <iframe
                      ref={frame}
                      class="browser-box-frame"
                      title="Browser box desktop"
                      src={BROWSER_DESKTOP}
                      allow="fullscreen; clipboard-read; clipboard-write"
                    />
                  )}
                </Show>
              </Show>
            </Show>
          )}
        </Show>
      </Show>
    </section>
  );
}
