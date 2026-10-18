// Toggles and admonitions in the block editor (docs/vault-format.md, "toggle", "admonition").
//
// A `toggle` node holds a `toggleSummary` (its summary line, plain text or a heading) and then
// any blocks; an `admonition` holds an `admonitionTitle` and then at least one block. Both open
// and close: a toggle always, an admonition when it is collapsible. Open or closed lives in the
// node view and the device's local storage (blocks/toggle.ts), never in the document, so opening
// one is not an edit.
//
// Keys:
// - `>> ` at the start of a paragraph makes it a toggle (`> ` is already the quote shortcut);
// - Tab moves the block holding the caret into the toggle or admonition just above it, at its
//   end, and Shift+Tab moves a block out of the toggle or admonition it is in, to just after it;
// - Enter at the end of a toggle's summary starts its first nested block when it is open, and
//   adds a line after it when it is closed;
// - Backspace at the start of a summary turns the toggle back into a paragraph, its nested
//   blocks staying where they are; at the start of an empty admonition title it unwraps the
//   admonition the same way.

import { type Editor, Extension, InputRule, Node, type NodeViewRenderer } from "@tiptap/core";
import type { Node as PMNode } from "@tiptap/pm/model";
import { type EditorState, NodeSelection, Plugin, PluginKey, Selection, TextSelection, type Transaction } from "@tiptap/pm/state";
import { Decoration, DecorationSet, type EditorView } from "@tiptap/pm/view";
import { For, createSignal } from "solid-js";
import { PALETTE } from "./markup";
import { ADMONITION_KINDS, PRESETS, admonitionLook, findTextFor, holdsFind, readOpen, writeOpen } from "../blocks/toggle";
import { TypeIcon } from "../components/Icon";
import { IconPicker } from "../components/IconPicker";
import { mountSolid } from "./solidNodeView";

/** The editor's scope for open state: the note and field it edits, or undefined. */
export type ToggleScope = () => string | undefined;

/** Node names of the blocks that hold a heading line and then blocks. */
export const FOLDING = new Set(["toggle", "admonition"]);
/** The heading lines of those blocks: never a block of their own. */
export const FOLD_HEADS = new Set(["toggleSummary", "admonitionTitle"]);

// ---------------------------------------------------------------------------------------------
// Where a block is: its path, as blocks/toggle.ts keys open state

/**
 * The path of the node at `pos`: the indexes from the top of the document down, as `2` or `3.0`.
 * Inside a toggle or an admonition the heading line is not counted, so the path matches the
 * content blocks of the note, where the summary is an item and not a block.
 */
export function blockPath(doc: PMNode, pos: number): string {
  const $pos = doc.resolve(pos);
  const parts: number[] = [];
  for (let d = 0; d <= $pos.depth; d++) {
    const parent = $pos.node(d);
    const index = $pos.index(d);
    parts.push(FOLDING.has(parent.type.name) ? index - 1 : index);
  }
  return parts.join(".");
}

/** All the text of the blocks nested in a toggle or admonition node, lower case. */
function nestedText(node: PMNode): string {
  const out: string[] = [];
  node.forEach((child, _o, i) => {
    if (i > 0) out.push(child.textContent);
  });
  return out.join("\n").toLowerCase();
}

// ---------------------------------------------------------------------------------------------
// Opening and closing

/** Ask the toggle or admonition whose DOM is `dom` to open, as the selection went inside it. */
const OPEN_EVENT = "vault-toggle-open";

function isOpen(view: EditorView, pos: number): boolean {
  const dom = view.nodeDOM(pos);
  return !(dom instanceof HTMLElement) || dom.dataset.open !== "false";
}

function openAt(view: EditorView, pos: number) {
  const dom = view.nodeDOM(pos);
  if (dom instanceof HTMLElement && dom.dataset.open === "false") dom.dispatchEvent(new CustomEvent(OPEN_EVENT));
}

/** A chevron that points right when closed and down when open (CSS turns it). */
function chevron(): SVGSVGElement {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("width", "16");
  svg.setAttribute("height", "16");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "2");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  const p = document.createElementNS(ns, "path");
  p.setAttribute("d", "m9 18 6-6-6-6");
  svg.append(p);
  return svg;
}

/**
 * The open and close button of a node view whose DOM is `dom`: it reads the device's state for
 * the block at its path, opens for a search hit inside it, and writes the state on each click.
 */
function foldControl(opts: {
  dom: HTMLElement;
  view: EditorView;
  getPos: () => number | undefined;
  scope: ToggleScope;
  node: () => PMNode;
  initial: boolean;
  label: string;
}): { button: HTMLButtonElement; set: (open: boolean, remember: boolean) => void; open: () => boolean } {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "toggle-button";
  button.contentEditable = "false";
  button.setAttribute("data-testid", "toggle-button");
  button.append(chevron());
  const path = () => {
    const pos = opts.getPos();
    return typeof pos === "number" ? blockPath(opts.view.state.doc, pos) : null;
  };
  const start = path();
  let open = start === null ? opts.initial : readOpen(opts.scope(), start, opts.initial);
  if (holdsFind(nestedText(opts.node()), findTextFor(opts.scope()))) open = true;
  const show = () => {
    opts.dom.dataset.open = open ? "true" : "false";
    button.setAttribute("aria-expanded", String(open));
    button.setAttribute("aria-label", `${open ? "Close" : "Open"} ${opts.label}`);
    button.title = open ? "Close" : "Open";
  };
  const set = (next: boolean, remember: boolean) => {
    open = next;
    const p = path();
    if (remember && p !== null) writeOpen(opts.scope(), p, open);
    show();
  };
  show();
  // The editor keeps its selection: pressing the button must not move the caret.
  button.addEventListener("mousedown", (e) => e.preventDefault());
  button.addEventListener("click", () => set(!open, true));
  opts.dom.addEventListener(OPEN_EVENT, () => set(true, true));
  return { button, set, open: () => open };
}

function toggleView(scope: ToggleScope): NodeViewRenderer {
  return ({ node: initial, view, getPos }) => {
    let node = initial;
    const dom = document.createElement("div");
    dom.className = "toggle-block";
    dom.setAttribute("data-toggle", "");
    dom.setAttribute("data-testid", "toggle");
    const content = document.createElement("div");
    content.className = "toggle-content";
    const fold = foldControl({ dom, view, getPos, scope, node: () => node, initial: false, label: "toggle" });
    dom.append(fold.button, content);
    return {
      dom,
      contentDOM: content,
      update(next) {
        if (next.type !== node.type) return false;
        node = next;
        return true;
      },
      stopEvent(e) {
        return e.target instanceof globalThis.Node && fold.button.contains(e.target);
      },
      ignoreMutation(m) {
        if (m.type === "selection") return false;
        return !content.contains(m.target) || (m.target === content && m.type === "attributes");
      },
    };
  };
}

function admonitionView(editor: Editor, scope: ToggleScope): NodeViewRenderer {
  return ({ node: initial, view, getPos }) => {
    let node = initial;
    const [attrs, setAttrs] = createSignal(initial.attrs);
    const dom = document.createElement("div");
    dom.className = "admonition";
    dom.setAttribute("data-admonition", "");
    dom.setAttribute("data-testid", "admonition");
    const head = document.createElement("div");
    head.className = "admonition-icon";
    head.contentEditable = "false";
    const tools = document.createElement("div");
    tools.className = "admonition-tools";
    tools.contentEditable = "false";
    tools.setAttribute("role", "toolbar");
    tools.setAttribute("aria-label", "Callout");
    const content = document.createElement("div");
    content.className = "admonition-content";
    const fold = foldControl({ dom, view, getPos, scope, node: () => node, initial: true, label: "callout" });
    dom.append(head, tools, content);

    const look = () => admonitionLook(attrs());
    const show = () => {
      const l = look();
      dom.dataset.kind = l.kind;
      dom.dataset.colour = l.colour;
      dom.dataset.icon = l.icon;
      dom.style.setProperty("--admonition", `var(--palette-${l.colour})`);
      dom.classList.toggle("collapsible", l.collapsible);
      if (l.collapsible) head.after(fold.button);
      else {
        fold.button.remove();
        if (!fold.open()) fold.set(true, false);
      }
    };
    show();

    const set = (change: Record<string, unknown>) => {
      const pos = getPos();
      if (typeof pos !== "number" || !editor.isEditable) return;
      view.dispatch(view.state.tr.setNodeMarkup(pos, undefined, { ...node.attrs, ...change }));
    };
    const disposeHead = mountSolid(editor, head, () => <TypeIcon name={look().icon} size={18} />);
    const id = `admonition-icon-${Math.random().toString(36).slice(2, 8)}`;
    const disposeTools = mountSolid(editor, tools, () => (
      <>
        <select aria-label="Callout kind" title="Kind" value={look().kind} onChange={(e) => set({ kind: e.currentTarget.value })}>
          <For each={ADMONITION_KINDS}>{(k) => <option value={k}>{PRESETS[k].label}</option>}</For>
        </select>
        <select
          aria-label="Callout colour"
          title="Colour"
          value={typeof attrs().colour === "string" ? attrs().colour : ""}
          onChange={(e) => set({ colour: e.currentTarget.value || null })}
        >
          <option value="">Kind's colour</option>
          <For each={PALETTE}>{(c) => <option value={c}>{c}</option>}</For>
        </select>
        {/* The icon shown, the kind's when the block names none; choosing the kind's own, or none, clears it. */}
        <IconPicker id={id} value={look().icon} onChange={(name) => set({ icon: name && name !== PRESETS[look().kind].icon ? name : null })} />
        <label class="admonition-collapsible">
          <input type="checkbox" checked={attrs().collapsible === true} onChange={(e) => set({ collapsible: e.currentTarget.checked })} /> Collapsible
        </label>
      </>
    ));

    return {
      dom,
      contentDOM: content,
      update(next) {
        if (next.type !== node.type) return false;
        node = next;
        setAttrs(next.attrs);
        show();
        return true;
      },
      stopEvent(e) {
        const t = e.target;
        return t instanceof globalThis.Node && (tools.contains(t) || head.contains(t) || fold.button.contains(t));
      },
      ignoreMutation(m) {
        if (m.type === "selection") return false;
        return !content.contains(m.target) || (m.target === content && m.type === "attributes");
      },
      destroy() {
        disposeHead();
        disposeTools();
      },
    };
  };
}

// ---------------------------------------------------------------------------------------------
// Nodes

export const ToggleSummary = Node.create({
  name: "toggleSummary",
  content: "inline*",
  defining: true,
  addAttributes() {
    return {
      level: {
        default: 0,
        parseHTML: (el) => Number(el.getAttribute("data-level")) || 0,
        renderHTML: (a) => ({ "data-level": String(a.level) }),
      },
    };
  },
  parseHTML() {
    return [{ tag: "div[data-toggle-summary]" }];
  },
  renderHTML({ node, HTMLAttributes }) {
    const level = Number(node.attrs.level) || 0;
    const heading = level ? { role: "heading", "aria-level": String(level) } : {};
    return ["div", { "data-toggle-summary": "", class: `toggle-text toggle-level-${level}`, ...heading, ...HTMLAttributes }, 0];
  },
});

export const AdmonitionTitle = Node.create({
  name: "admonitionTitle",
  content: "inline*",
  defining: true,
  parseHTML() {
    return [{ tag: "div[data-admonition-title]" }];
  },
  renderHTML() {
    return ["div", { "data-admonition-title": "", class: "admonition-title" }, 0];
  },
});

export const Toggle = Node.create<{ scope: ToggleScope }>({
  name: "toggle",
  group: "block",
  content: "toggleSummary block*",
  defining: true,
  draggable: true,
  addOptions() {
    return { scope: () => undefined };
  },
  parseHTML() {
    return [{ tag: "div[data-toggle]" }];
  },
  renderHTML() {
    return ["div", { "data-toggle": "" }, 0];
  },
  addNodeView() {
    return toggleView(this.options.scope);
  },
  addInputRules() {
    const type = this.type;
    return [
      // `>> ` at the start of a paragraph: `> ` is the quote shortcut, so a toggle takes two.
      new InputRule({
        find: /^>>\s$/,
        handler: ({ state, range }) => {
          const $from = state.doc.resolve(range.from);
          if ($from.parent.type.name !== "paragraph") return null;
          const start = $from.before();
          const rest = $from.parent.content.cut(range.to - $from.start());
          const summary = state.schema.nodes.toggleSummary!.create({ level: 0 }, rest);
          const tr = state.tr.replaceWith(start, $from.after(), type.create(null, summary));
          tr.setSelection(TextSelection.create(tr.doc, start + 2));
        },
      }),
    ];
  },
});

export const Admonition = Node.create<{ scope: ToggleScope }>({
  name: "admonition",
  group: "block",
  content: "admonitionTitle block+",
  defining: true,
  draggable: true,
  addOptions() {
    return { scope: () => undefined };
  },
  addAttributes() {
    return {
      kind: { default: "note", parseHTML: (el) => el.getAttribute("data-kind") ?? "note" },
      colour: { default: null, parseHTML: (el) => el.getAttribute("data-colour") },
      icon: { default: null, parseHTML: (el) => el.getAttribute("data-icon") },
      collapsible: { default: false, parseHTML: (el) => el.hasAttribute("data-collapsible") },
    };
  },
  parseHTML() {
    return [{ tag: "div[data-admonition]" }];
  },
  renderHTML({ node }) {
    const a: Record<string, string> = { "data-admonition": "", "data-kind": node.attrs.kind };
    if (node.attrs.colour) a["data-colour"] = node.attrs.colour;
    if (node.attrs.icon) a["data-icon"] = node.attrs.icon;
    if (node.attrs.collapsible) a["data-collapsible"] = "";
    return ["div", a, 0];
  },
  addNodeView() {
    return admonitionView(this.editor, this.options.scope);
  },
});

// ---------------------------------------------------------------------------------------------
// Transactions

/** The block of a container (the document, a column, a toggle or an admonition) holding the selection. */
function blockAtSelection(state: EditorState): { pos: number; node: PMNode; depth: number } | null {
  const sel = state.selection;
  if (sel instanceof NodeSelection) {
    return { pos: sel.from, node: sel.node, depth: sel.$from.depth + 1 };
  }
  const $f = sel.$from;
  for (let d = $f.depth; d > 0; d--) {
    const node = $f.node(d);
    if (FOLD_HEADS.has(node.type.name)) continue;
    const parent = $f.node(d - 1);
    if (["doc", "column", "toggle", "admonition"].includes(parent.type.name)) return { pos: $f.before(d), node, depth: d };
  }
  return null;
}

/** Whether the selection is inside a list item, where Tab and Shift+Tab nest list items. */
function inListItem(state: EditorState): boolean {
  const $f = state.selection.$from;
  for (let d = $f.depth; d > 0; d--) if (["listItem", "taskItem"].includes($f.node(d).type.name)) return true;
  return false;
}

/** `tr` with the selection put back on the block moved from `from` to `to`, as it was on it. */
function keepSelection(tr: Transaction, state: EditorState, from: number, to: number, size: number) {
  const sel = state.selection;
  if (sel instanceof NodeSelection) {
    tr.setSelection(NodeSelection.create(tr.doc, to));
    return;
  }
  const clamp = (p: number) => Math.max(to + 1, Math.min(to + size - 1, p - from + to));
  tr.setSelection(TextSelection.between(tr.doc.resolve(clamp(sel.from)), tr.doc.resolve(clamp(sel.to))));
}

/** Move the block holding the selection into the end of the toggle or admonition just above it. */
export function nestTr(state: EditorState): Transaction | null {
  if (inListItem(state)) return null;
  const at = blockAtSelection(state);
  if (!at) return null;
  const $at = state.doc.resolve(at.pos);
  const index = $at.index();
  const parent = $at.parent;
  // Not before the heading line of the block it is in.
  const first = FOLDING.has(parent.type.name) ? 1 : 0;
  if (index <= first) return null;
  const prev = parent.child(index - 1);
  if (!FOLDING.has(prev.type.name) || at.node.type.name === "columns") return null;
  const prevPos = at.pos - prev.nodeSize;
  const into = prevPos + prev.nodeSize - 1;
  const tr = state.tr.delete(at.pos, at.pos + at.node.nodeSize).insert(into, at.node);
  keepSelection(tr, state, at.pos, into, at.node.nodeSize);
  return tr.scrollIntoView();
}

/** Move the block holding the selection out of the toggle or admonition it is in, to just after it. */
export function unnestTr(state: EditorState): Transaction | null {
  if (inListItem(state)) return null;
  const at = blockAtSelection(state);
  if (!at) return null;
  const $at = state.doc.resolve(at.pos);
  if (!FOLDING.has($at.parent.type.name)) return null;
  const container = $at.parent;
  const end = $at.before() + container.nodeSize;
  const size = at.node.nodeSize;
  const tr = state.tr;
  // An admonition keeps a block of its own: a line to type on takes the place of its last one.
  if (container.type.name === "admonition" && container.childCount <= 2) {
    tr.replaceWith(at.pos, at.pos + size, state.schema.nodes.paragraph!.create());
  } else {
    tr.delete(at.pos, at.pos + size);
  }
  const to = tr.mapping.map(end);
  tr.insert(to, at.node);
  keepSelection(tr, state, at.pos, to, size);
  return tr.scrollIntoView();
}

/**
 * Turn the toggle (or unwrap the admonition) whose heading line holds the caret into its
 * blocks: a toggle's summary becomes a paragraph with the same text, and the nested blocks stay
 * where they are, at the same depth as it was.
 */
export function unwrapTr(state: EditorState): Transaction | null {
  const $f = state.selection.$from;
  const head = $f.parent;
  if (!FOLD_HEADS.has(head.type.name)) return null;
  const fold = $f.node($f.depth - 1);
  const pos = $f.before($f.depth - 1);
  const nodes: PMNode[] = [];
  if (head.type.name === "toggleSummary" || head.content.size > 0) nodes.push(state.schema.nodes.paragraph!.create(null, head.content));
  fold.forEach((child, _o, i) => {
    if (i > 0) nodes.push(child);
  });
  if (!nodes.length) nodes.push(state.schema.nodes.paragraph!.create());
  const tr = state.tr.replaceWith(pos, pos + fold.nodeSize, nodes);
  tr.setSelection(Selection.near(tr.doc.resolve(pos + 1)));
  return tr.scrollIntoView();
}

// ---------------------------------------------------------------------------------------------
// Keys, and the editor's own rules

/**
 * Tab, Shift+Tab, Enter and Backspace for toggles and admonitions; a selection that goes inside
 * a closed one opens it; and the hints in an empty summary or title.
 */
export const ToggleKeys = Extension.create({
  name: "toggleKeys",
  // Ahead of the default keymap and the lists', whose Tab and Backspace would otherwise act first.
  priority: 950,
  addKeyboardShortcuts() {
    const run = (make: (state: EditorState) => Transaction | null) => () => {
      const { view } = this.editor;
      const tr = make(view.state);
      if (!tr) return false;
      view.dispatch(tr);
      return true;
    };
    return {
      Tab: run(nestTr),
      "Shift-Tab": run(unnestTr),
      Enter: () => {
        const { state, view } = this.editor;
        const sel = state.selection;
        if (!sel.empty) return false;
        const $f = sel.$from;
        if ($f.parent.type.name !== "toggleSummary" || $f.parentOffset !== $f.parent.content.size) return false;
        const pos = $f.before($f.depth - 1);
        const toggle = $f.node($f.depth - 1);
        const para = state.schema.nodes.paragraph!.create();
        if (isOpen(view, pos)) {
          // The first nested block, before any already there.
          const at = $f.after();
          const tr = state.tr.insert(at, para);
          tr.setSelection(TextSelection.create(tr.doc, at + 1));
          view.dispatch(tr.scrollIntoView());
        } else {
          const at = pos + toggle.nodeSize;
          const tr = state.tr.insert(at, para);
          tr.setSelection(TextSelection.create(tr.doc, at + 1));
          view.dispatch(tr.scrollIntoView());
        }
        return true;
      },
      Backspace: () => {
        const { state, view } = this.editor;
        const sel = state.selection;
        if (!sel.empty || sel.$from.parentOffset !== 0) return false;
        const head = sel.$from.parent;
        // At the start of a title with text, there is nothing before it to join: keep it as it is.
        if (head.type.name === "admonitionTitle" && head.content.size > 0) return true;
        const tr = unwrapTr(state);
        if (!tr) return false;
        view.dispatch(tr);
        return true;
      },
    };
  },
  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: new PluginKey("toggleRules"),
        view() {
          return {
            update(view) {
              // The caret went inside a closed toggle (Tab, Enter, a search, the arrows): open it.
              const $f = view.state.selection.$from;
              for (let d = $f.depth; d > 0; d--) {
                const node = $f.node(d);
                if (!FOLDING.has(node.type.name) || $f.index(d) === 0) continue;
                openAt(view, $f.before(d));
              }
            },
          };
        },
        props: {
          decorations(state) {
            const marks: Decoration[] = [];
            // The admonition holding the selection shows its tools where there is no hover.
            const $f = state.selection.$from;
            for (let d = $f.depth; d > 0; d--) {
              if ($f.node(d).type.name !== "admonition") continue;
              const pos = $f.before(d);
              marks.push(Decoration.node(pos, pos + $f.node(d).nodeSize, { class: "admonition-active" }));
              break;
            }
            state.doc.descendants((node, pos, parent) => {
              if (!FOLD_HEADS.has(node.type.name)) return true;
              if (node.content.size === 0) {
                const hint = node.type.name === "toggleSummary" ? "Toggle" : admonitionLook(parent?.attrs).label;
                marks.push(Decoration.node(pos, pos + node.nodeSize, { class: "is-empty", "data-placeholder": hint }));
              }
              return false;
            });
            return marks.length ? DecorationSet.create(state.doc, marks) : null;
          },
        },
      }),
    ];
  },
});
