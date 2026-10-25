// Asset file names for read-only values: an asset in a list or a dict shows its file name, read
// once from the asset listing, rather than its id.

import { createRoot } from "solid-js";
import { createStore } from "solid-js/store";
import { api } from "../api";

const [names, setNames] = createRoot(() => createStore<Record<string, string>>({}));
let listing: Promise<void> | null = null;

/** The file name of asset `id`, once the listing has loaded; `undefined` until then or for an unknown id. Reactive. */
export function assetName(id: string): string | undefined {
  listing ??= api.assets().then(
    (all) => {
      for (const a of all) setNames(a.id, a.filename);
    },
    () => undefined,
  );
  return names[id];
}
