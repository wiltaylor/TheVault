// The Dispatched page (TV-80, TV-77 D4): the sent and called threads of every agent in Active,
// Needs you and Done, a Needs you row saying the agent's question or the proposals that wait, a Done
// row keeping a dot until it is opened in this browser (TV-77 Q11), each row linking to its thread
// in the agent's chat, and the list read again when a turn starts or ends and while one runs.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DispatchedThread } from "../api";

const dispatched = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, dispatched } };
});

const { Dispatched, IDLE_POLL_MS } = await import("./Dispatched");
const { resetOpened } = await import("../ai/opened");
const { RUNNING_POLL_MS, bumpTurns, resetRunning, viewThread } = await import("../ai/running");

const HOUR = 3_600_000;
const iso = (ms: number) => new Date(ms).toISOString().replace(/\.\d+Z$/, "+00:00");

function row(over: Partial<DispatchedThread>): DispatchedThread {
  return {
    id: "01JT",
    agent: "triage",
    title: "Roof: tidy",
    origin: "dispatch",
    notes: ["01JN1"],
    state: "done",
    pending: 0,
    question: null,
    last_activity: iso(Date.now() - 60_000),
    running: false,
    turn: null,
    started_ms: null,
    ...over,
  };
}

/** Links do not navigate in the tests: the pane router takes them in the app. */
const stay = (e: Event) => e.preventDefault();

beforeEach(() => {
  resetOpened(Date.now() - HOUR);
  document.addEventListener("click", stay);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.useRealTimers();
  resetRunning();
  document.removeEventListener("click", stay);
});

const section = (name: string) => within(screen.getByRole("list", { name }));

describe("the Dispatched page", () => {
  it("links a trigger approval once with its origin, then removes it after resolution", async () => {
    dispatched.mockResolvedValue({ threads: [row({ id: "trigger-thread", origin: "trigger:daily", state: "needs_you", pending: 2, notes: [] })] });
    render(() => <Dispatched />);
    const link = await screen.findByRole("link", { name: /Trigger: daily/ });
    expect(link.getAttribute("href")).toBe("/agents/triage?thread=trigger-thread");
    expect(section("Needs you").getAllByRole("link")).toHaveLength(1);
    expect(link).toHaveTextContent("2 proposals wait for you");
    dispatched.mockResolvedValue({ threads: [] });
    bumpTurns({ agent: "triage", thread: "trigger-thread", turn: null, running: false });
    await waitFor(() => expect(screen.queryByRole("link", { name: /Trigger: daily/ })).toBeNull());
    expect(screen.getByText("Nothing waits for you.")).toBeTruthy();
  });

  it("lists the threads in Active, Needs you and Done, each linking to its thread", async () => {
    dispatched.mockResolvedValue({
      threads: [
        row({ id: "01JA", title: "Roof: tidy", state: "active", running: true, turn: "t1" }),
        row({ id: "01JQ", title: "3 notes: sort", notes: ["a", "b", "c"], state: "needs_you", question: "Which folder?" }),
        row({ id: "01JP", title: "Bills: pay", origin: "mention", state: "needs_you", pending: 2 }),
        row({ id: "01JD", title: "Trip: plan", agent: "planner", state: "done" }),
      ],
    });
    render(() => <Dispatched />);
    await waitFor(() => expect(screen.getByRole("list", { name: "Active" })).toBeTruthy());
    expect(dispatched).toHaveBeenCalledWith();

    const active = section("Active").getByRole("link", { name: /Roof: tidy/ });
    expect(active.getAttribute("href")).toBe("/agents/triage?thread=01JA");
    expect(within(active).getByRole("img", { name: "Running" })).toBeTruthy();
    expect(active).toHaveTextContent("Sent 1 note");

    const needs = section("Needs you").getAllByRole("link");
    expect(needs.map((a) => a.getAttribute("href"))).toEqual(["/agents/triage?thread=01JQ", "/agents/triage?thread=01JP"]);
    expect(needs[0]).toHaveTextContent("Which folder?");
    expect(needs[0]).toHaveTextContent("Sent 3 notes");
    expect(needs[1]).toHaveTextContent("2 proposals wait for you");
    expect(needs[1]).toHaveTextContent("Called from a note");
    expect(within(needs[0]!).getByRole("img", { name: "Needs you" })).toBeTruthy();

    const done = section("Done").getByRole("link", { name: /Trip: plan/ });
    expect(done.getAttribute("href")).toBe("/agents/planner?thread=01JD");
    expect(done).toHaveTextContent("planner");
  });

  it("says when a section is empty", async () => {
    dispatched.mockResolvedValue({ threads: [row({ state: "done" })] });
    render(() => <Dispatched />);
    expect(await screen.findByText("No agent is working on sent notes now.")).toBeTruthy();
    expect(screen.getByText("Nothing waits for you.")).toBeTruthy();
    expect(screen.queryByText("Nothing is done yet.")).toBeNull();
  });

  it("keeps a dot on a Done row until it is opened, and none on one done before", async () => {
    dispatched.mockResolvedValue({
      threads: [row({ id: "01JNEW", title: "New one" }), row({ id: "01JOLD", title: "Old one", last_activity: iso(Date.now() - 2 * HOUR) })],
    });
    render(() => <Dispatched />);
    const fresh = await waitFor(() => section("Done").getByRole("link", { name: /New one/ }));
    const old = section("Done").getByRole("link", { name: /Old one/ });
    expect(within(fresh).getByRole("img", { name: "Not opened yet" })).toBeTruthy();
    expect(within(old).queryByRole("img", { name: "Not opened yet" })).toBeNull();
    fireEvent.click(fresh);
    await waitFor(() => expect(within(fresh).queryByRole("img", { name: "Not opened yet" })).toBeNull());
  });

  it("clears the dot when the thread is opened anywhere in this browser", async () => {
    dispatched.mockResolvedValue({ threads: [row({ id: "01JNEW", title: "New one" })] });
    render(() => <Dispatched />);
    const fresh = await waitFor(() => section("Done").getByRole("link", { name: /New one/ }));
    expect(within(fresh).getByRole("img", { name: "Not opened yet" })).toBeTruthy();
    viewThread("01JNEW")();
    await waitFor(() => expect(within(fresh).queryByRole("img", { name: "Not opened yet" })).toBeNull());
  });

  it("keeps a row, and the review it holds, across a refresh that returns fresh objects", async () => {
    const waiting = () => ({ threads: [row({ id: "01JW", state: "needs_you", pending: 2, title: "Batch" })] });
    dispatched.mockResolvedValueOnce(waiting()).mockResolvedValueOnce(waiting());
    render(() => <Dispatched />);
    const before = await screen.findByRole("button", { name: "Review by kind" });
    bumpTurns();
    await waitFor(() => expect(dispatched).toHaveBeenCalledTimes(2));
    // The same element: the row was not built again, so an open preview is not lost.
    expect(screen.getByRole("button", { name: "Review by kind" })).toBe(before);
  });

  it("reads again when a turn starts or ends", async () => {
    dispatched.mockResolvedValue({ threads: [row({ state: "active", running: true })] });
    render(() => <Dispatched />);
    await waitFor(() => expect(dispatched).toHaveBeenCalledTimes(1));
    dispatched.mockResolvedValue({ threads: [row({ state: "done" })] });
    bumpTurns({ agent: "triage", thread: "01JT", turn: null, running: false });
    await waitFor(() => expect(dispatched).toHaveBeenCalledTimes(2));
    expect(await screen.findByRole("list", { name: "Done" })).toBeTruthy();
  });

  it("reads again every few seconds while a thread is active, else every half minute", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: false });
    dispatched.mockResolvedValue({ threads: [row({ state: "active", running: true })] });
    render(() => <Dispatched />);
    await vi.advanceTimersByTimeAsync(0);
    expect(dispatched).toHaveBeenCalledTimes(1);
    dispatched.mockResolvedValue({ threads: [row({ state: "done" })] });
    await vi.advanceTimersByTimeAsync(RUNNING_POLL_MS);
    expect(dispatched).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(RUNNING_POLL_MS);
    expect(dispatched).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(IDLE_POLL_MS);
    expect(dispatched).toHaveBeenCalledTimes(3);
  });

  it("shows why the list could not be read", async () => {
    dispatched.mockRejectedValue(new Error("offline"));
    render(() => <Dispatched />);
    expect((await screen.findByRole("alert")).textContent).toBe("offline");
  });
});
