// A memory proposal's card (agents G21): a change to another agent's memories, its title and the
// lines of its text removed and added, with Apply and Discard. A delete shows the memory that
// goes. Apply writes nothing when the memory changed since the proposal, and the server checks
// the change again against the agent that proposed it.

import { For, Match, Show, Switch } from "solid-js";
import type { DiffLine } from "./diff";
import { type Part, useChatSession } from "./store";
import { wclDiff } from "./TypeProposalCard";

/** The Memory tab of agent `name`'s screen. */
export const memoryHref = (name: string) => `/agents/${encodeURIComponent(name)}/settings/memory`;

export function MemoryProposalCard(props: { part: Extract<Part, { kind: "memory_proposal" }>; turn: number; index: number }) {
  const session = useChatSession();
  const p = () => props.part.proposal;
  const head = () => {
    const x = p();
    if (x.op === "delete") return "Proposed forgetting a memory of ";
    return x.op === "change" ? "Proposed change to a memory of " : "Proposed new memory of ";
  };
  const who = () => p().agent_label || p().memory_agent;
  const titleChanged = () => p().op === "change" && p().before?.title !== p().title;
  const rows = () => {
    const x = p();
    if (x.op === "delete") return wclDiff(x.text, "");
    return wclDiff(x.op === "change" ? (x.before?.text ?? null) : null, x.text);
  };
  const open = () => props.part.state === "open" || props.part.state === "failed" || props.part.state === "applying";
  return (
    <section class="ai-proposal" aria-label={`${head()}${who()}`}>
      <p class="ai-proposal-head">
        {head()}
        <a href={memoryHref(p().memory_agent)}>{who()}</a>
      </p>
      <p class="ai-memory-title">
        <Show when={titleChanged()}>
          <del>{p().before?.title}</del>{" "}
        </Show>
        <Show when={p().op === "delete"} fallback={<strong>{p().title}</strong>}>
          <del>{p().title}</del>
        </Show>
      </p>
      <pre class="ai-diff-lines">
        <For each={rows()}>
          {(l) => (
            <Switch fallback={<span class="ai-same">{`  ${(l as DiffLine).text}\n`}</span>}>
              <Match when={l.type === "skip"}>
                <span class="muted">{`  … ${(l as { count: number }).count} unchanged ${(l as { count: number }).count === 1 ? "line" : "lines"}\n`}</span>
              </Match>
              <Match when={l.type === "del"}>
                <del>{`- ${(l as DiffLine).text}\n`}</del>
              </Match>
              <Match when={l.type === "add"}>
                <ins>{`+ ${(l as DiffLine).text}\n`}</ins>
              </Match>
            </Switch>
          )}
        </For>
      </pre>
      <Show when={props.part.error}>
        <p role="alert" class="ai-error">
          {props.part.error}
        </p>
      </Show>
      <Switch>
        <Match when={open()}>
          <div class="ai-proposal-actions">
            <button type="button" class="primary" disabled={props.part.state === "applying"} onClick={() => void session.applyMemoryProposal(props.turn, props.index)}>
              Apply
            </button>
            <button type="button" disabled={props.part.state === "applying"} onClick={() => session.discardMemoryProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "stale"}>
          <div class="ai-proposal-actions">
            <button type="button" onClick={() => session.discardMemoryProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "applied"}>
          <p role="status" class="ai-applied">
            Applied.{" "}
            <Show when={p().op !== "delete"} fallback={<span class="muted">The memory is in the trash, where you can restore it.</span>}>
              <a href={memoryHref(p().memory_agent)}>Open memories</a>
            </Show>
          </p>
        </Match>
        <Match when={props.part.state === "discarded"}>
          <p role="status" class="muted">
            Discarded.
          </p>
        </Match>
      </Switch>
    </section>
  );
}
