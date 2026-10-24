import { activePane, expect, logIn, test } from "./fixtures";

// The Log (activity Y4): a note edit made while the page is open arrives on the live stream and
// shows at the top of the list, under Today; the Notes chip and the search box find it; expanding
// it shows its kind; and its link opens the note.

function unique(prefix: string): string {
  return `${prefix} ${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

test("a note edit shows in the Log as it happens", async ({ page, admin }) => {
  await logIn(page);
  await page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: "Log" }).click();
  await expect(page).toHaveURL(/\/log$/);
  const pane = activePane(page);
  await expect(pane.getByRole("heading", { name: "Log", level: 1 })).toBeVisible();
  await expect(pane.getByRole("group", { name: "Category" }).getByRole("button", { name: "All" })).toHaveAttribute("aria-pressed", "true");

  const title = unique("Logged");
  const created = await admin.post("/api/notes?type=note", { data: { note: { fields: { title }, blocks: [] } } });
  expect(created.status(), await created.text()).toBe(201);
  const note = (await created.json()) as { id: string; version: string };
  const updated = await admin.put(`/api/notes/${note.id}`, {
    data: { base_version: note.version, note: { fields: { title: `${title} edited` }, blocks: [] } },
  });
  expect(updated.status(), await updated.text()).toBe(200);

  // The edit arrives without a reload.
  const entry = pane.locator(".log-entry", { has: page.locator(".log-summary", { hasText: title }) }).first();
  await expect(entry).toBeVisible();
  await expect(pane.getByRole("region", { name: "Today" })).toContainText(title);

  await pane.getByRole("group", { name: "Category" }).getByRole("button", { name: "Notes" }).click();
  await expect(page).toHaveURL(/category=notes/);
  await pane.getByRole("searchbox", { name: "Search the log" }).fill(title);
  await pane.getByRole("searchbox", { name: "Search the log" }).press("Enter");
  await expect(page).toHaveURL(/q=/);
  await expect(entry).toBeVisible();
  await expect(pane.locator(".log-summary").filter({ hasNotText: title })).toHaveCount(0);

  await entry.locator(".log-toggle").click();
  await expect(entry.locator(".log-detail")).toContainText(/note\.(create|update)/);

  await entry.locator("a.log-subject").click();
  await expect(page).toHaveURL(new RegExp(`/notes/${note.id}$`));
});
