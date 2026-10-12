// Renders a Solid component as a ProseMirror node view (D29). Every rich block and inline node
// the editor holds goes through this adapter.
//
// The node views it makes are atoms: ProseMirror owns no content inside them, the component
// owns all of it. ProseMirror is told to keep out of the component's events and DOM changes:
//
// - `stopEvent` claims every event aimed at an interactive element inside the component (an
//   input, a button, a list of choices), so typing into it never reaches the editor. Events on
//   the rest of the node still reach ProseMirror, so clicking selects the node and it can be
//   dragged.
// - `ignoreMutation` ignores every DOM change inside the component. ProseMirror would otherwise
//   read the component's re-renders as edits to the document and redraw the node.

import type { Editor, NodeViewRenderer } from "@tiptap/core";
import type { Node as PMNode } from "@tiptap/pm/model";
import type { NodeView } from "@tiptap/pm/view";
import { type Accessor, type JSX, createRoot, createSignal } from "solid-js";
import { insert } from "solid-js/web";

export interface SolidNodeViewProps {
  /** The node, updated whenever ProseMirror redraws it with new attrs. */
  node: Accessor<PMNode>;
  /** True while the node is the selected node. */
  selected: Accessor<boolean>;
  editor: Editor;
  /** The node's position in the document, or undefined once it is gone. */
  getPos: () => number | undefined;
  updateAttributes: (attrs: Record<string, unknown>) => void;
  /** Replace this node with `content`, or delete it when `content` is omitted. */
  replaceWith: (content?: PMNode) => void;
}

/** Elements inside a node view whose events belong to the component, not to ProseMirror. */
const INTERACTIVE = "input, textarea, select, button, [contenteditable='true'], [data-node-view-interactive]";

export function isInteractive(dom: HTMLElement, target: EventTarget | null): boolean {
  if (!(target instanceof Element) || !dom.contains(target)) return false;
  const hit = target.closest(INTERACTIVE);
  return hit !== null && dom.contains(hit);
}

export function solidNodeView(Component: (props: SolidNodeViewProps) => JSX.Element): NodeViewRenderer {
  return (props) => {
    const dom = document.createElement(props.node.isInline ? "span" : "div");
    dom.classList.add("solid-node-view");
    for (const [k, v] of Object.entries(props.HTMLAttributes)) {
      if (v !== null && v !== undefined && v !== false) dom.setAttribute(k, String(v));
    }
    const [node, setNode] = createSignal(props.node, { equals: false });
    const [selected, setSelected] = createSignal(false);
    const getPos = () => {
      const pos = props.getPos();
      return typeof pos === "number" ? pos : undefined;
    };

    const viewProps: SolidNodeViewProps = {
      node,
      selected,
      editor: props.editor,
      getPos,
      updateAttributes(attrs) {
        const pos = getPos();
        if (pos === undefined) return;
        const { view } = props.editor;
        view.dispatch(view.state.tr.setNodeMarkup(pos, undefined, { ...node().attrs, ...attrs }));
      },
      replaceWith(content) {
        const pos = getPos();
        if (pos === undefined) return;
        const { view } = props.editor;
        const end = pos + node().nodeSize;
        const tr = content ? view.state.tr.replaceWith(pos, end, content) : view.state.tr.delete(pos, end);
        view.dispatch(tr);
      },
    };

    let dispose = () => {};
    createRoot((d) => {
      dispose = d;
      insert(dom, () => Component(viewProps));
    });

    const view: NodeView = {
      dom,
      update(next) {
        if (next.type !== props.node.type) return false;
        setNode(next);
        return true;
      },
      selectNode() {
        dom.classList.add("ProseMirror-selectednode");
        setSelected(true);
      },
      deselectNode() {
        dom.classList.remove("ProseMirror-selectednode");
        setSelected(false);
      },
      stopEvent(event) {
        return isInteractive(dom, event.target);
      },
      ignoreMutation() {
        return true;
      },
      destroy() {
        dispose();
      },
    };
    return view;
  };
}
