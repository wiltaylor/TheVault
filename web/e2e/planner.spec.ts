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
    await expect(calendar(page).locator(`[data-day="${today}"]`).getByTestId("period-event").filter({ hasText: inside })).toBeVisible();
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
