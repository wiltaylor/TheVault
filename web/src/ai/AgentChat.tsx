// The agent screen's chat (agents G5, G7, G8): a bar at the top with the session picker, Rename,
// Archive and Delete for the chosen session, and whatever the screen adds at its end (the
// settings cog); and the chosen session below it, filling the rest of the pane, in the same
// conversation view as the AI panel. A session is a thread the server keeps, so reopening one
// shows its turns as they were and its pending proposals with Apply and Discard again. The first
// message of a new chat starts its thread.
// The session picker lists the agent's threads, newest activity first, each with its title, when
// it was last active and a badge counting the proposals that wait for the user; a search box
// filters them by title, New chat starts afresh, and "Show archived" lists the archived threads
// too, dimmed. Rename edits the chosen thread's title inline, Archive archives or unarchives it,
// and Delete moves it to the trash (agents G15): ordinary note writes, authored by the user.
// Turns run on the server in the background (agents G17): any thread can be opened while one
// answers, a running thread shows a spinner, read as "Running", and one that answered while it
// was not open shows a "New reply" dot until opened. The list reads again when the live stream
// says a turn of the agent started or ended, and every few seconds while one runs, for the apps.
// A thread open here and in the AI panel is one conversation, shown in both (agents G18); the
// screen and the panel each choose their thread by themselves, and the screen sends no open note.

import { For, type JSX, Show, createEffect, createMemo, createSignal, createUniqueId, on, onCleanup, onMount } from "solid-js";
import { type AgentThreadSummary, api } from "../api";
import { TypeIcon } from "../components/Icon";
import { NameInput } from "../components/PinnedTree";
import { errorText } from "../pages/TypeEditor";
import { Conversation } from "./ChatPanel";
import { NewReplyMark, RunningMark } from "./RunningMark";
import { RUNNING_POLL_MS, hasNewReply, lastTurnEvent, turnEnded, turnsChanged } from "./running";
import { createChatSession } from "./store";
import "./chat.css";

/** When a thread was last active, as the list shows it. */
export function lastActive(at: string | null | undefined): string {
  if (!at) return "";
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return at;
  return d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

export { RUNNING_POLL_MS } from "./running";

/** What a thread's badge says. */
export function pendingLabel(n: number): string {
  return n === 1 ? "1 proposal waits for you" : `${n} proposals wait for you`;
}

/** The threads whose title holds every word of `query`, ignoring case, in the order given. */
export function matchThreads(threads: AgentThreadSummary[], query: string): AgentThreadSummary[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return threads.filter((t) => words.every((w) => (t.title || "Untitled thread").toLowerCase().includes(w)));
}

const titleOf = (t: AgentThreadSummary) => t.title || "Untitled thread";

/**
 * The chat of agent `agent`, whose display name is `label`. The agent screen mounts it at
 * `/agents/<name>`; `thread`, from `?thread=<id>`, opens that thread, as a trigger run's link
 * does, and `onThread` hears which thread it shows, so the address can name it. `actions` goes at
 * the end of the bar.
 */
export function AgentChat(props: {
  agent: string;
  label?: string;
  thread?: string | null;
  onThread?: (id: string | null) => void;
  actions?: JSX.Element;
}) {
  const [threads, setThreads] = createSignal<AgentThreadSummary[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const [showArchived, setShowArchived] = createSignal(false);
  const [renaming, setRenaming] = createSignal(false);
  /** The open thread when the list leaves it out, such as an archived one a link opened. */
  const [unlisted, setUnlisted] = createSignal<AgentThreadSummary | null>(null);
  let asked = 0;
  let poll: ReturnType<typeof setTimeout> | undefined;
  let picker: HTMLButtonElement | undefined;
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
  // A memo, so the agent list read again (another agent added or changed anywhere) is no new agent:
  // the list of chats is not emptied and read again under the user.
  const agentName = createMemo(() => props.agent);
  createEffect(
    on(
      agentName,
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
  createEffect(
    on(
      () => session.chat.thread,
      (id) => {
        setRenaming(false);
        props.onThread?.(id);
      },
      { defer: true },
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
  const current = createMemo(() => {
    const id = session.chat.thread;
    if (!id) return null;
    return threads().find((t) => t.id === id) ?? (unlisted()?.id === id ? unlisted() : null);
  });
  // The open thread is not in the list once it has loaded: read its title and archived mark.
  createEffect(
    on([() => session.chat.thread, loaded, threads], ([id, done, list]) => {
      if (!id || !done || list.some((t) => t.id === id) || unlisted()?.id === id) return;
      const agent = props.agent;
      api.agentThread(agent, id).then(
        (t) => {
          if (session.chat.thread === id && props.agent === agent) setUnlisted(t);
        },
        () => undefined,
      );
    }),
  );

  /** Run a thread action, then list the threads again; a failure shows above the chat. */
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
    setRenaming(false);
    queueMicrotask(() => picker?.focus());
    if (!title || title === t.title) return;
    if (unlisted()?.id === t.id) setUnlisted({ ...t, title });
    act(api.renameAgentThread(props.agent, t.id, title));
  }

  /** Archive or unarchive thread `t` through the note routes, at its current version. */
  function setArchived(t: AgentThreadSummary, archived: boolean) {
    act(
      api.agentThread(props.agent, t.id).then(async (cur) => {
        await (archived ? api.archiveNote(t.id, cur.version) : api.unarchiveNote(t.id, cur.version));
        if (session.chat.thread === t.id) setUnlisted({ ...t, archived });
      }),
    );
  }

  function remove(t: AgentThreadSummary) {
    if (!confirm(`Delete the thread "${titleOf(t)}"? It moves to the trash, where you can restore it.`)) return;
    act(
      api.deleteAgentThread(props.agent, t.id).then(() => {
        if (session.chat.thread === t.id) session.newConversation();
      }),
    );
  }

  function choose(id: string | null) {
    if (id === null) session.newConversation();
    else if (session.chat.thread !== id) void session.openThread(id);
  }

  return (
    <section class="agent-chat" aria-label="Chat">
      <div class="agent-chat-bar">
        <Show
          when={renaming() && current()}
          fallback={
            <SessionPicker
              ref={(el) => (picker = el)}
              threads={threads()}
              current={session.chat.thread}
              currentInfo={current()}
              streaming={session.chat.streaming}
              showArchived={showArchived()}
              setShowArchived={(on) => {
                setShowArchived(on);
                void refresh();
              }}
              onOpen={() => void refresh()}
              choose={choose}
            />
          }
        >
          {(t) => (
            <div class="agent-session-rename">
              <NameInput label="Thread title" value={t().title} onDone={(title) => rename(t(), title)} />
            </div>
          )}
        </Show>
        <div class="agent-chat-actions">
          <button
            type="button"
            class="agent-chat-action"
            aria-label="Rename chat"
            title="Rename chat"
            disabled={!current() || renaming()}
            onClick={() => setRenaming(true)}
          >
            <TypeIcon name="pencil" size={16} />
          </button>
          <button
            type="button"
            class="agent-chat-action"
            aria-label={current()?.archived ? "Unarchive chat" : "Archive chat"}
            title={current()?.archived ? "Take this chat out of the archive" : "Archive this chat"}
            disabled={!current()}
            onClick={() => {
              const t = current();
              if (t) setArchived(t, !t.archived);
            }}
          >
            <TypeIcon name={current()?.archived ? "archive-restore" : "archive"} size={16} />
            <span class="agent-chat-action-label">{current()?.archived ? "Unarchive" : "Archive"}</span>
          </button>
          <button
            type="button"
            class="agent-chat-action danger"
            aria-label="Delete chat"
            title="Move this chat to the trash"
            disabled={!current()}
            onClick={() => {
              const t = current();
              if (t) remove(t);
            }}
          >
            <TypeIcon name="trash-2" size={16} />
            <span class="agent-chat-action-label">Delete</span>
          </button>
        </div>
        <Show when={props.actions}>
          <div class="agent-chat-bar-end">{props.actions}</div>
        </Show>
      </div>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <div class="agent-chat-body">
        <div class="ai-panel agent-thread-view">
          <Conversation
            session={session}
            placeholder={`Ask ${name()}`}
            empty={
              <p class="muted ai-empty">
                <Show when={loaded() && threads().length === 0 && !error() && !showArchived()}>No chats yet. </Show>A new chat with{" "}
                {name()}. Edits are proposals you apply, unless {name()}'s permissions let them go ahead without asking.
              </p>
            }
          />
        </div>
      </div>
    </section>
  );
}

/**
 * The session picker: a button naming the open chat that opens a list of the agent's threads, with
 * a search box over them driven by the arrow keys, Enter and Escape (the ARIA combobox pattern).
 */
function SessionPicker(props: {
  ref: (el: HTMLButtonElement) => void;
  threads: AgentThreadSummary[];
  current: string | null;
  currentInfo: AgentThreadSummary | null;
  streaming: boolean;
  showArchived: boolean;
  setShowArchived: (on: boolean) => void;
  onOpen: () => void;
  choose: (id: string | null) => void;
}) {
  const uid = createUniqueId();
  const [open, setOpen] = createSignal(false);
  const [query, setQuery] = createSignal("");
  /** The highlighted option: -1 is New chat, else an index into the shown threads. */
  const [active, setActive] = createSignal(-1);
  let anchor: HTMLButtonElement | undefined;
  const shown = createMemo(() => matchThreads(props.threads, query()));
  const label = () => (props.current ? (props.currentInfo ? titleOf(props.currentInfo) : "Untitled thread") : "New chat");
  const optionId = (i: number) => (i < 0 ? `${uid}-new` : `${uid}-opt-${i}`);

  function show() {
    setQuery("");
    setActive(Math.max(-1, props.threads.findIndex((t) => t.id === props.current)));
    setOpen(true);
    props.onOpen();
  }
  function close(refocus: boolean) {
    setOpen(false);
    if (refocus) anchor?.focus();
  }
  function pick(id: string | null) {
    close(true);
    props.choose(id);
  }
  createEffect(
    on(query, () => setActive(shown().length > 0 && query().trim() ? 0 : -1), { defer: true }),
  );
  createEffect(() => {
    if (!open()) return;
    document.getElementById(optionId(active()))?.scrollIntoView?.({ block: "nearest" });
  });

  function onKey(e: KeyboardEvent) {
    const n = shown().length;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      // New chat, then each thread shown, round and round.
      const at = active() + 1;
      setActive(((at + (e.key === "ArrowDown" ? 1 : -1) + n + 1) % (n + 1)) - 1);
    } else if (e.key === "Home" && e.ctrlKey) {
      e.preventDefault();
      setActive(-1);
    } else if (e.key === "End" && e.ctrlKey) {
      e.preventDefault();
      setActive(n - 1);
    } else if (e.key === "Enter" && !e.isComposing) {
      e.preventDefault();
      const i = active();
      pick(i < 0 ? null : (shown()[i]?.id ?? null));
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close(true);
    }
  }

  function Popup() {
    let el!: HTMLDivElement;
    let search!: HTMLInputElement;
    onMount(() => {
      search.focus();
      const outside = (e: PointerEvent) => {
        if (!el.contains(e.target as Node) && !anchor?.contains(e.target as Node)) close(false);
      };
      document.addEventListener("pointerdown", outside, true);
      onCleanup(() => document.removeEventListener("pointerdown", outside, true));
    });
    return (
      <div
        ref={el}
        class="agent-session-popup"
        role="dialog"
        aria-label="Chat sessions"
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            close(true);
          }
        }}
        onFocusOut={(e) => {
          const to = e.relatedTarget as Node | null;
          if (to && !el.contains(to) && !anchor?.contains(to)) close(false);
        }}
      >
        <input
          ref={search}
          type="search"
          class="agent-session-search"
          role="combobox"
          aria-label="Search chats"
          aria-expanded="true"
          aria-controls={`${uid}-list`}
          aria-autocomplete="list"
          aria-activedescendant={optionId(active())}
          placeholder="Search chats"
          autocomplete="off"
          spellcheck={false}
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
          onKeyDown={onKey}
        />
        <ul class="agent-session-list" role="listbox" id={`${uid}-list`} aria-label="Chats">
          <li
            id={optionId(-1)}
            role="option"
            class="agent-session-option agent-session-new"
            classList={{ active: active() === -1 }}
            aria-selected={props.current === null}
            onPointerDown={(e) => e.preventDefault()}
            onPointerEnter={() => setActive(-1)}
            onClick={() => pick(null)}
          >
            <TypeIcon name="plus" size={16} />
            <span>New chat</span>
          </li>
          <For each={shown()}>
            {(t, i) => (
              <li
                id={optionId(i())}
                role="option"
                class="agent-session-option"
                classList={{ active: active() === i(), "agent-thread-archived": t.archived === true }}
                aria-selected={props.current === t.id}
                onPointerDown={(e) => e.preventDefault()}
                onPointerEnter={() => setActive(i())}
                onClick={() => pick(t.id)}
              >
                <span class="agent-thread-name">
                  <span class="agent-thread-title">{titleOf(t)}</span>
                  <Show when={t.running} fallback={<Show when={hasNewReply(t.id) && props.current !== t.id}><NewReplyMark /></Show>}>
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
              </li>
            )}
          </For>
        </ul>
        <Show when={shown().length === 0 && query().trim()}>
          <p class="muted agent-session-none" role="status">
            No chat matches the search.
          </p>
        </Show>
        <label class="agent-threads-archived">
          <input type="checkbox" checked={props.showArchived} onChange={(e) => props.setShowArchived(e.currentTarget.checked)} />
          Show archived
        </label>
      </div>
    );
  }

  return (
    <div class="agent-session-picker">
      <button
        ref={(el) => {
          anchor = el;
          props.ref(el);
        }}
        type="button"
        class="agent-session-button"
        aria-haspopup="dialog"
        aria-expanded={open()}
        aria-label={`Chat session: ${label()}${(props.currentInfo?.pending ?? 0) > 0 ? `, ${pendingLabel(props.currentInfo!.pending)}` : ""}`}
        title={label()}
        onClick={() => (open() ? close(false) : show())}
        onKeyDown={(e) => {
          if ((e.key === "ArrowDown" || e.key === "ArrowUp") && !open()) {
            e.preventDefault();
            show();
          }
        }}
      >
        <span class="agent-session-current">{label()}</span>
        <Show when={props.currentInfo?.archived}>
          <span class="agent-badge agent-badge-off">Archived</span>
        </Show>
        <Show when={props.current && (props.streaming || props.currentInfo?.running)}>
          <RunningMark />
        </Show>
        <Show when={(props.currentInfo?.pending ?? 0) > 0}>
          <span class="agent-thread-badge" aria-hidden="true">
            {props.currentInfo!.pending}
          </span>
        </Show>
        <TypeIcon name="chevron-down" size={14} />
      </button>
      <Show when={open()}>
        <Popup />
      </Show>
    </div>
  );
}
