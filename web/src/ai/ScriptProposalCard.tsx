// A saved script proposal's card (TV-70): the script's lines removed and added, every handler,
// task, device file and tool that runs it with what each may reach, and a warning for a run that
// may reach any host, with Apply and Discard. The proposal always waits for the user, whatever the
// agent's switches say. Apply writes nothing when the script changed since the proposal.

import { Match, Show, Switch } from "solid-js";
import { RunsAs } from "./RunsAs";
import { type Part, type ScriptProposal, useChatSession } from "./store";
import { DiffLines } from "./ToolProposalCard";

/** The Scripts page's editor of script `name`. */
export const scriptHref = (name: string) => `/settings/scripts/${encodeURIComponent(name)}`;

/** The card's heading, before the script's name. */
function head(p: ScriptProposal): string {
  if (p.op === "delete") return "Proposed delete of script ";
  return p.op === "create" ? "Proposed new script " : "Proposed change to script ";
}

export function ScriptProposalCard(props: { part: Extract<Part, { kind: "script_proposal" }>; turn: number; index: number }) {
  const session = useChatSession();
  const p = () => props.part.proposal;
  const del = () => p().op === "delete";
  const open = () => props.part.state === "open" || props.part.state === "failed" || props.part.state === "applying";
  return (
    <section class="ai-proposal" aria-label={`${head(p())}${p().name}`}>
      <p class="ai-proposal-head">
        {head(p())}
        <code>{p().name}</code>
      </p>
      <p class="muted">{`scripts/${p().name}.ws${p().op === "create" ? " (new)" : ""}`}</p>
      <DiffLines before={p().before} after={del() ? "" : p().after} label="Script" />
      <Show when={!del()}>
        <RunsAs
          uses={p().runs_as}
          warnings={p().warnings}
          heading="Runs as: this change applies to each of these"
          empty="Nothing runs this script yet."
        />
      </Show>
      <Show when={props.part.error}>
        <p role="alert" class="ai-error">
          {props.part.error}
        </p>
      </Show>
      <Switch>
        <Match when={open()}>
          <div class="ai-proposal-actions">
            <button type="button" class="primary" disabled={props.part.state === "applying"} onClick={() => void session.applyScriptProposal(props.turn, props.index)}>
              Apply
            </button>
            <button type="button" disabled={props.part.state === "applying"} onClick={() => session.discardScriptProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "stale"}>
          <div class="ai-proposal-actions">
            <button type="button" onClick={() => session.discardScriptProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "applied"}>
          <p role="status" class="ai-applied">
            Applied.{" "}
            <Show when={!del()}>
              <a href={scriptHref(p().name)}>Open script</a>
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
