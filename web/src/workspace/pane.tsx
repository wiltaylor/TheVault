// What a pane gives the page it shows: a router of its own over the pane's history, and a way to
// name the page in the pane header and tab.

import { createRouter } from "@solidjs/router";
import { type JSX, createContext, createEffect, createMemo, on, untrack, useContext } from "solid-js";
import { createStore } from "solid-js/store";
import { assetIdOf, assetName, kindIcon, kindOf } from "../assetLibrary";
import { noteTitle, noteType } from "../editor/titles";
import { typeDef } from "../session";
import * as M from "./model";
import { goPane, navigatePane, workspace } from "./store";

export interface PaneApi {
  id: string;
  /** Name the page: its title and type icon, or null to fall back to the location's name. */
  setTitle(title: PageTitle | null): void;
}

export interface PageTitle {
  title: string;
  icon?: string | null;
}

const PaneContext = createContext<PaneApi>();

export const PaneProvider = PaneContext.Provider;

/** The pane the calling component is in, if any. */
export function usePane(): PaneApi | undefined {
  return useContext(PaneContext);
}

/** The titles mounted pages give their panes, which beat the names worked out from locations. */
const [liveTitles, setLiveTitles] = createStore<Record<string, PageTitle | undefined>>({});

export function setPaneTitle(pane: string, title: PageTitle | null) {
  setLiveTitles(pane, title ?? undefined);
}

const NAMES: [RegExp, string, string?][] = [
  [/^\/daily/, "Daily notes"],
  [/^\/settings/, "Settings"],
  [/^\/search/, "Search"],
  [/^\/inbox/, "Inbox"],
  [/^\/assets/, "Images and files"],
  [/^\/trash/, "Trash"],
  [/^\/archive/, "Archive"],
  [/^\/agents\//, "Agent"],
  [/^\/outbox/, "Outbox", "send"],
  [/^\/log/, "Log", "scroll-text"],
  [/^\/browser-box/, "Browser", "monitor"],
];

/** A name for the page at `url`, from the location alone. Reactive: note titles load. */
export function locationTitle(url: string): PageTitle {
  const path = url.split(/[?#]/)[0]!;
  // The old history address redirects to its note, so it is named as the note.
  const note = /^\/notes\/([^/]+)(?:\/history)?$/.exec(path);
  if (note && note[1] !== "new") {
    const id = note[1]!;
    const t = noteTitle(id);
    const title = t === undefined ? "…" : (t ?? "Missing note");
    const icon = typeDef(noteType(id) ?? "")?.icon ?? null;
    return { title: title || "Untitled", icon };
  }
  if (path === "/notes/new") {
    const type = new URLSearchParams(url.split("?")[1] ?? "").get("type") ?? "";
    const t = typeDef(type);
    return { title: `New ${t?.label.toLowerCase() ?? "note"}`, icon: t?.icon ?? null };
  }
  const asset = assetIdOf(path);
  if (asset) {
    const name = assetName(asset);
    return { title: name ?? "File", icon: kindIcon(name ? kindOf(name) : undefined) };
  }
  const list = /^\/types\/([^/]+)$/.exec(path);
  if (list) {
    const t = typeDef(list[1]!);
    return { title: t?.plural ?? list[1]!, icon: t?.icon ?? null };
  }
  for (const [re, name, icon] of NAMES) if (re.test(path)) return icon ? { title: name, icon } : { title: name };
  if (path === "/") return { title: "Daily notes" };
  return { title: "Not found" };
}

/** Reactive: the title of what `pane` shows. */
export function paneTitle(pane: string): PageTitle {
  const url = M.location(workspace(), pane);
  const live = liveTitles[pane];
  return live ?? locationTitle(url);
}

/** Scroll `pane`'s page back to its top, as a fresh page starts. */
export function scrollPaneTop(pane: string) {
  queueMicrotask(() => {
    const body = document.querySelector<HTMLElement>(`[data-pane-id="${pane}"] .pane-body`);
    body?.scrollTo?.({ top: 0 });
  });
}

/**
 * A router over `pane`'s own history. Pages inside use `useParams`, `useNavigate` and `<A>` as
 * they would under the browser's router; links are caught by the workspace (see links.ts).
 */
export function PaneRouter(props: { pane: string; children: JSX.Element }) {
  const pane = props.pane;
  const url = createMemo(() => M.location(workspace(), pane));
  const Router = createRouter({
    get: () => untrack(url),
    set: ({ value, replace, scroll }) => {
      navigatePane(pane, value, replace);
      if (scroll && !replace) scrollPaneTop(pane);
    },
    init: (notify) => {
      // The pane moved without this router: a link, back or forward, or a restore.
      createEffect(on(url, (v) => notify(v), { defer: true }));
      return () => {};
    },
    utils: { go: (delta) => goPane(pane, delta) },
  });
  return <Router>{props.children}</Router>;
}
