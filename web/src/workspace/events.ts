// The workspace's document-wide handlers: in-app links open in the pane they are in (or the
// active pane, outside any pane), Ctrl/Cmd+click and middle-click open a background tab, and the
// keyboard and mouse shortcuts act on the active pane.

import * as M from "./model";
import { closePane, cycleTab, goPane, navigatePane, onPopState, openTab, saveNow, workspace } from "./store";
import { scrollPaneTop } from "./pane";

/** The pane an element is in, else the active pane. */
export function paneAt(el: Element | null): string {
  const id = el?.closest("[data-pane-id]")?.getAttribute("data-pane-id");
  return id && workspace().panes[id] ? id : M.activePane(workspace());
}

/** The in-app location an event's link goes to, or null for anything the browser should have. */
function linkTarget(e: MouseEvent): { a: HTMLAnchorElement | SVGAElement; url: string } | null {
  const a = e
    .composedPath()
    .find((el): el is HTMLAnchorElement | SVGAElement => el instanceof Element && el.nodeName.toUpperCase() === "A");
  if (!a) return null;
  // Links in editable text are the editor's to follow, so that a click that selects stays put.
  if (a instanceof HTMLElement && a.isContentEditable) return null;
  if (a.hasAttribute("download")) return null;
  const rel = (a.getAttribute("rel") ?? "").split(/\s+/);
  if (rel.includes("external")) return null;
  const target = a instanceof SVGAElement ? a.target.baseVal : a.target;
  const href = a instanceof SVGAElement ? a.href.baseVal : a.getAttribute("href");
  if (target || !href) return null;
  const url = new URL(href, document.baseURI);
  if (url.origin !== window.location.origin) return null;
  const path = url.pathname + url.search + url.hash;
  return M.isAppPath(path) || url.pathname === "/" ? { a, url: path } : null;
}

/** Open `url` in a tab of its own behind the one showing, as a browser does with a middle-click. */
export function openInBackground(url: string) {
  openTab(url, true);
}

/** Whether a click asks for a new tab rather than this pane. */
export function wantsNewTab(e: MouseEvent): boolean {
  return e.button === 1 || ((e.ctrlKey || e.metaKey) && e.button === 0 && !e.shiftKey && !e.altKey);
}

/**
 * Follow `href` as a click on a link to it would, for links the browser does not follow itself,
 * such as those in the editor: an in-app location opens in the pane `from` is in, or in a
 * background tab when `background`; another site opens in a new browser tab. Anything else is
 * left alone. Returns whether it went anywhere.
 */
export function followLink(href: string, from: Element, background: boolean): boolean {
  const url = new URL(href, document.baseURI);
  const path = url.pathname + url.search + url.hash;
  if (url.origin === window.location.origin && (M.isAppPath(path) || url.pathname === "/")) {
    if (background) {
      openInBackground(path);
    } else {
      const pane = paneAt(from);
      navigatePane(pane, path);
      scrollPaneTop(pane);
    }
    return true;
  }
  if (!/^(https?|mailto):$/.test(url.protocol)) return false;
  window.open(url.href, "_blank", "noopener,noreferrer");
  return true;
}

function onClick(e: MouseEvent) {
  if (e.defaultPrevented || e.button !== 0 || e.altKey || e.shiftKey) return;
  const hit = linkTarget(e);
  if (!hit) return;
  e.preventDefault();
  if (e.ctrlKey || e.metaKey) {
    openInBackground(hit.url);
    return;
  }
  const pane = paneAt(hit.a);
  navigatePane(pane, hit.url);
  scrollPaneTop(pane);
}

function onAuxClick(e: MouseEvent) {
  if (e.defaultPrevented || e.button !== 1) return;
  const hit = linkTarget(e);
  if (!hit) return;
  e.preventDefault();
  openInBackground(hit.url);
}

function onMouseDown(e: MouseEvent) {
  // A middle press on a link would start the browser's autoscroll; the back and forward buttons
  // would leave the app.
  if ((e.button === 1 && linkTarget(e)) || e.button === 3 || e.button === 4) e.preventDefault();
}

function onMouseUp(e: MouseEvent) {
  if (e.button !== 3 && e.button !== 4) return;
  e.preventDefault();
  goPane(paneAt(e.target instanceof Element ? e.target : null), e.button === 3 ? -1 : 1);
}

const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);

function editing(el: EventTarget | null): boolean {
  return el instanceof HTMLElement && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));
}

function onKeyDown(e: KeyboardEvent) {
  if (e.defaultPrevented) return;
  const pane = () => M.activePane(workspace());
  if (e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
    // On a Mac, Option+arrow moves by word in text; leave it there.
    if ((e.key === "ArrowLeft" || e.key === "ArrowRight") && !(isMac && editing(e.target))) {
      e.preventDefault();
      goPane(pane(), e.key === "ArrowLeft" ? -1 : 1);
      return;
    }
    if (e.code === "KeyW") {
      e.preventDefault();
      closePane(pane());
      return;
    }
    if (e.key === "PageDown" || e.code === "BracketRight") {
      e.preventDefault();
      cycleTab(1);
      return;
    }
    if (e.key === "PageUp" || e.code === "BracketLeft") {
      e.preventDefault();
      cycleTab(-1);
      return;
    }
  }
  // Most browsers keep Ctrl+W for themselves; where they let a page have it, it closes the pane.
  if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.code === "KeyW") {
    e.preventDefault();
    closePane(pane());
  }
}

function onPop(e: PopStateEvent) {
  onPopState(e.state, window.location.pathname + window.location.search + window.location.hash);
}

/** Listen on the document for as long as the workspace shows; returns the cleanup. */
export function installWorkspaceEvents(): () => void {
  document.addEventListener("click", onClick);
  document.addEventListener("auxclick", onAuxClick);
  document.addEventListener("mousedown", onMouseDown);
  document.addEventListener("mouseup", onMouseUp);
  document.addEventListener("keydown", onKeyDown);
  window.addEventListener("popstate", onPop);
  window.addEventListener("beforeunload", saveNow);
  return () => {
    document.removeEventListener("click", onClick);
    document.removeEventListener("auxclick", onAuxClick);
    document.removeEventListener("mousedown", onMouseDown);
    document.removeEventListener("mouseup", onMouseUp);
    document.removeEventListener("keydown", onKeyDown);
    window.removeEventListener("popstate", onPop);
    window.removeEventListener("beforeunload", saveNow);
  };
}

/** The shortcuts, as the pane menu lists them. */
export const SHORTCUTS: [string, string][] = [
  ["Alt+←  /  Alt+→", "Back / forward in the pane"],
  ["Alt+W", "Close the pane"],
  ["Alt+PageDown  /  Alt+]", "Next tab"],
  ["Alt+PageUp  /  Alt+[", "Previous tab"],
  ["Ctrl+click, middle-click", "Open a link in a new tab"],
  ["Ctrl+K", "Search or run a command"],
  ["Ctrl+Shift+K", "Capture a fleeting note"],
];
