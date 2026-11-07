// A saved script proposal's card (TV-70): the script's lines removed and added, every handler,
// task, device file and tool that runs it with what each may reach, and a warning for a run that
// may reach any host, with Apply and Discard. The proposal always waits for the user, whatever the
// agent's switches say. Apply writes nothing when the script changed since the proposal.
// A script that a device file runs also shows that it runs on the phone at its next sync, each
// paired device with its SMS send and contacts write switches, and what the code can do there,
// with what is new marked (TV-98).

import { For, Match, Show, Switch } from "solid-js";
import { RunsAs } from "./RunsAs";
import { type Part, type ScriptOnPhone, type ScriptProposal, useChatSession } from "./store";
import { DiffLines } from "./ToolProposalCard";

/** The Scripts page's editor of script `name`. */
export const scriptHref = (name: string) => `/settings/scripts/${encodeURIComponent(name)}`;

/** The card's heading, before the script's name. */
function head(p: ScriptProposal): string {
  if (p.op === "delete") return "Proposed delete of script ";
  return p.op === "create" ? "Proposed new script " : "Proposed change to script ";
}

/** A paired device's switches as one line. */
export function switchesOf(d: { sms_send: boolean; contacts_write: boolean }): string {
  return `SMS send ${d.sms_send ? "on" : "off"}, contacts write ${d.contacts_write ? "on" : "off"}`;
}

/** Where the new code runs on a phone and what it can do there (TV-98, TV-72 U2-U4). */
export function OnPhone(props: { device: ScriptOnPhone }) {
  return (
    <div class="ai-on-phone">
      <p role="note" class="ai-on-phone-note">
        <strong>{props.device.note}</strong>
      </p>
      <p class="muted">Runs on</p>
      <Show when={props.device.devices.length > 0} fallback={<p class="muted">{`No ${props.device.platforms.join(" or ")} device is paired yet.`}</p>}>
        <ul class="ai-runs-as" aria-label="Runs on">
          <For each={props.device.devices}>
            {(d) => (
              <li>
                <strong>{d.name}</strong> <span class="muted">{`${d.platform}: ${switchesOf(d)}`}</span>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <p class="muted">This code can</p>
      <Show when={props.device.can.length > 0} fallback={<p class="muted">None of: send SMS, read SMS, read calls, add or delete contacts, show notifications, write notes.</p>}>
        <ul class="ai-runs-as" aria-label="This code can">
          <For each={props.device.can}>
            {(c) => (
              <li classList={{ "ai-new": c.new }}>
                <Show when={c.new} fallback={c.label}>
                  <strong>{`New: ${c.label}`}</strong>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
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
      <Show when={!del() && p().device}>{(d) => <OnPhone device={d()} />}</Show>
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
