import type { APIRequestContext, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// An event series (event-series E1, E4, E5, E6): a local weekly series made from the UI, with a
// custom field added to both `event` and `event_series` from the type editor. The server makes
// its occurrences; the field carries from the series to each occurrence, except one the user
// changed by hand.

test.use({ locale: "en-AU" });

const pad = (n: number) => String(n).padStart(2, "0");

function isoDay(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

interface Occurrence {
  id: string;
  fields: Record<string, unknown>;
}

/** The occurrences of series `id`, earliest first. */
async function occurrences(admin: APIRequestContext, id: string): Promise<Occurrence[]> {
  const res = await admin.post("/api/query", { data: { query: `from event where series_ref == "${id}"`, page_size: 1000 } });
  expect(res.status(), await res.text()).toBe(200);
  const rows = ((await res.json()) as { rows: Occurrence[] }).rows;
  const start = (o: Occurrence) => String((o.fields.when as { start?: string } | undefined)?.start ?? "");
  return rows.sort((a, b) => start(a).localeCompare(start(b)));
}

async function field(admin: APIRequestContext, id: string, name: string): Promise<unknown> {
  const res = await admin.get(`/api/notes/${id}`);
  return ((await res.json()) as { note: { fields: Record<string, unknown> } }).note.fields[name];
}

async function typeFile(admin: APIRequestContext, name: string): Promise<string> {
  return (await admin.get(`/api/types/${name}`, { headers: { Accept: "text/plain" } })).text();
}

async function properties(page: Page) {
  // The panel stays open from one note to the next, so the toggle is pressed only when it is shut,
  // once the note's page has loaded and says which.
  const toggle = activePane(page).getByRole("button", { name: "Properties", exact: true });
  await expect(toggle).toBeVisible();
  if ((await toggle.getAttribute("aria-expanded")) !== "true") await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  const sidebar = page.getByRole("complementary", { name: "Properties" });
  await expect(sidebar).toBeVisible();
  return sidebar;
}

async function saved(page: Page) {
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
}

/**
 * Add text field `name` to `event_series` in the type editor, ticking "Also add to Event". Other
 * tests write event notes all the time, which can make a plan stale before it is applied, so a
 * refused apply is previewed again.
 */
async function addSharedField(page: Page, name: string) {
  await logIn(page, "/settings/types/event_series/edit");
  await page.getByRole("button", { name: "Add field" }).click();
  const row = page.locator("tbody.field-row").last();
  await row.getByLabel("Name").fill(name);
  await row.getByLabel("Also add to Event").check();
  for (let attempt = 1; ; attempt++) {
    await page.getByRole("button", { name: "Preview changes" }).click();
    await expect(page.getByText(`Also adds ${name} to Event`)).toBeVisible();
    await page.getByRole("button", { name: "Apply changes" }).click();
    const done = await page
      .waitForURL(/\/types\/event_series$/, { timeout: 10_000 })
      .then(() => true)
      .catch(() => false);
    if (done || attempt >= 4) break;
  }
  await expect(page).toHaveURL(/\/types\/event_series$/);
}

test("a local weekly series makes its occurrences, which take its custom field unless changed by hand", async ({ page, admin }) => {
  test.setTimeout(120_000);
  const tag = token();
  const room = `room_${tag}`;
  const title = `Standup ${tag}`;
  const today = isoDay(new Date());

  // The field goes on both types, from one save of the type editor (E4).
  await addSharedField(page, room);
  await expect.poll(() => typeFile(admin, "event")).toContain(`field ${room} {`);
  await expect.poll(() => typeFile(admin, "event_series")).toContain(`field ${room} {`);

  // A series repeating weekly from today, made in the new note form with the rule builder (E6).
  await page.goto("/notes/new?type=event_series");
  const pane = activePane(page);
  await pane.getByLabel("Title").fill(title);
  const rule = pane.getByTestId("field-rule");
  await rule.locator("select").first().selectOption("weekly");
  await expect(rule.getByTestId("rule-summary")).toHaveText("Every week");
  const starts = pane.getByTestId("field-starts");
  await starts.getByLabel("Start", { exact: true }).fill(today);
  await starts.getByLabel("End", { exact: true }).fill(today);
  await pane.getByRole("button", { name: "Create" }).click();
  await expect(page).toHaveURL(/\/notes\/[0-9A-Z]{26}$/);
  const series = page.url().split("/").pop()!;
  expect(await field(admin, series, "rule")).toEqual(["RRULE:FREQ=WEEKLY"]);

  // The server made the occurrences a year ahead, the first today, linked to the series (E5).
  await expect.poll(async () => (await occurrences(admin, series)).length, { timeout: 20_000 }).toBeGreaterThanOrEqual(52);
  const [first, second, third] = await occurrences(admin, series);
  expect(first!.fields.when).toEqual({ start: today, end: today });

  // They show in the planner's week like any event.
  await page.goto("/planner/week");
  const calendar = activePane(page).getByTestId("period-calendar");
  await expect(
    calendar.locator(`[data-day="${today}"] [data-testid="period-event"], [data-testid="period-event"][data-days~="${today}"]`).filter({ hasText: title }),
  ).toHaveCount(1);

  // The series' custom field reaches every occurrence (E4).
  await page.goto(`/notes/${series}`);
  let sidebar = await properties(page);
  await sidebar.getByTestId(`field-${room}`).locator("input").fill("Blue");
  await saved(page);
  await expect.poll(async () => (await occurrences(admin, series)).every((o) => o.fields[room] === "Blue"), { timeout: 20_000 }).toBe(true);

  // An occurrence's page links to its series and shows the carried value.
  await page.goto(`/notes/${second!.id}`);
  await expect(activePane(page).getByTestId("series-link")).toHaveText(title);
  sidebar = await properties(page);
  const secondRoom = sidebar.getByTestId(`field-${room}`).locator("input");
  await expect(secondRoom).toHaveValue("Blue");

  // Changed by hand, it is the occurrence's own: the next change to the series leaves it alone.
  await secondRoom.fill("Red");
  await saved(page);
  await expect.poll(() => field(admin, second!.id, "series_overrides")).toEqual([room]);

  await page.goto(`/notes/${series}`);
  sidebar = await properties(page);
  await sidebar.getByTestId(`field-${room}`).locator("input").fill("Green");
  await saved(page);
  await expect.poll(() => field(admin, third!.id, room), { timeout: 20_000 }).toBe("Green");
  expect(await field(admin, first!.id, room)).toBe("Green");
  expect(await field(admin, second!.id, room)).toBe("Red");

  // The page of the series shows the rule as words.
  await expect(activePane(page).getByTestId("field-rule").getByTestId("rule-summary")).toHaveText("Every week");
});
