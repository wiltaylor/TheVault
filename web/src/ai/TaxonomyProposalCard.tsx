// A taxonomy proposal's card (ai D31): the taxonomy file's WCL lines removed and added, as the
// type proposal card shows them, the term renames with the number of notes they move, and Apply
// and Discard. A delete shows the file that goes. Apply writes nothing when the taxonomy changed
// since the proposal.

import { For, Match, Show, Switch } from "solid-js";
import type { DiffLine } from "./diff";
import { type Part, applyTaxonomyProposal, discardTaxonomyProposal } from "./store";
import { wclDiff } from "./TypeProposalCard";

const TAXONOMIES = "/settings/taxonomies";

export function TaxonomyProposalCard(props: { part: Extract<Part, { kind: "taxonomy_proposal" }>; turn: number; index: number }) {
  const p = () => props.part.proposal;
  const head = () => {
    const x = p();
    if (x.op === "delete") return "Proposed delete of taxonomy ";
    return x.exists ? "Proposed change to taxonomy " : "Proposed new taxonomy ";
  };
  const link = () => (p().op === "create" ? TAXONOMIES : `${TAXONOMIES}/${encodeURIComponent(p().name)}`);
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
        <a href={link()}>{p().name}</a>
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
      <Show when={change()?.renames.length}>
        <ul class="ai-taxonomy-renames" aria-label="Renamed terms">
          <For each={change()!.renames}>
            {(r) => (
              <li>
                <code>{r.from}</code> → <code>{r.to}</code>
              </li>
            )}
          </For>
        </ul>
        <p class="muted">
          {change()!.affected === 1 ? "Moves 1 note to the new terms." : `Moves ${change()!.affected} notes to the new terms.`}
        </p>
      </Show>
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
            <button type="button" class="primary" disabled={props.part.state === "applying"} onClick={() => void applyTaxonomyProposal(props.turn, props.index)}>
              Apply
            </button>
            <button type="button" disabled={props.part.state === "applying"} onClick={() => discardTaxonomyProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "stale"}>
          <div class="ai-proposal-actions">
            <button type="button" onClick={() => discardTaxonomyProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "applied"}>
          <p role="status" class="ai-applied">
            Applied. <a href={p().op === "delete" ? TAXONOMIES : `${TAXONOMIES}/${encodeURIComponent(p().name)}`}>Open in Settings &gt; Taxonomies</a>
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
