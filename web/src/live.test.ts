import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { type LiveHandlers, RETRY_MS, type Source, connectLive } from "./live";

class FakeSource implements Source {
  readyState = 0;
  closed = false;
  private listeners = new Map<string, ((e: MessageEvent) => void)[]>();
  constructor(readonly url: string) {}
  addEventListener(type: string, listener: (e: MessageEvent) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }
  close() {
    this.closed = true;
    this.readyState = 2;
  }
  emit(type: string, data?: string) {
    for (const l of this.listeners.get(type) ?? []) l(new MessageEvent(type, { data }));
  }
  opened() {
    this.readyState = 1;
    this.emit("open");
  }
  /** The connection dropped; the browser will retry it. */
  dropped() {
    this.readyState = 0;
    this.emit("error");
  }
  /** The browser gave up, as on an error status. */
  refused() {
    this.readyState = 2;
    this.emit("error");
  }
}

function setup() {
  const sources: FakeSource[] = [];
  const calls: string[] = [];
  const handlers: LiveHandlers = {
    event: (name, data) => calls.push(`${name} ${JSON.stringify(data)}`),
    open: (again) => calls.push(again ? "open again" : "open"),
    down: () => calls.push("down"),
  };
  const stop = connectLive(handlers, (url) => {
    const s = new FakeSource(url);
    sources.push(s);
    return s;
  });
  const last = () => {
    const s = sources[sources.length - 1];
    if (!s) throw new Error("no source was opened");
    return s;
  };
  return { sources, calls, stop, last };
}

describe("connectLive", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("subscribes to /api/events and passes on each event's parsed data", () => {
    const { last, calls } = setup();
    expect(last().url).toBe("/api/events");
    last().opened();
    last().emit("notification.created", '{"id":"A","unread":1}');
    last().emit("note.changed", '{"id":"N","type":"task","version":"v2","deleted":false}');
    last().emit("notification.read", "not json");
    // A new entry of the activity log (activity Y3).
    last().emit("activity", '{"item":{"id":"01K","kind":"note.update"}}');
    expect(calls).toEqual([
      "open",
      'notification.created {"id":"A","unread":1}',
      'note.changed {"id":"N","type":"task","version":"v2","deleted":false}',
      'activity {"item":{"id":"01K","kind":"note.update"}}',
    ]);
  });

  it("reports a dropped stream once and says the next open follows a gap", () => {
    const { last, calls, sources } = setup();
    last().opened();
    last().dropped();
    last().dropped();
    last().opened();
    expect(calls).toEqual(["open", "down", "open again"]);
    // The browser retried the same source.
    expect(sources).toHaveLength(1);
  });

  it("opens the stream again after a refusal", () => {
    const { last, calls, sources } = setup();
    last().refused();
    expect(calls).toEqual(["down"]);
    expect(sources[0]?.closed).toBe(true);
    vi.advanceTimersByTime(RETRY_MS - 1);
    expect(sources).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(sources).toHaveLength(2);
    last().refused();
    expect(calls).toEqual(["down"]);
    vi.advanceTimersByTime(RETRY_MS);
    last().opened();
    expect(calls).toEqual(["down", "open again"]);
  });

  it("is down at once where there is no EventSource", () => {
    const calls: string[] = [];
    const stop = connectLive({ event: () => {}, open: () => calls.push("open"), down: () => calls.push("down") }, () => null);
    expect(calls).toEqual(["down"]);
    stop();
  });

  it("stops listening and retrying once stopped", () => {
    const { last, calls, stop, sources } = setup();
    last().refused();
    stop();
    vi.advanceTimersByTime(RETRY_MS * 2);
    expect(sources).toHaveLength(1);
    const second = setup();
    second.last().opened();
    second.stop();
    expect(second.last().closed).toBe(true);
    second.last().emit("note.changed", "{}");
    expect(second.calls).toEqual(["open"]);
    expect(calls).toEqual(["down"]);
  });
});
