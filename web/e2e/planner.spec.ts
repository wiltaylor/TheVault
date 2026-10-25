import type { APIRequestContext, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// The planner: a Day | Week | Month | Quarter | Year switcher over one note per period, each
// period note with a calendar of the events that overlap its period.

test.use({ locale: "en-AU" });

const pad = (n: number) => String(n).padStart(2, "0");

function isoDay(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

async function event(admin: APIRequestContext, title: string, start: string, end: string): Promise<string> {
  const res = await admin.post("/api/notes?type=event", { data: { note: { fields: { title, when: { start, end } }, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  return ((await res.json()) as { id: string }).id;
}

const switcher = (page: Page) => activePane(page).getByRole("group", { name: "Period" });
const calendar = (page: Page) => activePane(page).getByTestId("period-calendar");

/** The events a calendar shows on `day`: in its cell or column, or spanning it in a week's all-day row. */
const onDay = (page: Page, day: string) =>
  calendar(page).locator(`[data-day="${day}"] [data-testid="period-event"], [data-testid="period-event"][data-days~="${day}"]`);

/**
 * The note the planner made for the period holding today, as the server has it. The page makes
 * it as the period opens, so this waits until it is there.
 */
async function made(admin: APIRequestContext, period: string, today: string): Promise<{ id: string; title: string }> {
  const path = `/api/planner/${period}/${today}`;
  await expect.poll(async () => (await admin.get(path)).status()).toBe(200);
  const body = (await (await admin.get(path)).json()) as { id: string; note: { fields: { title: string } } };
  return { id: body.id, title: body.note.fields.title };
}

test("each period opens its current note, whose calendar shows the events in the period and no others", async ({ page, admin }) => {
  const now = new Date();
  const today = isoDay(now);
  const tag = `P${Math.random().toString(36).slice(2, 8)}`;
  const inside = `${tag} inside`;
  const outside = `${tag} outside`;
  const insideId = await event(admin, inside, today, today);
  // Before this year began: outside this week, month, quarter and year alike.
  const before = isoDay(new Date(now.getFullYear() - 1, 11, 20));
  await event(admin, outside, before, before);

  await logIn(page, "/planner");
  await expect(switcher(page).getByRole("button", { name: "Day", exact: true })).toHaveAttribute("aria-pressed", "true");

  for (const [label, period] of [
    ["Week", "week"],
    ["Month", "month"],
    ["Quarter", "quarter"],
  ] as const) {
    await switcher(page).getByRole("button", { name: label, exact: true }).click();
    await expect(page).toHaveURL(new RegExp(`/planner/${period}$`));
    await expect(switcher(page).getByRole("button", { name: label, exact: true })).toHaveAttribute("aria-pressed", "true");
    await expect(calendar(page)).toBeVisible();
    const note = await made(admin, period, today);
    await expect(activePane(page).getByRole("heading", { name: note.title, exact: true })).toBeVisible();
    await expect(onDay(page, today).filter({ hasText: inside })).toBeVisible();
    await expect(calendar(page).getByText(outside)).toHaveCount(0);
  }

  await switcher(page).getByRole("button", { name: "Year", exact: true }).click();
  await expect(page).toHaveURL(/\/planner\/year$/);
  const year = await made(admin, "year", today);
  expect(year.title).toBe(String(now.getFullYear()));
  await expect(activePane(page).getByRole("heading", { name: year.title, exact: true })).toBeVisible();
  const day = calendar(page).locator(`[data-testid="period-year-day"][data-day="${today}"]`);
  await expect(day.getByTestId("period-count")).toBeVisible();
  await day.click();
  const picked = calendar(page).getByTestId("period-picked");
  await expect(picked.getByTestId("period-event").filter({ hasText: inside })).toBeVisible();
  await expect(calendar(page).getByText(outside)).toHaveCount(0);
  // An event opens on click.
  await picked.getByTestId("period-event").filter({ hasText: inside }).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${insideId}$`));

  // The planner remembers the period chosen last.
  await page.goto("/planner");
  await expect(switcher(page).getByRole("button", { name: "Year", exact: true })).toHaveAttribute("aria-pressed", "true");
});

test("a day of a week's calendar opens that day's daily note", async ({ page }) => {
  const today = isoDay(new Date());
  await logIn(page, "/planner/week");
  await expect(calendar(page)).toBeVisible();
  const days = calendar(page).getByTestId("period-day");
  await expect(days).toHaveCount(7);
  await expect(days.first()).toHaveAccessibleName(/^Monday /);
  await calendar(page).locator(`[data-day="${today}"]`).getByTestId("period-day").click();
  await expect(page).toHaveURL(/\/planner\/day$/);
  await expect(activePane(page).getByRole("navigation", { name: "Day" }).getByLabel("Day", { exact: true })).toHaveValue(today);
  await expect(activePane(page).getByText("Daily note", { exact: true })).toBeVisible();
});

test.describe("a week's time grid", () => {
  // A zone without daylight saving, so the times written are the times shown.
  test.use({ timezoneId: "Australia/Brisbane", viewport: { width: 1400, height: 900 } });

  test("shows seven equal day columns, timed events by the hour and all-day ones above", async ({ page, admin }) => {
    // A Monday early in a year no other test uses.
    const jan = new Date(Date.UTC(3000 + Math.floor(Math.random() * 900), 0, 7));
    const monday = jan.getTime() - ((jan.getUTCDay() + 6) % 7) * 86_400_000;
    const day = (i: number) => new Date(monday + i * 86_400_000).toISOString().slice(0, 10);
    const at = (i: number, time: string) => `${day(i)}T${time}:00+10:00`;
    const tag = `W${Math.random().toString(36).slice(2, 8)}`;
    await event(admin, `${tag} standup`, at(0, "09:00"), at(0, "10:00"));
    await event(admin, `${tag} review`, at(0, "09:30"), at(0, "11:00"));
    await event(admin, `${tag} gym`, at(2, "06:00"), at(2, "07:00"));
    await event(admin, `${tag} holiday`, day(3), day(3));
    await event(admin, `${tag} trip`, day(4), day(6));

    await logIn(page, `/planner/week/${day(0)}`);
    await activePane(page).getByRole("button", { name: "Create note for this week" }).click();
    await expect(calendar(page).getByTestId("period-week-grid")).toBeVisible();

    // Seven columns of one width, each under its own header.
    const cols = calendar(page).locator(".week-col");
    await expect(cols).toHaveCount(7);
    const boxes = await Promise.all(Array.from({ length: 7 }, (_, i) => cols.nth(i).boundingBox()));
    const widths = boxes.map((b) => b!.width);
    expect(Math.max(...widths) - Math.min(...widths)).toBeLessThanOrEqual(1);
    const heads = calendar(page).locator(".week-head-day");
    for (let i = 0; i < 7; i++) {
      const head = (await heads.nth(i).boundingBox())!;
      expect(Math.abs(head.x - boxes[i]!.x)).toBeLessThanOrEqual(1);
    }

    // The two that overlap sit side by side, each half the column.
    const standup = onDay(page, day(0)).filter({ hasText: `${tag} standup` });
    const review = onDay(page, day(0)).filter({ hasText: `${tag} review` });
    const s = (await standup.boundingBox())!;
    const r = (await review.boundingBox())!;
    expect(r.x).toBeGreaterThan(s.x + s.width - 2);
    expect(Math.abs(s.width - r.width)).toBeLessThanOrEqual(1);
    expect(s.width).toBeLessThan(widths[0]! / 2);
    // Half an hour later starts half an hour row lower; an hour and a half is half again as tall.
    expect(r.height / s.height).toBeCloseTo(1.5, 1);
    // The early event widens the hours shown to 06:00, and the grid opens there.
    await expect(calendar(page).locator(".week-hour").first()).toHaveText("06:00");
    const gym = (await onDay(page, day(2)).filter({ hasText: `${tag} gym` }).boundingBox())!;
    const scroller = (await calendar(page).locator(".week-scroll").boundingBox())!;
    expect(gym.y).toBeGreaterThanOrEqual(scroller.y);
    expect(gym.y + gym.height).toBeLessThanOrEqual(scroller.y + scroller.height);

    // All-day events span their days in the row above.
    const holiday = onDay(page, day(3)).filter({ hasText: `${tag} holiday` });
    await expect(holiday).toHaveAttribute("data-days", day(3));
    const trip = onDay(page, day(5)).filter({ hasText: `${tag} trip` });
    await expect(trip).toHaveAttribute("data-days", `${day(4)} ${day(5)} ${day(6)}`);
    const t = (await trip.boundingBox())!;
    expect(Math.abs(t.x - boxes[4]!.x)).toBeLessThanOrEqual(4);
    expect(Math.abs(t.x + t.width - (boxes[6]!.x + boxes[6]!.width))).toBeLessThanOrEqual(4);

    // A click opens the event.
    await review.click();
    await expect(page).toHaveURL(/\/notes\/[0-9A-Z]+$/);
  });
});

test("another period with no note offers to create one", async ({ page, admin }) => {
  const year = 2200 + Math.floor(Math.random() * 700);
  const start = `${year}-04-01`;
  await logIn(page, `/planner/quarter/${year}-05-17`);
  await expect(page).toHaveURL(new RegExp(`/planner/quarter/${year}-05-17$`));
  await expect(activePane(page).getByText("There is no note for this quarter.")).toBeVisible();
  await expect(activePane(page).getByRole("heading", { name: `Q2 ${year}` })).toBeVisible();
  expect((await admin.get(`/api/planner/quarter/${start}`)).status()).toBe(404);
  await activePane(page).getByRole("button", { name: "Create note for this quarter" }).click();
  await expect(calendar(page)).toBeVisible();
  const body = (await (await admin.get(`/api/planner/quarter/${year}-06-30`)).json()) as { note: { fields: { title: string; date: string } } };
  expect(body.note.fields.date).toBe(start);
  expect(body.note.fields.title).toBe(`Q2 ${year}`);
  await activePane(page).getByRole("button", { name: "Next quarter" }).click();
  await expect(page).toHaveURL(new RegExp(`/planner/quarter/${year}-07-01$`));
});

test("the old daily notes addresses open the planner's day", async ({ page }) => {
  await logIn(page, "/daily/2031-03-14");
  await expect(page).toHaveURL(/\/planner\/day\/2031-03-14$/);
  await expect(activePane(page).getByRole("navigation", { name: "Day" }).getByLabel("Day", { exact: true })).toHaveValue("2031-03-14");
  await page.goto("/daily");
  await expect(page).toHaveURL(/\/planner\/day$/);
});

test("the sidebar calendar links each week number to its weekly note and the month to its monthly note", async ({ page }) => {
  const s = { y: 2300 + Math.floor(Math.random() * 600), m: 1 + Math.floor(Math.random() * 12) };
  const first = `${s.y}-${pad(s.m)}-01`;
  await logIn(page, `/planner/day/${s.y}-${pad(s.m)}-10`);
  await page.getByRole("button", { name: "Show right sidebar" }).click();
  const side = page.getByRole("region", { name: "Calendar" });
  await expect(side).toBeVisible();
  const weeks = side.locator("[data-week]");
  // The second week lies wholly in the month, so the calendar stays on it.
  const monday = (await weeks.nth(1).getAttribute("data-week"))!;
  await expect(weeks.nth(1)).toHaveAccessibleName(/^Week \d+, \d+$/);
  await weeks.nth(1).click();
  await expect(page).toHaveURL(new RegExp(`/planner/week/${monday}$`));
  await expect(activePane(page).getByText("There is no note for this week.")).toBeVisible();
  await side.getByTestId("calendar-month-link").click();
  await expect(page).toHaveURL(new RegExp(`/planner/month/${first}$`));
  await expect(activePane(page).getByText("There is no note for this month.")).toBeVisible();
});
