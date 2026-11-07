// A scheduled task proposal's card (TV-71): the schedule in words with its time zone and the next
// runs, what the task may reach (hosts, secrets, timeout, retries) with a warning for any host,
// for secrets with hosts and for a secret that does not exist, the task file's lines removed and
// added and the script's when the proposal writes it, and what else runs the script, with Apply
// and Discard. The proposal always waits for the user, and never holds `fs` paths: the user adds
// those in the task editor. Apply writes nothing when the task or its script changed since.

import { For, Match, Show, Switch } from "solid-js";
import { when } from "../admin/format";
import { RunsAs } from "./RunsAs";
import { type Part, type ProposedTask, type TaskProposal, useChatSession } from "./store";
import { DiffLines } from "./ToolProposalCard";

/** The Scheduled page's editor of task `name`. */
export const taskHref = (name: string) => `/scheduled/${encodeURIComponent(name)}`;

/** The card's heading, before the task's name. */
function head(p: TaskProposal): string {
  if (p.op === "delete") return "Proposed delete of task ";
  return p.op === "create" ? "Proposed new task " : "Proposed change to task ";
}

/** What a task may reach and how it runs, one line each. */
export function taskGrants(t: ProposedTask): string[] {
  const out: string[] = [];
  out.push(`Calls ${t.function} in scripts/${t.script}.ws`);
  if (t.allow_domains.includes("*")) out.push("Hosts: any host, including your local network");
  else if (t.allow_domains.length > 0) out.push(`Hosts: ${t.allow_domains.join(", ")}`);
  else out.push("Hosts: none");
  out.push(t.secrets.length > 0 ? `Secrets: ${t.secrets.join(", ")}` : "Secrets: none");
  out.push(`Timeout: ${Math.round(t.timeout_ms / 1000)} s`);
  out.push(t.retries === 0 ? "Retries: none" : `Retries: ${t.retries}, the first after ${Math.round(t.retry_delay_ms / 1000)} s, each later one twice as long`);
  return out;
}

export function TaskProposalCard(props: { part: Extract<Part, { kind: "task_proposal" }>; turn: number; index: number }) {
  const session = useChatSession();
  const p = () => props.part.proposal;
  const del = () => p().op === "delete";
  const scriptPath = () => `scripts/${p().script ?? p().task?.script ?? ""}.ws`;
  const open = () => props.part.state === "open" || props.part.state === "failed" || props.part.state === "applying";
  return (
    <section class="ai-proposal" aria-label={`${head(p())}${p().name}`}>
      <p class="ai-proposal-head">
        {head(p())}
        <code>{p().name}</code>
      </p>
      <p class="muted">{`tasks/${p().name}.wcl${p().op === "create" ? " (new)" : ""}`}</p>
      <Show when={p().task}>
        {(t) => (
          <>
            <p>
              <strong>{t().label}</strong> {t().schedule_text}
            </p>
            <Show when={!del()}>
              <Show when={!t().paused} fallback={<p class="muted">Paused: it runs only when run by hand.</p>}>
                <Show when={p().next_runs.length > 0} fallback={<p class="muted">It never runs on this schedule.</p>}>
                  <p class="muted">Once applied, it runs next at:</p>
                  <ul class="ai-runs-as" aria-label="Next runs">
                    <For each={p().next_runs}>{(ms) => <li>{when(ms)}</li>}</For>
                  </ul>
                </Show>
              </Show>
              <ul class="ai-runs-as" aria-label="Grants">
                <For each={taskGrants(t())}>{(g) => <li class="muted">{g}</li>}</For>
              </ul>
            </Show>
          </>
        )}
      </Show>
      <DiffLines before={p().before} after={del() ? "" : p().after} label="Task file" />
      <Show when={!del() && p().script_after}>
        {(after) => (
          <>
            <p class="muted">{`${scriptPath()}${p().script_exists ? "" : " (new)"}`}</p>
            <DiffLines before={p().script_before ?? null} after={after()} label="Script" />
          </>
        )}
      </Show>
      <Show when={del()}>
        <p class="muted">{`Its script ${scriptPath()} stays, and so does its run history.`}</p>
      </Show>
      <RunsAs
        uses={p().runs_as}
        warnings={p().warnings}
        heading={p().script_after ? `${scriptPath()} also runs as these, and the script change applies to each` : `${scriptPath()} also runs as these`}
      />
      <Show when={props.part.error}>
        <p role="alert" class="ai-error">
          {props.part.error}
        </p>
      </Show>
      <Switch>
        <Match when={open()}>
          <div class="ai-proposal-actions">
            <button type="button" class="primary" disabled={props.part.state === "applying"} onClick={() => void session.applyTaskProposal(props.turn, props.index)}>
              Apply
            </button>
            <button type="button" disabled={props.part.state === "applying"} onClick={() => session.discardTaskProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "stale"}>
          <div class="ai-proposal-actions">
            <button type="button" onClick={() => session.discardTaskProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "applied"}>
          <p role="status" class="ai-applied">
            Applied.{" "}
            <Show when={!del()}>
              <a href={taskHref(p().name)}>Open task</a>
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
