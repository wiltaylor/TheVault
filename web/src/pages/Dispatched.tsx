// The Dispatched page, `/dispatched` (TV-80, TV-77 D4): the threads of notes sent to an agent and
// of @ calls in notes, of every agent, in three sections. Active: a turn runs on the thread now.
// Needs you: the agent asked a question or a proposal waits, and the row says which. Done: the
// rest; a Done row keeps a dot until the thread is opened, kept per browser (TV-77 Q11). A row
// opens its thread in the agent's chat, where Apply, Discard and the reply box work. The agent
// chat lists leave these threads out. The page reads `GET /api/dispatched` again when a turn
// starts or ends, when a note changes elsewhere, every few seconds while a thread is active, and
// otherwise every half minute, as the apps, which may have no live stream, need.

import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { AGENT_ICON, agentLabel, agents } from "../agents";
import { type DispatchedThread, api } from "../api";
import { RunningMark } from "../ai/RunningMark";
import { markOpened } from "../ai/opened";
import { RUNNING_POLL_MS, turnsChanged } from "../ai/running";
import { onNoteChangedElsewhere } from "../changes";
import { TypeIcon } from "../components/Icon";
import { SECTIONS, activityMs, bySection, hasDot, originText, threadLink, waitingText } from "../dispatched/model";
import { relativeTime } from "../notifications/model";
import { errorText } from "./TypeEditor";
import "../dispatched/dispatched.css";
import { ApplyKind } from "../dispatched/ApplyKind";

/** How often the page reads again while no thread is active. */
export const IDLE_POLL_MS = 30_000;

function labelOf(name: string): string {
  const a = agents().find((x) => x.name === name);
  return a ? agentLabel(a) : name;
}

function iconOf(name: string): string {
  return agents().find((x) => x.name === name)?.icon || AGENT_ICON;
}

export function Dispatched() {
  const [threads, setThreads] = createSignal<DispatchedThread[]>([]);
  const [error, setError] = createSignal<string | null>(null);
  const [loaded, setLoaded] = createSignal(false);
  const [now, setNow] = createSignal(Date.now());
  let asked = 0;
  let poll: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(poll));

  async function refresh() {
    const mine = ++asked;
    clearTimeout(poll);
    try {
      const got = await api.dispatched();
      if (mine !== asked) return;
      setThreads(got.threads);
      setError(null);
    } catch (e) {
      if (mine !== asked) return;
      setError(errorText(e));
    } finally {
      if (mine === asked) {
        setLoaded(true);
        setNow(Date.now());
        const busy = threads().some((t) => t.state === "active");
        poll = setTimeout(() => void refresh(), busy ? RUNNING_POLL_MS : IDLE_POLL_MS);
      }
    }
  }

  onMount(() => void refresh());
  createEffect(on(turnsChanged, () => void refresh(), { defer: true }));
  onCleanup(onNoteChangedElsewhere(() => void refresh()));

  const sections = () => bySection(threads());
  // Each read gives fresh objects, and For keys by identity: key the rows by thread ID, so a
  // row and the review it holds (ApplyKind) survive a refresh (TV-274).
  const byId = createMemo(() => new Map(threads().map((t) => [t.id, t])));

  function Row(props: { t: DispatchedThread }) {
    const t = () => props.t;
    const waiting = () => (t().state === "needs_you" ? waitingText(t()) : "");
    return (
      <li class="dispatched-row" classList={{ unopened: hasDot(t()) }}>
        <a href={threadLink(t().agent, t().id)} class="dispatched-link" onClick={() => markOpened(t().id)}>
          <span class="dispatched-icon">
            <TypeIcon name={iconOf(t().agent)} />
          </span>
          <span class="dispatched-main">
            <span class="dispatched-title">{t().title || "Untitled thread"}</span>
            <span class="dispatched-meta muted">
              {labelOf(t().agent)} · {originText(t())}
              <Show when={activityMs(t())}>{(ms) => <> · {relativeTime(ms(), now())}</>}</Show>
            </span>
            <Show when={waiting()}>{(w) => <span class="dispatched-waiting">{w()}</span>}</Show>
          </span>
          <Show when={t().state === "active"}>
            <RunningMark />
          </Show>
          <Show when={t().state === "needs_you"}>
            <span class="dispatched-needs" role="img" aria-label="Needs you" title="Needs you">
              !
            </span>
          </Show>
          <Show when={hasDot(t())}>
            <span class="ai-new-reply" role="img" aria-label="Not opened yet" title="Not opened yet" />
          </Show>
        </a>
        <Show when={t().state === "needs_you" && t().pending > 0}>
          <ApplyKind agent={t().agent} thread={t().id} />
        </Show>
      </li>
    );
  }

  return (
    <section class="list-page dispatched-page">
      <header class="note-header">
        <h1>Dispatched</h1>
      </header>
      <p class="muted dispatched-intro">Sent notes, @ calls and trigger proposals waiting for your review.</p>
      <Show when={error()}>
        {(e) => (
          <p class="error" role="alert">
            {e()}
          </p>
        )}
      </Show>
      <Show when={loaded()}>
        <For each={SECTIONS}>
          {(s) => (
            <section class="dispatched-section" aria-labelledby={`dispatched-${s.state}`}>
              <h2 id={`dispatched-${s.state}`}>
                {s.label}
                <Show when={sections()[s.state].length}>{(n) => <span class="muted"> ({n()})</span>}</Show>
              </h2>
              <Show when={sections()[s.state].length > 0} fallback={<p class="muted">{s.empty}</p>}>
                <ul class="dispatched-list" aria-label={s.label}>
                  <For each={sections()[s.state].map((t) => t.id)}>{(id) => <Row t={byId().get(id)!} />}</For>
                </ul>
              </Show>
            </section>
          )}
        </For>
      </Show>
    </section>
  );
}
