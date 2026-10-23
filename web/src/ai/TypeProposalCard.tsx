// A type proposal's card (ai D18): the type file's WCL lines removed and added, with Apply and
// Discard. Apply re-reads the type and writes nothing when it changed since the proposal.

import { For, Match, Show, Switch } from "solid-js";
import { type DiffLine, lineDiff } from "./diff";
import { type Part, useChatSession } from "./store";

/** Unchanged lines kept around each change; longer unchanged runs fold into one line. */
const CONTEXT = 2;

type Row = DiffLine | { type: "skip"; count: number };

/** The WCL diff of `before` and `after`, unchanged runs beyond `CONTEXT` lines folded. */
export function wclDiff(before: string | null, after: string): Row[] {
  const lines = (s: string | null) => (s ? s.replace(/\n$/, "").split("\n") : []);
  const all = lineDiff(lines(before), lines(after));
  const near = all.map((_, i) => all.slice(Math.max(0, i - CONTEXT), i + CONTEXT + 1).some((l) => l.type !== "same"));
  const out: Row[] = [];
  for (let i = 0; i < all.length; i++) {
    if (near[i]) {
      out.push(all[i]!);
      continue;
    }
    const last = out[out.length - 1];
    if (last?.type === "skip") last.count++;
    else out.push({ type: "skip", count: 1 });
  }
  return out;
}

export function TypeProposalCard(props: { part: Extract<Part, { kind: "type_proposal" }>; turn: number; index: number }) {
  const session = useChatSession();
  const p = () => props.part.proposal;
  const label = () => (p().exists ? `Proposed change to type ${p().name}` : `Proposed new type ${p().name}`);
  const open = () => props.part.state === "open" || props.part.state === "failed" || props.part.state === "applying";
  return (
    <section class="ai-proposal" aria-label={label()}>
      <p class="ai-proposal-head">
        {p().exists ? "Proposed change to type " : "Proposed new type "}
        <a href={`/types/${encodeURIComponent(p().name)}`}>{p().name}</a>
      </p>
      <pre class="ai-diff-lines">
        <For each={wclDiff(p().before, p().after)}>
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
      <Show when={p().issues.length > 0}>
        <ul class="ai-type-issues">
          <For each={p().issues}>{(i) => <li class="muted">{i.at ? `${i.at}: ${i.message}` : i.message}</li>}</For>
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
            <button type="button" class="primary" disabled={props.part.state === "applying"} onClick={() => void session.applyTypeProposal(props.turn, props.index)}>
              Apply
            </button>
            <button type="button" disabled={props.part.state === "applying"} onClick={() => session.discardTypeProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "stale"}>
          <div class="ai-proposal-actions">
            <button type="button" onClick={() => session.discardTypeProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "applied"}>
          <p role="status" class="ai-applied">
            Applied.
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
