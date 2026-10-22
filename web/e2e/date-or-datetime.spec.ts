import type { APIRequestContext } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// A date_range of = "date_or_datetime" field (google-sync D15): edited with an All day box, and
// found by the Search page's date filters by the days it covers in the browser's time zone.

test.use({ timezoneId: "America/New_York" });

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

async function eventType(admin: APIRequestContext): Promise<string> {
  const type = `happening_${token()}`;
  const wcl =
    `label = "Happening"\n\nfield when {\n  kind = "date_range"\n  label = "When"\n  of = "date_or_datetime"\n}\n\n` +
    `template default {\n  tab "Main" {\n    slot "title"\n    slot "when"\n  }\n}\n`;
  const put = await admin.put(`/api/types/${type}`, { data: { wcl } });
  expect(put.status(), await put.text()).toBe(200);
  return type;
}

async function create(admin: APIRequestContext, type: string, title: string, when: { start: string; end: string }) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields: { title, when }, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  return ((await res.json()) as { id: string }).id;
}

test("All day switches both ends between dates and times in the browser's zone", async ({ page, admin }) => {
  const type = await eventType(admin);
  const id = await create(admin, type, "Offsite", { start: "2026-10-20", end: "2026-10-21" });
  const when = async () =>
    ((await (await admin.get(`/api/notes/${id}`)).json()) as { note: { fields: { when?: unknown } } }).note.fields.when;
  await logIn(page, `/notes/${id}`);
  const field = page.getByTestId("field-when");
  const allDay = field.getByLabel("All day");
  await expect(allDay).toBeChecked();
  await expect(field.getByLabel("Start", { exact: true })).toHaveAttribute("type", "date");

  await allDay.uncheck();
  await expect(field.getByLabel("Start", { exact: true })).toHaveAttribute("type", "datetime-local");
  await expect.poll(when).toEqual({ start: "2026-10-20T00:00:00-04:00", end: "2026-10-21T23:59:00-04:00" });
  await field.getByLabel("End", { exact: true }).fill("2026-10-21T17:30");
  await expect.poll(when).toEqual({ start: "2026-10-20T00:00:00-04:00", end: "2026-10-21T17:30:00-04:00" });

  await allDay.check();
  await expect.poll(when).toEqual({ start: "2026-10-20", end: "2026-10-21" });
});

test("a date filter finds a timed value by its local day and a dated one by its whole day", async ({ page, admin }) => {
  const type = await eventType(admin);
  // 21:00 on the 20th in New York is 01:00 UTC on the 21st.
  await create(admin, type, "Late dinner", { start: "2026-10-20T21:00:00-04:00", end: "2026-10-20T22:30:00-04:00" });
  await create(admin, type, "Holiday", { start: "2026-10-21", end: "2026-10-21" });
  await logIn(page, `/search?type=${type}&sort=title`);
  const rows = activePane(page).getByTestId("note-row").locator(".note-title");
  await expect(rows).toHaveText(["Holiday", "Late dinner"]);

  await activePane(page).getByLabel("Add date filter").selectOption({ label: "When" });
  const filter = activePane(page).locator('.filter-row[data-field="when"]');
  await filter.getByLabel("Operator").selectOption({ label: "on" });
  await filter.getByLabel("Date", { exact: true }).fill("2026-10-20");
  await expect(rows).toHaveText(["Late dinner"]);
  await filter.getByLabel("Date", { exact: true }).fill("2026-10-21");
  await expect(rows).toHaveText(["Holiday"]);
  await filter.getByLabel("Operator").selectOption({ label: "before" });
  await filter.getByLabel("Date", { exact: true }).fill("2026-10-21");
  await expect(rows).toHaveText(["Late dinner"]);
});
