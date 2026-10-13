import type { APIRequestContext } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function createTask(admin: APIRequestContext, title: string) {
  const res = await admin.post("/api/notes?type=task", { data: { note: { fields: { title }, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string; version: string };
}

async function unpinAll(admin: APIRequestContext) {
  const { pins } = (await (await admin.get("/api/pins")).json()) as { pins: { id: string }[] };
  for (const p of pins) await admin.delete(`/api/pins/${p.id}`);
}

test.describe.configure({ mode: "serial" });

test("pinning a note puts it in the sidebar, on every device, until it is unpinned", async ({ page, admin, browser }) => {
  await unpinAll(admin);
  const title = `Pinned ${token()}`;
  const note = await createTask(admin, title);

  await logIn(page, `/notes/${note.id}`);
  await expect(page.getByRole("navigation", { name: "Pinned" })).toHaveCount(0);
  const pin = page.getByRole("button", { name: "Pin", exact: true });
  await expect(pin).toHaveAttribute("aria-pressed", "false");
  await pin.click();

  const pinned = page.getByRole("navigation", { name: "Pinned" });
  await expect(pinned.getByRole("link", { name: title })).toBeVisible();
  await expect(page.getByRole("button", { name: "Unpin" })).toHaveAttribute("aria-pressed", "true");

  // The pin is on the server: a fresh browser, as another device would be, sees it.
  const other = await browser.newContext();
  const otherPage = await other.newPage();
  await logIn(otherPage, "/types");
  await otherPage.getByRole("navigation", { name: "Pinned" }).getByRole("link", { name: title }).click();
  await expect(otherPage).toHaveURL(new RegExp(`/notes/${note.id}$`));
  await other.close();

  await page.goto("/types");
  await pinned.getByRole("link", { name: title }).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${note.id}$`));
  await page.getByRole("button", { name: "Unpin" }).click();
  await expect(page.getByRole("navigation", { name: "Pinned" })).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole("button", { name: "Pin", exact: true })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Pinned" })).toHaveCount(0);
});

test("a pinned note's new title shows in the sidebar after it is saved", async ({ page, admin }) => {
  await unpinAll(admin);
  const title = `Rename me ${token()}`;
  const note = await createTask(admin, title);
  const res = await admin.put(`/api/pins/${note.id}`);
  expect(res.status(), await res.text()).toBe(200);

  await logIn(page, `/notes/${note.id}`);
  const pinned = page.getByRole("navigation", { name: "Pinned" });
  await expect(pinned.getByRole("link", { name: title })).toBeVisible();

  await page.getByRole("button", { name: "Edit" }).click();
  const renamed = `Renamed ${token()}`;
  await page.getByLabel("Title").fill(renamed);
  await page.getByRole("button", { name: "Save" }).click();
  await expect(pinned.getByRole("link", { name: renamed })).toBeVisible();
  await expect(pinned.getByRole("link", { name: title })).toHaveCount(0);
  await unpinAll(admin);
});
