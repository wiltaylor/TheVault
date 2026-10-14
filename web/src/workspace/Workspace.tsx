// The tabbed, split-pane workspace: a tab bar, and the active tab's panes laid out by its layout
// tree. Every pane stays mounted once shown, positioned from the tree, so splitting, docking,
// swapping and moving between tabs never reload a page.

import {
  For,
  Show,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  untrack,
} from "solid-js";
import { TypeIcon } from "../components/Icon";
import { paneRoutes } from "../routes";
import * as M from "./model";
import { SHORTCUTS, installWorkspaceEvents } from "./events";
import { PaneProvider, PaneRouter, paneTitle, setPaneTitle } from "./pane";
import {
  activatePane,
  activateTab,
  closePane,
  closeTab,
  dockPane,
  goPane,
  loadWorkspace,
  movePaneToNewTab,
  movePaneToTab,
  moveTab,
  openTab,
  setRatio,
  splitPane,
  swapPanes,
  unloadWorkspace,
  workspace,
} from "./store";

/** Phones show one pane at a time. */
const PHONE = "(max-width: 600px)";

function phoneSignal() {
  const mq = typeof window.matchMedia === "function" ? window.matchMedia(PHONE) : null;
  const [phone, setPhone] = createSignal(mq?.matches ?? false);
  if (mq) {
    const on = () => setPhone(mq.matches);
    mq.addEventListener("change", on);
    onCleanup(() => mq.removeEventListener("change", on));
  }
  return phone;
}

/** Where a dragged pane would land. */
type DropTarget =
  | { kind: "pane"; pane: string; zone: M.Edge | "center" }
  | { kind: "tab"; tab: string }
  | { kind: "new-tab" };

interface PaneDrag {
  pane: string;
  x: number;
  y: number;
  target: DropTarget | null;
}

const [paneDrag, setPaneDrag] = createSignal<PaneDrag | null>(null);
const [tabDrag, setTabDrag] = createSignal<{ tab: string; x: number; index: number } | null>(null);

/** Moves smaller than this are clicks, not drags. */
const SLOP = 5;

function tabTitle(tab: M.TabState): { title: string; icon?: string | null } {
  return paneTitle(tab.active);
}

/** The workspace, opening `url` (the address the app loaded at) in its active pane. */
export function Workspace(props: { url: string }) {
  const phone = phoneSignal();
  untrack(() => loadWorkspace(props.url));
  onCleanup(unloadWorkspace);
  onMount(() => onCleanup(installWorkspaceEvents()));
  // The browser's tab names the active pane's page.
  createEffect(() => {
    const w = workspace();
    document.title = M.isEmpty(w) ? "TheVault" : `${paneTitle(M.activePane(w)).title} · TheVault`;
  });
  onCleanup(() => (document.title = "TheVault"));

  const paneIds = createMemo(
    () => Object.keys(workspace().panes),
    undefined,
    { equals: (a, b) => a.length === b.length && a.every((x, i) => x === b[i]) },
  );
  const tab = () => M.activeTab(workspace());
  const rects = createMemo(() => M.layoutRects(tab().layout));
  const splitIds = createMemo(
    () => rects().dividers.map((d) => d.split),
    undefined,
    { equals: (a, b) => a.length === b.length && a.every((x, i) => x === b[i]) },
  );

  let area!: HTMLDivElement;

  return (
    <div class="workspace" classList={{ "ws-phone": phone() }}>
      <TabBar phone={phone()} />
      <div class="ws-area" classList={{ "ws-empty": M.isEmpty(workspace()) }} ref={area}>
        <For each={paneIds()}>{(id) => <Pane id={id} phone={phone()} rects={rects} />}</For>
        <Show when={!phone()}>
          <For each={splitIds()}>{(id) => <DividerView id={id} rects={rects} area={() => area} />}</For>
        </Show>
        <DropPreview area={() => area} />
      </div>
      <Show when={paneDrag()}>
        {(d) => (
          <div class="ws-drag-ghost" style={{ left: `${d().x + 12}px`, top: `${d().y + 12}px` }}>
            {paneTitle(d().pane).title}
          </div>
        )}
      </Show>
    </div>
  );
}

/** The tabs: click to show, drag to reorder, × or a middle-click to close, + for a new one. */
function TabBar(props: { phone: boolean }) {
  let bar!: HTMLDivElement;
  const tabIds = createMemo(
    () => workspace().tabs.map((t) => t.id),
    undefined,
    { equals: (a, b) => a.length === b.length && a.every((x, i) => x === b[i]) },
  );

  function startTabDrag(e: PointerEvent, id: string) {
    if (e.button !== 0 || (e.target as Element).closest(".ws-tab-close")) return;
    const startX = e.clientX;
    const startY = e.clientY;
    let dragging = false;
    const move = (ev: PointerEvent) => {
      if (!dragging) {
        if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < SLOP) return;
        dragging = true;
      }
      setTabDrag({ tab: id, x: ev.clientX, index: insertIndex(ev.clientX, id) });
    };
    const end = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      window.removeEventListener("keydown", esc, true);
      const d = tabDrag();
      setTabDrag(null);
      if (dragging && d && ev.type === "pointerup") moveTab(id, d.index);
      else if (!dragging && ev.type === "pointerup") activateTab(id);
    };
    const esc = (ev: KeyboardEvent) => {
      if (ev.key !== "Escape") return;
      ev.preventDefault();
      dragging = false;
      setTabDrag(null);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("keydown", esc, true);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
    window.addEventListener("keydown", esc, true);
  }

  /** Where tab `id` would go, dropped at `x`: an index among the other tabs. */
  function insertIndex(x: number, id: string): number {
    const others = [...bar.querySelectorAll<HTMLElement>("[data-tab-id]")].filter((t) => t.dataset.tabId !== id);
    let i = 0;
    for (const t of others) {
      const r = t.getBoundingClientRect();
      if (x > r.left + r.width / 2) i++;
    }
    return i;
  }

  /** Where the insertion mark stands during a tab drag. */
  const markX = () => {
    const d = tabDrag();
    if (!d) return null;
    const others = [...bar.querySelectorAll<HTMLElement>("[data-tab-id]")].filter((t) => t.dataset.tabId !== d.tab);
    const b = bar.getBoundingClientRect();
    if (!others.length) return null;
    const ref = others[d.index];
    const x = ref ? ref.getBoundingClientRect().left : others.at(-1)!.getBoundingClientRect().right;
    return x - b.left + bar.scrollLeft;
  };

  return (
    <div
      ref={bar}
      class="ws-tabbar"
      classList={{ "drop-new": paneDrag()?.target?.kind === "new-tab" }}
      role="tablist"
      aria-label="Open tabs"
    >
      <For each={tabIds()}>
        {(id) => {
          const t = { id };
          const state = () => workspace().tabs.find((x) => x.id === id);
          const title = () => {
            const s = state();
            return s ? tabTitle(s) : { title: "" };
          };
          const active = () => workspace().active === id;
          const count = () => {
            const s = state();
            return s ? M.panesOf(s.layout).length : 0;
          };
          return (
            <div
              class="ws-tab"
              role="tab"
              tabindex={active() ? 0 : -1}
              aria-selected={active()}
              data-tab-id={t.id}
              aria-label={title().title}
              title={title().title}
              classList={{
                active: active(),
                dragging: tabDrag()?.tab === t.id,
                "drop-target": paneDrag()?.target?.kind === "tab" && (paneDrag()!.target as { tab: string }).tab === t.id,
              }}
              onPointerDown={(e) => startTabDrag(e, t.id)}
              onAuxClick={(e) => {
                if (e.button === 1) {
                  e.preventDefault();
                  closeTab(t.id);
                }
              }}
              onMouseDown={(e) => {
                if (e.button === 1) e.preventDefault();
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  activateTab(t.id);
                }
              }}
            >
              <Show when={title().icon !== undefined}>
                <TypeIcon name={title().icon} size={14} />
              </Show>
              <span class="ws-tab-title">{title().title}</span>
              <Show when={count() > 1}>
                <span class="ws-tab-count" title={`${count()} panes`}>
                  {count()}
                </span>
              </Show>
              <button
                type="button"
                class="ws-tab-close"
                aria-label="Close tab"
                title="Close tab"
                tabindex={-1}
                onPointerDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(t.id);
                }}
              >
                ×
              </button>
            </div>
          );
        }}
      </For>
      <button type="button" class="ws-tab-new" aria-label="New tab" title="New tab" onClick={() => openTab(M.HOME)}>
        +
      </button>
      <Show when={markX() !== null}>
        <span class="ws-tab-mark" style={{ left: `${markX()}px` }} />
      </Show>
      <Show when={!props.phone && paneDrag()?.target?.kind === "new-tab"}>
        <span class="ws-tab-ghost">New tab</span>
      </Show>
    </div>
  );
}

/** One pane: its header with the title and menu, and the page it shows. */
function Pane(props: { id: string; phone: boolean; rects: () => ReturnType<typeof M.layoutRects> }) {
  const id = props.id;
  const tabId = createMemo(() => M.tabOf(workspace(), id)?.id);
  const active = createMemo(() => M.activePane(workspace()) === id);
  const visible = createMemo(() => tabId() === workspace().active && (!props.phone || active()));
  const rect = createMemo(() => {
    if (props.phone) return { x: 0, y: 0, w: 1, h: 1 };
    return props.rects().panes.find((p) => p.pane === id) ?? { x: 0, y: 0, w: 1, h: 1 };
  });
  const [mounted, setMounted] = createSignal(false);
  createEffect(() => {
    if (visible()) setMounted(true);
  });
  const siblings = createMemo(() => {
    const t = M.tabOf(workspace(), id);
    return t ? M.panesOf(t.layout) : [id];
  });
  onCleanup(() => setPaneTitle(id, null));

  const pct = (n: number) => `${n * 100}%`;

  function startPaneDrag(e: PointerEvent) {
    if (props.phone || e.button !== 0 || (e.target as Element).closest("button, select, input, [role='menu']")) return;
    const startX = e.clientX;
    const startY = e.clientY;
    let dragging = false;
    const move = (ev: PointerEvent) => {
      if (!dragging) {
        if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < SLOP) return;
        dragging = true;
        document.body.classList.add("ws-dragging");
      }
      setPaneDrag({ pane: id, x: ev.clientX, y: ev.clientY, target: dropTargetAt(ev.clientX, ev.clientY, id) });
    };
    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", stop);
      window.removeEventListener("keydown", esc, true);
      document.body.classList.remove("ws-dragging");
      setPaneDrag(null);
    };
    const up = () => {
      const target = paneDrag()?.target ?? null;
      stop();
      if (dragging && target) drop(id, target);
    };
    const esc = (ev: KeyboardEvent) => {
      if (ev.key !== "Escape") return;
      ev.preventDefault();
      ev.stopPropagation();
      stop();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", stop);
    window.addEventListener("keydown", esc, true);
  }

  return (
    <section
      class="pane"
      classList={{ active: active(), dragged: paneDrag()?.pane === id }}
      data-pane-id={id}
      data-active={active() ? "true" : undefined}
      aria-label={`Pane: ${paneTitle(id).title}`}
      style={{
        display: visible() ? undefined : "none",
        left: pct(rect().x),
        top: pct(rect().y),
        width: pct(rect().w),
        height: pct(rect().h),
        "--pane-gap-left": rect().x > 0 ? "1px" : "0px",
        "--pane-gap-top": rect().y > 0 ? "1px" : "0px",
      }}
      onPointerDown={() => activatePane(id)}
      onFocusIn={() => activatePane(id)}
    >
      <div class="pane-inner">
        <header class="pane-header" onPointerDown={startPaneDrag}>
          <button
            type="button"
            class="pane-nav"
            aria-label="Back"
            title="Back (Alt+←)"
            disabled={!M.canGo(workspace(), id, -1)}
            onClick={() => goPane(id, -1)}
          >
            ‹
          </button>
          <button
            type="button"
            class="pane-nav"
            aria-label="Forward"
            title="Forward (Alt+→)"
            disabled={!M.canGo(workspace(), id, 1)}
            onClick={() => goPane(id, 1)}
          >
            ›
          </button>
          <Show
            when={props.phone && siblings().length > 1}
            fallback={
              <span class="pane-title" data-testid="pane-title">
                <Show when={paneTitle(id).icon !== undefined}>
                  <TypeIcon name={paneTitle(id).icon} size={14} />
                </Show>
                <span>{paneTitle(id).title}</span>
              </span>
            }
          >
            <PaneSwitcher id={id} panes={siblings()} />
          </Show>
          <PaneMenu id={id} phone={props.phone} />
        </header>
        <div class="pane-frame">
          <div class="pane-body">
            <Show when={mounted()}>
              <PaneProvider value={{ id, setTitle: (t) => setPaneTitle(id, t) }}>
                <PaneRouter pane={id}>{paneRoutes()}</PaneRouter>
              </PaneProvider>
            </Show>
          </div>
        </div>
      </div>
    </section>
  );
}

/** Closes a popup when a press lands outside `el` or Escape is pressed. */
function dismissable(el: () => HTMLElement | undefined, open: () => boolean, close: () => void) {
  const onDown = (e: PointerEvent) => {
    if (open() && !el()?.contains(e.target as Node)) close();
  };
  const onKey = (e: KeyboardEvent) => {
    if (open() && e.key === "Escape") {
      e.preventDefault();
      close();
    }
  };
  document.addEventListener("pointerdown", onDown, true);
  document.addEventListener("keydown", onKey);
  onCleanup(() => {
    document.removeEventListener("pointerdown", onDown, true);
    document.removeEventListener("keydown", onKey);
  });
}

/** The ⋯ menu: split, close, move to a new tab, and the keyboard shortcuts. */
function PaneMenu(props: { id: string; phone: boolean }) {
  const [open, setOpen] = createSignal(false);
  let root: HTMLDivElement | undefined;
  dismissable(() => root, open, () => setOpen(false));
  const act = (fn: () => void) => () => {
    setOpen(false);
    fn();
  };
  const alone = () => {
    const t = M.tabOf(workspace(), props.id);
    return !t || M.panesOf(t.layout).length === 1;
  };
  return (
    <div class="pane-menu" ref={root}>
      <button
        type="button"
        class="pane-menu-button"
        aria-label="Pane actions"
        aria-haspopup="menu"
        aria-expanded={open()}
        title="Pane actions"
        onClick={() => setOpen(!open())}
      >
        ⋯
      </button>
      <Show when={open()}>
        <div class="pane-menu-list" role="menu" aria-label="Pane actions">
          <Show when={!props.phone}>
            <button type="button" role="menuitem" onClick={act(() => splitPane(props.id, "right"))}>
              Split right
            </button>
            <button type="button" role="menuitem" onClick={act(() => splitPane(props.id, "bottom"))}>
              Split down
            </button>
          </Show>
          <button type="button" role="menuitem" onClick={act(() => closePane(props.id))}>
            Close pane <kbd>Alt+W</kbd>
          </button>
          <button type="button" role="menuitem" disabled={alone()} onClick={act(() => movePaneToNewTab(props.id))}>
            Move to new tab
          </button>
          <div class="pane-menu-keys" role="note" aria-label="Keyboard shortcuts">
            <p>Keyboard shortcuts</p>
            <dl>
              <For each={SHORTCUTS}>
                {([keys, what]) => (
                  <>
                    <dt>{keys}</dt>
                    <dd>{what}</dd>
                  </>
                )}
              </For>
            </dl>
          </div>
        </div>
      </Show>
    </div>
  );
}

/** On a phone: the active pane's title, opening a list of the tab's panes to switch between. */
function PaneSwitcher(props: { id: string; panes: string[] }) {
  const [open, setOpen] = createSignal(false);
  let root: HTMLDivElement | undefined;
  dismissable(() => root, open, () => setOpen(false));
  const index = () => props.panes.indexOf(props.id) + 1;
  return (
    <div class="pane-switcher" ref={root}>
      <button
        type="button"
        class="pane-switcher-button"
        aria-haspopup="listbox"
        aria-expanded={open()}
        aria-label="Switch pane"
        onClick={() => setOpen(!open())}
      >
        <Show when={paneTitle(props.id).icon !== undefined}>
          <TypeIcon name={paneTitle(props.id).icon} size={14} />
        </Show>
        <span class="pane-switcher-title">{paneTitle(props.id).title}</span>
        <span class="pane-switcher-count">
          {index()}/{props.panes.length}
        </span>
        <span aria-hidden="true">▾</span>
      </button>
      <Show when={open()}>
        <div class="pane-switcher-list" role="listbox" aria-label="Panes in this tab">
          <For each={props.panes}>
            {(p) => (
              <button
                type="button"
                role="option"
                aria-selected={p === props.id}
                onClick={() => {
                  setOpen(false);
                  activatePane(p);
                }}
              >
                <Show when={paneTitle(p).icon !== undefined}>
                  <TypeIcon name={paneTitle(p).icon} size={14} />
                </Show>
                <span>{paneTitle(p).title}</span>
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

/** The line between a split's two sides: drag it, or focus it and use the arrow keys. */
function DividerView(props: { id: string; rects: () => ReturnType<typeof M.layoutRects>; area: () => HTMLDivElement }) {
  const d = createMemo(() => props.rects().dividers.find((x) => x.split === props.id));
  const [active, setActive] = createSignal(false);

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    setActive(true);
    document.body.classList.add(d()?.dir === "row" ? "ws-resizing-row" : "ws-resizing-column");
    const move = (ev: PointerEvent) => {
      const div = d();
      if (!div) return;
      const b = props.area().getBoundingClientRect();
      const ratio =
        div.dir === "row"
          ? (ev.clientX - b.left - div.area.x * b.width) / (div.area.w * b.width)
          : (ev.clientY - b.top - div.area.y * b.height) / (div.area.h * b.height);
      if (Number.isFinite(ratio)) setRatio(div.split, ratio);
    };
    const end = () => {
      setActive(false);
      document.body.classList.remove("ws-resizing-row", "ws-resizing-column");
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", end);
      el.removeEventListener("pointercancel", end);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", end);
    el.addEventListener("pointercancel", end);
  }

  function onKeyDown(e: KeyboardEvent) {
    const div = d();
    if (!div) return;
    const less = div.dir === "row" ? "ArrowLeft" : "ArrowUp";
    const more = div.dir === "row" ? "ArrowRight" : "ArrowDown";
    const step = e.shiftKey ? 0.1 : 0.02;
    if (e.key === less || e.key === more) {
      e.preventDefault();
      setRatio(div.split, div.ratio + (e.key === more ? step : -step));
    } else if (e.key === "Home" || e.key === "End") {
      e.preventDefault();
      setRatio(div.split, e.key === "Home" ? M.MIN_RATIO : 1 - M.MIN_RATIO);
    }
  }

  return (
    <Show when={d()}>
      {(div) => {
        const style = () => {
          const { area, ratio, dir } = div();
          return dir === "row"
            ? { left: `${(area.x + area.w * ratio) * 100}%`, top: `${area.y * 100}%`, height: `${area.h * 100}%` }
            : { top: `${(area.y + area.h * ratio) * 100}%`, left: `${area.x * 100}%`, width: `${area.w * 100}%` };
        };
        return (
          <div
            class="ws-divider"
            classList={{ [`ws-divider-${div().dir}`]: true, active: active() }}
            role="separator"
            tabindex={0}
            aria-label="Resize panes"
            aria-orientation={div().dir === "row" ? "vertical" : "horizontal"}
            aria-valuemin={M.MIN_RATIO * 100}
            aria-valuemax={(1 - M.MIN_RATIO) * 100}
            aria-valuenow={Math.round(div().ratio * 100)}
            data-split-id={div().split}
            style={style()}
            onPointerDown={onPointerDown}
            onKeyDown={onKeyDown}
          />
        );
      }}
    </Show>
  );
}

/** What is under a point while pane `dragged` is dragged. */
function dropTargetAt(x: number, y: number, dragged: string): DropTarget | null {
  const el = document.elementFromPoint(x, y);
  if (!el) return null;
  const tab = el.closest<HTMLElement>("[data-tab-id]");
  if (tab) {
    const id = tab.dataset.tabId!;
    return M.tabOf(workspace(), dragged)?.id === id ? null : { kind: "tab", tab: id };
  }
  if (el.closest(".ws-tabbar")) {
    const from = M.tabOf(workspace(), dragged);
    return from && M.panesOf(from.layout).length > 1 ? { kind: "new-tab" } : null;
  }
  const paneEl = el.closest<HTMLElement>("[data-pane-id]");
  const pane = paneEl?.dataset.paneId;
  if (!paneEl || !pane || pane === dragged) return null;
  const r = paneEl.getBoundingClientRect();
  return { kind: "pane", pane, zone: M.dropZone((x - r.left) / r.width, (y - r.top) / r.height) };
}

function drop(pane: string, target: DropTarget) {
  switch (target.kind) {
    case "pane":
      if (target.zone === "center") swapPanes(pane, target.pane);
      else dockPane(pane, target.pane, target.zone);
      return;
    case "tab":
      movePaneToTab(pane, target.tab);
      return;
    case "new-tab":
      movePaneToNewTab(pane);
      return;
  }
}

/** A translucent overlay on the region a dragged pane would take. */
function DropPreview(props: { area: () => HTMLDivElement }) {
  const box = createMemo(() => {
    const t = paneDrag()?.target;
    if (!t || t.kind !== "pane") return null;
    const el = props.area().querySelector<HTMLElement>(`[data-pane-id="${t.pane}"]`);
    if (!el) return null;
    const a = props.area().getBoundingClientRect();
    const r = el.getBoundingClientRect();
    let { left, top, width, height } = { left: r.left - a.left, top: r.top - a.top, width: r.width, height: r.height };
    if (t.zone === "left" || t.zone === "right") {
      width /= 2;
      if (t.zone === "right") left += width;
    } else if (t.zone === "top" || t.zone === "bottom") {
      height /= 2;
      if (t.zone === "bottom") top += height;
    }
    return { left, top, width, height, zone: t.zone };
  });
  return (
    <Show when={box()}>
      {(b) => (
        <div
          class="ws-drop-preview"
          classList={{ "ws-drop-swap": b().zone === "center" }}
          data-zone={b().zone}
          style={{ left: `${b().left}px`, top: `${b().top}px`, width: `${b().width}px`, height: `${b().height}px` }}
        >
          <span>{b().zone === "center" ? "Swap" : `Dock ${b().zone}`}</span>
        </div>
      )}
    </Show>
  );
}
