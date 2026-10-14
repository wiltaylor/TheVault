import type { NoteDoc, TypeDef } from "../api";
import { noteTitle } from "../editor/titles";
import { refValue } from "../notes";
import { formatValue } from "./Render";

/**
 * The text `{{name}}` shows for a note: the value formatted as everywhere else, references by
 * their notes' titles, or null when the type has no field `name`.
 */
export function refText(type: TypeDef | undefined, doc: NoteDoc, name: string): string | null {
  const v = refValue(type, doc, name);
  if (v === null) return null;
  const f = type?.fields.find((x) => x.name === name);
  if (f?.kind === "reference") {
    const ids = (Array.isArray(v) ? v : [v]).filter((x): x is string => typeof x === "string");
    return ids.map((id) => noteTitle(id) ?? id).join(", ");
  }
  return formatValue(v);
}
