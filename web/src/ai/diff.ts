// A proposal's diff, for its card: a field's old and new value, and a block's text line by line.

import type { Json } from "../api";

export interface DiffLine {
  type: "same" | "del" | "add";
  text: string;
}

/** The lines of text a block (or a list of blocks) holds, nested blocks indented under it. */
export function blockLines(v: Json, depth = 0): string[] {
  if (v === null || v === undefined) return [];
  if (Array.isArray(v)) return v.flatMap((b) => blockLines(b, depth));
  if (typeof v !== "object") return [String(v)];
  const kind = typeof v.kind === "string" ? v.kind : "";
  const fields = (v.fields && typeof v.fields === "object" && !Array.isArray(v.fields) ? v.fields : {}) as Record<string, Json>;
  const out: string[] = [];
  const pad = "  ".repeat(depth);
  const text = typeof fields.text === "string" ? fields.text : null;
  const others = Object.entries(fields).filter(([k]) => k !== "text");
  if (text !== null) {
    const mark = kind === "heading" ? "# " : kind === "item" ? (fields.checked === true ? "[x] " : "- ") : "";
    out.push(`${pad}${mark}${text}`);
  } else if (kind && kind !== "content") {
    const attrs = others.map(([k, x]) => `${k}=${JSON.stringify(x)}`).join(" ");
    out.push(`${pad}${kind}${attrs ? ` ${attrs}` : ""}`);
  }
  const children = Array.isArray(v.blocks) ? v.blocks : [];
  // A content block is only a container: its blocks sit at its own depth.
  const inner = out.length > 0 ? depth + 1 : depth;
  for (const c of children) out.push(...blockLines(c, inner));
  return out;
}

/** The lines of `a` and `b` aligned: kept, removed and added, by longest common subsequence. */
export function lineDiff(a: readonly string[], b: readonly string[]): DiffLine[] {
  const n = a.length;
  const m = b.length;
  const lcs: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i]![j] = a[i] === b[j] ? lcs[i + 1]![j + 1]! + 1 : Math.max(lcs[i + 1]![j]!, lcs[i]![j + 1]!);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ type: "same", text: a[i]! });
      i++;
      j++;
    } else if (lcs[i + 1]![j]! >= lcs[i]![j + 1]!) {
      out.push({ type: "del", text: a[i++]! });
    } else {
      out.push({ type: "add", text: b[j++]! });
    }
  }
  while (i < n) out.push({ type: "del", text: a[i++]! });
  while (j < m) out.push({ type: "add", text: b[j++]! });
  return out;
}
