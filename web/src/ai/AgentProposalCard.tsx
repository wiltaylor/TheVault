// An agent proposal's card (agents G20): the agent file's WCL lines removed and added, as the
// type proposal card shows them, and Apply and Discard. A delete shows the file that goes. Apply
// writes nothing when the agent changed since the proposal, and the server checks the change
// again against the agent that proposed it.

import { For, Match, Show, Switch } from "solid-js";
import type { DiffLine } from "./diff";
import { type Part, useChatSession } from "./store";
import { wclDiff } from "./TypeProposalCard";

/** The agent screen of agent `name`. */
const agentHref = (name: string) => `/agents/${encodeURIComponent(name)}`;

export function AgentProposalCard(props: { part: Extract<Part, { kind: "agent_proposal" }>; turn: number; index: number }) {
  const session = useChatSession();
  const p = () => props.part.proposal;
  const head = () => {
    const x = p();
    if (x.op === "delete") return "Proposed delete of agent ";
    return x.exists ? "Proposed change to agent " : "Proposed new agent ";
  };
  const change = () => {
    const x = p();
    return x.op === "delete" ? undefined : x;
  };
  // A delete's lines all go.
  const rows = () => {
    const x = p();
    return x.op === "delete" ? wclDiff(x.before, "") : wclDiff(x.before, x.after);
  };
  const open = () => props.part.state === "open" || props.part.state === "failed" || props.part.state === "applying";
  return (
    <section class="ai-proposal" aria-label={`${head()}${p().name}`}>
      <p class="ai-proposal-head">
        {head()}
        <Show when={p().op !== "create"} fallback={<code>{p().name}</code>}>
          <a href={agentHref(p().name)}>{p().name}</a>
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
      <Show when={change()?.issues.length}>
        <ul class="ai-type-issues">
          <For each={change()!.issues}>{(i) => <li class="muted">{i.at ? `${i.at}: ${i.message}` : i.message}</li>}</For>
        </ul>
      </Show>
      <Show when={props.part.error}>
        <p role="alert" class="ai-error">
          {props.part.error}
        </p>
      </Show>
      <Switch>
        <Match when={open()}>
          <div class="ai-proposal-actions">
            <button type="button" class="primary" disabled={props.part.state === "applying"} onClick={() => void session.applyAgentProposal(props.turn, props.index)}>
              Apply
            </button>
            <button type="button" disabled={props.part.state === "applying"} onClick={() => session.discardAgentProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "stale"}>
          <div class="ai-proposal-actions">
            <button type="button" onClick={() => session.discardAgentProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "applied"}>
          <p role="status" class="ai-applied">
            Applied.{" "}
            <Show when={p().op !== "delete"} fallback={<span class="muted">The agent's threads stay.</span>}>
              <a href={agentHref(p().name)}>Open agent</a>
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
