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
  /** Only top-level paragraphs open the menu when true. */
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
  if (options.topLevelOnly && $from.depth !== 1) return null;
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

/** The index of the top-level block holding the selection. */
function topIndex(state: EditorState): number {
  return state.selection.$from.index(0);
}

/** The position before top-level block `index`. */
function topPos(doc: PMNode, index: number): number {
  let pos = 0;
  for (let i = 0; i < index; i++) pos += doc.child(i).nodeSize;
  return pos;
}

/**
 * A transaction moving top-level block `index` one place up (`by` -1) or down (+1), keeping the
 * selection on it, or null when it is already at that end.
 */
export function moveBlockTr(state: EditorState, index: number, by: -1 | 1): Transaction | null {
  const { doc } = state;
  const target = index + by;
  if (target < 0 || target >= doc.childCount) return null;
  const first = Math.min(index, target);
  const a = doc.child(first);
  const b = doc.child(first + 1);
  const start = topPos(doc, first);
  const moved = doc.child(index);
  const oldStart = topPos(doc, index);
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
 * Alt+ArrowUp and Alt+ArrowDown move the block holding the caret. Before any key binding runs,
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
    const move = (by: -1 | 1) => () => {
      const { state, view } = this.editor;
      const tr = moveBlockTr(state, topIndex(state), by);
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

/** The top-level block under the pointer's height, with its DOM and position. */
function blockAtY(view: EditorView, y: number): { pos: number; node: PMNode; dom: HTMLElement } | null {
  const { doc } = view.state;
  let pos = 0;
  for (let i = 0; i < doc.childCount; i++) {
    const node = doc.child(i);
    const dom = view.nodeDOM(pos);
    if (dom instanceof HTMLElement) {
      const r = dom.getBoundingClientRect();
      if (y >= r.top - 4 && y <= r.bottom + 4) return { pos, node, dom };
    }
    pos += node.nodeSize;
  }
  return null;
}

/** The top-level block holding the selection. */
function blockAtSelection(view: EditorView): { pos: number; node: PMNode; dom: HTMLElement } | null {
  const { doc, selection } = view.state;
  const index = selection.$from.index(0);
  if (index >= doc.childCount) return null;
  const pos = topPos(doc, index);
  const dom = view.nodeDOM(pos);
  return dom instanceof HTMLElement ? { pos, node: doc.child(index), dom } : null;
}

/**
 * A gutter beside the top-level block under the pointer: "+" adds a block after it, through the
 * same menu as `/`, and the "⋮⋮" handle drags it. ProseMirror drops a dragged block where the
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
            let current: { pos: number; node: PMNode; dom: HTMLElement } | null = null;

            function place(hit: { pos: number; node: PMNode; dom: HTMLElement } | null) {
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
            }

            const onMove = (e: MouseEvent) => {
              if (gutter.contains(e.target as Node)) return;
              place(blockAtY(view, e.clientY));
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
              doc.forEach((node, pos) => {
                if (node.type.name !== "paragraph" || node.content.size > 0) return;
                const here = selection.empty && selection.from === pos + 1;
                if (lone || here) {
                  marks.push(Decoration.node(pos, pos + node.nodeSize, { class: "is-empty", "data-placeholder": text }));
                }
              });
              return marks.length ? DecorationSet.create(doc, marks) : null;
            },
          },
        }),
      ];
    },
  });
}
