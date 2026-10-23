// A call of an agent committed in a note (agents G19), shown as a chip: the agent's icon, the
// link text (`@<Label>` and the request), and while the call's thread runs the running spinner
// (agents G17). Until the server has seen the call it says "Sending…"; a call the server did
// not run says so. Clicking the chip opens the call's thread.

import { type JSX, Match, Switch } from "solid-js";
import { callStatus, callTarget } from "../agentCalls";
import { AGENT_ICON, agents } from "../agents";
import { RunningMark } from "../ai/RunningMark";
import { TypeIcon } from "./Icon";

/** The icon of agent `name`, as the agent list has it. */
export function AgentCallIcon(props: { agent: string }) {
  const icon = () => agents().find((a) => a.name === props.agent)?.icon || AGENT_ICON;
  return (
    <span class="agent-call-icon" aria-hidden="true">
      <TypeIcon name={icon()} size={13} />
    </span>
  );
}

/** What became of call `call`: running, still sending, or not run; nothing once it is answered and done. */
export function AgentCallState(props: { call: string }) {
  const status = () => callStatus(props.call);
  return (
    <Switch>
      <Match when={status().running}>
        <RunningMark />
      </Match>
      <Match when={status().state === "sending"}>
        <span class="agent-call-note" data-testid="agent-call-sending">
          Sending…
        </span>
      </Match>
      <Match when={status().state === "unanswered"}>
        <span class="agent-call-note" title="This agent may not be called from this note, or is disabled or missing">
          Not answered
        </span>
      </Match>
    </Switch>
  );
}

/** The chip, outside the editor: a link to the call's thread around `children`, the link text. */
export function AgentCallChip(props: { agent: string; call: string; children: JSX.Element }) {
  return (
    <a
      class="agent-call"
      href={callTarget(props.agent, props.call)}
      data-testid="agent-call"
      data-agent={props.agent}
      data-call={props.call}
      data-state={callStatus(props.call).state}
    >
      <AgentCallIcon agent={props.agent} />
      <span class="agent-call-text">{props.children}</span>
      <AgentCallState call={props.call} />
    </a>
  );
}
