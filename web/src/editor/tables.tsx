// Tables in the block editor (docs/vault-format.md, "table").
//
// A `table` node holds `tableRow`s of `tableHeader` or `tableCell` nodes, each a line of inline
// content edited like a paragraph's, through prosemirror-tables: cell selections, and pasting
// cells into cells. The first row is the header row when its cells are header cells; a header
// cell anywhere else becomes a plain one. Each cell carries its column's alignment.
//
// Keys, in a cell:
// - Tab and Shift+Tab move to the next and previous cell; Tab in the last cell adds a row;
// - Enter moves to the cell below, and out of the table after its last row; Shift+Enter is a line
//   break inside the cell.
//
// The clipboard: a copied table, or a range of its cells, is a GFM Markdown table as text and a
// `<table>` as HTML. Pasted text holding a Markdown table makes a table; an HTML table (from a
// spreadsheet or a web page) makes one too, through the cells' parse rules.

import { type Editor, Extension, type JSONContent, Node, type NodeViewRenderer } from "@tiptap/core";
import { Fragment, type Node as PMNode, type ResolvedPos, type Schema, Slice } from "@tiptap/pm/model";
import { type Command, type EditorState, Plugin, PluginKey, Selection, TextSelection, type Transaction } from "@tiptap/pm/state";
import {
  TableMap,
  addColumnAfter,
  addColumnBefore,
  addRowAfter,
  addRowBefore,
  deleteColumn,
  deleteRow,
  goToNextCell,
  isInTable,
  selectedRect,
  tableEditing,
} from "@tiptap/pm/tables";
import { Decoration, DecorationSet, type EditorView } from "@tiptap/pm/view";
import { For, createSignal } from "solid-js";
import { copyText } from "../clipboard";
import { type Align, TABLE_ALIGNS, type TableData, isAlign, markdownPieces, newTable, tableToMarkdown } from "../blocks/table";
import { TypeIcon } from "../components/Icon";
import { parseInline, serializeInline } from "./markup";
import { mountSolid } from "./solidNodeView";
import { noteTitle } from "./titles";

/** prosemirror-tables finds the table's node types by these roles. */
const ROLES: Record<string, string> = { table: "table", tableRow: "row", tableCell: "cell", tableHeader: "header_cell" };

/** The containers a table may not sit in, as the vault's lists and quotes hold only text. */
const NO_TABLES = new Set(["listItem", "taskItem", "blockquote"]);
const LISTS = new Set(["bulletList", "orderedList", "taskList"]);

// ---------------------------------------------------------------------------------------------
// Between the editor's nodes and the table's cells

/** The alignment a pasted cell's element shows, from its style or its `align` attribute. */
function alignOfDom(el: HTMLElement): Align {
  const a = (el.style.textAlign || el.getAttribute("align") || "").toLowerCase();
  return a === "center" ? "center" : a === "right" || a === "end" ? "right" : "left";
}

/** The editor node for table `t`, its cells' text parsed as inline markup. */
export function tableNode(schema: Schema, t: TableData): PMNode {
  const { table, tableRow, tableCell, tableHeader } = schema.nodes;
  return table!.create(
    null,
    t.rows.map((row, r) =>
      tableRow!.create(
        null,
        row.map((cell, c) => {
          const nodes = parseInline(cell).map((n) => schema.nodeFromJSON(n));
          return (t.header && r === 0 ? tableHeader! : tableCell!).create({ align: t.align[c] ?? "left" }, nodes);
        }),
      ),
    ),
  );
}

/** Table `node`'s cells as markup, square, its header row and alignments: what it would save as. */
export function tableData(node: PMNode): TableData {
  const map = TableMap.get(node);
  const rows: string[][] = [];
  const align: Align[] = [];
  for (let r = 0; r < map.height; r++) {
    const row: string[] = [];
    for (let c = 0; c < map.width; c++) {
      const at = map.map[r * map.width + c]!;
      const cell = node.nodeAt(at)!;
      // A cell spanning several is its text in its first place and empty in the others.
      const first = map.findCell(at);
      row.push(first.left === c && first.top === r ? serializeInline(cell.content.toJSON() ?? []) : "");
      if (r === 0) align.push(isAlign(cell.attrs.align) ? cell.attrs.align : "left");
    }
    rows.push(row);
  }
  let header = node.childCount > 0 && node.child(0).childCount > 0;
  node.firstChild?.forEach((cell) => {
    if (cell.type.name !== "tableHeader") header = false;
  });
  return { header, align, rows };
}

/** The table holding `$pos`, with its position, or null. */
function tableAround($pos: ResolvedPos): { pos: number; node: PMNode } | null {
  for (let d = $pos.depth; d > 0; d--) {
    if ($pos.node(d).type.name === "table") return { pos: $pos.before(d), node: $pos.node(d) };
  }
  return null;
}

// ---------------------------------------------------------------------------------------------
// Commands

/**
 * The caret back into the table at `pos` for its tools to act on: at `last`, where it last was in
 * this table, or in the first cell. Focus going to the tools can move the page's selection.
 */
function intoTable(view: EditorView, pos: number, last: number | null) {
  const node = view.state.doc.nodeAt(pos);
  const at = last !== null && node && last > pos && last < pos + node.nodeSize ? last : null;
  if (at === null && isInTable(view.state) && tableAround(view.state.selection.$from)?.pos === pos) return;
  if (at !== null && view.state.selection.from === at && isInTable(view.state)) return;
  view.dispatch(view.state.tr.setSelection(TextSelection.near(view.state.doc.resolve(at ?? pos + 3))));
}

/** Set the alignment of the selected cells' columns, in every row. */
export function setColumnAlign(align: Align): Command {
  return (state, dispatch) => {
    if (!isInTable(state)) return false;
    const rect = selectedRect(state);
    if (dispatch) {
      const tr = state.tr;
      const done = new Set<number>();
      for (let r = 0; r < rect.map.height; r++) {
        for (let c = rect.left; c < rect.right; c++) {
          const at = rect.map.map[r * rect.map.width + c]!;
          if (done.has(at)) continue;
          done.add(at);
          const cell = rect.table.nodeAt(at)!;
          tr.setNodeMarkup(rect.tableStart + at, null, { ...cell.attrs, align });
        }
      }
      dispatch(tr);
    }
    return true;
  };
}

/** The alignment of the column holding the selection's head. */
function columnAlign(state: EditorState): Align {
  if (!isInTable(state)) return "left";
  const rect = selectedRect(state);
  const cell = rect.table.nodeAt(rect.map.map[rect.left]!);
  return isAlign(cell?.attrs.align) ? cell.attrs.align : "left";
}

/** Make the first row of the table holding the selection the header row, or a plain row. */
export function setHeaderRow(on: boolean): Command {
  return (state, dispatch) => {
    const t = tableAround(state.selection.$from);
    if (!t) return false;
    if (dispatch) {
      const { tableCell, tableHeader } = state.schema.nodes;
      const tr = state.tr;
      const row = t.node.firstChild!;
      let at = t.pos + 2;
      row.forEach((cell) => {
        tr.setNodeMarkup(at, on ? tableHeader : tableCell, cell.attrs);
        at += cell.nodeSize;
      });
      dispatch(tr);
    }
    return true;
  };
}

/** Whether the table holding the selection has a header row. */
function hasHeader(state: EditorState): boolean {
  const t = tableAround(state.selection.$from);
  return !!t && t.node.firstChild?.firstChild?.type.name === "tableHeader";
}

/**
 * A row above the selection. Above the header row, the new row becomes the header and the old
 * one a plain row, so the header stays the first row.
 */
export const addRowAbove: Command = (state, dispatch) => {
  if (!isInTable(state)) return false;
  const top = selectedRect(state).top === 0 && hasHeader(state);
  if (!dispatch) return true;
  let next: Transaction | null = null;
  addRowBefore(state, (tr) => {
    next = tr;
  });
  if (!next) return false;
  const tr = next as Transaction;
  if (top) {
    const t = tableAround(tr.selection.$from)!;
    const { tableCell, tableHeader } = state.schema.nodes;
    let at = t.pos + 1;
    for (let r = 0; r < 2; r++) {
      const row = t.node.child(r);
      let cellAt = at + 1;
      row.forEach((cell) => {
        tr.setNodeMarkup(cellAt, r === 0 ? tableHeader : tableCell, cell.attrs);
        cellAt += cell.nodeSize;
      });
      at += row.nodeSize;
    }
  }
  dispatch(tr);
  return true;
};

/** Delete the table holding the selection, leaving the caret where it was. */
export const deleteTableHere: Command = (state, dispatch) => {
  const t = tableAround(state.selection.$from);
  if (!t) return false;
  if (dispatch) {
    const tr = state.tr.delete(t.pos, t.pos + t.node.nodeSize);
    if (tr.doc.content.size === 0) tr.insert(0, state.schema.nodes.paragraph!.create());
    tr.setSelection(Selection.near(tr.doc.resolve(Math.min(t.pos, tr.doc.content.size))));
    dispatch(tr.scrollIntoView());
  }
  return true;
};

/** Tab in a cell: the next cell, or in the last cell a new row and its first cell. */
const nextCellOrRow: Command = (state, dispatch) => {
  if (!isInTable(state)) return false;
  if (goToNextCell(1)(state, dispatch)) return true;
  if (!dispatch) return true;
  // A new row after the last, its first cell for the caret.
  const row = selectedRect(state).map.height;
  addRowAfter(state, (tr) => {
    const t = tableAround(tr.selection.$from)!;
    let at = t.pos + 1;
    for (let r = 0; r < row; r++) at += t.node.child(r).nodeSize;
    tr.setSelection(TextSelection.create(tr.doc, at + 2));
    dispatch(tr.scrollIntoView());
  });
  return true;
};

/** Enter in a cell: the cell below in the same column, or after the last row, out of the table. */
const cellBelow: Command = (state, dispatch) => {
  if (!isInTable(state)) return false;
  if (!dispatch) return true;
  const rect = selectedRect(state);
  const t = tableAround(state.selection.$from)!;
  if (rect.bottom < rect.map.height) {
    const at = rect.map.map[rect.bottom * rect.map.width + rect.left]!;
    const cell = rect.table.nodeAt(at)!;
    dispatch(state.tr.setSelection(TextSelection.create(state.doc, rect.tableStart + at + cell.nodeSize - 1)).scrollIntoView());
    return true;
  }
  // Out of the table, onto the line after it, which is made when there is none to type on.
  const after = t.pos + t.node.nodeSize;
  const next = state.doc.resolve(after).nodeAfter;
  const tr = state.tr;
  if (next?.type.name !== "paragraph") tr.insert(after, state.schema.nodes.paragraph!.create());
  tr.setSelection(TextSelection.create(tr.doc, after + 1));
  dispatch(tr.scrollIntoView());
  return true;
};

// ---------------------------------------------------------------------------------------------
// The clipboard

/** A leaf node's text: a mention's label or its note's title, a placeholder's title. */
function leafText(leaf: PMNode): string {
  if (leaf.type.name === "mention") return String(leaf.attrs.label ?? noteTitle(String(leaf.attrs.id)) ?? `[[${leaf.attrs.id}]]`);
  if (leaf.type.name === "placeholderLink") return String(leaf.attrs.title);
  if (leaf.type.name === "hardBreak") return "\n";
  return "";
}

/**
 * The text a copied slice holding a table or a divider is: each table as Markdown, a divider as
 * `---`, other blocks as text.
 */
export function sliceText(slice: Slice): string | undefined {
  let tables = false;
  const parts: string[] = [];
  // A range of cells copies as the rows holding them, which make one table.
  let rows: PMNode[] = [];
  const endRows = () => {
    if (!rows.length) return;
    parts.push(tableToMarkdown(tableData(rows[0]!.type.schema.nodes.table!.create(null, rows)), noteTitle));
    rows = [];
  };
  slice.content.forEach((node) => {
    if (node.type.name === "tableRow") {
      tables = true;
      rows.push(node);
      return;
    }
    endRows();
    if (node.type.name === "table") {
      tables = true;
      parts.push(tableToMarkdown(tableData(node), noteTitle));
    } else if (node.type.name === "horizontalRule") {
      // A divider copies as a Markdown thematic break.
      tables = true;
      parts.push("---");
    } else {
      parts.push(node.isBlock ? node.textBetween(0, node.content.size, "\n\n", leafText) : node.isText ? (node.text ?? "") : leafText(node));
    }
  });
  endRows();
  return tables ? parts.join("\n\n") : undefined;
}

/**
 * Pasted HTML with each table cell's paragraphs and other blocks as lines of the cell, since a
 * cell holds one line of inline content: a spreadsheet or a word processor wraps cells' text in
 * `<p>` and `<div>`.
 */
export function flattenCells(html: string): string {
  if (!/<t[dh][\s>]/i.test(html)) return html;
  const doc = new DOMParser().parseFromString(html, "text/html");
  for (const cell of doc.querySelectorAll("td, th")) {
    const blocks = [...cell.querySelectorAll("p, div, h1, h2, h3, h4, h5, h6, li, blockquote, pre, ul, ol, table")].reverse();
    for (const b of blocks) {
      const lines = doc.createDocumentFragment();
      lines.append(...b.childNodes);
      if (b.nextSibling && b.textContent?.trim()) lines.append(doc.createElement("br"));
      b.replaceWith(lines);
    }
  }
  return doc.body.innerHTML;
}

/**
 * The editor slice for pasted text holding Markdown tables or thematic breaks, the lines around
 * them as paragraphs.
 */
export function markdownSlice(schema: Schema, text: string): Slice | null {
  const pieces = markdownPieces(text);
  if (!pieces) return null;
  const nodes: PMNode[] = [];
  for (const p of pieces) {
    if (p.kind === "table") nodes.push(tableNode(schema, p.table));
    else if (p.kind === "divider") nodes.push(schema.nodes.horizontalRule!.create());
    else for (const line of p.lines) if (line.trim()) nodes.push(schema.nodes.paragraph!.create(null, schema.text(line)));
  }
  const open = (n: PMNode | undefined) => (n?.type.name === "paragraph" ? 1 : 0);
  return new Slice(Fragment.from(nodes), open(nodes[0]), open(nodes.at(-1)));
}

// ---------------------------------------------------------------------------------------------
// Keeping tables in shape

/**
 * A transaction that puts the document's tables right, or null when they are: header cells only
 * in the first row, that row all header or all plain cells, and no table inside a list or quote,
 * where it moves to just after the outermost one.
 */
export function tidyTablesTr(state: EditorState): Transaction | null {
  const tr = state.tr;
  const { tableCell, tableHeader } = state.schema.nodes;
  let move: { pos: number; node: PMNode; to: number } | null = null;
  state.doc.descendants((node, pos) => {
    if (move) return false;
    if (node.type.name !== "table") return true;
    const $pos = state.doc.resolve(pos);
    const held = Array.from({ length: $pos.depth }, (_, i) => $pos.node(i + 1).type.name);
    if (held.some((n) => NO_TABLES.has(n))) {
      // After the outermost list or quote that holds it.
      const outer = held.findIndex((n) => NO_TABLES.has(n) || LISTS.has(n)) + 1;
      move = { pos, node, to: $pos.after(outer) };
      return false;
    }
    node.forEach((row, rowOffset, r) => {
      const header = r === 0 && row.firstChild?.type === tableHeader;
      let cellAt = pos + 1 + rowOffset + 1;
      row.forEach((cell) => {
        const want = header ? tableHeader! : tableCell!;
        if (cell.type !== want) tr.setNodeMarkup(cellAt, want, cell.attrs);
        cellAt += cell.nodeSize;
      });
    });
    return false;
  });
  const m = move as { pos: number; node: PMNode; to: number } | null;
  if (m) tr.insert(m.to, m.node).delete(m.pos, m.pos + m.node.nodeSize);
  return tr.docChanged ? tr : null;
}

// ---------------------------------------------------------------------------------------------
// The table's node view and its tools

const ALIGN_LABEL: Record<Align, string> = { left: "Left", center: "Center", right: "Right" };

function tableView(editor: Editor): NodeViewRenderer {
  return ({ node: initial, view, getPos }) => {
    let node = initial;
    const dom = document.createElement("div");
    dom.className = "table-block";
    dom.setAttribute("data-testid", "table-block");
    const tools = document.createElement("div");
    tools.className = "table-tools";
    tools.contentEditable = "false";
    tools.setAttribute("role", "toolbar");
    tools.setAttribute("aria-label", "Table");
    const scroll = document.createElement("div");
    scroll.className = "table-scroll";
    const table = document.createElement("table");
    table.className = "content-table";
    const body = document.createElement("tbody");
    table.append(body);
    scroll.append(table);
    dom.append(tools, scroll);

    // Re-read the editor's state on every transaction, for the selected column's alignment, and
    // keep where the selection last was in this table, for the tools to act there.
    const [tick, setTick] = createSignal(0);
    let last: number | null = null;
    const onTransaction = ({ transaction }: { transaction: Transaction }) => {
      const pos = getPos();
      const sel = view.state.selection;
      if (typeof pos === "number" && isInTable(view.state) && tableAround(sel.$from)?.pos === pos) last = sel.from;
      else if (last !== null) last = transaction.mapping.map(last);
      setTick(tick() + 1);
    };
    editor.on("transaction", onTransaction);

    const run = (cmd: Command) => () => {
      const pos = getPos();
      if (typeof pos !== "number" || !editor.isEditable) return;
      // Take in a selection the page made a moment ago, such as the click just before.
      (view as unknown as { domObserver?: { flush?: () => void } }).domObserver?.flush?.();
      intoTable(view, pos, last);
      cmd(view.state, view.dispatch, view);
      view.focus();
    };
    const mine = () => {
      tick();
      const pos = getPos();
      return typeof pos === "number" && isInTable(view.state) && tableAround(view.state.selection.$from)?.pos === pos;
    };
    const lastAlign = () => {
      tick();
      if (last === null) return "left";
      const $at = view.state.doc.resolve(Math.min(last, view.state.doc.content.size));
      for (let d = $at.depth; d > 0; d--) {
        const cell = $at.node(d);
        if (cell.type.spec.tableRole === "cell" || cell.type.spec.tableRole === "header_cell") {
          // The column's alignment is the first row's cell's in it.
          const t = tableAround($at);
          if (!t) break;
          const map = TableMap.get(t.node);
          const col = map.colCount($at.before(d) - t.pos - 1);
          const top = t.node.nodeAt(map.map[col]!);
          return isAlign(top?.attrs.align) ? top.attrs.align : "left";
        }
      }
      return "left";
    };
    const header = () => {
      tick();
      return node.firstChild?.firstChild?.type.name === "tableHeader";
    };
    const align = () => (mine() ? columnAlign(view.state) : lastAlign());
    const [copied, setCopied] = createSignal(false);
    const copy = async () => {
      if (await copyText(tableToMarkdown(tableData(node), noteTitle))) {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      }
    };
    const button = (label: string, icon: string, onClick: () => void, pressed?: () => boolean) => (
      <button
        type="button"
        aria-label={label}
        title={label}
        aria-pressed={pressed ? pressed() : undefined}
        onMouseDown={(e) => e.preventDefault()}
        onClick={onClick}
      >
        <TypeIcon name={icon} size={16} />
      </button>
    );
    const dispose = mountSolid(editor, tools, () => (
      <>
        {button("Add row above", "between-horizontal-start", run(addRowAbove))}
        {button("Add row below", "between-horizontal-end", run(addRowAfter))}
        {button("Add column left", "between-vertical-start", run(addColumnBefore))}
        {button("Add column right", "between-vertical-end", run(addColumnAfter))}
        {button("Delete row", "rows-3", run(deleteRow))}
        {button("Delete column", "columns-3", run(deleteColumn))}
        {button("Header row", "panel-top", () => run(setHeaderRow(!header()))(), header)}
        <select
          aria-label="Column alignment"
          title="Column alignment"
          value={align()}
          onChange={(e) => run(setColumnAlign(e.currentTarget.value as Align))()}
        >
          <For each={TABLE_ALIGNS}>{(a) => <option value={a}>{ALIGN_LABEL[a]}</option>}</For>
        </select>
        <button type="button" class="table-copy" title="Copy as Markdown" onMouseDown={(e) => e.preventDefault()} onClick={() => void copy()}>
          {copied() ? "Copied" : "Copy as Markdown"}
        </button>
        {button("Delete table", "trash-2", run(deleteTableHere))}
      </>
    ));

    return {
      dom,
      contentDOM: body,
      update(next) {
        if (next.type !== node.type) return false;
        node = next;
        setTick(tick() + 1);
        return true;
      },
      stopEvent(e) {
        return e.target instanceof globalThis.Node && tools.contains(e.target);
      },
      ignoreMutation(m) {
        if (m.type === "selection") return false;
        return !body.contains(m.target) || (m.target === body && m.type === "attributes");
      },
      destroy() {
        editor.off("transaction", onTransaction);
        dispose();
      },
    };
  };
}

// ---------------------------------------------------------------------------------------------
// Nodes

function cellAttributes() {
  return {
    colspan: {
      default: 1,
      parseHTML: (el: HTMLElement) => Number(el.getAttribute("colspan")) || 1,
      renderHTML: (a: Record<string, unknown>) => (Number(a.colspan) > 1 ? { colspan: String(a.colspan) } : {}),
    },
    rowspan: {
      default: 1,
      parseHTML: (el: HTMLElement) => Number(el.getAttribute("rowspan")) || 1,
      renderHTML: (a: Record<string, unknown>) => (Number(a.rowspan) > 1 ? { rowspan: String(a.rowspan) } : {}),
    },
    colwidth: { default: null, rendered: false },
    align: {
      default: "left",
      parseHTML: alignOfDom,
      renderHTML: (a: Record<string, unknown>) => (a.align === "center" || a.align === "right" ? { style: `text-align: ${String(a.align)}` } : {}),
    },
  };
}

export const TableCell = Node.create({
  name: "tableCell",
  content: "inline*",
  isolating: true,
  addAttributes: cellAttributes,
  parseHTML() {
    return [{ tag: "td" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["td", HTMLAttributes, 0];
  },
});

export const TableHeader = Node.create({
  name: "tableHeader",
  content: "inline*",
  isolating: true,
  addAttributes: cellAttributes,
  parseHTML() {
    return [{ tag: "th" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["th", HTMLAttributes, 0];
  },
});

export const TableRow = Node.create({
  name: "tableRow",
  content: "(tableCell | tableHeader)+",
  parseHTML() {
    return [{ tag: "tr" }];
  },
  renderHTML() {
    return ["tr", 0];
  },
});

export const Table = Node.create({
  name: "table",
  group: "block",
  content: "tableRow+",
  isolating: true,
  draggable: true,
  parseHTML() {
    return [{ tag: "table" }];
  },
  renderHTML() {
    return ["table", ["tbody", 0]];
  },
  addNodeView() {
    return tableView(this.editor);
  },
});

/** The editor node for a new table: three rows of three empty cells, the first the header row. */
export function newTableJSON(rows = 3, cols = 3): JSONContent {
  const t = newTable(rows, cols);
  return {
    type: "table",
    content: t.rows.map((row, r) => ({
      type: "tableRow",
      content: row.map(() => ({ type: r === 0 ? "tableHeader" : "tableCell", attrs: { align: "left" } })),
    })),
  };
}

/** Table editing: roles, keys, the clipboard, the table holding the selection, and tidying. */
export const TableKeys = Extension.create({
  name: "tableKeys",
  // Ahead of the toggles' Tab and the default Enter, and of TipTap's own clipboard text.
  priority: 1000,
  extendNodeSchema(extension) {
    const role = ROLES[extension.name];
    return role ? { tableRole: role } : {};
  },
  addKeyboardShortcuts() {
    const run = (cmd: Command) => () => {
      const { view } = this.editor;
      if (!isInTable(view.state)) return false;
      cmd(view.state, view.dispatch, view);
      // In a table, these keys never fall through to the blocks' own.
      return true;
    };
    return {
      Tab: run(nextCellOrRow),
      "Shift-Tab": run(goToNextCell(-1)),
      Enter: run(cellBelow),
      // TipTap's line break keeps out of isolating nodes, which cells are.
      "Shift-Enter": run((state, dispatch) => {
        dispatch?.(state.tr.replaceSelectionWith(state.schema.nodes.hardBreak!.create()).scrollIntoView());
        return true;
      }),
    };
  },
  addProseMirrorPlugins() {
    let repasting = false;
    return [
      tableEditing({ allowTableNodeSelection: true }),
      new Plugin({
        key: new PluginKey("tables"),
        appendTransaction: (trs, _old, state) => (trs.some((t) => t.docChanged) ? tidyTablesTr(state) : null),
        props: {
          clipboardTextSerializer: (slice) => sliceText(slice) ?? "",
          clipboardTextParser: (text, _context, _plain, view) => markdownSlice(view.state.schema, text) ?? (undefined as unknown as Slice),
          transformPastedHTML: (html) => flattenCells(html),
          handlePaste(view, event) {
            // The paste below comes back through here.
            if (repasting) return false;
            const data = event.clipboardData;
            const html = data?.getData("text/html") ?? "";
            const text = data?.getData("text/plain") ?? "";
            // HTML with a table of its own pastes as that table; other HTML, such as a code
            // editor's, loses to the Markdown table in its text, and HTML with no `<hr>` loses
            // to a Markdown thematic break in its text.
            if (!html || /<table[\s>]/i.test(html) || !view.editable) return false;
            const pieces = markdownPieces(text);
            const wins = pieces?.some((p) => p.kind === "table" || (p.kind === "divider" && !/<hr[\s/>]/i.test(html)));
            if (!wins) return false;
            repasting = true;
            try {
              view.pasteText(text, event);
            } finally {
              repasting = false;
            }
            return true;
          },
          decorations(state) {
            // The table holding the selection shows its tools where there is no hover.
            const t = tableAround(state.selection.$from);
            return t ? DecorationSet.create(state.doc, [Decoration.node(t.pos, t.pos + t.node.nodeSize, { class: "table-active" })]) : null;
          },
        },
      }),
    ];
  },
});
