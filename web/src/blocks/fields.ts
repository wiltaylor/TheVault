// Reading and changing a content block's items (docs/vault-format.md, "Content blocks").

import type { Block, Json } from "../api";

/** A string item, or "" when it is missing. */
export function text(block: Block, item: string): string {
  const v = block.fields?.[item];
  return typeof v === "string" ? v : "";
}

/** `block` with `item` set to `value`, or without `item` when `value` is undefined. */
export function withItem(block: Block, item: string, value: Json | undefined): Block {
  const fields = { ...block.fields };
  if (value === undefined) delete fields[item];
  else fields[item] = value;
  return { ...block, fields };
}

/** `block` with optional string `item` set, or removed when `value` is empty. */
export function withText(block: Block, item: string, value: string): Block {
  return withItem(block, item, value === "" ? undefined : value);
}

/** The nested blocks of `kind`. */
export function children(block: Block, kind: string): Block[] {
  return (block.blocks ?? []).filter((b) => b.kind === kind);
}
