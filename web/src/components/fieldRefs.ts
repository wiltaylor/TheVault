// `{{name}}` in a template's fixed blocks shows a field of the note being shown, read-only
// (docs/vault-format.md, "Field references"). Whatever renders a template's fixed blocks provides
// the note's values here; everything else provides nothing, and `{{name}}` stays text.

import { createContext } from "solid-js";
import { parseInline } from "../editor/markup";

export interface FieldRefValues {
  /** The text of field `name` of the note shown, or null when the type has no such field. */
  text(name: string): string | null;
}

export const FieldRefs = createContext<FieldRefValues | undefined>(undefined);

/**
 * `text` as plain text with each `{{name}}` filled in from `refs`, for an image's alt text and
 * caption in a template. Without `refs` (outside a template) the text comes back as written.
 */
export function fillFieldRefs(text: string, refs: FieldRefValues | undefined): string {
  if (!refs || !text.includes("{{")) return text;
  return parseInline(text, { fieldRefs: true })
    .map((n) => {
      if (n.type === "text") return n.text;
      if (n.type === "fieldRef") return refs.text(n.attrs.name) ?? `{{${n.attrs.name}}}`;
      if (n.type === "mention") return n.attrs.label ?? "";
      return " ";
    })
    .join("");
}
