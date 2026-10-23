// The agent screen's Chat tab (agents G5, G7, G8): the agent's threads on the left, newest
// activity first, each with its title, when it was last active and a badge counting the
// proposals that wait for the user; New thread; and the chosen thread on the right, in the same
// conversation view as the AI panel. A thread is a note the server keeps, so reopening one shows
// its turns as they were and its pending proposals with Apply and Discard again. The first
// message of a new thread starts it.

import { For, Show, createEffect, createSignal, on, onCleanup } from "solid-js";
import { type AgentThreadSummary, api } from "../api";
import { Conversation, ModelPicker } from "./ChatPanel";
import { createChatSession } from "./store";
import "./chat.css";

/** When a thread was last active, as the list shows it. */
export function lastActive(at: string | null | undefined): string {
  if (!at) return "";
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return at;
  return d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

/** What a thread's badge says. */
export function pendingLabel(n: number): string {
  return n === 1 ? "1 proposal waits for you" : `${n} proposals wait for you`;
}

/**
 * The Chat tab of agent `agent`, whose display name is `label`. The agent screen mounts it at
 * `/agents/<name>`; `thread`, from `?thread=<id>`, opens that thread, as a trigger run's link does.
 */
export function AgentChat(props: { agent: string; label?: string; thread?: string | null }) {
  const [threads, setThreads] = createSignal<AgentThreadSummary[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  let asked = 0;
  async function refresh() {
    const mine = ++asked;
    const agent = props.agent;
    try {
      const got = await api.agentThreads(agent);
      if (mine !== asked) return;
      setThreads(got.threads);
      setError(null);
    } catch (e) {
      if (mine !== asked) return;
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (mine === asked) setLoaded(true);
    }
  }
  const session = createChatSession({ agent: props.agent, threads: true, onChange: () => void refresh() });
  onCleanup(() => session.close());
  createEffect(
    on(
      () => props.agent,
      (agent) => {
        session.setAgent(agent);
        setThreads([]);
        setLoaded(false);
        void refresh();
      },
    ),
  );
  createEffect(
    on(
      () => props.thread,
      (thread) => {
        if (thread && session.chat.thread !== thread) void session.openThread(thread);
      },
    ),
  );
  const name = () => props.label || props.agent;
  return (
    <section class="agent-chat" aria-label="Chat">
      <aside class="agent-threads" aria-label="Threads">
        <button type="button" class="primary" onClick={() => session.newConversation()} disabled={session.chat.streaming}>
          New thread
        </button>
        <Show when={error()}>
          <p class="error" role="alert">
            {error()}
          </p>
        </Show>
        <Show when={loaded() && threads().length === 0 && !error()}>
          <p class="muted">No threads yet. Ask {name()} something to start one.</p>
        </Show>
        <ul class="agent-thread-list">
          <For each={threads()}>
            {(t) => (
              <li>
                <button
                  type="button"
                  class="agent-thread-item"
                  aria-current={session.chat.thread === t.id ? "true" : undefined}
                  disabled={session.chat.streaming}
                  onClick={() => void session.openThread(t.id)}
                >
                  <span class="agent-thread-title">{t.title || "Untitled thread"}</span>
                  <Show when={t.pending > 0}>
                    <span class="agent-thread-badge" title={pendingLabel(t.pending)} aria-label={pendingLabel(t.pending)}>
                      {t.pending}
                    </span>
                  </Show>
                  <Show when={t.last_activity}>
                    <time class="muted" datetime={t.last_activity ?? undefined}>
                      {lastActive(t.last_activity)}
                    </time>
                  </Show>
                </button>
              </li>
            )}
          </For>
        </ul>
      </aside>
      <div class="ai-panel agent-thread-view">
        <div class="ai-toolbar">
          <ModelPicker session={session} />
        </div>
        <Conversation
          session={session}
          placeholder={`Ask ${name()}`}
          empty={
            <p class="muted ai-empty">
              A new thread with {name()}. Edits are proposals you apply, unless {name()}'s permissions let them go ahead without asking.
            </p>
          }
        />
      </div>
    </section>
  );
}
