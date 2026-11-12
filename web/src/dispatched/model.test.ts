// The Dispatched page's model (TV-80): sections by state, what a Needs you row says, how a thread
// started, and the dot a Done thread keeps until it is opened in this browser (TV-77 Q11).

import { afterEach, describe, expect, it } from "vitest";
import type { DispatchedThread } from "../api";
import { markOpened, resetOpened } from "../ai/opened";
import { resetRunning, turnEnded, viewThread } from "../ai/running";
import { activityMs, bySection, hasDot, originText, threadLink, waitingText } from "./model";

const HOUR = 3_600_000;

function row(over: Partial<DispatchedThread> = {}): DispatchedThread {
  return {
    id: "01JT",
    agent: "triage",
    title: "Roof: tidy",
    origin: "dispatch",
    notes: ["01JN1"],
    state: "done",
    pending: 0,
    question: null,
    last_activity: null,
    running: false,
    turn: null,
    started_ms: null,
    ...over,
  };
}

const iso = (ms: number) => new Date(ms).toISOString().replace(/\.\d+Z$/, "+00:00");

afterEach(() => {
  resetRunning();
  resetOpened();
});

describe("the Dispatched model", () => {
  it("puts each thread in the section of its state, in the order given", () => {
    const s = bySection([row({ id: "a", state: "done" }), row({ id: "b", state: "active" }), row({ id: "c", state: "needs_you" }), row({ id: "d", state: "done" })]);
    expect(s.active.map((t) => t.id)).toEqual(["b"]);
    expect(s.needs_you.map((t) => t.id)).toEqual(["c"]);
    expect(s.done.map((t) => t.id)).toEqual(["a", "d"]);
  });

  it("says the agent's question, else how many proposals wait", () => {
    expect(waitingText(row({ question: "Which roof?", pending: 2 }))).toBe("Which roof?");
    expect(waitingText(row({ pending: 1 }))).toBe("1 proposal waits for you");
    expect(waitingText(row({ pending: 3 }))).toBe("3 proposals wait for you");
    expect(waitingText(row())).toBe("");
  });

  it("says how the thread started", () => {
    expect(originText(row({ notes: ["a"] }))).toBe("Sent 1 note");
    expect(originText(row({ notes: ["a", "b"] }))).toBe("Sent 2 notes");
    expect(originText(row({ origin: "mention" }))).toBe("Called from a note");
    expect(originText(row({ origin: "trigger:daily", notes: [] }))).toBe("Trigger: daily");
    expect(originText(row({ origin: "trigger:mail:arrived" }))).toBe("Trigger: mail:arrived");
  });

  it("links to the thread in the agent's chat", () => {
    expect(threadLink("my agent", "01JT")).toBe("/agents/my%20agent?thread=01JT");
  });

  it("reads the last activity, or 0 when there is none", () => {
    expect(activityMs(row({ last_activity: "2026-11-07T05:00:00+00:00" }))).toBe(Date.parse("2026-11-07T05:00:00Z"));
    expect(activityMs(row())).toBe(0);
    expect(activityMs(row({ last_activity: "soon" }))).toBe(0);
  });

  it("gives a Done thread a dot when it was active after this browser last opened it", () => {
    const now = Date.now();
    resetOpened(now - HOUR);
    const fresh = row({ id: "fresh", last_activity: iso(now - 60_000) });
    const old = row({ id: "old", last_activity: iso(now - 2 * HOUR) });
    expect(hasDot(fresh)).toBe(true);
    // Done before this browser kept the map: no dot.
    expect(hasDot(old)).toBe(false);
    // Only Done threads have one.
    expect(hasDot({ ...fresh, state: "active" })).toBe(false);
    expect(hasDot({ ...fresh, state: "needs_you" })).toBe(false);
    markOpened("fresh");
    expect(hasDot(fresh)).toBe(false);
    // Active again after it was opened, and done: the dot is back.
    expect(hasDot({ ...fresh, last_activity: iso(now + 5_000) })).toBe(true);
  });

  it("gives no dot to a thread open now, nor after it is closed, and one to a New reply", () => {
    const now = Date.now();
    resetOpened(now - HOUR);
    const t = row({ id: "open", last_activity: iso(now - 60_000) });
    const close = viewThread("open");
    expect(hasDot(t)).toBe(false);
    close();
    expect(hasDot(t)).toBe(false);
    // A turn that ended while this tab ran and nobody had it open, even before the map's start.
    const early = row({ id: "early", last_activity: iso(now - 2 * HOUR) });
    turnEnded("early");
    expect(hasDot(early)).toBe(true);
  });
});
