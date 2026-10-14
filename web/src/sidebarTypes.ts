// The left sidebar's Types section: every type, alphabetically, under a disclosure that starts
// collapsed and remembers whether it is open per browser.

import { createRoot, createSignal } from "solid-js";
import type { TypeDef } from "./api";

export const TYPES_OPEN_KEY = "thevault.sidebar.types";

/** What the sidebar shows for a type: its plural, as the gallery and its note list do. */
export function typeNavLabel(t: Pick<TypeDef, "name" | "plural" | "label">): string {
  return t.plural || t.label || t.name;
}

/** `types` by their sidebar label, ignoring case, then by name so the order is stable. */
export function sortTypesForNav<T extends Pick<TypeDef, "name" | "plural" | "label">>(types: readonly T[]): T[] {
  return [...types].sort(
    (a, b) =>
      typeNavLabel(a).localeCompare(typeNavLabel(b), undefined, { sensitivity: "base" }) ||
      a.name.localeCompare(b.name),
  );
}

function load(): boolean {
  try {
    return localStorage.getItem(TYPES_OPEN_KEY) === "true";
  } catch {
    return false;
  }
}

const [typesOpen, set] = createRoot(() => createSignal(load()));

export { typesOpen };

export function setTypesOpen(open: boolean) {
  set(open);
  try {
    localStorage.setItem(TYPES_OPEN_KEY, String(open));
  } catch {
    // Private browsing: the section stays as it is until the page closes.
  }
}
