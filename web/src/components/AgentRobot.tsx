// The robot a thread sent notes or called from a note shows (TV-77 D6, TV-78): it moves while a
// turn runs on the thread (`active`), carries a "!" while the agent waits for the user
// (`needs_you`: a question or a proposal to apply), and stands still once it is `done`, with a dot
// until this browser opens the thread after its last activity (TV-77 Q11), by the same rule as the
// Dispatched page's dot (`hasDot`). Clicking it opens the thread. With motion reduced, the active
// robot stands still and shows a ring instead.

import { For, Show } from "solid-js";
import { agentLabel, agents } from "../agents";
import { markOpened } from "../ai/opened";
import type { DispatchedThread } from "../api";
import { activityMs, hasDot, threadLink } from "../dispatched/model";

/** The address of thread `id` of `agent`, as the agent screen opens it. */
export const dispatchedHref = threadLink;

/** What the robot of thread `t` shows: its state, and for a done thread whether it has the dot. */
export function robotLook(t: Pick<DispatchedThread, "id" | "state" | "last_activity">): "active" | "needs_you" | "done_new" | "done" {
  if (t.state === "active") return "active";
  if (t.state === "needs_you") return "needs_you";
  return hasDot(t) ? "done_new" : "done";
}

/** The robot, drawn inline so it never waits on the icon set. */
export function RobotGlyph() {
  return (
    <svg class="icon robot-glyph" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M12 8V4H8" />
      <rect width="16" height="12" x="4" y="8" rx="2" />
      <path d="M2 14h2" />
      <path d="M20 14h2" />
      <path d="M15 13v2" />
      <path d="M9 13v2" />
    </svg>
  );
}

/** The robot of thread `t`, a link that opens it. */
export function AgentRobot(props: { thread: DispatchedThread }) {
  const who = () => {
    const a = agents().find((x) => x.name === props.thread.agent);
    return a ? agentLabel(a) : props.thread.agent;
  };
  const look = () => robotLook(props.thread);
  const label = () => {
    switch (look()) {
      case "active":
        return `${who()} is working on it`;
      case "needs_you":
        return props.thread.question ? `${who()} needs you: ${props.thread.question}` : `${who()} needs you`;
      case "done_new":
        return `${who()} is done: not opened yet`;
      default:
        return `${who()} is done`;
    }
  };
  return (
    <a
      class="agent-robot"
      href={dispatchedHref(props.thread.agent, props.thread.id)}
      data-testid="agent-robot"
      data-state={look()}
      data-thread={props.thread.id}
      aria-label={label()}
      title={`${label()} (${props.thread.title})`}
      onClick={() => markOpened(props.thread.id, Math.max(Date.now(), activityMs(props.thread)))}
    >
      <RobotGlyph />
      <Show when={look() === "needs_you"}>
        <span class="agent-robot-badge" aria-hidden="true">
          !
        </span>
      </Show>
      <Show when={look() === "done_new"}>
        <span class="agent-robot-dot" aria-hidden="true" />
      </Show>
    </a>
  );
}

/** The robots of `threads`, newest first, or nothing when there are none. */
export function AgentRobots(props: { threads: readonly DispatchedThread[] }) {
  return (
    <Show when={props.threads.length > 0}>
      <span class="agent-robots" data-testid="agent-robots">
        <For each={props.threads}>{(t) => <AgentRobot thread={t} />}</For>
      </span>
    </Show>
  );
}
