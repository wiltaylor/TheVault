import type { APIRequestContext, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// The month calendar at the top of the right sidebar beside the daily notes page or a daily note:
// today ringed, the shown day selected, a dot on each day with a note, and a click or the
// keyboard opening another day's note in the pane.

test.use({ locale: "en-AU" });

const pad = (n: number) => String(n).padStart(2, "0");

function isoDay(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** A month far from today and from other runs, so tests never share daily notes. */
function spareMonth(): { y: number; m: number; day: (d: number) => string } {
  const y = 2300 + Math.floor(Math.random() * 600);
  const m = 1 + Math.floor(Math.random() * 12);
  return { y, m, day: (d: number) => `${y}-${pad(m)}-${pad(d)}` };
}

function title(y: number, m: number): string {
  return new Date(y, m - 1, 1).toLocaleDateString("en-AU", { month: "long", year: "numeric" });
}

const calendar = (page: Page) => page.getByRole("region", { name: "Calendar" });
const cell = (page: Page, day: string) => calendar(page).locator(`[data-day="${day}"]`);

async function openDaily(admin: APIRequestContext, day: string) {
  const res = await admin.post(`/api/daily/${day}`);
  expect(res.ok(), await res.text()).toBe(true);
}

async function showSidebar(page: Page) {
  await page.getByRole("button", { name: "Show right sidebar" }).click();
  await expect(calendar(page)).toBeVisible();
}

test("on /daily the calendar shows this month from Monday with today ringed and selected", async ({ page }) => {
  const now = new Date();
  const today = isoDay(now);
  await logIn(page, "/daily");
  await showSidebar(page);
  await expect(calendar(page).getByRole("heading")).toHaveText(title(now.getFullYear(), now.getMonth() + 1));
  await expect(calendar(page).getByRole("columnheader").first()).toHaveAccessibleName("Monday");
  await expect(cell(page, today)).toHaveAttribute("aria-current", "date");
  await expect(cell(page, today).locator("xpath=..")).toHaveAttribute("aria-selected", "true");
  // Opening today makes its note, so today has a dot.
  await expect(cell(page, today)).toHaveAccessibleName(/has a note$/);
  // The calendar sits above the graph.
  const calTop = (await calendar(page).boundingBox())!.y;
  const graphTop = (await page.getByRole("region", { name: "Graph" }).boundingBox())!.y;
  expect(calTop).toBeLessThan(graphTop);
});

test("days with notes have dots, and a click opens another day and the selection follows", async ({ page, admin }) => {
  const s = spareMonth();
  await openDaily(admin, s.day(5));
  await openDaily(admin, s.day(20));
  await logIn(page, `/daily/${s.day(5)}`);
  await showSidebar(page);
  await expect(calendar(page).getByRole("heading")).toHaveText(title(s.y, s.m));
  await expect(cell(page, s.day(5))).toHaveAccessibleName(/has a note$/);
  await expect(cell(page, s.day(20))).toHaveAccessibleName(/has a note$/);
  await expect(cell(page, s.day(12))).not.toHaveAccessibleName(/has a note/);
  await expect(cell(page, s.day(20)).locator(".calendar-dot")).toHaveCount(1);
  await expect(cell(page, s.day(12)).locator(".calendar-dot")).toHaveCount(0);

  await cell(page, s.day(12)).click();
  await expect(page).toHaveURL(new RegExp(`/daily/${s.day(12)}$`));
  await expect(activePane(page).getByText("There is no note for this day.")).toBeVisible();
  await expect(cell(page, s.day(12)).locator("xpath=..")).toHaveAttribute("aria-selected", "true");
  await expect(cell(page, s.day(5)).locator("xpath=..")).toHaveAttribute("aria-selected", "false");

  // A middle-click or Ctrl+click opens a day in a background tab and leaves the pane where it is.
  const tabs = page.getByRole("tablist", { name: "Open tabs" }).getByRole("tab");
  await expect(tabs).toHaveCount(1);
  await cell(page, s.day(20)).click({ button: "middle" });
  await expect(tabs).toHaveCount(2);
  await cell(page, s.day(21)).click({ modifiers: ["ControlOrMeta"] });
  await expect(tabs).toHaveCount(3);
  await expect(page).toHaveURL(new RegExp(`/daily/${s.day(12)}$`));

  // Creating the note puts a dot on its day.
  await activePane(page).getByRole("button", { name: "Create note for this day" }).click();
  await expect(cell(page, s.day(12))).toHaveAccessibleName(/has a note$/);

  // The daily page's own arrows move the selection too.
  await activePane(page).getByRole("button", { name: "Next day" }).click();
  await expect(page).toHaveURL(new RegExp(`/daily/${s.day(13)}$`));
  await expect(cell(page, s.day(13)).locator("xpath=..")).toHaveAttribute("aria-selected", "true");

  // Next and previous month.
  const next = new Date(s.y, s.m, 1);
  const prev = new Date(s.y, s.m - 2, 1);
  await calendar(page).getByRole("button", { name: "Next month" }).click();
  await expect(calendar(page).getByRole("heading")).toHaveText(title(next.getFullYear(), next.getMonth() + 1));
  await calendar(page).getByRole("button", { name: "Previous month" }).click();
  await calendar(page).getByRole("button", { name: "Previous month" }).click();
  await expect(calendar(page).getByRole("heading")).toHaveText(title(prev.getFullYear(), prev.getMonth() + 1));
  // Today goes back to this month without opening anything.
  await calendar(page).getByRole("button", { name: "Today" }).click();
  const now = new Date();
  await expect(calendar(page).getByRole("heading")).toHaveText(title(now.getFullYear(), now.getMonth() + 1));
  await expect(page).toHaveURL(new RegExp(`/daily/${s.day(13)}$`));
});

test("the keyboard moves between days and months, and Enter opens a day", async ({ page }) => {
  const s = spareMonth();
  await logIn(page, `/daily/${s.day(10)}`);
  await showSidebar(page);
  await cell(page, s.day(10)).focus();
  await page.keyboard.press("ArrowRight");
  await expect(cell(page, s.day(11))).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(cell(page, s.day(18))).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("ArrowLeft");
  await expect(cell(page, s.day(10))).toBeFocused();

  await page.keyboard.press("PageDown");
  const next = new Date(s.y, s.m, 10);
  await expect(calendar(page).getByRole("heading")).toHaveText(title(next.getFullYear(), next.getMonth() + 1));
  await expect(cell(page, isoDay(next))).toBeFocused();
  await page.keyboard.press("PageUp");
  await expect(calendar(page).getByRole("heading")).toHaveText(title(s.y, s.m));
  await expect(cell(page, s.day(10))).toBeFocused();

  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(new RegExp(`/daily/${s.day(11)}$`));
  await expect(cell(page, s.day(11)).locator("xpath=..")).toHaveAttribute("aria-selected", "true");
});

test("a daily note opened as a note gets the calendar; another note does not", async ({ page, admin }) => {
  const s = spareMonth();
  const daily = await (await admin.post(`/api/daily/${s.day(7)}`)).json();
  const res = await admin.post("/api/notes?type=fleeting", { data: { note: { fields: { title: "Not a day" }, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  const other = ((await res.json()) as { id: string }).id;

  await logIn(page, `/notes/${daily.id}`);
  await showSidebar(page);
  await expect(cell(page, s.day(7)).locator("xpath=..")).toHaveAttribute("aria-selected", "true");

  await page.goto(`/notes/${other}`);
  await expect(page.getByRole("region", { name: "Graph" })).toBeVisible();
  await expect(calendar(page)).toHaveCount(0);
});

test("the calendar collapses and stays collapsed", async ({ page }) => {
  await logIn(page, "/daily");
  await showSidebar(page);
  const toggle = calendar(page).getByRole("button", { name: "Calendar" });
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(calendar(page).getByRole("grid")).toHaveCount(0);
  await page.reload();
  await expect(calendar(page).getByRole("button", { name: "Calendar" })).toHaveAttribute("aria-expanded", "false");
  await calendar(page).getByRole("button", { name: "Calendar" }).click();
  await expect(calendar(page).getByRole("grid")).toBeVisible();
});
