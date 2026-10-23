// The agent screen's Chat tab (agents G5, G7, G8): the agent's threads on the left, newest
// activity first, each with its title, when it was last active and a badge counting the
// proposals that wait for the user; New thread; and the chosen thread on the right, in the same
// conversation view as the AI panel. A thread is a note the server keeps, so reopening one shows
// its turns as they were and its pending proposals with Apply and Discard again. The first
// message of a new thread starts it. Each thread's ⋯ menu (or a right-click) renames it inline,
// archives or unarchives it, and deletes it to the trash (agents G15): ordinary note writes,
// authored by the user. Archived threads are hidden until "Show archived" lists them, dimmed.
// Turns run on the server in the background (agents G17): any thread can be opened while one
// answers, a running thread shows a spinner, read as "Running", and one that answered while it
// was not open shows a "New reply" dot until opened. The list reads again when the live stream
// says a turn of the agent started or ended, and every few seconds while one runs, for the apps.

import { For, Show, createEffect, createSignal, on, onCleanup, onMount } from "solid-js";
import { type AgentThreadSummary, api } from "../api";
import { TypeIcon } from "../components/Icon";
import { NameInput } from "../components/PinnedTree";
import { errorText } from "../pages/TypeEditor";
import { Conversation, ModelPicker } from "./ChatPanel";
import { NewReplyMark, RunningMark } from "./RunningMark";
import { hasNewReply, lastTurnEvent, turnEnded, turnsChanged } from "./running";
import { createChatSession } from "./store";
import "./chat.css";

/** When a thread was last active, as the list shows it. */
export function lastActive(at: string | null | undefined): string {
  if (!at) return "";
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return at;
  return d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

/** How often the list reads again while a thread of it runs (agents G17). */
export const RUNNING_POLL_MS = 4_000;

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
  const [showArchived, setShowArchived] = createSignal(false);
  const [menu, setMenu] = createSignal<string | null>(null);
  const [renaming, setRenaming] = createSignal<string | null>(null);
  let asked = 0;
  let poll: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(poll));
  async function refresh() {
    const mine = ++asked;
    const agent = props.agent;
    try {
      const got = await (showArchived() ? api.agentThreads(agent, true) : api.agentThreads(agent));
      if (mine !== asked) return;
      // A thread that stopped running since the last read answered: a new reply, unless open.
      const was = new Set(threads().filter((t) => t.running).map((t) => t.id));
      for (const t of got.threads) if (was.has(t.id) && !t.running) turnEnded(t.id);
      setThreads(got.threads);
      setError(null);
      clearTimeout(poll);
      if (got.threads.some((t) => t.running)) poll = setTimeout(() => void refresh(), RUNNING_POLL_MS);
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
  // A turn of this agent started or ended somewhere.
  createEffect(
    on(
      turnsChanged,
      () => {
        const e = lastTurnEvent();
        if (!e || e.agent === props.agent) void refresh();
      },
      { defer: true },
    ),
  );
  const name = () => props.label || props.agent;
  const titleOf = (t: AgentThreadSummary) => t.title || "Untitled thread";

  /** Run a thread action, then list the threads again; a failure shows above the list. */
  function act(p: Promise<unknown>) {
    setError(null);
    p.then(
      () => refresh(),
      async (e: unknown) => {
        await refresh();
        setError(errorText(e));
      },
    );
  }

  function rename(t: AgentThreadSummary, title: string | null) {
    setRenaming(null);
    if (!title || title === t.title) return;
    act(api.renameAgentThread(props.agent, t.id, title));
  }

  /** Archive or unarchive thread `t` through the note routes, at its current version. */
  function setArchived(t: AgentThreadSummary, archived: boolean) {
    setMenu(null);
    act(
      api.agentThread(props.agent, t.id).then((cur) => (archived ? api.archiveNote(t.id, cur.version) : api.unarchiveNote(t.id, cur.version))),
    );
  }

  function remove(t: AgentThreadSummary) {
    setMenu(null);
    if (!confirm(`Delete the thread "${titleOf(t)}"? It moves to the trash, where you can restore it.`)) return;
    act(
      api.deleteAgentThread(props.agent, t.id).then(() => {
        if (session.chat.thread === t.id) session.newConversation();
      }),
    );
  }

  function Menu(p: { thread: AgentThreadSummary; anchor: () => HTMLButtonElement | undefined }) {
    let el!: HTMLDivElement;
    const close = (refocus: boolean) => {
      setMenu(null);
      if (refocus) p.anchor()?.focus();
    };
    const items = () => [...el.querySelectorAll<HTMLElement>("[role=menuitem]")];
    onMount(() => {
      items()[0]?.focus();
      const outside = (e: PointerEvent) => {
        if (!el.contains(e.target as Node) && !p.anchor()?.contains(e.target as Node)) close(false);
      };
      document.addEventListener("pointerdown", outside, true);
      onCleanup(() => document.removeEventListener("pointerdown", outside, true));
    });
    function onKey(e: KeyboardEvent) {
      const list = items();
      const at = list.indexOf(document.activeElement as HTMLElement);
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        close(true);
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        list[(at + (e.key === "ArrowDown" ? 1 : -1) + list.length) % list.length]?.focus();
      } else if (e.key === "Tab") {
        close(false);
      }
    }
    const t = () => p.thread;
    return (
      <div
        ref={el}
        class="pin-menu"
        role="menu"
        aria-label={`Actions for ${titleOf(t())}`}
        onKeyDown={onKey}
        onPointerDown={(e) => e.stopPropagation()}
      >
        <button
          type="button"
          role="menuitem"
          onClick={() => {
            setMenu(null);
            setRenaming(t().id);
          }}
        >
          Rename
        </button>
        <button type="button" role="menuitem" onClick={() => setArchived(t(), !t().archived)}>
          {t().archived ? "Unarchive" : "Archive"}
        </button>
        <button type="button" role="menuitem" class="danger" onClick={() => remove(t())}>
          Delete
        </button>
      </div>
    );
  }
  return (
    <section class="agent-chat" aria-label="Chat">
      <aside class="agent-threads" aria-label="Threads">
        <button type="button" class="primary" onClick={() => session.newConversation()}>
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
            {(t) => {
              let more: HTMLButtonElement | undefined;
              return (
                <li>
                  <div
                    class="agent-thread-row"
                    classList={{ "menu-open": menu() === t.id, "agent-thread-archived": t.archived === true }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setMenu(t.id);
                    }}
                  >
                    <Show
                      when={renaming() === t.id}
                      fallback={
                        <button
                          type="button"
                          class="agent-thread-item"
                          aria-current={session.chat.thread === t.id ? "true" : undefined}
                          onClick={() => {
                            if (session.chat.thread !== t.id) void session.openThread(t.id);
                          }}
                        >
                          <span class="agent-thread-name">
                            <span class="agent-thread-title">{titleOf(t)}</span>
                            <Show when={t.running} fallback={<Show when={hasNewReply(t.id) && session.chat.thread !== t.id}><NewReplyMark /></Show>}>
                              <RunningMark />
                            </Show>
                          </span>
                          <Show when={t.archived}>
                            <span class="visually-hidden"> (archived)</span>
                          </Show>
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
                      }
                    >
                      <NameInput label="Thread title" value={t.title} onDone={(title) => rename(t, title)} />
                    </Show>
                    <button
                      ref={more}
                      type="button"
                      class="pin-more agent-thread-more"
                      aria-label={`Actions for ${titleOf(t)}`}
                      aria-haspopup="menu"
                      aria-expanded={menu() === t.id}
                      title="More actions"
                      onClick={() => setMenu(menu() === t.id ? null : t.id)}
                    >
                      <TypeIcon name="ellipsis" size={16} />
                    </button>
                    <Show when={menu() === t.id}>
                      <Menu thread={t} anchor={() => more} />
                    </Show>
                  </div>
                </li>
              );
            }}
          </For>
        </ul>
        <label class="agent-threads-archived">
          <input
            type="checkbox"
            checked={showArchived()}
            onChange={(e) => {
              setShowArchived(e.currentTarget.checked);
              void refresh();
            }}
          />
          Show archived
        </label>
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
