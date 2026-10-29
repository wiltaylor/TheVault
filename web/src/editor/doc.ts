// A content field's blocks (docs/vault-format.md, "Content blocks", D34) to and from the editor's
// ProseMirror JSON. Headings, paragraphs, lists and quotes become editable nodes with their text
// parsed as inline markup (D41). A `columns` layout becomes `columns` and `column` nodes holding
// the same editable blocks, and a `toggle` or an `admonition` becomes a node holding its heading
// line (the summary, the title) and then its editable blocks. A `table` becomes `table` and
// `tableRow` nodes of `tableHeader` and `tableCell` nodes, each cell's text parsed as inline
// markup. A `divider` becomes a `horizontalRule`. Every other block kind rides along untouched in
// a `vaultBlock`.

import type { JSONContent } from "@tiptap/core";
import type { Block, TemplateItem } from "../api";
import { columnWidth, columnsOf, slotOfBlock } from "../blocks/columns";
import { type Align, isAlign, tableBlock, tableOf } from "../blocks/table";
import { parseInline, serializeInline } from "./markup";

const LIST_NODE: Record<string, string> = { bullet: "bulletList", numbered: "orderedList", todo: "taskList" };
const LIST_STYLE: Record<string, string> = { bulletList: "bullet", orderedList: "numbered", taskList: "todo" };

function str(block: Block, item: string): string {
  const v = block.fields?.[item];
  return typeof v === "string" ? v : "";
}

/** True while converting a template's blocks, whose text may hold `{{name}}` field references. */
let fieldRefs = false;

function inline(text: string): JSONContent[] | undefined {
  const nodes = parseInline(text, { fieldRefs }) as JSONContent[];
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
    case "divider":
      return { type: "horizontalRule" };
    case "list":
      return listToDoc(block, str(block, "style"));
    case "columns":
      return {
        type: "columns",
        content: columnsOf(block).map((col) => {
          const content = (col.blocks ?? []).map(blockToDoc);
          return { type: "column", attrs: { width: columnWidth(col) }, content: content.length ? content : [{ type: "paragraph" }] };
        }),
      };
    case "toggle": {
      const level = Math.min(3, Math.max(0, Math.trunc(Number(block.fields?.level ?? 0)) || 0));
      const summary = inline(str(block, "text"));
      return {
        type: "toggle",
        content: [
          summary ? { type: "toggleSummary", attrs: { level }, content: summary } : { type: "toggleSummary", attrs: { level } },
          ...(block.blocks ?? []).map(blockToDoc),
        ],
      };
    }
    case "admonition": {
      const title = inline(str(block, "title"));
      const inner = (block.blocks ?? []).map(blockToDoc);
      return {
        type: "admonition",
        attrs: {
          kind: str(block, "kind") || "note",
          colour: str(block, "colour") || null,
          icon: str(block, "icon") || null,
          collapsible: block.fields?.collapsible === true,
        },
        content: [
          title ? { type: "admonitionTitle", content: title } : { type: "admonitionTitle" },
          // An admonition in the editor always has a line to type on.
          ...(inner.length ? inner : [{ type: "paragraph" }]),
        ],
      };
    }
    case "table": {
      const t = tableOf(block);
      return {
        type: "table",
        content: t.rows.map((row, r) => ({
          type: "tableRow",
          content: row.map((cell, c) => {
            const type = t.header && r === 0 ? "tableHeader" : "tableCell";
            const content = inline(cell);
            const attrs = { align: t.align[c] ?? "left" };
            return content ? { type, attrs, content } : { type, attrs };
          }),
        })),
      };
    }
    case "slot":
      // A property block inside a template's column.
      if (fieldRefs) {
        const label = block.fields?.label;
        return { type: "templateSlot", attrs: { slot: slotOfBlock(block) ?? "", label: typeof label === "string" && label ? label : null } };
      }
      return { type: "vaultBlock", attrs: { block } };
    default:
      return { type: "vaultBlock", attrs: { block } };
  }
}

/** Blocks without the empty paragraphs at their end, which are only room for the caret. */
function trimmed(blocks: Block[]): Block[] {
  const out = [...blocks];
  for (let last = out.at(-1); last?.kind === "paragraph" && last.fields?.text === ""; last = out.at(-1)) out.pop();
  return out;
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

/** The blocks nested in a toggle or an admonition, without the empty lines at their end. */
function nestedBlocks(nodes: JSONContent[]): Block[] {
  return trimmed(nodes.map(docToBlock).filter((b): b is Block => b !== null));
}

function docToBlock(node: JSONContent): Block | null {
  switch (node.type) {
    case "heading":
      return { kind: "heading", fields: { level: Number(node.attrs?.level ?? 1), text: textOf(node) } };
    case "paragraph":
      return { kind: "paragraph", fields: { text: textOf(node) } };
    case "blockquote":
      return { kind: "quote", fields: { text: linesOf(node) } };
    case "horizontalRule":
      return { kind: "divider" };
    case "bulletList":
    case "orderedList":
    case "taskList": {
      const style = LIST_STYLE[node.type] ?? "bullet";
      return { kind: "list", fields: { style }, blocks: itemsOf(node, style) };
    }
    case "columns":
      return {
        kind: "columns",
        blocks: (node.content ?? [])
          .filter((c) => c.type === "column")
          .map((col) => {
            const blocks = trimmed((col.content ?? []).map(docToBlock).filter((b): b is Block => b !== null));
            const out: Block = { kind: "column", fields: { width: Number(col.attrs?.width ?? 1) || 1 } };
            if (blocks.length) out.blocks = blocks;
            return out;
          }),
      };
    case "toggle": {
      const [summary, ...rest] = node.content ?? [];
      const level = Number(summary?.attrs?.level ?? 0) || 0;
      const fields: Block["fields"] = level ? { level, text: summary ? textOf(summary) : "" } : { text: summary ? textOf(summary) : "" };
      const blocks = nestedBlocks(rest);
      return blocks.length ? { kind: "toggle", fields, blocks } : { kind: "toggle", fields };
    }
    case "admonition": {
      const [title, ...rest] = node.content ?? [];
      const fields: Block["fields"] = { kind: String(node.attrs?.kind ?? "note") };
      const text = title ? textOf(title) : "";
      if (text) fields.title = text;
      if (node.attrs?.colour) fields.colour = String(node.attrs.colour);
      if (node.attrs?.icon) fields.icon = String(node.attrs.icon);
      if (node.attrs?.collapsible === true) fields.collapsible = true;
      const blocks = nestedBlocks(rest);
      return blocks.length ? { kind: "admonition", fields, blocks } : { kind: "admonition", fields };
    }
    case "table": {
      // A cell spanning several columns (pasted HTML) is its text, then empty cells.
      const span = (cell: JSONContent) => Math.max(1, Number(cell.attrs?.colspan ?? 1) || 1);
      const rows = (node.content ?? []).map((row) =>
        (row.content ?? []).flatMap((cell) => [textOf(cell), ...Array<string>(span(cell) - 1).fill("")]),
      );
      const first = node.content?.[0]?.content ?? [];
      const align = first.flatMap((cell): Align[] => Array<Align>(span(cell)).fill(isAlign(cell.attrs?.align) ? cell.attrs.align : "left"));
      return tableBlock({ header: first.length > 0 && first.every((c) => c.type === "tableHeader"), align, rows });
    }
    case "templateSlot": {
      // A property block inside a template's column: `slot "<field>"`, with its label if any.
      const label = node.attrs?.label;
      const out: Block = { kind: "slot", labels: [String(node.attrs?.slot ?? "")] };
      if (typeof label === "string" && label) out.fields = { label };
      return out;
    }
    case "vaultBlock": {
      const block = (node.attrs?.block as Block | null) ?? null;
      // A card whose note is not chosen yet is not content, as an empty last line is not.
      return block?.kind === "card" && !block.fields?.note ? null : block;
    }
    default:
      return null;
  }
}

/**
 * A content field's blocks from the editor document. Empty paragraphs at the end are dropped:
 * they are the editor's resting place for the caret, not content.
 */
export function docToBlocks(doc: JSONContent): Block[] {
  return trimmed((doc.content ?? []).map(docToBlock).filter((b): b is Block => b !== null));
}

/**
 * The editor document for a template tab: its fixed blocks as editable blocks, with `{{name}}`
 * as field references, and each slot as a property block.
 */
export function itemsToDoc(items: readonly TemplateItem[]): JSONContent {
  fieldRefs = true;
  try {
    const content = items.map((item): JSONContent =>
      "slot" in item ? { type: "templateSlot", attrs: { slot: item.slot, label: item.label } } : blockToDoc(item.block),
    );
    return { type: "doc", content: content.length ? content : [{ type: "paragraph" }] };
  } finally {
    fieldRefs = false;
  }
}

/** A template tab's items from the editor document. Empty paragraphs at the end are dropped. */
export function docToItems(doc: JSONContent): TemplateItem[] {
  const items: TemplateItem[] = [];
  for (const node of doc.content ?? []) {
    if (node.type === "templateSlot") {
      const label = node.attrs?.label;
      items.push({ slot: String(node.attrs?.slot ?? ""), label: typeof label === "string" && label ? label : null });
      continue;
    }
    const block = docToBlock(node);
    if (block) items.push({ block });
  }
  for (let last = items.at(-1); last && "block" in last && last.block.kind === "paragraph" && last.block.fields?.text === ""; last = items.at(-1)) {
    items.pop();
  }
  return items;
}
