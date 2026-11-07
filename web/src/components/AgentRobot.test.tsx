// The robot of a thread about a note (TV-78): moving while active, "!" while it needs the user,
// still with a dot when done until this browser opens it, and a link to the thread. The note's
// threads are read from GET /api/dispatched, again on each ai_turn, and polled while one is active.

import { fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { refreshAgents } from "../agents";
import { OPENED_KEY, bumpTurns, markOpened, resetRunning, viewThread } from "../ai/running";
import { type AgentSummary, type DispatchedThread, api } from "../api";
import { resetDispatched, watchNoteThreads } from "../dispatch";
import { AgentRobot, AgentRobots } from "./AgentRobot";

const NOTE = "01JB8Z4K2MQ7V3X9T5R6W8Y2ZB";
const AGENTS = [{ name: "clerk", label: "Clerk", icon: null, built_in: false, enabled: true }] as AgentSummary[];

function t(over: Partial<DispatchedThread> = {}): DispatchedThread {
  return {
    id: "01THREAD",
    agent: "clerk",
    title: "Roof: tidy",
    origin: "dispatch",
    notes: [NOTE],
    state: "done",
    pending: 0,
    question: null,
    last_activity: "2026-11-07T05:00:00Z",
    running: false,
    turn: null,
    started_ms: null,
    ...over,
  };
}

beforeEach(async () => {
  resetRunning();
  resetDispatched();
  vi.spyOn(api, "agents").mockResolvedValue({ agents: AGENTS, tools: [] });
  await refreshAgents();
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("the robot", () => {
  it("moves while active", () => {
    const r = render(() => <AgentRobot thread={t({ state: "active", running: true })} />);
    const robot = r.getByTestId("agent-robot");
    expect(robot).toHaveAttribute("data-state", "active");
    expect(robot).toHaveAccessibleName("Clerk is working on it");
    expect(robot).toHaveAttribute("href", "/agents/clerk?thread=01THREAD");
    r.unmount();
  });

  it("shows ! when it needs you, with the question", () => {
    const r = render(() => <AgentRobot thread={t({ state: "needs_you", question: "Which folder?" })} />);
    const robot = r.getByTestId("agent-robot");
    expect(robot).toHaveAttribute("data-state", "needs_you");
    expect(robot).toHaveTextContent("!");
    expect(robot).toHaveAccessibleName("Clerk needs you: Which folder?");
    r.unmount();
  });

  it("keeps a dot when done until it is opened, and gets it back on later activity", () => {
    const r = render(() => <AgentRobot thread={t()} />);
    const robot = r.getByTestId("agent-robot");
    expect(robot).toHaveAttribute("data-state", "done_new");
    expect(robot.querySelector(".agent-robot-dot")).not.toBeNull();
    fireEvent.click(robot);
    expect(robot).toHaveAttribute("data-state", "done");
    expect(robot.querySelector(".agent-robot-dot")).toBeNull();
    expect(JSON.parse(localStorage.getItem(OPENED_KEY) ?? "{}")).toHaveProperty("01THREAD");
    r.unmount();
    // A later reply, after the opening, brings the dot back.
    const later = t({ last_activity: new Date(Date.now() + 60_000).toISOString() });
    const again = render(() => <AgentRobot thread={later} />);
    expect(again.getByTestId("agent-robot")).toHaveAttribute("data-state", "done_new");
    again.unmount();
  });

  it("has no dot for a thread opened in a conversation", () => {
    const stop = viewThread("01THREAD");
    stop();
    const r = render(() => <AgentRobot thread={t()} />);
    expect(r.getByTestId("agent-robot")).toHaveAttribute("data-state", "done");
    r.unmount();
  });

  it("trusts the server's last activity over a clock that is behind", () => {
    markOpened("01THREAD", "2999-01-01T00:00:00Z");
    const r = render(() => <AgentRobot thread={t({ last_activity: "2999-01-01T00:00:00Z" })} />);
    expect(r.getByTestId("agent-robot")).toHaveAttribute("data-state", "done");
    r.unmount();
  });
});

describe("the robots of a note", () => {
  function Note() {
    const threads = watchNoteThreads(() => NOTE);
    return <AgentRobots threads={threads()} />;
  }

  it("reads the note's threads, again on ai_turn, and polls while one is active", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const read = vi.spyOn(api, "dispatched").mockResolvedValue({ threads: [t({ state: "active", running: true })] });
    const r = render(() => <Note />);
    await waitFor(() => expect(r.getByTestId("agent-robot")).toHaveAttribute("data-state", "active"));
    expect(read).toHaveBeenCalledWith({ notes: [NOTE] });
    // Active: read again after the poll interval, without the live stream.
    read.mockResolvedValue({ threads: [t({ state: "needs_you", pending: 1 })] });
    await vi.advanceTimersByTimeAsync(4_000);
    await waitFor(() => expect(r.getByTestId("agent-robot")).toHaveAttribute("data-state", "needs_you"));
    const calls = read.mock.calls.length;
    // Not active: no more polling, but an ai_turn reads again.
    await vi.advanceTimersByTimeAsync(20_000);
    expect(read.mock.calls.length).toBe(calls);
    read.mockResolvedValue({ threads: [t()] });
    bumpTurns();
    await waitFor(() => expect(r.getByTestId("agent-robot")).toHaveAttribute("data-state", "done_new"));
    r.unmount();
  });

  it("shows nothing for a note no thread is about", async () => {
    const read = vi.spyOn(api, "dispatched").mockResolvedValue({ threads: [] });
    const r = render(() => <Note />);
    await waitFor(() => expect(read).toHaveBeenCalled());
    expect(r.queryByTestId("agent-robots")).toBeNull();
    r.unmount();
  });
});
