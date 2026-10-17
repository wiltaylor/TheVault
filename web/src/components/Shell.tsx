import { Navigate, useLocation } from "@solidjs/router";
import { For, type JSX, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { CaptureModal } from "../capture/CaptureModal";
import { CommandPalette } from "../palette/CommandPalette";
import { trackRecents } from "../palette/recents";
import { PALETTE_KEYS, closePalette, isPaletteKey, openPalette, paletteOpen } from "../palette/store";
import { captureOpen, inboxCount, isCaptureKey, openCapture, refreshInboxCount } from "../capture/store";
import { assetIdOf } from "../assetLibrary";
import { GraphView } from "../graph/GraphPanel";
import { HistoryPanel } from "../history/HistoryPanel";
import { endPreviews } from "../history/preview";
import { sessionState } from "../session";
import {
  type Side,
  MAX_WIDTH,
  MIN_WIDTH,
  type Panel,
  leftDrawer,
  noteOf,
  resizeSidebar,
  rightDrawer,
  setLeftDrawer,
  setGraphOpen,
  setPanel,
  setRightDrawer,
  sidebars,
  toggleSidebar,
} from "../sidebars";
import { theme, toggleTheme } from "../theme";
import { isActiveLink } from "../workspace/links";
import { activePane, location as paneLocation } from "../workspace/model";
import { workspace } from "../workspace/store";
import { Workspace } from "../workspace/Workspace";
import { AssetUsagePanel } from "./AssetUsage";
import { dayOfLocation, todayDay } from "../daily/calendar";
import { DailyCalendar } from "../daily/DailyCalendar";
import { ArchiveIcon, CalendarIcon, CaptureIcon, GearIcon, InboxIcon, SearchIcon, ThemeGlyph, TrashIcon } from "./Icon";
import { NotificationsBell, NotificationsLive } from "../notifications/NotificationsPanel";
import { PinnedTree } from "./PinnedTree";
import { SyncStatusButton } from "./SyncStatus";
import { Toasts } from "./Toasts";
import { TypesNav } from "./TypesNav";

/** The sidebar's main links. Each opens in the active pane; a middle-click opens a background tab. */
const NAV: { href: string; label: string; icon: () => JSX.Element; badge?: () => number | null }[] = [
  { href: "/daily", label: "Daily notes", icon: CalendarIcon },
  { href: "/search", label: "Search", icon: SearchIcon },
  { href: "/inbox", label: "Inbox", icon: InboxIcon, badge: inboxCount },
];

/** Below these widths the sidebars are drawers over the app rather than columns beside it. */
const LEFT_DRAWER = "(max-width: 1024px)";
const RIGHT_DRAWER = "(max-width: 600px)";

function mediaSignal(query: string) {
  const mq = typeof window.matchMedia === "function" ? window.matchMedia(query) : null;
  const [on, setOn] = createSignal(mq?.matches ?? false);
  if (mq) {
    const change = () => setOn(mq.matches);
    mq.addEventListener("change", change);
    onCleanup(() => mq.removeEventListener("change", change));
  }
  return on;
}

/** A chevron pointing `dir`, for the collapse and reopen buttons. */
function Chevron(props: { dir: "left" | "right" }) {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <Show when={props.dir === "left"} fallback={<path d="m9 6 6 6-6 6" />}>
        <path d="m15 6-6 6 6 6" />
      </Show>
    </svg>
  );
}

/** The inner edge of a sidebar: drag it, or focus it and use the arrow keys, to set the width. */
function Resizer(props: { side: Side }) {
  const width = () => sidebars()[props.side].width;
  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    document.body.classList.add("ws-resizing-row");
    const move = (ev: PointerEvent) =>
      resizeSidebar(props.side, props.side === "left" ? ev.clientX : window.innerWidth - ev.clientX);
    const end = () => {
      document.body.classList.remove("ws-resizing-row");
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", end);
      el.removeEventListener("pointercancel", end);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", end);
    el.addEventListener("pointercancel", end);
  }
  function onKeyDown(e: KeyboardEvent) {
    const grow = props.side === "left" ? "ArrowRight" : "ArrowLeft";
    const shrink = props.side === "left" ? "ArrowLeft" : "ArrowRight";
    const step = e.shiftKey ? 48 : 16;
    if (e.key === grow || e.key === shrink) {
      e.preventDefault();
      resizeSidebar(props.side, width() + (e.key === grow ? step : -step));
    }
  }
  return (
    <div
      class={`sidebar-resizer sidebar-resizer-${props.side}`}
      role="separator"
      tabindex={0}
      aria-orientation="vertical"
      aria-label={props.side === "left" ? "Resize sidebar" : "Resize right sidebar"}
      aria-valuemin={MIN_WIDTH}
      aria-valuemax={MAX_WIDTH}
      aria-valuenow={width()}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
    />
  );
}

/** The button that hides the right sidebar, in whichever of its headers is at the top. */
function HideRight() {
  return (
    <button
      type="button"
      class="sidebar-toggle"
      aria-label="Hide right sidebar"
      title="Hide right sidebar (Ctrl+Shift+\)"
      onClick={() => toggleSidebar("right", false)}
    >
      <Chevron dir="right" />
    </button>
  );
}

const PANEL_TABS: { panel: Panel; label: string }[] = [
  { panel: "properties", label: "Properties" },
  { panel: "history", label: "History" },
];

/**
 * The right sidebar: context for the active pane. The graph of its note's links sits at the top;
 * below it, one panel at a time (Properties, History). Beside an asset's page it lists the notes
 * using the asset instead.
 */
function RightSidebar(props: { shown: boolean }) {
  const pane = () => activePane(workspace());
  const assetId = createMemo(() => assetIdOf(paneLocation(workspace(), pane())));
  return (
    <Show when={assetId()} keyed fallback={<NoteSidebar shown={props.shown} />}>
      {(id) => (
        <div id="right-sidebar" class="right-sidebar">
          <Resizer side="right" />
          <div class="sidebar-head right-head">
            <h2 class="sidebar-heading" id="usage-heading">
              Used by
            </h2>
            <HideRight />
          </div>
          <Show when={props.shown}>
            <AssetUsagePanel id={id} pane={pane()} />
          </Show>
        </div>
      )}
    </Show>
  );
}

/** The right sidebar beside a note, or any page but an asset's. */
function NoteSidebar(props: { shown: boolean }) {
  const pane = () => activePane(workspace());
  const source = () => noteOf(pane());
  const panel = () => sidebars().right.panel;
  // The History panel starts again for each pane and note it follows.
  const historyOf = createMemo(() => {
    const s = source();
    const id = s?.noteId();
    return s && id ? { pane: pane(), source: s, id } : null;
  }, null, { equals: (a, b) => a?.pane === b?.pane && a?.source === b?.source && a?.id === b?.id });
  function onTabKey(e: KeyboardEvent) {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const i = PANEL_TABS.findIndex((t) => t.panel === panel());
    const next = PANEL_TABS[(i + (e.key === "ArrowRight" ? 1 : PANEL_TABS.length - 1)) % PANEL_TABS.length]!;
    setPanel(next.panel);
    document.getElementById(`${next.panel}-tab`)?.focus();
  }
  // The graph follows the active pane's note, once it has been created.
  const graphOf = createMemo(() => source()?.noteId() || "");
  const graphOpen = () => sidebars().right.graph;
  // Beside the daily notes page or a daily note, a calendar to pick another day sits on top.
  const calendarDay = createMemo(() => dayOfLocation(paneLocation(workspace(), pane()), todayDay()) || source()?.day?.() || "");
  return (
    <div id="right-sidebar" class="right-sidebar">
      <Resizer side="right" />
      <Show when={calendarDay()}>
        {(day) => (
          <DailyCalendar day={day()} noteId={graphOf()} shown={props.shown}>
            <HideRight />
          </DailyCalendar>
        )}
      </Show>
      <Show when={graphOf()}>
        <section class="graph-section" aria-label="Graph">
          <div class="sidebar-head graph-head">
            <button
              type="button"
              class="graph-toggle"
              aria-expanded={graphOpen()}
              aria-controls="graph-body"
              title={graphOpen() ? "Collapse the graph" : "Expand the graph"}
              onClick={() => setGraphOpen(!graphOpen())}
            >
              <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                <path d="m9 6 6 6-6 6" />
              </svg>
              Graph
            </button>
            <Show when={!calendarDay()}>
              <HideRight />
            </Show>
          </div>
          <Show when={graphOpen() && props.shown}>
            <div id="graph-body" class="graph-body">
              <GraphView noteId={graphOf()} />
            </div>
          </Show>
        </section>
      </Show>
      <div class="sidebar-head right-head">
        <div class="panel-tabs" role="tablist" aria-label="Panels" onKeyDown={onTabKey}>
          <For each={PANEL_TABS}>
            {(t) => (
              <button
                type="button"
                role="tab"
                id={`${t.panel}-tab`}
                aria-selected={panel() === t.panel}
                aria-controls={`${t.panel}-panel`}
                tabindex={panel() === t.panel ? 0 : -1}
                onClick={() => setPanel(t.panel)}
              >
                {t.label}
              </button>
            )}
          </For>
        </div>
        <Show when={!graphOf() && !calendarDay()}>
          <HideRight />
        </Show>
      </div>
      <Show when={panel() === "properties"}>
        <aside id="properties-panel" class="side-panel" aria-label="Properties" tabindex={-1}>
          <Show when={props.shown}>
            <Show when={source()} keyed fallback={<p class="muted">Open a note to see its properties.</p>}>
              {(s) => s.render()}
            </Show>
          </Show>
        </aside>
      </Show>
      <Show when={panel() === "history"}>
        <aside id="history-panel" class="side-panel" aria-label="History" tabindex={-1}>
          <Show when={props.shown}>
            <Show when={historyOf()} keyed fallback={<p class="muted">Open a note to see its history.</p>}>
              {(h) => <HistoryPanel pane={h.pane} source={h.source} />}
            </Show>
          </Show>
        </aside>
      </Show>
    </div>
  );
}

/**
 * The signed-in layout: the left sidebar (navigation, theme, trash and settings), the workspace,
 * and the right sidebar. Links in the sidebars open in the workspace's active pane. Each sidebar
 * collapses to a slim edge on a wide screen and is a drawer on a narrow one.
 */
export function Shell() {
  // Where the browser's router says the app is: the address loaded, or the one a login goes on to.
  const location = useLocation();
  const leftIsDrawer = mediaSignal(LEFT_DRAWER);
  const rightIsDrawer = mediaSignal(RIGHT_DRAWER);
  const rightShown = () => (rightIsDrawer() ? rightDrawer() : sidebars().right.open);
  // A version shows in a pane only while the History panel lists its versions: switching the
  // panel, closing the sidebar or moving to another pane returns to the live note. On a phone the
  // drawer closes to show the version, so there only switching the panel ends it.
  createEffect(() => {
    const listed = sidebars().right.panel === "history" && (rightIsDrawer() || sidebars().right.open);
    endPreviews(listed ? activePane(workspace()) : undefined);
  });
  const closeDrawers = () => {
    setLeftDrawer(false);
    setRightDrawer(false);
  };

  // Every note a pane comes to show joins the palette's recent notes.
  trackRecents();

  onMount(() => {
    // Ctrl/Cmd+\ shows or hides the left sidebar, with Shift the right; Alt+\ does too, where a
    // browser keeps the Ctrl form.
    const onKey = (e: KeyboardEvent) => {
      if (e.code !== "Backslash" || e.defaultPrevented) return;
      if (!(e.ctrlKey || e.metaKey || e.altKey)) return;
      e.preventDefault();
      toggleSidebar(e.shiftKey ? "right" : "left");
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => document.removeEventListener("keydown", onKey));
    // Ctrl/Cmd+Shift+K opens the capture dialog from anywhere.
    const onCaptureKey = (e: KeyboardEvent) => {
      if (!isCaptureKey(e) || e.defaultPrevented) return;
      e.preventDefault();
      openCapture();
    };
    document.addEventListener("keydown", onCaptureKey);
    onCleanup(() => document.removeEventListener("keydown", onCaptureKey));
    // Ctrl/Cmd+K opens the command palette from anywhere, and closes it again.
    const onPaletteKey = (e: KeyboardEvent) => {
      if (!isPaletteKey(e) || e.defaultPrevented || captureOpen()) return;
      e.preventDefault();
      if (paletteOpen()) closePalette();
      else openPalette();
    };
    document.addEventListener("keydown", onPaletteKey);
    onCleanup(() => document.removeEventListener("keydown", onPaletteKey));
    // The inbox's count, now and whenever the window comes back, for captures made elsewhere.
    void refreshInboxCount();
    const onVisible = () => {
      if (document.visibilityState === "visible") void refreshInboxCount();
    };
    document.addEventListener("visibilitychange", onVisible);
    onCleanup(() => document.removeEventListener("visibilitychange", onVisible));
  });

  return (
    <Show
      when={sessionState() === "in"}
      fallback={
        <Show when={sessionState() === "out"} fallback={<p class="muted boot">Loading…</p>}>
          <Navigate href={`/login?next=${encodeURIComponent(window.location.pathname + window.location.search)}`} />
        </Show>
      }
    >
      <div
        class="shell"
        classList={{
          "menu-open": leftIsDrawer() && leftDrawer(),
          "right-open": rightIsDrawer() && rightDrawer(),
          "left-collapsed": !leftIsDrawer() && !sidebars().left.open,
          "right-collapsed": !rightIsDrawer() && !sidebars().right.open,
        }}
        style={{ "--sidebar": `${sidebars().left.width}px`, "--right-sidebar": `${sidebars().right.width}px` }}
      >
        <header class="topbar">
          <button
            type="button"
            class="menu-button"
            aria-label="Menu"
            aria-expanded={leftDrawer()}
            aria-controls="sidebar"
            onClick={() => {
              setRightDrawer(false);
              setLeftDrawer(!leftDrawer());
            }}
          >
            ☰
          </button>
          <button
            type="button"
            class="topbar-palette"
            aria-label="Search or run a command"
            onClick={() => {
              setLeftDrawer(false);
              setRightDrawer(false);
              openPalette();
            }}
          >
            <SearchIcon />
            <span>Search or run a command</span>
          </button>
          <Show when={rightIsDrawer()}>
            <NotificationsBell
              class="menu-button topbar-bell"
              sheet
              beforeOpen={() => {
                setLeftDrawer(false);
                setRightDrawer(false);
              }}
            />
          </Show>
          <button
            type="button"
            class="menu-button right-button"
            aria-label="Right sidebar"
            aria-expanded={rightShown()}
            aria-controls="right-sidebar"
            onClick={() => {
              setLeftDrawer(false);
              toggleSidebar("right");
            }}
          >
            <PanelIcon />
          </button>
        </header>
        <div id="sidebar" class="sidebar">
          <div class="sidebar-head">
            <button
              type="button"
              class="sidebar-toggle"
              aria-label="Hide sidebar"
              title="Hide sidebar (Ctrl+\)"
              onClick={() => toggleSidebar("left", false)}
            >
              <Chevron dir="left" />
            </button>
          </div>
          <nav aria-label="Main">
            <ul>
              <li>
                <button
                  type="button"
                  class="nav-link nav-button palette-open"
                  title={`Search or run a command (${PALETTE_KEYS})`}
                  onClick={() => {
                    setLeftDrawer(false);
                    openPalette();
                  }}
                >
                  <SearchIcon />
                  <span>Search or run a command</span>
                  <kbd aria-hidden="true">{PALETTE_KEYS}</kbd>
                </button>
              </li>
              <For each={NAV}>
                {(item) => (
                  <li>
                    <a
                      href={item.href}
                      class="nav-link"
                      classList={{ active: isActiveLink(item.href) }}
                      onClick={() => setLeftDrawer(false)}
                    >
                      {item.icon()}
                      <span>{item.label}</span>
                      <Show when={item.badge?.()}>
                        {(n) => (
                          <span class="nav-badge" title={`${n()} waiting`}>
                            {n()}
                          </span>
                        )}
                      </Show>
                    </a>
                  </li>
                )}
              </For>
              <li>
                <button
                  type="button"
                  class="nav-link nav-button"
                  title="Capture a fleeting note (Ctrl+Shift+K)"
                  onClick={() => {
                    setLeftDrawer(false);
                    openCapture();
                  }}
                >
                  <CaptureIcon />
                  <span>Capture</span>
                </button>
              </li>
            </ul>
          </nav>
          <PinnedTree onNavigate={() => setLeftDrawer(false)} />
          <TypesNav onNavigate={() => setLeftDrawer(false)} />
          <div class="sidebar-footer">
            <button
              type="button"
              role="switch"
              class="theme-switch"
              aria-checked={theme() === "dark"}
              aria-label="Dark mode"
              title={theme() === "dark" ? "Switch to light mode" : "Switch to dark mode"}
              onClick={toggleTheme}
            >
              <span class="theme-switch-track" aria-hidden="true">
                <span class="theme-switch-thumb">
                  <ThemeGlyph dark={theme() === "dark"} />
                </span>
              </span>
            </button>
            <SyncStatusButton />
            <a
              href="/archive"
              class="footer-button"
              classList={{ active: isActiveLink("/archive") }}
              aria-label="Archive"
              title="Archive"
              onClick={() => setLeftDrawer(false)}
            >
              <ArchiveIcon />
            </a>
            <a
              href="/trash"
              class="footer-button"
              classList={{ active: isActiveLink("/trash") }}
              aria-label="Trash"
              title="Trash"
              onClick={() => setLeftDrawer(false)}
            >
              <TrashIcon />
            </a>
            <a
              href="/settings"
              class="footer-button"
              classList={{ active: isActiveLink("/settings") }}
              aria-label="Settings"
              title="Settings"
              onClick={() => setLeftDrawer(false)}
            >
              <GearIcon />
            </a>
          </div>
          <Resizer side="left" />
        </div>
        <div class="scrim" onClick={closeDrawers} />
        <main class="content">
          <Show when={!leftIsDrawer() && !sidebars().left.open}>
            <button
              type="button"
              class="edge-reopen edge-left"
              aria-label="Show sidebar"
              title="Show sidebar (Ctrl+\)"
              onClick={() => toggleSidebar("left", true)}
            >
              <Chevron dir="right" />
            </button>
          </Show>
          <Workspace url={location.pathname + location.search + location.hash} />
          <Show when={!rightIsDrawer() && !sidebars().right.open}>
            <button
              type="button"
              class="edge-reopen edge-right"
              aria-label="Show right sidebar"
              title="Show right sidebar (Ctrl+Shift+\)"
              onClick={() => toggleSidebar("right", true)}
            >
              <Chevron dir="left" />
            </button>
          </Show>
        </main>
        <RightSidebar shown={rightShown()} />
        <button
          type="button"
          class="capture-fab"
          hidden={captureOpen()}
          aria-label="Quick capture"
          title="Capture a fleeting note (Ctrl+Shift+K)"
          onClick={openCapture}
        >
          <svg viewBox="0 0 24 24" width="26" height="26" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round">
            <path d="M12 5v14M5 12h14" />
          </svg>
        </button>
        <Show when={captureOpen()}>
          <CaptureModal />
        </Show>
        <Show when={paletteOpen()}>
          <CommandPalette />
        </Show>
        <Toasts />
        <NotificationsLive />
      </div>
    </Show>
  );
}

/** A window with a right-hand panel, for the right sidebar's button on narrow screens. */
function PanelIcon() {
  return (
    <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round">
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <line x1="15" y1="4" x2="15" y2="20" />
    </svg>
  );
}
