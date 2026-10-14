// Column layouts in the block editor (docs/vault-format.md, "columns"). A `columns` node holds two
// to six `column` nodes, and each column holds ordinary blocks, edited as the top level is. The
// rules that keep a layout well formed run after every change:
//
// - a column left with no blocks goes, and the layout closes up;
// - a layout left with one column unwraps into that column's blocks;
// - a layout never sits inside a column: the schema only allows one at the top of the document.
//
// The node views add what the document does not hold: the gap between two columns, which drags
// (or takes Left and Right when focused) to resize them, and a small toolbar to add a column,
// remove one and make the widths equal again.

import { type Editor, Extension, Node, type NodeViewRenderer } from "@tiptap/core";
import type { Node as PMNode } from "@tiptap/pm/model";
import { Slice } from "@tiptap/pm/model";
import { type EditorState, Plugin, PluginKey, Selection, TextSelection, type Transaction } from "@tiptap/pm/state";
import { ReplaceAroundStep } from "@tiptap/pm/transform";
import { Decoration, DecorationSet } from "@tiptap/pm/view";
import { COLUMNS_MAX, addedWidth, nudgePair, roundWidth, splitPair } from "../blocks/columns";

/** The document: blocks, and layouts at the top level only. */
export const VaultDoc = Node.create({
  name: "doc",
  topNode: true,
  content: "(block|layout)+",
});

const widthOf = (n: PMNode): number => {
  const w = Number(n.attrs.width);
  return Number.isFinite(w) && w > 0 ? w : 1;
};

// ---------------------------------------------------------------------------------------------
// Transactions. Each takes the position of a `columns` node.

/** The `columns` node at `pos`, or null. */
function columnsAt(state: EditorState, pos: number): PMNode | null {
  const node = state.doc.nodeAt(pos);
  return node?.type.name === "columns" ? node : null;
}

/** The position of column `index` of the layout at `pos`. */
export function columnPos(layout: PMNode, pos: number, index: number): number {
  let at = pos + 1;
  for (let i = 0; i < index; i++) at += layout.child(i).nodeSize;
  return at;
}

/** Set the widths of the columns `index - 1` and `index` of the layout at `pos`. */
export function setPairTr(state: EditorState, pos: number, index: number, widths: [number, number]): Transaction | null {
  const layout = columnsAt(state, pos);
  if (!layout || index < 1 || index >= layout.childCount) return null;
  const tr = state.tr;
  const left = columnPos(layout, pos, index - 1);
  const right = left + layout.child(index - 1).nodeSize;
  tr.setNodeAttribute(left, "width", roundWidth(widths[0]));
  tr.setNodeAttribute(right, "width", roundWidth(widths[1]));
  return tr;
}

/** Give every column of the layout at `pos` the same width. */
export function equalWidthsTr(state: EditorState, pos: number): Transaction | null {
  const layout = columnsAt(state, pos);
  if (!layout) return null;
  const tr = state.tr;
  layout.forEach((_col, offset) => tr.setNodeAttribute(pos + 1 + offset, "width", 1));
  return tr;
}

/** Add an empty column at the end of the layout at `pos`, with the caret in it. */
export function addColumnTr(state: EditorState, pos: number): Transaction | null {
  const layout = columnsAt(state, pos);
  if (!layout || layout.childCount >= COLUMNS_MAX) return null;
  const { schema } = state;
  const widths: number[] = [];
  layout.forEach((c) => widths.push(widthOf(c)));
  const col = schema.nodes.column!.create({ width: addedWidth(widths) }, schema.nodes.paragraph!.create());
  const at = pos + layout.nodeSize - 1;
  const tr = state.tr.insert(at, col);
  tr.setSelection(TextSelection.create(tr.doc, at + 2));
  return tr;
}

/**
 * Remove column `index` of the layout at `pos`. Its blocks are not lost: they move to the end of
 * the column before it, or to the start of the next one when it is the first. A layout left
 * with one column then unwraps (see `normalizeTr`).
 */
export function removeColumnTr(state: EditorState, pos: number, index: number): Transaction | null {
  const layout = columnsAt(state, pos);
  if (!layout || index < 0 || index >= layout.childCount) return null;
  const col = layout.child(index);
  const at = columnPos(layout, pos, index);
  const tr = state.tr;
  // Blocks worth keeping: everything but empty paragraphs.
  const kept: PMNode[] = [];
  col.forEach((b) => {
    if (!(b.type.name === "paragraph" && b.content.size === 0)) kept.push(b);
  });
  tr.delete(at, at + col.nodeSize);
  if (kept.length && layout.childCount > 1) {
    const mapped = tr.mapping.map(pos);
    const after = tr.doc.nodeAt(mapped)!;
    const target = index > 0 ? index - 1 : 0;
    const tpos = columnPos(after, mapped, target);
    const into = index > 0 ? tpos + after.child(target).nodeSize - 1 : tpos + 1;
    tr.insert(into, kept);
  }
  const near = Math.min(tr.mapping.map(at, -1), tr.doc.content.size);
  tr.setSelection(Selection.near(tr.doc.resolve(near), -1));
  return tr;
}

/**
 * The changes that make every layout well formed again, or null when all are: columns without
 * blocks go, a layout with one column left becomes its blocks, and one with none goes. Steps
 * keep the caret where it was, inside whatever is left.
 */
export function normalizeTr(state: EditorState): Transaction | null {
  const found: number[] = [];
  state.doc.forEach((node, offset) => {
    if (node.type.name === "columns") found.push(offset);
  });
  if (!found.length) return null;
  const tr = state.tr;
  // Last first, so the positions of the ones before stay as they are.
  for (const pos of found.reverse()) {
    let layout = tr.doc.nodeAt(pos)!;
    let kept = 0;
    layout.forEach((c) => {
      if (c.childCount > 0) kept++;
    });
    if (kept === 0) {
      // Nothing left in it: a line to type on takes its place.
      tr.replaceWith(pos, pos + layout.nodeSize, state.schema.nodes.paragraph!.create());
      continue;
    }
    for (let i = layout.childCount - 1; i >= 0; i--) {
      const col = layout.child(i);
      if (col.childCount > 0) continue;
      const at = columnPos(layout, pos, i);
      tr.delete(at, at + col.nodeSize);
      layout = tr.doc.nodeAt(pos)!;
    }
    if (layout.childCount === 1) {
      // Take the layout and its column away and keep the column's blocks where they are.
      const end = pos + layout.nodeSize;
      tr.step(new ReplaceAroundStep(pos, end, pos + 2, end - 2, Slice.empty, 0, true));
    }
  }
  return tr.docChanged ? tr : null;
}

// ---------------------------------------------------------------------------------------------
// Node views

/** A small icon of `n` columns, with a sign: `+`, `-` or `=`. */
function icon(sign: "+" | "-" | "="): SVGSVGElement {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("width", "15");
  svg.setAttribute("height", "15");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "2");
  svg.setAttribute("stroke-linecap", "round");
  const paths: Record<typeof sign, string[]> = {
    "+": ["M3 5h8v14H3z", "M17 8v8", "M13 12h8"],
    "-": ["M3 5h8v14H3z", "M13 12h8"],
    "=": ["M3 5h7v14H3z", "M14 5h7v14h-7z"],
  };
  for (const d of paths[sign]) {
    const p = document.createElementNS(ns, "path");
    p.setAttribute("d", d);
    svg.append(p);
  }
  return svg;
}

function button(label: string, sign: "+" | "-" | "=", testid: string): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "columns-tool";
  b.title = label;
  b.setAttribute("aria-label", label);
  b.setAttribute("data-testid", testid);
  b.append(icon(sign));
  // The editor keeps its selection: the focused column is the one these act on.
  b.addEventListener("mousedown", (e) => e.preventDefault());
  return b;
}

/** The index of the column holding the selection, when it is inside the layout at `pos`. */
function focusedColumn(state: EditorState, pos: number): number | null {
  const $from = state.selection.$from;
  if ($from.depth < 2 || $from.before(1) !== pos) return null;
  return $from.index(1);
}

function columnsView(editor: Editor): NodeViewRenderer {
  return ({ node: initial, view, getPos }) => {
    let node = initial;
    const dom = document.createElement("div");
    dom.className = "columns-block";
    dom.setAttribute("data-columns", "");
    dom.setAttribute("data-testid", "columns");
    const tools = document.createElement("div");
    tools.className = "columns-tools";
    tools.contentEditable = "false";
    tools.setAttribute("role", "toolbar");
    tools.setAttribute("aria-label", "Columns");
    const add = button("Add a column", "+", "columns-add");
    const remove = button("Remove column", "-", "columns-remove");
    const equal = button("Equal widths", "=", "columns-equal");
    tools.append(add, remove, equal);
    const row = document.createElement("div");
    row.className = "columns-row";
    dom.append(tools, row);

    const run = (make: (state: EditorState, pos: number) => Transaction | null) => {
      const pos = getPos();
      if (pos === undefined || !editor.isEditable) return;
      const tr = make(view.state, pos);
      if (tr) view.dispatch(tr.scrollIntoView());
      view.focus();
    };
    add.addEventListener("click", () => run(addColumnTr));
    remove.addEventListener("click", () =>
      run((state, pos) => removeColumnTr(state, pos, focusedColumn(state, pos) ?? node.childCount - 1)),
    );
    equal.addEventListener("click", () => run(equalWidthsTr));

    const refresh = () => {
      add.disabled = node.childCount >= COLUMNS_MAX;
      let same = true;
      node.forEach((c) => {
        if (widthOf(c) !== widthOf(node.firstChild!)) same = false;
      });
      equal.disabled = same;
    };
    refresh();

    return {
      dom,
      contentDOM: row,
      update(next) {
        if (next.type !== node.type) return false;
        node = next;
        refresh();
        return true;
      },
      stopEvent(e) {
        return e.target instanceof globalThis.Node && tools.contains(e.target);
      },
      ignoreMutation(m) {
        if (m.type === "selection") return false;
        return !row.contains(m.target) || (m.target === row && m.type === "attributes");
      },
    };
  };
}

function columnView(editor: Editor): NodeViewRenderer {
  return ({ node: initial, view, getPos }) => {
    let node = initial;
    const dom = document.createElement("div");
    dom.className = "column";
    dom.setAttribute("data-column", "");
    dom.setAttribute("data-testid", "column");
    const content = document.createElement("div");
    content.className = "column-content";
    // The gap before this column: it resizes this column and the one before it.
    const gap = document.createElement("div");
    gap.className = "column-resize";
    gap.contentEditable = "false";
    gap.tabIndex = 0;
    gap.setAttribute("role", "separator");
    gap.setAttribute("aria-orientation", "vertical");
    gap.setAttribute("aria-label", "Resize columns");
    gap.setAttribute("data-testid", "column-resize");
    gap.title = "Drag to resize the columns; Left and Right arrows when focused";
    dom.append(gap, content);

    const show = () => {
      dom.style.flexGrow = String(widthOf(node));
      dom.setAttribute("data-width", String(widthOf(node)));
    };
    show();

    /** This column's layout position and index, and the column before it. */
    const pair = () => {
      const pos = getPos();
      if (pos === undefined) return null;
      const $pos = view.state.doc.resolve(pos);
      const index = $pos.index();
      const layout = $pos.parent;
      if (layout.type.name !== "columns" || index < 1) return null;
      const layoutPos = $pos.before();
      const prevPos = pos - layout.child(index - 1).nodeSize;
      const prevDom = view.nodeDOM(prevPos);
      if (!(prevDom instanceof HTMLElement)) return null;
      return { layoutPos, index, prev: layout.child(index - 1), prevDom };
    };

    const label = (a: number, b: number) => {
      const pct = Math.round((a / (a + b)) * 100);
      gap.setAttribute("aria-valuenow", String(pct));
      gap.setAttribute("aria-valuetext", `${pct}% to the left column of the two`);
    };

    gap.addEventListener("keydown", (e) => {
      if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
      e.preventDefault();
      e.stopPropagation();
      const p = pair();
      if (!p || !editor.isEditable) return;
      const widths = nudgePair(widthOf(p.prev), widthOf(node), e.key === "ArrowLeft" ? -0.05 : 0.05);
      const tr = setPairTr(view.state, p.layoutPos, p.index, widths);
      if (tr) view.dispatch(tr);
      label(...widths);
    });

    gap.addEventListener("pointerdown", (e) => {
      if (e.button !== 0 || !editor.isEditable) return;
      const p = pair();
      if (!p) return;
      e.preventDefault();
      gap.setPointerCapture(e.pointerId);
      gap.focus({ preventScroll: true });
      const a = widthOf(p.prev);
      const b = widthOf(node);
      const left = (p.prevDom.querySelector(":scope > .column-content") ?? p.prevDom).getBoundingClientRect();
      const right = content.getBoundingClientRect();
      // The room the two columns' content shares, and the space between them.
      const room = left.width + right.width;
      const between = right.left - left.right;
      let widths: [number, number] = [a, b];
      dom.classList.add("resizing");
      const move = (ev: PointerEvent) => {
        widths = splitPair(a, b, (ev.clientX - left.left - between / 2) / room);
        p.prevDom.style.flexGrow = String(widths[0]);
        dom.style.flexGrow = String(widths[1]);
        label(...widths);
      };
      const up = () => {
        gap.removeEventListener("pointermove", move);
        gap.removeEventListener("pointerup", up);
        gap.removeEventListener("pointercancel", up);
        dom.classList.remove("resizing");
        const tr = setPairTr(view.state, p.layoutPos, p.index, widths);
        if (tr && (widths[0] !== a || widths[1] !== b)) view.dispatch(tr);
        else show();
      };
      gap.addEventListener("pointermove", move);
      gap.addEventListener("pointerup", up);
      gap.addEventListener("pointercancel", up);
    });

    return {
      dom,
      contentDOM: content,
      update(next) {
        if (next.type !== node.type) return false;
        node = next;
        show();
        return true;
      },
      stopEvent(e) {
        return e.target instanceof globalThis.Node && gap.contains(e.target);
      },
      ignoreMutation(m) {
        if (m.type === "selection") return false;
        return !content.contains(m.target) || (m.target === content && m.type === "attributes");
      },
    };
  };
}

// ---------------------------------------------------------------------------------------------
// Nodes

export const Columns = Node.create({
  name: "columns",
  group: "layout",
  content: "column+",
  isolating: true,
  selectable: true,
  draggable: true,
  parseHTML() {
    return [{ tag: "div[data-columns]" }];
  },
  renderHTML() {
    return ["div", { "data-columns": "" }, 0];
  },
  addNodeView() {
    return columnsView(this.editor);
  },
});

export const Column = Node.create({
  name: "column",
  content: "block*",
  isolating: true,
  addAttributes() {
    return {
      width: {
        default: 1,
        parseHTML: (el) => Number(el.getAttribute("data-width")) || 1,
        renderHTML: (attrs) => ({ "data-width": String(attrs.width) }),
      },
    };
  },
  parseHTML() {
    return [{ tag: "div[data-column]" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["div", { "data-column": "", ...HTMLAttributes }, 0];
  },
  addNodeView() {
    return columnView(this.editor);
  },
});

/**
 * The rules that keep layouts well formed, after every change; Backspace in the empty only
 * block of a column removes the column; and the layout holding the selection is marked, so its
 * toolbar shows where there is no hover.
 */
export const ColumnRules = Extension.create({
  name: "columnRules",
  // Ahead of the default keymap, whose Backspace would otherwise act first.
  priority: 900,
  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: new PluginKey("columnRules"),
        appendTransaction(trs, _old, state) {
          return trs.some((t) => t.docChanged) ? normalizeTr(state) : null;
        },
        props: {
          decorations(state) {
            const $from = state.selection.$from;
            if ($from.depth < 1) return null;
            const top = $from.node(1);
            if (top?.type.name !== "columns") return null;
            const pos = $from.before(1);
            return DecorationSet.create(state.doc, [Decoration.node(pos, pos + top.nodeSize, { class: "columns-active" })]);
          },
        },
      }),
    ];
  },
  addKeyboardShortcuts() {
    return {
      Backspace: () => {
        const { state, view } = this.editor;
        const sel = state.selection;
        if (!sel.empty) return false;
        const $from = sel.$from;
        const d = $from.depth;
        if (d < 3 || $from.parentOffset !== 0) return false;
        const col = $from.node(d - 1);
        if (col.type.name !== "column" || col.childCount !== 1) return false;
        if ($from.parent.type.name !== "paragraph" || $from.parent.content.size !== 0) return false;
        const at = $from.before(d - 1);
        // The caret goes to the end of the column before, or into the next when this was the first.
        const bias = $from.index(d - 2) > 0 ? -1 : 1;
        const tr = state.tr.delete(at, at + col.nodeSize);
        tr.setSelection(Selection.near(tr.doc.resolve(Math.min(at, tr.doc.content.size)), bias));
        view.dispatch(tr.scrollIntoView());
        return true;
      },
    };
  },
});
