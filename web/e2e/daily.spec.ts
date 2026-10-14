import { expect, logIn, test } from "./fixtures";

function isoDay(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** A day far from today and from other runs, so tests never share a daily note. */
function spareDay(): string {
  const year = 2200 + Math.floor(Math.random() * 700);
  const month = 1 + Math.floor(Math.random() * 12);
  const day = 1 + Math.floor(Math.random() * 28);
  return `${year}-${String(month).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
}

test("logging in opens today's daily note, made once however often it is opened", async ({ page, admin }) => {
  const today = isoDay(new Date());
  await logIn(page);
  await expect(page).toHaveURL(/\/daily$/);
  await expect(page.getByRole("navigation", { name: "Day" }).getByLabel("Day", { exact: true })).toHaveValue(today);
  await expect(page.getByText("Daily note", { exact: true })).toBeVisible();

  const first = await (await admin.get(`/api/daily/${today}`)).json();
  expect(first.note.fields.date).toBe(today);
  await page.reload();
  await expect(page.getByText("Daily note", { exact: true })).toBeVisible();
  const again = await (await admin.get(`/api/daily/${today}`)).json();
  expect(again.id).toBe(first.id);
  const count = await (
    await admin.post("/api/query", { data: { query: `from dailynote where date == "${today}"`, page_size: 10 } })
  ).json();
  expect(count.total).toBe(1);
});

test("another day with no note offers to create one, and the arrows step a day at a time", async ({ page, admin }) => {
  const day = spareDay();
  await logIn(page, `/daily/${day}`);
  await expect(page.getByText("There is no note for this day.")).toBeVisible();
  expect((await admin.get(`/api/daily/${day}`)).status()).toBe(404);

  await page.getByRole("button", { name: "Create note for this day" }).click();
  await expect(page.getByText("Daily note", { exact: true })).toBeVisible();
  const made = await admin.get(`/api/daily/${day}`);
  expect(made.status()).toBe(200);
  const body = await made.json();
  await expect(page.getByRole("heading", { name: body.note.fields.title })).toBeVisible();

  await page.getByRole("button", { name: "Next day" }).click();
  const [y, m, d] = day.split("-").map(Number);
  const next = isoDay(new Date(y!, m! - 1, d! + 1));
  await expect(page).toHaveURL(new RegExp(`/daily/${next}$`));
  await expect(page.getByText("There is no note for this day.")).toBeVisible();
  await page.getByRole("button", { name: "Previous day" }).click();
  await expect(page).toHaveURL(new RegExp(`/daily/${day}$`));
  await expect(page.getByRole("heading", { name: body.note.fields.title })).toBeVisible();

  await page.getByRole("navigation", { name: "Day" }).getByLabel("Day", { exact: true }).fill(next);
  await expect(page).toHaveURL(new RegExp(`/daily/${next}$`));
  await page.getByRole("button", { name: "Today" }).click();
  await expect(page).toHaveURL(/\/daily$/);
});

test("the daily note type can be edited but has no delete button", async ({ page }) => {
  await logIn(page, "/settings/types/dailynote/edit");
  await expect(page.getByText("Built in: customise it freely, but it cannot be deleted.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Delete type" })).toHaveCount(0);
});
