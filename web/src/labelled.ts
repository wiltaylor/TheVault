// Lists of labelled values, such as a person's phones: which fields are one. The chips that edit
// them are in components/LabelledList.tsx.

import type { FieldDef } from "./api";

/** How a list of labelled values stores each item: the key holding the value, and whether `label` must be written. */
export interface LabelledShape {
  key: string;
  labelRequired: boolean;
}

/**
 * The shape of `f`'s items when it is a list of labelled values: a list whose item is a dict with
 * exactly two declared keys, both text, one of them `label`, and no other entries. Else undefined.
 */
export function labelledShape(f: FieldDef): LabelledShape | undefined {
  const item = f.kind === "list" && !f.listed ? f.item : undefined;
  if (!item || item.kind !== "dict" || (item.values !== null && item.values !== undefined)) return undefined;
  const keys = item.keys ?? [];
  if (keys.length !== 2 || keys.some((k) => k.kind !== "string" || k.multiple)) return undefined;
  const label = keys.find((k) => k.name === "label");
  const value = keys.find((k) => k.name !== "label");
  return label && value ? { key: value.name, labelRequired: label.required === true } : undefined;
}
