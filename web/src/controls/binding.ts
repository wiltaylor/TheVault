// What a `control` block is bound to: the fields of the note it shows on, through that note's own
// edit path. Whatever renders a note's template or content provides one; a control with none
// around it (a canvas outside a note, say) is shown disabled.

import { createContext, useContext } from "solid-js";
import type { FieldDef, Json, TypeDef } from "../api";

export interface ControlBinding {
  /** The type of the note the controls edit, or of the template being edited. */
  type: () => TypeDef | undefined;
  /** The field's current value in the note, as edited. */
  value: (f: FieldDef) => Json | undefined;
  /** Write the field through the note's autosave. Without it, controls are read-only. */
  set?: (f: FieldDef, v: Json | undefined) => void;
  /** Write several fields in one edit, such as a labelled list's values and labels. */
  setMany?: (values: Record<string, Json | undefined>) => void;
  /** Why the controls are read-only, when they are. */
  reason?: string;
  /** The template editor: controls show what they will look like, with sample values. */
  preview?: boolean;
  /** The note being edited, for reference searches that exclude it. */
  noteId?: () => string | undefined;
}

export const ControlBindingContext = createContext<ControlBinding | undefined>(undefined);

export function useControlBinding(): ControlBinding | undefined {
  return useContext(ControlBindingContext);
}
