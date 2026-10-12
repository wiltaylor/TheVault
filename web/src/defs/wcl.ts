// Writing the data-only WCL subset of docs/vault-format.md from JSON. The definition editors
// build type, template and taxonomy files with these; the server canonicalises what they send.

import type { Block, Json } from "../api";

const IDENT = /^[A-Za-z_][A-Za-z0-9_]*$/;

/** Block kinds whose labels are display text, always written as strings. */
const STRING_LABELS = new Set(["tab", "slot", "term", "handler"]);

export function wclString(s: string): string {
  const escaped = s.replace(/[\\"\n\t\r]/g, (c) => ({ "\\": "\\\\", '"': '\\"', "\n": "\\n", "\t": "\\t", "\r": "\\r" })[c]!);
  return `"${escaped}"`;
}

/** A number as WCL writes it. A float is written with a decimal point. */
export function wclNumber(n: number, float = false): string {
  if (!Number.isFinite(n)) throw new Error(`${n} is not a finite number`);
  const s = String(n);
  return float && Number.isInteger(n) && !/e/i.test(s) ? `${s}.0` : s;
}

/** A value. `float` writes every whole number in it with a decimal point. */
export function wclValue(v: Json, float = false): string {
  if (v === null) throw new Error("WCL has no null; leave the item out instead");
  if (typeof v === "string") return wclString(v);
  if (typeof v === "number") return wclNumber(v, float);
  if (typeof v === "boolean") return v ? "true" : "false";
  if (Array.isArray(v)) return `[${v.map((x) => wclValue(x, float)).join(", ")}]`;
  const members = Object.entries(v).map(([k, x]) => {
    if (!IDENT.test(k)) throw new Error(`${JSON.stringify(k)} is not a record key`);
    return `${k}: ${wclValue(x, float)}`;
  });
  return members.length ? `{ ${members.join(", ")} }` : "{}";
}

function label(kind: string, l: string): string {
  return !STRING_LABELS.has(kind) && IDENT.test(l) ? l : wclString(l);
}

/** One `name = value` line, or nothing when the value is absent. */
export function wclField(name: string, v: Json | undefined, indent = "", float = false): string {
  return v === undefined || v === null ? "" : `${indent}${name} = ${wclValue(v, float)}\n`;
}

/** A block and everything in it. */
export function wclBlock(b: Block, indent = ""): string {
  const head = [b.kind, ...(b.labels ?? []).map((l) => label(b.kind, l))].join(" ");
  const fields = Object.entries(b.fields ?? {})
    .map(([k, v]) => wclField(k, v, `${indent}  `))
    .join("");
  const blocks = (b.blocks ?? []).map((c) => wclBlock(c, `${indent}  `));
  if (!fields && blocks.length === 0) return `${indent}${head}\n`;
  return `${indent}${head} {\n${[fields, ...blocks].filter(Boolean).join("\n")}${indent}}\n`;
}
