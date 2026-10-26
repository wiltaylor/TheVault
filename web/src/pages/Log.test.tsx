// The Log page (activity Y4, Y5), against a fake of `GET /api/activity` (activity Y3): the
// category chips, the actor picker, the search box, pause, day groups, expanding an entry, subject
// links, an AI turn with its tool calls nested under it, infinite scroll, live entries from the
// `activity` event, the apps (polling, and the web app message when the token lacks admin), and
// the AI activity preset that Show in log opens.

import { Route, Router } from "@solidjs/router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ActivityFilter, ActivityItem, Json } from "../api";

const activity = vi.fn();
const scripts = vi.fn();
const tasks = vi.fn();
const devices = vi.fn();
const agentsList = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, activity, scripts, tasks, devices, agents: agentsList } };
});
const inApp = vi.hoisted(() => ({ now: null as null | { app: true } }));
vi.mock("../inApp", () => ({ appInfo: async () => inApp.now }));

const { Log, APP_POLL_MS, SEARCH_DELAY_MS } = await import("./Log");
const { ApiError } = await import("../api");
const { refreshAgents } = await import("../agents");
const { activityResync, liveActivity } = await import("../activity/live");

const NOW = Date.now();
const HOUR = 3_600_000;
let seq = 0;

/** An entry `ago` milliseconds old; ids grow with time, as ULIDs do. */
function entry(over: Partial<ActivityItem> & { ago?: number } = {}): ActivityItem {
  const { ago = 0, ...rest } = over;
  const at = NOW - ago;
  seq += 1;
  return {
    id: `01K${String(at).padStart(16, "0")}${String(seq).padStart(4, "0")}`,
    at_ms: at,
    kind: "note.update",
    category: "notes",
    actor_kind: "session",
    actor: "owner",
    summary: "Updated a note",
    subject_kind: null,
    subject: null,
    detail: null,
    ...rest,
  };
}

/** The fake server's log, newest first. */
let log: ActivityItem[] = [];

/** `GET /api/activity` over `log`: the filters, `before` and `limit`, newest first. */
function fakeActivity(f: ActivityFilter = {}) {
  const sorted = [...log].sort((a, b) => (a.id < b.id ? 1 : -1));
  const hits = sorted.filter(
    (i) =>
      (!f.category || i.category === f.category) &&
      (!f.actor || `${i.actor_kind}:${i.actor}` === f.actor) &&
      (!f.kind || i.kind === f.kind) &&
      (!f.q || i.summary.toLowerCase().includes(f.q.toLowerCase())) &&
      (!f.before || i.id < f.before),
  );
  const limit = f.limit ?? 100;
  const items = hits.slice(0, limit);
  return Promise.resolve({ items, next: hits.length > limit ? items[items.length - 1]!.id : null });
}

beforeEach(async () => {
  log = [];
  inApp.now = null;
  activity.mockImplementation(fakeActivity);
  scripts.mockResolvedValue(["tidy"]);
  tasks.mockResolvedValue([{ name: "nightly", next_run: null, retry: null, running: false, failing: false, latest_run: null }]);
  devices.mockResolvedValue([{ id: "d1", name: "Pixel", platform: "android", capabilities: [], token: "t", push: false, last_seen: null, created: 0 }]);
  agentsList.mockResolvedValue({
    agents: [
      { name: "assistant", label: "Assistant", icon: "bot", enabled: true, built_in: true },
      { name: "triage", label: "Inbox triage", icon: "mail", enabled: true, built_in: false },
    ],
    tools: [],
  });
  await refreshAgents();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.useRealTimers();
});

async function open(path = "/log") {
  window.history.replaceState({}, "", path);
  render(() => (
    <Router>
      <Route path="/log" component={Log} />
    </Router>
  ));
  await screen.findByRole("heading", { name: "Log" });
}

const summaries = () => [...document.querySelectorAll(".log-summary")].map((e) => e.textContent);
const chip = (name: string) => within(screen.getByRole("group", { name: "Category" })).getByRole("button", { name });
const lastCall = () => activity.mock.calls[activity.mock.calls.length - 1]![0] as ActivityFilter;

describe("the Log page", () => {
  it("lists entries newest first, grouped by day, with time, actor, category and summary", async () => {
    // Ages measured from local midnight, so the first two are today and the third yesterday
    // whatever the time of day the test runs.
    const midnight = new Date(NOW);
    midnight.setHours(0, 0, 0, 0);
    const sinceMidnight = NOW - midnight.getTime();
    log = [
      entry({ ago: Math.floor(sinceMidnight / 2), summary: "Created Groceries" }),
      entry({ ago: Math.min(1000, Math.floor(sinceMidnight / 4)), summary: "Updated Groceries" }),
      entry({ ago: sinceMidnight + 2 * HOUR, summary: "Logged in", category: "auth", kind: "auth.login" }),
    ];
    await open();
    await waitFor(() => expect(summaries()).toEqual(["Updated Groceries", "Created Groceries", "Logged in"]));
    expect(activity).toHaveBeenCalledWith({ category: null, actor: null, kind: null, q: null, limit: 100 });
    const days = screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent);
    expect(days[0]).toBe("Today");
    // The third entry is a day older: it heads a group of its own.
    expect(days).toHaveLength(2);
    const today = screen.getByRole("region", { name: "Today" });
    expect(within(today).getAllByRole("listitem")).toHaveLength(2);
    const first = today.querySelector(".log-entry")!;
    expect(first.querySelector(".log-actor")?.textContent).toBe("You");
    expect(first.querySelector(".log-category")?.textContent).toBe("Notes");
    expect(first.querySelector("time")).toBeTruthy();
  });

  it("filters by category with the chips, All first", async () => {
    log = [entry({ summary: "Edited a note" }), entry({ summary: "Searched notes", category: "ai", kind: "ai.tool", actor_kind: "agent", actor: "assistant" })];
    await open();
    await waitFor(() => expect(summaries()).toHaveLength(2));
    const names = within(screen.getByRole("group", { name: "Category" }))
      .getAllByRole("button")
      .map((b) => b.textContent);
    expect(names).toEqual(["All", "AI", "Notes", "Definitions", "Automation", "Outbox", "Sync", "Auth"]);
    expect(chip("All").getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(chip("AI"));
    await waitFor(() => expect(summaries()).toEqual(["Searched notes"]));
    expect(lastCall().category).toBe("ai");
    expect(chip("AI").getAttribute("aria-pressed")).toBe("true");
    expect(window.location.search).toContain("category=ai");
    expect(screen.getByText("AI activity")).toBeTruthy();
    fireEvent.click(chip("All"));
    await waitFor(() => expect(summaries()).toHaveLength(2));
  });

  it("offers you, each agent, script, task and device in the actor picker, and filters by one", async () => {
    log = [
      entry({ summary: "By you" }),
      entry({ summary: "By triage", actor_kind: "agent", actor: "triage", category: "ai", kind: "ai.tool" }),
      entry({ summary: "By a token", actor_kind: "token", actor: "cli" }),
    ];
    await open();
    await waitFor(() => expect(summaries()).toHaveLength(3));
    const picker = screen.getByRole("combobox", { name: "Actor" }) as HTMLSelectElement;
    await waitFor(() => expect(picker.querySelector('option[value="device:Pixel"]')).toBeTruthy());
    const options = [...picker.options].map((o) => [o.value, o.textContent]);
    expect(options).toEqual([
      ["", "Everyone"],
      ["session:owner", "You"],
      ["agent:assistant", "Assistant"],
      ["agent:triage", "Inbox triage"],
      ["handler:tidy", "Script tidy"],
      ["task:nightly", "Task nightly"],
      ["device:Pixel", "Pixel"],
      // Anyone else on the page.
      ["token:cli", "Token cli"],
    ]);
    fireEvent.change(picker, { target: { value: "agent:triage" } });
    await waitFor(() => expect(summaries()).toEqual(["By triage"]));
    expect(lastCall().actor).toBe("agent:triage");
    // An agent's badge carries its icon.
    expect(document.querySelector(".log-actor svg")).toBeTruthy();
    expect(document.querySelector(".log-actor")?.textContent).toBe("Inbox triage");
  });

  it("searches the summaries after a pause in typing, and at once on Enter", async () => {
    log = [entry({ summary: "Updated Groceries" }), entry({ summary: "Updated Roof" })];
    await open();
    await waitFor(() => expect(summaries()).toHaveLength(2));
    const box = screen.getByRole("searchbox", { name: "Search the log" });
    fireEvent.input(box, { target: { value: "roof" } });
    await new Promise((r) => setTimeout(r, SEARCH_DELAY_MS + 50));
    await waitFor(() => expect(summaries()).toEqual(["Updated Roof"]));
    expect(lastCall().q).toBe("roof");
    fireEvent.input(box, { target: { value: "groceries" } });
    fireEvent.keyDown(box, { key: "Enter" });
    await waitFor(() => expect(summaries()).toEqual(["Updated Groceries"]));
  });

  it("expands an entry to show its detail fields", async () => {
    log = [
      entry({
        summary: "Ran search_notes",
        category: "ai",
        kind: "ai.tool",
        actor_kind: "agent",
        actor: "assistant",
        detail: { tool: "search_notes", args: "groceries", error: false } as Record<string, Json>,
      }),
    ];
    await open();
    const toggle = await screen.findByRole("button", { name: /^Ran search_notes/ });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(document.querySelector(".log-detail")).toBeNull();
    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    const detail = document.querySelector(".log-detail") as HTMLElement;
    const text = [...detail.children].map((e) => e.textContent);
    expect(text).toContain("ai.tool");
    expect(text).toContain("agent:assistant");
    expect(text.slice(-6)).toEqual(["tool", "search_notes", "args", "groceries", "error", "false"]);
    fireEvent.click(toggle);
    expect(document.querySelector(".log-detail")).toBeNull();
  });

  it("links each entry to its subject", async () => {
    log = [
      entry({ summary: "note", subject_kind: "note", subject: "note:01JNOTE", detail: { title: "Groceries" } }),
      entry({ summary: "thread", category: "ai", kind: "ai.turn.end", actor_kind: "agent", actor: "triage", subject_kind: "thread", subject: "01JTHREAD" }),
      entry({ summary: "run", category: "automation", kind: "task.run", actor_kind: "task", actor: "nightly", subject_kind: "task_run", subject: "task_run:7", detail: { task: "nightly" } }),
      entry({ summary: "draft", category: "outbox", kind: "outbox.drafted", subject_kind: "outbox", subject: "outbox:01JOUT" }),
      entry({ summary: "type", category: "definitions", kind: "type.update", subject_kind: "type", subject: "type:task" }),
      entry({ summary: "plain", category: "auth", kind: "auth.login" }),
    ];
    await open();
    await waitFor(() => expect(summaries()).toHaveLength(6));
    const linkOf = (summary: string) => {
      const li = [...document.querySelectorAll(".log-entry")].find((e) => e.querySelector(".log-summary")?.textContent === summary)!;
      return li.querySelector("a.log-subject");
    };
    expect(linkOf("note")?.getAttribute("href")).toBe("/notes/01JNOTE");
    expect(linkOf("note")?.textContent).toBe("Groceries");
    expect(linkOf("thread")?.getAttribute("href")).toBe("/agents/triage?thread=01JTHREAD");
    expect(linkOf("run")?.getAttribute("href")).toBe("/settings/scheduled/nightly");
    expect(linkOf("draft")?.getAttribute("href")).toBe("/outbox?item=01JOUT");
    expect(linkOf("type")?.getAttribute("href")).toBe("/types/task");
    expect(linkOf("plain")).toBeNull();
  });

  it("nests an AI turn's tool calls under the turn", async () => {
    const ai = { category: "ai", actor_kind: "agent", actor: "assistant", subject_kind: "thread", subject: "thread:01JT" };
    log = [
      entry({ ...ai, ago: 5000, kind: "ai.turn.start", summary: "Turn started" }),
      entry({ ...ai, ago: 4000, kind: "ai.tool", summary: "search_notes groceries" }),
      entry({ ...ai, ago: 3000, kind: "ai.tool", summary: "get_note Groceries" }),
      entry({ ...ai, ago: 2000, kind: "ai.turn.end", summary: "Turn ended" }),
      // Another thread's tool call stays at the top level.
      entry({ ...ai, subject: "thread:01JOTHER", ago: 1000, kind: "ai.tool", summary: "elsewhere" }),
    ];
    await open();
    const turn = await screen.findByRole("list", { name: "In this turn" });
    expect([...turn.querySelectorAll(".log-summary")].map((e) => e.textContent)).toEqual([
      "search_notes groceries",
      "get_note Groceries",
      "Turn ended",
    ]);
    const top = [...document.querySelectorAll(".log-list > .log-entry > .log-row .log-summary")].map((e) => e.textContent);
    expect(top).toEqual(["elsewhere", "Turn started"]);
  });

  it("loads older pages as it scrolls", async () => {
    log = Array.from({ length: 150 }, (_, i) => entry({ ago: i * 1000, summary: `entry ${i}` }));
    let seen: ((entries: { isIntersecting: boolean }[]) => void) | null = null;
    class IO {
      constructor(cb: (entries: { isIntersecting: boolean }[]) => void) {
        seen = cb;
      }
      observe() {}
      disconnect() {}
    }
    vi.stubGlobal("IntersectionObserver", IO);
    try {
      await open();
      await waitFor(() => expect(summaries()).toHaveLength(100));
      expect(summaries()[0]).toBe("entry 0");
      seen!([{ isIntersecting: true }]);
      await waitFor(() => expect(summaries()).toHaveLength(150));
      expect(lastCall().before).toBe(log[99]!.id);
      expect(summaries()[149]).toBe("entry 149");
      expect(screen.queryByRole("button", { name: "Load older" })).toBeNull();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("loads an older page with Load older", async () => {
    log = Array.from({ length: 120 }, (_, i) => entry({ ago: i * 1000, summary: `entry ${i}` }));
    await open();
    fireEvent.click(await screen.findByRole("button", { name: "Load older" }));
    await waitFor(() => expect(summaries()).toHaveLength(120));
  });

  it("adds live entries from the activity event that pass the filters", async () => {
    log = [entry({ ago: 1000, summary: "Old" })];
    await open("/log?category=notes");
    await waitFor(() => expect(summaries()).toEqual(["Old"]));
    liveActivity({ item: entry({ summary: "New edit" }) });
    await waitFor(() => expect(summaries()).toEqual(["New edit", "Old"]));
    // An entry of another category is not this page's.
    liveActivity(entry({ summary: "Logged in", category: "auth", kind: "auth.login" }));
    liveActivity({ nonsense: true });
    expect(summaries()).toEqual(["New edit", "Old"]);
  });

  it("reads the newest page again when the stream comes back", async () => {
    log = [entry({ ago: 1000, summary: "Old" })];
    await open();
    await waitFor(() => expect(summaries()).toEqual(["Old"]));
    log.push(entry({ summary: "Missed while down" }));
    activityResync();
    await waitFor(() => expect(summaries()).toEqual(["Missed while down", "Old"]));
  });

  it("holds live entries while paused, and shows them on resume", async () => {
    log = [entry({ ago: 1000, summary: "Old" })];
    await open();
    await waitFor(() => expect(summaries()).toEqual(["Old"]));
    const pause = screen.getByRole("button", { name: "Pause" });
    fireEvent.click(pause);
    expect(pause.getAttribute("aria-pressed")).toBe("true");
    liveActivity(entry({ summary: "First" }));
    liveActivity(entry({ summary: "Second" }));
    expect(summaries()).toEqual(["Old"]);
    expect(screen.getByRole("button", { name: "2 new entries: show" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    expect(summaries()).toEqual(["Second", "First", "Old"]);
    expect(screen.queryByRole("button", { name: /new entr/ })).toBeNull();
  });

  it("opens on an agent's AI activity from Show in log", async () => {
    log = [
      entry({ summary: "Triage tool", category: "ai", kind: "ai.tool", actor_kind: "agent", actor: "triage" }),
      entry({ summary: "Assistant tool", category: "ai", kind: "ai.tool", actor_kind: "agent", actor: "assistant" }),
      entry({ summary: "A note edit" }),
    ];
    await open("/log?category=ai&actor=agent:triage");
    await waitFor(() => expect(summaries()).toEqual(["Triage tool"]));
    expect(activity.mock.calls[0]![0]).toMatchObject({ category: "ai", actor: "agent:triage" });
    expect(chip("AI").getAttribute("aria-pressed")).toBe("true");
    expect((screen.getByRole("combobox", { name: "Actor" }) as HTMLSelectElement).value).toBe("agent:triage");
    expect(screen.getByText("AI activity of Inbox triage")).toBeTruthy();
  });

  it("shows why the log failed to load", async () => {
    activity.mockRejectedValue(new ApiError(500, { error: "internal", message: "state.db is locked" }));
    await open();
    expect((await screen.findByRole("alert")).textContent).toBe("state.db is locked");
  });
});

describe("the Log in the apps", () => {
  it("says to open the log in the web app when the app's token may not read it", async () => {
    inApp.now = { app: true };
    activity.mockRejectedValue(new ApiError(403, { error: "needs_web_app", message: "Use the web app" }));
    await open();
    expect((await screen.findByRole("status")).textContent).toMatch(/^Open the log in the web app\./);
    expect(screen.queryByRole("group", { name: "Category" })).toBeNull();
  });

  it("reads the newest page every 10 seconds while open", async () => {
    inApp.now = { app: true };
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    log = [entry({ ago: 1000, summary: "Old" })];
    await open();
    await waitFor(() => expect(summaries()).toEqual(["Old"]));
    const calls = activity.mock.calls.length;
    log.push(entry({ summary: "Synced from the server" }));
    vi.advanceTimersByTime(APP_POLL_MS);
    await waitFor(() => expect(summaries()).toEqual(["Synced from the server", "Old"]));
    expect(activity.mock.calls.length).toBe(calls + 1);
    cleanup();
    vi.advanceTimersByTime(APP_POLL_MS * 3);
    expect(activity.mock.calls.length).toBe(calls + 1);
  });
});
