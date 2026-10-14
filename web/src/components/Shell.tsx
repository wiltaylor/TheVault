import { Navigate, useLocation } from "@solidjs/router";
import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
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
  setPanel,
  setRightDrawer,
  sidebars,
  toggleSidebar,
} from "../sidebars";
import { theme, toggleTheme } from "../theme";
import { isActiveLink } from "../workspace/links";
import { activePane } from "../workspace/model";
import { workspace } from "../workspace/store";
import { Workspace } from "../workspace/Workspace";
import { CalendarIcon, GearIcon, SearchIcon, ThemeGlyph, TrashIcon } from "./Icon";
import { PinnedTree } from "./PinnedTree";

/** The sidebar's main links. Each opens in the active pane; a middle-click opens a background tab. */
const NAV = [
  { href: "/daily", label: "Daily notes", icon: CalendarIcon },
  { href: "/search", label: "Search", icon: SearchIcon },
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

const PANEL_TABS: { panel: Panel; label: string }[] = [
  { panel: "properties", label: "Properties" },
  { panel: "history", label: "History" },
];

/** The right sidebar: context for the active pane, one panel at a time (Properties, History). */
function RightSidebar(props: { shown: boolean }) {
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
  return (
    <div id="right-sidebar" class="right-sidebar">
      <Resizer side="right" />
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
        <button
          type="button"
          class="sidebar-toggle"
          aria-label="Hide right sidebar"
          title="Hide right sidebar (Ctrl+Shift+\)"
          onClick={() => toggleSidebar("right", false)}
        >
          <Chevron dir="right" />
        </button>
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
                    </a>
                  </li>
                )}
              </For>
            </ul>
          </nav>
          <PinnedTree onNavigate={() => setLeftDrawer(false)} />
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
