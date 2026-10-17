// The sidebar footer's sync icon, in the desktop and Android apps only (D18, D19, D20, D27): green
// when synced, yellow while syncing or while changes wait, red when sync failed. Its name and
// tooltip say the state in words; clicking it opens the details and a "Sync now" button.

import { Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { Portal } from "solid-js/web";
import type { AppSync } from "../api";
import { appInfo } from "../inApp";
import { lastSynced, needsAction, syncLabel } from "../sync/model";
import { followSync, syncNow, syncStatus } from "../sync/store";
import { showToast } from "./Toasts";

const PANEL_ID = "sync-panel";

/** Two arrows chasing each other round a circle. */
function SyncGlyph() {
  return (
    <svg
      class="icon sync-glyph"
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
      <path d="M3 3v5h5" />
      <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16" />
      <path d="M16 16h5v5" />
    </svg>
  );
}

/** The status dot: filled when synced, a ring while syncing, a filled dot with a bar when red. */
function Dot(props: { tone: AppSync["state"] }) {
  return (
    <svg class="sync-dot" width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
      <Show
        when={props.tone === "syncing"}
        fallback={
          <>
            <circle cx="5" cy="5" r="4.5" fill="currentColor" stroke="var(--panel)" stroke-width="1" />
            <Show when={props.tone === "error"}>
              <path d="M5 2.6v2.8M5 7.2v.1" stroke="var(--panel)" stroke-width="1.4" stroke-linecap="round" />
            </Show>
          </>
        }
      >
        <circle cx="5" cy="5" r="3.6" fill="var(--panel)" stroke="currentColor" stroke-width="2" />
      </Show>
    </svg>
  );
}

/** The icon, shown once the page knows it runs in an app. In a browser it renders nothing. */
export function SyncStatusButton() {
  const [inApp, setInApp] = createSignal(false);
  onMount(() => {
    let stop: (() => void) | undefined;
    let gone = false;
    void appInfo().then((info) => {
      if (!info || gone) return;
      setInApp(true);
      stop = followSync();
    });
    onCleanup(() => {
      gone = true;
      stop?.();
    });
  });
  return (
    <Show when={inApp() && syncStatus()}>
      {(s) => <Button status={s()} />}
    </Show>
  );
}

/** The phases the one-time notice has been shown for, on this page load. */
const noticed = new Set<string>();

function Button(props: { status: AppSync }) {
  let button!: HTMLButtonElement;
  const [open, setOpen] = createSignal(false);
  const [now, setNow] = createSignal(Date.now());
  const clock = setInterval(() => setNow(Date.now()), 15_000);
  onCleanup(() => clearInterval(clock));
  const label = () => syncLabel(props.status, now());

  // A state the user has to act on is said once, as well as by the red icon.
  createEffect(() => {
    const text = needsAction(props.status);
    if (!text || noticed.has(props.status.phase)) return;
    noticed.add(props.status.phase);
    showToast(text, props.status.phase === "update_required" ? { label: "Check for updates", href: "/settings" } : undefined);
  });

  return (
    <>
      <button
        ref={button}
        type="button"
        class={`footer-button sync-button sync-${props.status.state}`}
        data-sync-state={props.status.state}
        data-phase={props.status.phase}
        data-queued={props.status.queued}
        aria-label={label()}
        title={label()}
        aria-haspopup="dialog"
        aria-expanded={open()}
        aria-controls={open() ? PANEL_ID : undefined}
        onClick={() => setOpen(!open())}
      >
        <SyncGlyph />
        <Dot tone={props.status.state} />
      </button>
      <Show when={open()}>
        <Portal>
          <Panel anchor={button} status={props.status} now={now()} label={label()} close={() => setOpen(false)} />
        </Portal>
      </Show>
    </>
  );
}

function Panel(props: { anchor: HTMLButtonElement; status: AppSync; now: number; label: string; close: () => void }) {
  let panel!: HTMLElement;
  const [place, setPlace] = createSignal({ bottom: 0, left: 0 });
  const [busy, setBusy] = createSignal(false);

  function position() {
    const r = props.anchor.getBoundingClientRect();
    const width = Math.min(288, window.innerWidth - 16);
    setPlace({
      bottom: window.innerHeight - r.top + 8,
      left: Math.max(8, Math.min(r.left - 8, window.innerWidth - width - 8)),
    });
  }

  function close(refocus: boolean) {
    props.close();
    if (refocus) props.anchor.focus();
  }

  onMount(() => {
    position();
    const onPointer = (e: PointerEvent) => {
      const t = e.target as Node | null;
      if (t && (panel.contains(t) || props.anchor.contains(t))) return;
      close(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) {
        e.preventDefault();
        close(true);
      }
    };
    window.addEventListener("resize", position);
    document.addEventListener("pointerdown", onPointer, true);
    document.addEventListener("keydown", onKey);
    panel.focus();
    onCleanup(() => {
      window.removeEventListener("resize", position);
      document.removeEventListener("pointerdown", onPointer, true);
      document.removeEventListener("keydown", onKey);
    });
  });

  const exact = () => (props.status.last_ok_ms === null ? undefined : new Date(props.status.last_ok_ms).toLocaleString());

  return (
    <section
      ref={panel}
      id={PANEL_ID}
      class="sync-panel"
      style={{ bottom: `${place().bottom}px`, left: `${place().left}px` }}
      role="dialog"
      aria-labelledby="sync-heading"
      tabindex={-1}
      onFocusOut={(e) => {
        const to = e.relatedTarget as Node | null;
        if (to && !panel.contains(to) && !props.anchor.contains(to)) close(false);
      }}
    >
      <h2 id="sync-heading">Sync</h2>
      <p class={`sync-summary sync-${props.status.state}`} role="status">
        <Dot tone={props.status.state} />
        <span>{props.label}</span>
      </p>
      <dl class="sync-details">
        <dt>Last synced</dt>
        <dd title={exact()}>{lastSynced(props.status, props.now)}</dd>
        <dt>Waiting</dt>
        <dd>{props.status.queued === 0 ? "Nothing" : `${props.status.queued} ${props.status.queued === 1 ? "change" : "changes"}`}</dd>
        <dt>Server</dt>
        <dd class="sync-server">{props.status.server ?? "Not paired"}</dd>
        <Show when={props.status.state === "error" && props.status.last_error}>
          {(message) => (
            <>
              <dt>Last error</dt>
              <dd class="sync-error">{message()}</dd>
            </>
          )}
        </Show>
      </dl>
      <div class="sync-actions">
        <Show when={props.status.phase === "update_required"}>
          <a href="/settings" class="sync-link" onClick={() => close(false)}>
            Check for updates
          </a>
        </Show>
        <button
          type="button"
          class="primary"
          disabled={busy() || props.status.phase === "unpaired"}
          onClick={async () => {
            setBusy(true);
            try {
              await syncNow();
            } finally {
              setBusy(false);
            }
          }}
        >
          {busy() ? "Syncing…" : "Sync now"}
        </button>
      </div>
    </section>
  );
}
