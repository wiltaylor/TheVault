// A sub agent's session (subagents U6), at /subagent/<agent>/<thread>: the thread a
// `start_subagent` call ran on, opened beside the chat from the call's block. It is read only:
// its messages, tool calls and results, streaming live while it runs, and the proposals it raised
// as they stand, which are applied in the chat that started it. Stop stops this sub agent alone.

import { Show, createResource, onCleanup } from "solid-js";
import { useParams } from "@solidjs/router";
import { agentLabel, agents } from "../agents";
import { api } from "../api";
import { Conversation } from "./ChatPanel";
import { createChatSession } from "./store";
import "./chat.css";

export function SubagentSession() {
  const params = useParams();
  return (
    <Show when={params.agent && params.thread ? `${params.agent}/${params.thread}` : null} keyed>
      {(key) => <Session agent={key.split("/")[0]!} thread={key.split("/")[1]!} />}
    </Show>
  );
}

function Session(props: { agent: string; thread: string }) {
  const session = createChatSession({ agent: props.agent });
  onCleanup(() => session.close());
  void session.openThread(props.thread);
  const [info] = createResource(() => api.agentThread(props.agent, props.thread));
  const label = () => {
    const a = agents().find((x) => x.name === props.agent);
    return a ? agentLabel(a) : props.agent;
  };
  return (
    <section class="ai-sub-session" aria-label="Sub agent session">
      <header class="ai-sub-session-bar">
        <h1>{info()?.title || "Sub agent"}</h1>
        <p class="muted">
          A sub agent's session, run by {label()}. Read only: its proposals are applied in the chat that started it.
        </p>
      </header>
      <Show when={info.error}>
        <p class="error" role="alert">
          {info.error instanceof Error ? info.error.message : String(info.error)}
        </p>
      </Show>
      <Conversation session={session} readOnly empty={<p class="muted">Nothing here yet.</p>} />
    </section>
  );
}
