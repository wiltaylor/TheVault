// Calls of agents in the editor (agents G19): a link whose URL is `agent:<name>?call=<ULID>`
// stays a link, its text editable as any link's, and is decorated as a chip: the agent's icon
// before it, and after it the running spinner while the call's thread runs, or "Sending…" until
// the server has seen the call. `FollowLinks` opens the call's thread when it is clicked.

import { Extension } from "@tiptap/core";
import type { Node as PMNode } from "@tiptap/pm/model";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";
import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { parseAgentCall } from "../agentCalls";
import { AgentCallIcon, AgentCallState } from "../components/AgentCallChip";

/** One call in the document: where its link text runs, its agent and its call id. */
export interface CallRange {
  from: number;
  to: number;
  agent: string;
  call: string;
}

/** The calls in `doc`, in order: each run of inline content carrying one call's link. */
export function callRanges(doc: PMNode): CallRange[] {
  const out: CallRange[] = [];
  doc.descendants((node, pos) => {
    if (!node.isInline) return true;
    const link = node.marks.find((m) => m.type.name === "link");
    const c = link ? parseAgentCall(String(link.attrs.href ?? "")) : null;
    if (!c) return false;
    const last = out[out.length - 1];
    if (last && last.to === pos && last.call === c.call) last.to = pos + node.nodeSize;
    else out.push({ from: pos, to: pos + node.nodeSize, ...c });
    return false;
  });
  return out;
}

const disposers = new WeakMap<Node, () => void>();

/** A widget showing `content`, marked with the call it belongs to so a click on it opens the call. */
function widget(r: CallRange, part: string, content: () => JSX.Element): HTMLElement {
  const el = document.createElement("span");
  el.className = `agent-call-widget agent-call-${part}`;
  el.contentEditable = "false";
  el.dataset.agentCall = r.call;
  el.dataset.agent = r.agent;
  disposers.set(el, render(content, el));
  return el;
}

function decorations(doc: PMNode): DecorationSet {
  const decos: Decoration[] = [];
  const destroy = (node: Node) => {
    disposers.get(node)?.();
    disposers.delete(node);
  };
  for (const r of callRanges(doc)) {
    decos.push(
      Decoration.widget(r.from, () => widget(r, "start", () => <AgentCallIcon agent={r.agent} />), {
        side: -1,
        key: `agent-call-start:${r.call}:${r.agent}`,
        ignoreSelection: true,
        destroy,
      }),
      Decoration.inline(r.from, r.to, { class: "agent-call", "data-testid": "agent-call", "data-call": r.call, "data-agent": r.agent }),
      Decoration.widget(r.to, () => widget(r, "end", () => <AgentCallState call={r.call} />), {
        side: 1,
        key: `agent-call-end:${r.call}`,
        ignoreSelection: true,
        destroy,
      }),
    );
  }
  return DecorationSet.create(doc, decos);
}

/** The agent call a mouse event in the editor is on: its link text or one of its widgets. */
export function agentCallAt(target: EventTarget | null): { agent: string; call: string } | null {
  if (!(target instanceof Element)) return null;
  const w = target.closest<HTMLElement>("[data-agent-call]");
  if (w?.dataset.agentCall && w.dataset.agent) return { agent: w.dataset.agent, call: w.dataset.agentCall };
  const a = target.closest("a[href]");
  return a ? parseAgentCall(a.getAttribute("href") ?? "") : null;
}

export const AgentCallChips = Extension.create({
  name: "agentCallChips",
  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: new PluginKey("agentCallChips"),
        state: {
          init: (_, state) => decorations(state.doc),
          apply: (tr, old) => (tr.docChanged ? decorations(tr.doc) : old),
        },
        props: {
          decorations(state) {
            return this.getState(state);
          },
        },
      }),
    ];
  },
});
