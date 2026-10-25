import type { APIRequestContext } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// The `time` field kind: a 24-hour time of day with no date. Built in the type editor, set on a
// note page, shown as stored whatever the browser's locale, and filtered by the clock on Search.

// A locale whose own clock is 12-hour, so a 24-hour value on the page is the stored one.
test.use({ locale: "en-US", timezoneId: "America/New_York" });

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function typeFile(admin: APIRequestContext, name: string): Promise<string> {
  return (await admin.get(`/api/types/${name}`, { headers: { Accept: "text/plain" } })).text();
}

async function createNote(admin: APIRequestContext, type: string, fields: Record<string, unknown>) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  return ((await res.json()) as { id: string }).id;
}

async function opensOf(admin: APIRequestContext, id: string): Promise<unknown> {
  const res = await admin.get(`/api/notes/${id}`);
  return ((await res.json()) as { note: { fields: Record<string, unknown> } }).note.fields.opens;
}

test("a time field is made in the type editor, set on a note, shown as 24-hour and filtered by the clock", async ({
  page,
  admin,
}) => {
  const shop = `shop_${token()}`;
  await logIn(page, "/settings/types/new");
  await page.getByLabel("Name", { exact: true }).fill(shop);
  await page.getByLabel("Label", { exact: true }).fill("Shop");
  await page.getByRole("button", { name: "Add field" }).click();
  const row = page.getByTestId("field-row-0");
  await row.getByLabel("Name", { exact: true }).fill("opens");
  await row.getByLabel("Label", { exact: true }).fill("Opens");
  await row.getByLabel("Kind", { exact: true }).selectOption({ label: "Time" });
  await expect(row.getByLabel("Kind", { exact: true })).toHaveValue("time");
  const fallback = row.getByLabel("Default for opens");
  await expect(fallback).toHaveAttribute("type", "time");
  await fallback.fill("09:00");
  await page.getByRole("button", { name: "Create type" }).click();
  await expect(page).toHaveURL(new RegExp(`/types/${shop}$`));
  expect(await typeFile(admin, shop)).toMatch(/field opens \{\n\s+kind\s+= "time"\n\s+label\s+= "Opens"\n\s+default\s+= "09:00"\n\}/);

  const bakery = await createNote(admin, shop, { title: "Bakery", opens: "07:30" });
  await createNote(admin, shop, { title: "Bar", opens: "17:00:30" });
  const cafe = await createNote(admin, shop, { title: "Cafe", opens: "12:00" });

  // Set on the note page: the time box writes HH:MM.
  await page.goto(`/notes/${cafe}`);
  const box = page.getByTestId("field-opens").locator('input[type="time"]');
  await expect(box).toHaveValue("12:00");
  await box.fill("08:15");
  await expect.poll(() => opensOf(admin, cafe)).toBe("08:15");
  expect(await opensOf(admin, bakery)).toBe("07:30");

  // Shown as stored, 24-hour, in the type's list.
  await page.goto(`/types/${shop}`);
  const pane = activePane(page);
  await pane.getByRole("group", { name: "View" }).getByRole("button", { name: "List" }).click();
  const rows = pane.getByTestId("note-row");
  await expect(rows).toHaveCount(3);
  await expect(rows.filter({ hasText: "Bar" })).toContainText("17:00:30");
  await expect(rows.filter({ hasText: "Cafe" })).toContainText("08:15");
  await expect(pane.getByTestId("note-table")).not.toContainText(/\b(AM|PM)\b/i);

  // Filtered by the clock on the Search page, seconds or not.
  await page.goto(`/search?type=${shop}&sort=title`);
  const titles = () => activePane(page).getByTestId("note-row").locator(".note-title");
  await expect(titles()).toHaveText(["Bakery", "Bar", "Cafe"]);
  await activePane(page).getByLabel("Add property filter").selectOption({ label: "Opens" });
  const filter = activePane(page).locator('.filter-row[data-field="opens"]');
  await expect(filter.getByLabel("Operator").locator("option")).toHaveText(["at", "before", "after", "between", "is empty", "is not empty"]);
  await filter.getByLabel("Operator").selectOption({ label: "before" });
  await filter.getByLabel("Time", { exact: true }).fill("09:00");
  await expect(titles()).toHaveText(["Bakery", "Cafe"]);
  await filter.getByLabel("Operator").selectOption({ label: "after" });
  await filter.getByLabel("Time", { exact: true }).fill("17:00");
  await expect(titles()).toHaveText(["Bar"]);
  await filter.getByLabel("Operator").selectOption({ label: "at" });
  await filter.getByLabel("Time", { exact: true }).fill("08:15");
  await expect(titles()).toHaveText(["Cafe"]);
});
