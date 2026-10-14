// What a chip picker over a taxonomy's terms needs, shared by note fields and form controls:
// suggestions with depth and colour, chip colours, and for an open taxonomy a way to add a term.

import type { Term } from "../api";
import { newTermPicked, taxonomyDef } from "../session";
import { newTermPath, termChipColour, termSuggestions } from "./ChipPicker";

function holds(terms: Term[], path: string): boolean {
  return terms.some((t) => t.path === path || holds(t.children, path));
}

/** The picker props for taxonomy `name`. `fixed` offers no new terms, even when it is open. */
export function termPicker(name: () => string, fixed = false) {
  const terms = () => taxonomyDef(name())?.terms ?? [];
  const open = () => !fixed && taxonomyDef(name())?.allow_new === true;
  return {
    suggest: (text: string) => termSuggestions(terms(), text),
    chipColour: (v: string) => termChipColour(terms(), v),
    /** For an open taxonomy: the path typed text names, existing or new. */
    create: () => (open() ? (text: string) => newTermPath(terms(), text) : undefined),
    /**
     * Note picked values the taxonomy lacks, so the taxonomies load again after the save that
     * adds them. Returns `values`.
     */
    picked: <T>(values: T): T => {
      if (Array.isArray(values) && values.some((v) => typeof v === "string" && !holds(terms(), v))) newTermPicked();
      return values;
    },
  };
}
