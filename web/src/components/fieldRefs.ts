// `{{name}}` in a template's fixed blocks shows a field of the note being shown, read-only
// (docs/vault-format.md, "Field references"). Whatever renders a template's fixed blocks provides
// the note's values here; everything else provides nothing, and `{{name}}` stays text.

import { createContext } from "solid-js";

export interface FieldRefValues {
  /** The text of field `name` of the note shown, or null when the type has no such field. */
  text(name: string): string | null;
}

export const FieldRefs = createContext<FieldRefValues | undefined>(undefined);
