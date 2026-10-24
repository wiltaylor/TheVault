// An Outbox item's compact chips (outbox X14), the same on the Outbox page and the item's page:
// its status, and "AI" beside it when an agent drafted it. Who drafted an item the user made is
// not shown at all.

import { type Json } from "../api";
import { Show } from "solid-js";
import { agentLabel, agents } from "../agents";
import { STATUS_TEXT, draftingAgent, statusOf } from "./model";

/** The label of agent `name`, as the sidebar knows it, else the name. */
export function agentName(name: string): string {
  const a = agents().find((x) => x.name === name);
  return a ? agentLabel(a) : name;
}

export function OutboxChips(props: {
  fields: Record<string, Json>;
  /** What the status chip says instead of the status's own text, such as "Waiting for phone". */
  statusText?: string;
}) {
  const status = () => statusOf(props.fields);
  const agent = () => draftingAgent(props.fields);
  return (
    <span class="outbox-chips">
      <span class="outbox-chip outbox-status" data-status={status()}>
        {props.statusText ?? STATUS_TEXT[status()] ?? status()}
      </span>
      <Show when={agent()}>
        {(name) => (
          <span class="outbox-chip outbox-ai-chip" title={`Drafted by ${agentName(name())}`} aria-label={`AI: drafted by ${agentName(name())}`}>
            AI
          </span>
        )}
      </Show>
    </span>
  );
}
