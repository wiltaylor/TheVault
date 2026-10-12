// A content field's blocks (docs/vault-format.md, "Content blocks", D34) to and from the editor's
// ProseMirror JSON. Headings, paragraphs, lists and quotes become editable nodes with their text
// parsed as inline markup (D41). Every other block kind rides along untouched in a `vaultBlock`.

import type { JSONContent } from "@tiptap/core";
import type { Block } from "../api";
import { parseInline, serializeInline } from "./markup";

const LIST_NODE: Record<string, string> = { bullet: "bulletList", numbered: "orderedList", todo: "taskList" };
const LIST_STYLE: Record<string, string> = { bulletList: "bullet", orderedList: "numbered", taskList: "todo" };

function str(block: Block, item: string): string {
  const v = block.fields?.[item];
  return typeof v === "string" ? v : "";
}

function inline(text: string): JSONContent[] | undefined {
  const nodes = parseInline(text) as JSONContent[];
  return nodes.length ? nodes : undefined;
}

function paragraph(text: string): JSONContent {
  const content = inline(text);
  return content ? { type: "paragraph", content } : { type: "paragraph" };
}

function listToDoc(block: Block, style: string): JSONContent {
  const type = LIST_NODE[style] ?? "bulletList";
  const items = (block.blocks ?? []).filter((b) => b.kind === "item");
  return {
    type,
    content: items.map((item) => {
      const content: JSONContent[] = [paragraph(str(item, "text"))];
      if ((item.blocks ?? []).some((b) => b.kind === "item")) content.push(listToDoc(item, style));
      return type === "taskList"
        ? { type: "taskItem", attrs: { checked: item.fields?.checked === true }, content }
        : { type: "listItem", content };
    }),
  };
}

function blockToDoc(block: Block): JSONContent {
  switch (block.kind) {
    case "heading": {
      const level = Math.min(3, Math.max(1, Number(block.fields?.level ?? 1)));
      const content = inline(str(block, "text"));
      return content ? { type: "heading", attrs: { level }, content } : { type: "heading", attrs: { level } };
    }
    case "paragraph":
      return paragraph(str(block, "text"));
    case "quote":
      return { type: "blockquote", content: [paragraph(str(block, "text"))] };
    case "list":
      return listToDoc(block, str(block, "style"));
    default:
      return { type: "vaultBlock", attrs: { block } };
  }
}

/** The editor document for a content field's blocks. */
export function blocksToDoc(blocks: readonly Block[]): JSONContent {
  const content = blocks.map(blockToDoc);
  return { type: "doc", content: content.length ? content : [{ type: "paragraph" }] };
}

/** The markup text of a textblock. */
function textOf(node: JSONContent): string {
  return serializeInline((node.content ?? []) as Parameters<typeof serializeInline>[0]);
}

/** The text of a container's textblocks, one line each, skipping nested lists. */
function linesOf(node: JSONContent): string {
  const lines: string[] = [];
  for (const child of node.content ?? []) {
    if (LIST_STYLE[child.type ?? ""]) continue;
    if (child.type === "paragraph" || child.type === "heading") lines.push(textOf(child));
    else lines.push(linesOf(child));
  }
  return lines.join("\n");
}

function itemsOf(list: JSONContent, style: string): Block[] {
  return (list.content ?? []).map((item) => {
    const fields: Block["fields"] = { text: linesOf(item) };
    if (style === "todo") fields.checked = item.attrs?.checked === true;
    const nested = (item.content ?? []).filter((c) => LIST_STYLE[c.type ?? ""]).flatMap((c) => itemsOf(c, style));
    return nested.length ? { kind: "item", fields, blocks: nested } : { kind: "item", fields };
  });
}

function docToBlock(node: JSONContent): Block | null {
  switch (node.type) {
    case "heading":
      return { kind: "heading", fields: { level: Number(node.attrs?.level ?? 1), text: textOf(node) } };
    case "paragraph":
      return { kind: "paragraph", fields: { text: textOf(node) } };
    case "blockquote":
      return { kind: "quote", fields: { text: linesOf(node) } };
    case "bulletList":
    case "orderedList":
    case "taskList": {
      const style = LIST_STYLE[node.type] ?? "bullet";
      return { kind: "list", fields: { style }, blocks: itemsOf(node, style) };
    }
    case "vaultBlock":
      return (node.attrs?.block as Block | null) ?? null;
    default:
      return null;
  }
}

/**
 * A content field's blocks from the editor document. Empty paragraphs at the end are dropped:
 * they are the editor's resting place for the caret, not content.
 */
export function docToBlocks(doc: JSONContent): Block[] {
  const blocks = (doc.content ?? []).map(docToBlock).filter((b): b is Block => b !== null);
  for (let last = blocks.at(-1); last?.kind === "paragraph" && last.fields?.text === ""; last = blocks.at(-1)) {
    blocks.pop();
  }
  return blocks;
}
