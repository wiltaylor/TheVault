// The bell at the top right and the panel it opens: the notifications scripts, agents and the
// server have left, newest first. Clicking one marks it read and opens its note or page in the
// active pane; Ctrl/Cmd+click or a middle-click opens it in a background tab instead.

import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { Portal } from "solid-js/web";
import type { AppNotification } from "../api";
import { BellIcon, TypeIcon } from "../components/Icon";
import { typeDef } from "../session";
import { openInBackground, wantsNewTab } from "../workspace/events";
import * as M from "../workspace/model";
import { scrollPaneTop } from "../workspace/pane";
import { navigatePane, workspace } from "../workspace/store";
import { badgeText, bellLabel, relativeTime, sourceLabel, targetOf } from "./model";
import {
  announced,
  clearReadItems,
  closePanel,
  dismiss,
  listFailed,
  listFailure,
  listLoaded,
  listLoading,
  loadMore,
  notificationList,
  panelOpen,
  readAll,
  readNotification,
  startPolling,
  togglePanel,
  unreadCount,
} from "./store";
import "./notifications.css";

const PANEL_ID = "notifications-panel";

/**
 * Follows the server's live stream (polling while it is down) while the app shows, and announces
 * new notifications politely to screen readers. Mount it once.
 */
export function NotificationsLive() {
  onMount(() => onCleanup(startPolling()));
  return (
    <div class="visually-hidden" aria-live="polite" aria-atomic="true">
      {announced()}
    </div>
  );
}

/**
 * The bell button with its unread badge, and the panel it opens. `sheet` lays the panel out as a
 * full-width sheet under the phone's top bar rather than a dropdown under the button.
 */
export function NotificationsBell(props: { class?: string; sheet?: boolean; beforeOpen?: () => void }) {
  let button!: HTMLButtonElement;
  const badge = () => badgeText(unreadCount());
  return (
    <>
      <button
        ref={button}
        type="button"
        class={`notif-bell ${props.class ?? ""}`}
        aria-label={bellLabel(unreadCount())}
        title="Notifications"
        aria-haspopup="dialog"
        aria-expanded={panelOpen()}
        aria-controls={panelOpen() ? PANEL_ID : undefined}
        onClick={() => {
          if (!panelOpen()) props.beforeOpen?.();
          togglePanel();
        }}
      >
        <BellIcon />
        <Show when={badge()}>
          <span class="notif-badge" aria-hidden="true">
            {badge()}
          </span>
        </Show>
      </button>
      <Show when={panelOpen()}>
        <Portal>
          <Panel anchor={button} sheet={!!props.sheet} />
        </Portal>
      </Show>
    </>
  );
}

function Panel(props: { anchor: HTMLButtonElement; sheet: boolean }) {
  let panel!: HTMLElement;
  const [now, setNow] = createSignal(Date.now());
  const [place, setPlace] = createSignal<{ top: number; right: number }>({ top: 0, right: 0 });

  function position() {
    const r = props.anchor.getBoundingClientRect();
    // A sheet hangs from the bottom of the bar the bell is in; a dropdown just under the bell.
    const bar = props.anchor.closest("header")?.getBoundingClientRect();
    const top = props.sheet ? (bar ?? r).bottom : r.bottom + 6;
    setPlace({ top, right: Math.max(8, window.innerWidth - r.right) });
  }

  function close(refocus: boolean) {
    closePanel();
    if (refocus) props.anchor.focus();
  }

  onMount(() => {
    position();
    const clock = setInterval(() => setNow(Date.now()), 30_000);
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
    // Focus moves into the panel, to the heading until the list arrives.
    panel.focus();
    onCleanup(() => {
      clearInterval(clock);
      window.removeEventListener("resize", position);
      document.removeEventListener("pointerdown", onPointer, true);
      document.removeEventListener("keydown", onKey);
    });
  });

  // Leaving the panel with Tab closes it.
  function onFocusOut(e: FocusEvent) {
    const to = e.relatedTarget as Node | null;
    if (!to || panel.contains(to) || props.anchor.contains(to)) return;
    close(false);
  }

  /** The arrow keys, Home and End move between the items. */
  function onListKey(e: KeyboardEvent) {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
    const items = [...panel.querySelectorAll<HTMLElement>(".notif-open")];
    if (!items.length) return;
    e.preventDefault();
    const at = items.indexOf(document.activeElement as HTMLElement);
    let next = 0;
    if (e.key === "End") next = items.length - 1;
    else if (e.key === "ArrowDown") next = at < 0 ? 0 : Math.min(items.length - 1, at + 1);
    else if (e.key === "ArrowUp") next = at <= 0 ? 0 : at - 1;
    items[next]!.focus();
  }

  const items = () => notificationList().items;
  const hasRead = () => items().some((n) => n.read !== null) || !!notificationList().next;

  function open(n: AppNotification, e: MouseEvent) {
    const href = targetOf(n);
    void readNotification(n.id);
    if (!href) return;
    e.preventDefault();
    if (wantsNewTab(e)) {
      openInBackground(href);
      return;
    }
    close(false);
    const pane = M.activePane(workspace());
    navigatePane(pane, href);
    scrollPaneTop(pane);
  }

  return (
    <section
      ref={panel}
      id={PANEL_ID}
      class="notif-panel"
      classList={{ "notif-sheet": props.sheet }}
      style={
        props.sheet
          ? { top: `${place().top}px`, "max-height": `calc(100dvh - ${place().top}px - var(--safe-bottom))` }
          : { top: `${place().top}px`, right: `${place().right}px` }
      }
      role="dialog"
      aria-labelledby="notifications-heading"
      tabindex={-1}
      onFocusOut={onFocusOut}
      onKeyDown={onListKey}
    >
      <header class="notif-head">
        <h2 id="notifications-heading">Notifications</h2>
        <div class="notif-actions">
          <button type="button" class="notif-action" disabled={unreadCount() === 0} onClick={() => void readAll()}>
            Mark all read
          </button>
          <button type="button" class="notif-action" disabled={!hasRead()} onClick={() => void clearReadItems()}>
            Clear read
          </button>
        </div>
      </header>
      <div class="notif-scroll">
        <Show
          when={listLoaded()}
          fallback={
            <p class="notif-empty muted">{listFailed() ? (listFailure() ?? "Notifications could not be loaded.") : "Loading…"}</p>
          }
        >
          <Show when={items().length} fallback={<p class="notif-empty muted">No notifications</p>}>
            <ul class="notif-list" aria-label="Notifications">
              <For each={items()}>
                {(n) => (
                  <li class="notif-item" classList={{ unread: n.read === null }} data-notification-id={n.id}>
                    <Item n={n} now={now()} onOpen={open} />
                    <button
                      type="button"
                      class="notif-dismiss"
                      aria-label={`Dismiss ${n.title}`}
                      title="Dismiss"
                      onClick={(e) => {
                        // Focus moves to the next item, or the one before, rather than being lost.
                        const li = e.currentTarget.closest("li");
                        const near = (li?.nextElementSibling ?? li?.previousElementSibling)?.querySelector<HTMLElement>(".notif-open");
                        void dismiss(n.id);
                        (near ?? panel).focus();
                      }}
                    >
                      ×
                    </button>
                  </li>
                )}
              </For>
            </ul>
            <Show when={notificationList().next}>
              <button type="button" class="notif-more" disabled={listLoading()} onClick={() => void loadMore()}>
                {listLoading() ? "Loading…" : "Load more"}
              </button>
            </Show>
          </Show>
        </Show>
      </div>
    </section>
  );
}

/** One notification: a link to its note or page, or a button that only marks it read. */
function Item(props: { n: AppNotification; now: number; onOpen: (n: AppNotification, e: MouseEvent) => void }) {
  const n = () => props.n;
  const href = () => targetOf(n());
  const icon = () => {
    const t = n().note_type;
    return t ? (typeDef(t)?.icon ?? null) : undefined;
  };
  const when = () => new Date(n().created);
  const content = () => (
    <>
      <span class="notif-dot" aria-hidden="true" />
      <Show when={n().read === null}>
        <span class="visually-hidden">Unread: </span>
      </Show>
      <Show when={icon() !== undefined}>
        <span class="notif-icon">
          <TypeIcon name={icon()} size={16} />
        </span>
      </Show>
      <span class="notif-main">
        <span class="notif-title">{n().title}</span>
        <Show when={n().body}>
          <span class="notif-body">{n().body}</span>
        </Show>
        <span class="notif-meta">
          <time datetime={when().toISOString()} title={when().toLocaleString()}>
            {relativeTime(n().created, props.now)}
          </time>
          <span aria-hidden="true"> · </span>
          <span class="notif-source">{sourceLabel(n().source)}</span>
        </span>
      </span>
    </>
  );
  return (
    <Show
      when={href()}
      fallback={
        <button type="button" class="notif-open" onClick={(e) => props.onOpen(n(), e)}>
          {content()}
        </button>
      }
    >
      {(to) => (
        <a
          class="notif-open"
          href={to()}
          onClick={(e) => {
            if (e.button === 0 && !e.altKey && !e.shiftKey) props.onOpen(n(), e);
          }}
          onAuxClick={(e) => {
            if (e.button === 1) props.onOpen(n(), e);
          }}
        >
          {content()}
        </a>
      )}
    </Show>
  );
}
