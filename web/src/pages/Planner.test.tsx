import { MemoryRouter, Route, createMemoryHistory, useLocation } from "@solidjs/router";
import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { Note } from "../api";

// The planner page: a Day | Week | Month | Quarter | Year switcher, previous, Today and next for
// the period chosen, the current period's note opened (and made) on arrival, and another
// period's offered to create.

const planner = vi.fn();
const openPlanner = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, planner, openPlanner } };
});
vi.mock("./NoteView", () => ({ NoteView: (p: { id: string }) => <p data-testid="note-view">{p.id}</p> }));

const { ApiError } = await import("../api");
const { Planner, DailyRedirect } = await import("./Planner");
const { PERIOD_KEY } = await import("../planner/period");

const note = (period: string, date: string): Note => ({ id: `${period}:${date}`, type: "x", version: "v", wcl: "", note: { fields: { date }, blocks: [] } });

beforeAll(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  // Sunday 25 October 2026, the last day of ISO week 43.
  vi.setSystemTime(new Date(2026, 9, 25, 12, 0));
});
afterAll(() => vi.useRealTimers());

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  localStorage.clear();
});

openPlanner.mockImplementation(async (p: string, d: string) => note(p, d));
planner.mockImplementation(async () => {
  throw new ApiError(404, { error: "not_found", message: "none" });
});

let path = () => "";
function Where() {
  const loc = useLocation();
  path = () => loc.pathname;
  return null;
}

function show(at: string) {
  const history = createMemoryHistory();
  history.set({ value: at });
  render(() => (
    <MemoryRouter history={history}>
      <Route path={["/planner", "/planner/:period", "/planner/:period/:date"]} component={() => (<><Where /><Planner /></>)} />
      <Route path={["/daily", "/daily/:date"]} component={DailyRedirect} />
    </MemoryRouter>
  ));
}

const pressed = () => screen.getAllByRole("button", { pressed: true }).map((b) => b.textContent);

describe("the planner", () => {
  it("opens the current day's note, making it, on /planner", async () => {
    show("/planner");
    expect(await screen.findByTestId("note-view")).toBeTruthy();
    expect(openPlanner).toHaveBeenCalledWith("day", "2026-10-25");
    expect(pressed()).toEqual(["Day"]);
  });

  it("switches period, opens the current one's note and remembers the choice", async () => {
    show("/planner/day");
    await screen.findByTestId("note-view");
    fireEvent.click(screen.getByRole("button", { name: "Week" }));
    await waitFor(() => expect(path()).toBe("/planner/week"));
    await waitFor(() => expect(openPlanner).toHaveBeenCalledWith("week", "2026-10-19"));
    expect(localStorage.getItem(PERIOD_KEY)).toBe("week");
    fireEvent.click(screen.getByRole("button", { name: "Quarter" }));
    await waitFor(() => expect(openPlanner).toHaveBeenCalledWith("quarter", "2026-10-01"));
    expect(pressed()).toEqual(["Quarter"]);
  });

  it("opens the period chosen last", async () => {
    localStorage.setItem(PERIOD_KEY, "month");
    show("/planner");
    await screen.findByTestId("note-view");
    expect(openPlanner).toHaveBeenCalledWith("month", "2026-10-01");
  });

  it("steps a period at a time and offers to create another period's note", async () => {
    show("/planner/week");
    await screen.findByTestId("note-view");
    fireEvent.click(screen.getByRole("button", { name: "Next week" }));
    await waitFor(() => expect(path()).toBe("/planner/week/2026-10-26"));
    expect(await screen.findByText("There is no note for this week.")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Week 44, 2026" })).toBeTruthy();
    expect(planner).toHaveBeenCalledWith("week", "2026-10-26");
    expect(openPlanner).not.toHaveBeenCalledWith("week", "2026-10-26");

    fireEvent.click(screen.getByRole("button", { name: "Create note for this week" }));
    await waitFor(() => expect(openPlanner).toHaveBeenCalledWith("week", "2026-10-26"));

    fireEvent.click(screen.getByRole("button", { name: "Previous week" }));
    await waitFor(() => expect(path()).toBe("/planner/week"));
    fireEvent.click(screen.getByRole("button", { name: "Previous week" }));
    await waitFor(() => expect(path()).toBe("/planner/week/2026-10-12"));
    fireEvent.click(screen.getByRole("button", { name: "Today" }));
    await waitFor(() => expect(path()).toBe("/planner/week"));
  });

  it("keeps the day shown when switching from a past period", async () => {
    show("/planner/day/2026-03-14");
    await screen.findByText("There is no note for this day.");
    fireEvent.click(screen.getByRole("button", { name: "Year" }));
    await waitFor(() => expect(path()).toBe("/planner/year"));
    fireEvent.click(screen.getByRole("button", { name: "Previous year" }));
    await waitFor(() => expect(path()).toBe("/planner/year/2025-01-01"));
    fireEvent.click(screen.getByRole("button", { name: "Month" }));
    await waitFor(() => expect(path()).toBe("/planner/month/2025-01-01"));
  });

  it("sends the old daily notes page to the planner's day", async () => {
    show("/daily/2026-03-14");
    await waitFor(() => expect(path()).toBe("/planner/day/2026-03-14"));
    cleanup();
    show("/daily");
    await waitFor(() => expect(path()).toBe("/planner/day"));
  });
});
