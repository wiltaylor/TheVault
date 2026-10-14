// Notion-style block tools for the content editor: a menu that opens on a trigger typed in the
// text (`/` for blocks, `{{` for field references in a template), a drag handle in the left
// gutter of every top-level block, and Alt+ArrowUp / Alt+ArrowDown to move the current block.

import { Extension } from "@tiptap/core";
import type { Node as PMNode, Slice } from "@tiptap/pm/model";
import { NodeSelection, Plugin, PluginKey, type EditorState, TextSelection, type Transaction } from "@tiptap/pm/state";
import { Decoration, DecorationSet, type EditorView } from "@tiptap/pm/view";

// ---------------------------------------------------------------------------------------------
// Trigger matching. Pure, so it is unit tested.

export interface TriggerMatch {
  /** Characters before the caret the trigger and its query take up. */
  length: number;
  query: string;
}

/**
 * `/` typed at the start of an otherwise empty paragraph, or after a space, followed by the
 * query typed so far. `before` and `after` are the paragraph's text either side of the caret.
 */
export function matchSlash(before: string, after: string): TriggerMatch | null {
  const m = /(^|\s)\/([^\s/]*)$/.exec(before);
  if (!m) return null;
  const atStart = m.index === 0 && m[1] === "";
  if (atStart && after !== "") return null;
  return { length: m[2]!.length + 1, query: m[2]! };
}

/** `{{` followed by the start of a field name. */
export function matchFieldRef(before: string): TriggerMatch | null {
  const m = /\{\{([A-Za-z0-9_]*)$/.exec(before);
  return m ? { length: m[0].length, query: m[1]! } : null;
}

/** Whether `text` matches a menu filter: every word of the query starts a word of the text. */
export function matchesQuery(text: string, query: string): boolean {
  const words = text.toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
  return query
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter(Boolean)
    .every((q) => words.some((w) => w.startsWith(q)));
}

// ---------------------------------------------------------------------------------------------
// Suggestion menus

export interface SuggestState {
  from: number;
  to: number;
  query: string;
}

export interface SuggestOptions {
  key: string;
  /** Only paragraphs at the top level or directly in a column open the menu when true. */
  topLevelOnly: boolean;
  match: (before: string, after: string) => TriggerMatch | null;
  /** The menu's state after every change: where its text is and what it holds, or null. */
  onChange: (state: SuggestState | null) => void;
  /** A key pressed while the editor has focus. True when the menu took it. */
  onKey: (event: KeyboardEvent) => boolean;
}

/** The trigger before the caret, when the selection is a caret in a paragraph. */
export function suggestAt(state: EditorState, options: Pick<SuggestOptions, "match" | "topLevelOnly">): SuggestState | null {
  const sel = state.selection;
  if (!sel.empty) return null;
  const $from = sel.$from;
  if ($from.parent.type.name !== "paragraph") return null;
  if (options.topLevelOnly && !isContainer($from.node($from.depth - 1))) return null;
  const before = $from.parent.textBetween(0, $from.parentOffset, undefined, "￼");
  const after = $from.parent.textBetween($from.parentOffset, $from.parent.content.size, undefined, "￼");
  const m = options.match(before, after);
  if (!m) return null;
  return { from: $from.pos - m.length, to: $from.pos, query: m.query };
}

export function suggestion(options: SuggestOptions) {
  return Extension.create({
    name: options.key,
    // Ahead of the keymaps, so Enter and the arrows reach an open menu first.
    priority: 1000,
    addProseMirrorPlugins() {
      return [
        new Plugin({
          key: new PluginKey(options.key),
          view() {
            return {
              update(view) {
                options.onChange(view.hasFocus() ? suggestAt(view.state, options) : null);
              },
              destroy() {
                options.onChange(null);
              },
            };
          },
          props: {
            handleKeyDown(_view, event) {
              return options.onKey(event);
            },
            handleDOMEvents: {
              blur(view) {
                // A click on the menu keeps focus (its items stop mousedown), so blur means gone.
                options.onChange(null);
                void view;
                return false;
              },
              focus(view) {
                options.onChange(suggestAt(view.state, options));
                return false;
              },
            },
          },
        }),
      ];
    },
  });
}

// ---------------------------------------------------------------------------------------------
// Moving blocks

/** Whether `node` holds a sequence of blocks the block tools work in: the document or a column. */
export function isContainer(node: PMNode | null | undefined): boolean {
  return node?.type.name === "doc" || node?.type.name === "column";
}

/** A block of a container: the document or a column. */
export interface ContainerBlock {
  pos: number;
  node: PMNode;
  index: number;
  /** The container, and the position its content starts at. */
  parent: PMNode;
  start: number;
}

/**
 * The block of a container holding the selection: a top-level block, or a block in a column.
 * A selected block node is that block.
 */
export function blockOfSelection(state: EditorState): ContainerBlock | null {
  const sel = state.selection;
  if (sel instanceof NodeSelection && isContainer(sel.$from.parent)) {
    const $p = sel.$from;
    return { pos: sel.from, node: sel.node, index: $p.index(), parent: $p.parent, start: $p.start() };
  }
  const $f = sel.$from;
  for (let d = $f.depth; d > 0; d--) {
    const parent = $f.node(d - 1);
    if (isContainer(parent)) return { pos: $f.before(d), node: $f.node(d), index: $f.index(d - 1), parent, start: $f.start(d - 1) };
  }
  return null;
}

/** The position before child `index` of `parent`, whose content starts at `start`. */
function childPos(parent: PMNode, start: number, index: number): number {
  let pos = start;
  for (let i = 0; i < index; i++) pos += parent.child(i).nodeSize;
  return pos;
}

/**
 * A transaction moving block `index` of a container one place up (`by` -1) or down (+1),
 * keeping the selection on it, or null when it is already at that end. The container is the
 * document unless `within` names a column and where its content starts.
 */
export function moveBlockTr(
  state: EditorState,
  index: number,
  by: -1 | 1,
  within?: { parent: PMNode; start: number },
): Transaction | null {
  const parent = within?.parent ?? state.doc;
  const base = within?.start ?? 0;
  const target = index + by;
  if (target < 0 || target >= parent.childCount) return null;
  const first = Math.min(index, target);
  const a = parent.child(first);
  const b = parent.child(first + 1);
  const start = childPos(parent, base, first);
  const moved = parent.child(index);
  const oldStart = childPos(parent, base, index);
  const tr = state.tr.replaceWith(start, start + a.nodeSize + b.nodeSize, [b, a]);
  const newStart = by < 0 ? start : start + b.nodeSize;
  const sel = state.selection;
  if (sel instanceof NodeSelection && sel.from === oldStart) {
    tr.setSelection(NodeSelection.create(tr.doc, newStart));
  } else {
    const shift = newStart - oldStart;
    const clamp = (p: number) => Math.max(newStart + 1, Math.min(newStart + moved.nodeSize - 1, p + shift));
    tr.setSelection(TextSelection.create(tr.doc, clamp(sel.from), clamp(sel.to)));
  }
  return tr.scrollIntoView();
}

/**
 * Alt+ArrowUp and Alt+ArrowDown move the block holding the caret, within its column when it is
 * in one. Before any key binding runs,
 * the caret is read from the page: a click or a native caret move just before a key press may
 * not have reached the editor's state yet, and a binding would act at the old caret.
 */
export const MoveBlock = Extension.create({
  name: "moveBlock",
  priority: 1001,
  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: new PluginKey("readCaretFirst"),
        props: {
          handleKeyDown(view) {
            (view as unknown as { domObserver?: { flush?: () => void } }).domObserver?.flush?.();
            return false;
          },
        },
      }),
    ];
  },
  addKeyboardShortcuts() {
    // The block moves within its container: a block in a column stays in that column.
    const move = (by: -1 | 1) => () => {
      const { state, view } = this.editor;
      const at = blockOfSelection(state);
      const tr = at && moveBlockTr(state, at.index, by, at);
      if (!tr) return true;
      view.dispatch(tr);
      return true;
    };
    return { "Alt-ArrowUp": move(-1), "Alt-ArrowDown": move(1) };
  },
});

// ---------------------------------------------------------------------------------------------
// Focus without losing a selection in flight

/** The parts of ProseMirror's view this module reaches past its public API for. */
interface ViewInternals {
  focused: boolean;
  input: { lastFocus: number };
  domObserver: { flush(): void; stop(): void; start(): void };
}

/**
 * ProseMirror's own focus handler, without its one race. On focus, ProseMirror waits 20 ms and
 * then writes its selection back into the page whenever the page's selection differs from the
 * one it last read. A selection the keyboard changed in the meantime (Shift+Arrow straight after
 * a click) differs only because the browser has not yet reported it, so that write threw the
 * change away: under load the timer runs late, and a selection lost its last extension. Here the
 * timer reads the page's selection first, so ProseMirror takes the change instead of undoing it.
 *
 * Its priority is the lowest, so every other focus handler (TipTap's focus events) runs first;
 * returning true then stands in for the built-in handler.
 */
export const SteadyFocus = Extension.create({
  name: "steadyFocus",
  priority: 1,
  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: new PluginKey("steadyFocus"),
        props: {
          handleDOMEvents: {
            focus(view) {
              const v = view as unknown as ViewInternals;
              v.input.lastFocus = Date.now();
              if (!v.focused) {
                v.domObserver.stop();
                view.dom.classList.add("ProseMirror-focused");
                v.domObserver.start();
                v.focused = true;
                setTimeout(() => {
                  if (view.isDestroyed || !view.hasFocus()) return;
                  v.domObserver.flush();
                }, 20);
              }
              return true;
            },
          },
        },
      }),
    ];
  },
});

// ---------------------------------------------------------------------------------------------
// Drag handles

export interface DraggedBlock {
  /** The block's position when the drag started. */
  pos: number;
  node: PMNode;
}

export interface BlockHandleOptions {
  /** Told when a block's drag starts, and with null when it ends. */
  onDrag?: (block: DraggedBlock | null) => void;
  /** The "+" beside block `node` at `pos` was pressed: add a block after it. */
  onAdd?: (pos: number, node: PMNode) => void;
}

/** Touch screens and phones have no hover: there the gutter follows the block being edited. */
function noHover(): boolean {
  return typeof window !== "undefined" && !!window.matchMedia?.("(max-width: 600px), (pointer: coarse)").matches;
}

/** A block the gutter can sit beside: its position, node and DOM, and whether it is in a column. */
export interface BlockHit {
  pos: number;
  node: PMNode;
  dom: HTMLElement;
  nested: boolean;
}

const near = (r: DOMRect, y: number) => y >= r.top - 4 && y <= r.bottom + 4;

/**
 * The block of column layout `layout` at `pos` under the pointer: the column under `x` (or, with
 * the columns stacked, under `y`), then its block under `y`. Null over the layout's own space,
 * between and below the blocks, where the layout itself is the block.
 */
function columnBlockAt(view: EditorView, layout: PMNode, pos: number, x: number, y: number): BlockHit | null {
  const cols: { pos: number; node: PMNode; rect: DOMRect }[] = [];
  layout.forEach((node, offset) => {
    const dom = view.nodeDOM(pos + 1 + offset);
    if (dom instanceof HTMLElement) cols.push({ pos: pos + 1 + offset, node, rect: dom.getBoundingClientRect() });
  });
  if (!cols.length) return null;
  // Left of the first column is its gutter; right of the last is still the last.
  const cx = Math.min(Math.max(x, cols[0]!.rect.left), Math.max(...cols.map((c) => c.rect.right)) - 1);
  const col = cols.find((c) => cx >= c.rect.left && cx < c.rect.right && near(c.rect, y));
  if (!col) return null;
  let at = col.pos + 1;
  for (let i = 0; i < col.node.childCount; i++) {
    const node = col.node.child(i);
    const dom = view.nodeDOM(at);
    if (dom instanceof HTMLElement && near(dom.getBoundingClientRect(), y)) return { pos: at, node, dom, nested: true };
    at += node.nodeSize;
  }
  return null;
}

/** The block under the pointer, with its DOM and position: a top-level block, or one in a column. */
export function blockAt(view: EditorView, x: number, y: number): BlockHit | null {
  const { doc } = view.state;
  let pos = 0;
  for (let i = 0; i < doc.childCount; i++) {
    const node = doc.child(i);
    const dom = view.nodeDOM(pos);
    if (dom instanceof HTMLElement && near(dom.getBoundingClientRect(), y)) {
      if (node.type.name === "columns") return columnBlockAt(view, node, pos, x, y) ?? { pos, node, dom, nested: false };
      return { pos, node, dom, nested: false };
    }
    pos += node.nodeSize;
  }
  return null;
}

/** The block holding the selection, at the top level or in a column. */
function blockAtSelection(view: EditorView): BlockHit | null {
  const at = blockOfSelection(view.state);
  if (!at) return null;
  const dom = view.nodeDOM(at.pos);
  return dom instanceof HTMLElement ? { pos: at.pos, node: at.node, dom, nested: at.parent.type.name === "column" } : null;
}

/**
 * A gutter beside the block under the pointer, at the top level or in a column: "+" adds a block
 * after it, through the same menu as `/`, and the "⋮⋮" handle drags it, into, out of and between
 * columns too. ProseMirror drops a dragged block where the
 * drop cursor shows, as it does for any dragged selection. Without hover (touch, phones) the
 * gutter sits beside the block holding the caret.
 */
export function blockHandle(options: BlockHandleOptions = {}) {
  return Extension.create({
    name: "blockHandle",
    addProseMirrorPlugins() {
      const editor = this.editor;
      return [
        new Plugin({
          key: new PluginKey("blockHandle"),
          view(view) {
            const host = view.dom.parentElement;
            const gutter = document.createElement("div");
            gutter.className = "block-gutter";
            gutter.hidden = true;
            const add = document.createElement("button");
            add.type = "button";
            add.className = "block-add";
            add.textContent = "+";
            add.title = "Add a block below";
            add.setAttribute("aria-label", "Add block");
            add.setAttribute("data-testid", "block-add");
            const handle = document.createElement("div");
            handle.className = "block-handle";
            handle.draggable = true;
            handle.textContent = "⋮⋮";
            handle.title = "Drag to move; Alt+↑ and Alt+↓ move the current block";
            handle.setAttribute("data-testid", "block-handle");
            handle.setAttribute("aria-hidden", "true");
            gutter.append(add, handle);
            host?.appendChild(gutter);
            let current: BlockHit | null = null;

            function place(hit: BlockHit | null) {
              if (!editor.isEditable || !host || !hit) return;
              current = hit;
              const hr = host.getBoundingClientRect();
              const br = hit.dom.getBoundingClientRect();
              // Inside a scaled box, such as a card on a zoomed canvas, screen pixels are not CSS pixels.
              const scale = host.offsetHeight > 0 ? hr.height / host.offsetHeight || 1 : 1;
              // Beside the first line of the block.
              const line = Math.min(br.height, 28 * scale);
              gutter.style.top = `${(br.top - hr.top + line / 2) / scale - 12}px`;
              gutter.hidden = false;
              // A block in a column has its gutter just left of it, in the space before the column.
              gutter.style.left = hit.nested ? `${(br.left - hr.left) / scale - gutter.offsetWidth - 2}px` : "";
            }

            const onMove = (e: MouseEvent) => {
              if (gutter.contains(e.target as Node)) return;
              place(blockAt(view, e.clientX, e.clientY));
            };
            const onLeave = (e: MouseEvent) => {
              if (noHover()) return;
              if (e.relatedTarget instanceof Node && host?.contains(e.relatedTarget)) return;
              gutter.hidden = true;
            };
            host?.addEventListener("mousemove", onMove);
            host?.addEventListener("mouseleave", onLeave);

            // The editor keeps focus: pressing "+" must not take it.
            add.addEventListener("mousedown", (e) => e.preventDefault());
            add.addEventListener("click", () => {
              if (!current) return;
              const node = view.state.doc.nodeAt(current.pos);
              if (node) options.onAdd?.(current.pos, node);
            });

            handle.addEventListener("dragstart", (e) => {
              if (!current || !e.dataTransfer) return;
              // Re-read the block: the document may have changed since the handle moved.
              const node = view.state.doc.nodeAt(current.pos);
              if (!node) return;
              const sel = NodeSelection.create(view.state.doc, current.pos);
              view.dispatch(view.state.tr.setSelection(sel));
              const slice: Slice = sel.content();
              const { dom, text } = view.serializeForClipboard(slice);
              e.dataTransfer.clearData();
              e.dataTransfer.setData("text/html", dom.innerHTML);
              e.dataTransfer.setData("text/plain", text);
              e.dataTransfer.effectAllowed = "copyMove";
              e.dataTransfer.setDragImage(current.dom, 0, 0);
              // As ProseMirror's own drags do: the dropped copy replaces this node, wherever the selection is.
              (view as { dragging: unknown }).dragging = { slice, move: true, node: sel };
              options.onDrag?.({ pos: current.pos, node });
            });
            handle.addEventListener("dragend", () => {
              (view as { dragging: unknown }).dragging = null;
              options.onDrag?.(null);
            });

            return {
              update(v) {
                if (noHover() && v.hasFocus()) {
                  place(blockAtSelection(v));
                } else if (current && !current.dom.isConnected) {
                  gutter.hidden = true;
                  current = null;
                }
              },
              destroy() {
                host?.removeEventListener("mousemove", onMove);
                host?.removeEventListener("mouseleave", onLeave);
                gutter.remove();
              },
            };
          },
        }),
      ];
    },
  });
}

// ---------------------------------------------------------------------------------------------
// Dropping a block beside another

/** Where a block dropped at a point goes beside another block, making or widening a layout. */
export interface EdgeDrop {
  target: BlockHit;
  side: "left" | "right";
}

/** How far into a block from its right edge, and out from its left, a drop lands beside it. */
const EDGE = 48;

/**
 * The block a drop at `x`, `y` lands beside, and on which side: over a block's last 48 pixels
 * (a fifth of a narrow one) on the right, or in the space left of it. A layout itself takes no
 * drop beside it, nor does a layout that already has six columns.
 */
export function edgeDropAt(view: EditorView, x: number, y: number, dragged: PMNode | null, maxColumns = 6): EdgeDrop | null {
  if (!dragged || dragged.type.name === "columns") return null;
  const target = blockAt(view, x, y);
  if (!target || target.node.type.name === "columns") return null;
  const r = target.dom.getBoundingClientRect();
  if (y < r.top || y > r.bottom || r.width === 0) return null;
  let side: EdgeDrop["side"] | null = null;
  if (x > r.right - Math.min(EDGE, r.width / 5) && x <= r.right + 8) side = "right";
  else if (x < r.left && x >= r.left - EDGE) side = "left";
  if (!side) return null;
  if (target.nested) {
    const layout = view.state.doc.resolve(target.pos).node(1);
    if (layout.childCount >= maxColumns) return null;
  }
  return { target, side };
}

/**
 * The transaction for a block dropped beside another: the dragged block `from` (a node at
 * `from.pos`) leaves where it was, and beside a top-level block the two become a layout of two
 * equal columns; beside a block in a column, it gets a new column there, which takes half of
 * that column's width.
 */
export function edgeDropTr(state: EditorState, from: { pos: number; node: PMNode }, drop: { pos: number; side: "left" | "right" }): Transaction | null {
  const { schema } = state;
  const target = state.doc.nodeAt(drop.pos);
  if (!target || drop.pos === from.pos) return null;
  // Not into itself.
  if (drop.pos > from.pos && drop.pos < from.pos + from.node.nodeSize) return null;
  const tr = state.tr.delete(from.pos, from.pos + from.node.nodeSize);
  const tpos = tr.mapping.map(drop.pos);
  const $t = tr.doc.resolve(tpos);
  const moved = from.node;
  if ($t.parent.type.name === "column") {
    const col = $t.parent;
    const colPos = $t.before();
    const half = Math.max(0.01, Math.round((Number(col.attrs.width) || 1) * 50) / 100);
    tr.setNodeAttribute(colPos, "width", half);
    const at = drop.side === "right" ? colPos + col.nodeSize : colPos;
    tr.insert(at, schema.nodes.column!.create({ width: half }, moved));
    tr.setSelection(NodeSelection.create(tr.doc, at + 1));
    return tr;
  }
  if ($t.parent.type.name !== "doc") return null;
  const now = tr.doc.nodeAt(tpos)!;
  const pair = drop.side === "right" ? [now, moved] : [moved, now];
  const layout = schema.nodes.columns!.create(
    null,
    pair.map((n) => schema.nodes.column!.create({ width: 1 }, n)),
  );
  tr.replaceWith(tpos, tpos + now.nodeSize, layout);
  // The moved block, selected where it landed.
  const movedAt = drop.side === "right" ? tpos + 1 + (now.nodeSize + 2) + 1 : tpos + 2;
  tr.setSelection(NodeSelection.create(tr.doc, movedAt));
  return tr;
}

/**
 * Drops beside a block, Notion's way: a block dragged by its handle onto another block's right
 * edge, or just left of it, sits beside it in columns. A bar shows where it will go, in place of
 * the drop cursor.
 */
export const EdgeDropColumns = Extension.create({
  name: "edgeDropColumns",
  addProseMirrorPlugins() {
    let bar: HTMLDivElement | null = null;
    const hide = () => {
      bar?.remove();
      bar = null;
      document.body.classList.remove("columns-edge-drop");
    };
    const dragged = (view: EditorView): { pos: number; node: PMNode } | null => {
      const d = (view as unknown as { dragging: { node?: NodeSelection } | null }).dragging;
      return d?.node instanceof NodeSelection ? { pos: d.node.from, node: d.node.node } : null;
    };
    return [
      new Plugin({
        key: new PluginKey("edgeDropColumns"),
        props: {
          handleDOMEvents: {
            dragover(view, e) {
              const drop = edgeDropAt(view, e.clientX, e.clientY, dragged(view)?.node ?? null);
              const host = view.dom.parentElement;
              if (!drop || !host) {
                hide();
                return false;
              }
              const hr = host.getBoundingClientRect();
              const scale = host.offsetHeight > 0 ? hr.height / host.offsetHeight || 1 : 1;
              const r = drop.target.dom.getBoundingClientRect();
              if (!bar) {
                bar = document.createElement("div");
                bar.className = "columns-drop-bar";
                bar.setAttribute("data-testid", "columns-drop-bar");
                host.appendChild(bar);
              }
              const x = drop.side === "right" ? r.right : r.left - 6;
              bar.style.left = `${(x - hr.left) / scale}px`;
              bar.style.top = `${(r.top - hr.top) / scale}px`;
              bar.style.height = `${r.height / scale}px`;
              document.body.classList.add("columns-edge-drop");
              return false;
            },
            dragleave(view, e) {
              if (!(e.relatedTarget instanceof Node && view.dom.contains(e.relatedTarget))) hide();
              return false;
            },
            dragend() {
              hide();
              return false;
            },
          },
          handleDrop(view, event) {
            hide();
            const from = dragged(view);
            const drop = edgeDropAt(view, event.clientX, event.clientY, from?.node ?? null);
            if (!from || !drop) return false;
            const tr = edgeDropTr(view.state, from, { pos: drop.target.pos, side: drop.side });
            if (!tr) return false;
            view.dispatch(tr.scrollIntoView());
            view.focus();
            return true;
          },
        },
      }),
    ];
  },
});

// ---------------------------------------------------------------------------------------------
// Placeholder

/**
 * A hint in an empty paragraph: in the paragraph holding the caret, and in an empty document's
 * only line, so an empty field still shows where to type.
 */
export function placeholder(text: string) {
  return Extension.create({
    name: "blockPlaceholder",
    addProseMirrorPlugins() {
      return [
        new Plugin({
          key: new PluginKey("blockPlaceholder"),
          props: {
            decorations(state) {
              const { doc, selection } = state;
              const marks: Decoration[] = [];
              const lone = doc.childCount === 1;
              // Paragraphs at the top level and in columns: a column's empty line shows it too.
              doc.descendants((node, pos, parent) => {
                if (node.type.name === "columns" || node.type.name === "column") return true;
                if (node.type.name !== "paragraph" || node.content.size > 0) return false;
                const here = selection.empty && selection.from === pos + 1;
                if ((lone && parent === doc) || here) {
                  marks.push(Decoration.node(pos, pos + node.nodeSize, { class: "is-empty", "data-placeholder": text }));
                }
                return false;
              });
              return marks.length ? DecorationSet.create(doc, marks) : null;
            },
          },
        }),
      ];
    },
  });
}
