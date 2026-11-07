// The threads of a list's notes (TV-79): one GET /api/dispatched for all the rows shown, grouped
// by note, read again when the rows change, on each ai_turn, after a send, and while one is active.

import { render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RUNNING_POLL_MS, bumpTurns, resetRunning } from "./ai/running";
import { type DispatchedThread, api } from "./api";
import { FILTER_MAX, notesSent, resetDispatched, threadsByNote, watchListThreads } from "./dispatch";

function t(id: string, notes: string[], over: Partial<DispatchedThread> = {}): DispatchedThread {
  return {
    id,
    agent: "clerk",
    title: id,
    origin: "dispatch",
    notes,
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

beforeEach(() => {
  resetRunning();
  resetDispatched();
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("threadsByNote", () => {
  it("puts each thread under every listed note it is about, once, in the order given", () => {
    const a = t("A", ["n1", "n2", "n1"]);
    const b = t("B", ["n2", "other"]);
    const got = threadsByNote(["n1", "n2", "n3"], [a, b, a]);
    expect(got.get("n1")).toEqual([a]);
    expect(got.get("n2")).toEqual([a, b]);
    expect(got.has("n3")).toBe(false);
    expect(got.has("other")).toBe(false);
  });
});

describe("watchListThreads", () => {
  it("reads the threads of every row in one call and again when the rows change", async () => {
    const dispatched = vi.spyOn(api, "dispatched").mockResolvedValue({ threads: [t("A", ["n1", "n3"])] });
    const [ids, setIds] = createSignal<string[]>(["n1", "n2", "n3"]);
    let threads!: ReturnType<typeof watchListThreads>;
    const r = render(() => {
      threads = watchListThreads(ids);
      return null;
    });
    await waitFor(() => expect(threads().get("n3")?.map((x) => x.id)).toEqual(["A"]));
    expect(dispatched).toHaveBeenCalledTimes(1);
    expect(dispatched).toHaveBeenCalledWith({ notes: ["n1", "n2", "n3"] });
    expect(threads().get("n1")?.map((x) => x.id)).toEqual(["A"]);
    expect(threads().has("n2")).toBe(false);
    setIds(["n1", "n2", "n3"]);
    await Promise.resolve();
    expect(dispatched).toHaveBeenCalledTimes(1);
    setIds(["n4"]);
    await waitFor(() => expect(dispatched).toHaveBeenCalledTimes(2));
    expect(dispatched).toHaveBeenLastCalledWith({ notes: ["n4"] });
    await waitFor(() => expect(threads().size).toBe(0));
    r.unmount();
  });

  it("asks for at most the filter's limit of notes per call", async () => {
    const dispatched = vi.spyOn(api, "dispatched").mockResolvedValue({ threads: [] });
    const many = Array.from({ length: FILTER_MAX + 3 }, (_, i) => `n${i}`);
    const r = render(() => {
      watchListThreads(() => many);
      return null;
    });
    await waitFor(() => expect(dispatched).toHaveBeenCalledTimes(2));
    expect(dispatched.mock.calls[0]![0]!.notes).toHaveLength(FILTER_MAX);
    expect(dispatched.mock.calls[1]![0]!.notes).toEqual(many.slice(FILTER_MAX));
    r.unmount();
  });

  it("reads again on an ai_turn and after notes of the list were sent", async () => {
    const dispatched = vi.spyOn(api, "dispatched").mockResolvedValue({ threads: [] });
    const r = render(() => {
      watchListThreads(() => ["n1", "n2"]);
      return null;
    });
    await waitFor(() => expect(dispatched).toHaveBeenCalledTimes(1));
    bumpTurns();
    await waitFor(() => expect(dispatched).toHaveBeenCalledTimes(2));
    notesSent(["elsewhere"]);
    await Promise.resolve();
    expect(dispatched).toHaveBeenCalledTimes(2);
    notesSent(["n2"]);
    await waitFor(() => expect(dispatched).toHaveBeenCalledTimes(3));
    r.unmount();
    bumpTurns();
    await Promise.resolve();
    expect(dispatched).toHaveBeenCalledTimes(3);
  });

  it("polls while a thread of the list is active", async () => {
    vi.useFakeTimers();
    const dispatched = vi
      .spyOn(api, "dispatched")
      .mockResolvedValueOnce({ threads: [t("A", ["n1"], { state: "active", running: true })] })
      .mockResolvedValue({ threads: [t("A", ["n1"])] });
    const r = render(() => {
      watchListThreads(() => ["n1"]);
      return null;
    });
    await vi.waitFor(() => expect(dispatched).toHaveBeenCalledTimes(1));
    await vi.advanceTimersByTimeAsync(RUNNING_POLL_MS);
    expect(dispatched).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(RUNNING_POLL_MS * 3);
    expect(dispatched).toHaveBeenCalledTimes(2);
    r.unmount();
  });
});
