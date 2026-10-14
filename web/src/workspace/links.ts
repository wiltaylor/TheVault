// Which links outside the panes point where the active pane is, for their highlight.

import * as M from "./model";
import { workspace } from "./store";

/** Reactive: the path the active pane shows, without its query or hash. */
export function activeLocation(): string {
  const w = workspace();
  return M.location(w, M.activePane(w)).split(/[?#]/)[0]!;
}

/** Reactive: whether `href` is the active pane's location or a section above it, as `<A>` judges. */
export function isActiveLink(href: string, end = false): boolean {
  const here = activeLocation().toLowerCase();
  const to = href.toLowerCase();
  if (end || to === "/") return here === to;
  return here === to || here.startsWith(`${to}/`);
}
