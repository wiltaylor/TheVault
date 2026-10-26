// A script tool proposal's card (script-tools T3): the tool file's WCL lines removed and added,
// and the script's, as the type proposal card shows them, with Apply and Discard. A delete shows
// the tool file that goes and, when nothing else names it, the script. Apply writes nothing when
// the tool or its script changed since the proposal.

import { For, Match, Show, Switch } from "solid-js";
import type { DiffLine } from "./diff";
import { type Part, type ToolProposal, useChatSession } from "./store";
import { wclDiff } from "./TypeProposalCard";

/** The Tools page's editor of tool `name`. */
export const toolHref = (name: string) => `/settings/tools/${encodeURIComponent(name)}`;

/** The card's heading, before the tool's name. */
function head(p: ToolProposal): string {
  if (p.op === "delete") return "Proposed delete of tool ";
  return p.op === "create" ? "Proposed new tool " : "Proposed change to tool ";
}

/** Lines removed and added, with the unchanged ones folded. */
function DiffLines(props: { before: string | null; after: string | null; label: string }) {
  return (
    <pre class="ai-diff-lines" aria-label={props.label}>
      <For each={wclDiff(props.before, props.after ?? "")}>
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
  );
}

export function ToolProposalCard(props: { part: Extract<Part, { kind: "tool_proposal" }>; turn: number; index: number }) {
  const session = useChatSession();
  const p = () => props.part.proposal;
  const del = () => p().op === "delete";
  const scriptPath = () => (p().script ? `scripts/${p().script}.ws` : "the script");
  const open = () => props.part.state === "open" || props.part.state === "failed" || props.part.state === "applying";
  return (
    <section class="ai-proposal" aria-label={`${head(p())}${p().name}`}>
      <p class="ai-proposal-head">
        {head(p())}
        <Show when={p().op !== "create"} fallback={<code>{p().name}</code>}>
          <a href={toolHref(p().name)}>{p().name}</a>
        </Show>
      </p>
      <p class="muted">{`tools/${p().name}.wcl`}</p>
      <DiffLines before={p().before} after={del() ? "" : p().after} label="Tool file" />
      <Show when={!del() && p().script_after !== null}>
        <p class="muted">{`${scriptPath()}${p().script_exists === false || p().script_before === null ? " (new)" : ""}`}</p>
        <DiffLines before={p().script_before} after={p().script_after} label="Script" />
      </Show>
      <Show when={del()}>
        <p class="muted">
          {p().script_delete ? `Its script ${scriptPath()} is deleted too: nothing else names it.` : `Its script ${scriptPath()} stays.`}
        </p>
      </Show>
      <Show when={p().problems.length > 0}>
        <ul class="ai-type-issues" aria-label="Problems">
          <For each={p().problems}>{(m) => <li class="muted">{m}</li>}</For>
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
            <button type="button" class="primary" disabled={props.part.state === "applying"} onClick={() => void session.applyToolProposal(props.turn, props.index)}>
              Apply
            </button>
            <button type="button" disabled={props.part.state === "applying"} onClick={() => session.discardToolProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "stale"}>
          <div class="ai-proposal-actions">
            <button type="button" onClick={() => session.discardToolProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "applied"}>
          <p role="status" class="ai-applied">
            Applied.{" "}
            <Show when={!del()}>
              <a href={toolHref(p().name)}>Open tool</a>
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
