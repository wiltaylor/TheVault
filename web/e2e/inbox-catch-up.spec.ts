// The inbox's one-time catch-up (TV-30): pick a date, see the count, confirm, and every email,
// message and call before it is processed. Other specs share the vault, so this one works before
// 2000, where no other spec puts notes.

import type { APIRequestContext } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

async function create(admin: APIRequestContext, type: string, fields: Record<string, unknown>) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string };
}

async function processed(admin: APIRequestContext, id: string): Promise<unknown> {
  const res = await admin.get(`/api/notes/${id}`);
  return ((await res.json()) as { note: { fields: Record<string, unknown> } }).note.fields.processed;
}

test("Mark older processed counts the emails, messages and calls before a date and marks them, once", async ({ page, admin }) => {
  const tag = `Catch-up ${Date.now().toString(36)}`;
  const mail = await create(admin, "email", { title: `${tag} mail`, sent: "1999-06-01T10:00:00Z" });
  const sms = await create(admin, "message", { title: `${tag} sms`, direction: "in", sent: "1999-06-02T10:00:00Z", device_ref: `sms:${tag}` });
  const call = await create(admin, "call", { title: `${tag} call`, direction: "missed", started: "1999-06-03T10:00:00Z", device_ref: `call:${tag}` });
  const later = await create(admin, "email", { title: `${tag} later`, sent: "2000-06-01T10:00:00Z" });

  await logIn(page, "/inbox");
  await activePane(page).getByRole("button", { name: "Mark older processed…" }).click();
  const dialog = page.getByRole("dialog", { name: "Mark older items processed" });
  await expect(dialog.getByLabel("Before")).toBeFocused();
  await dialog.getByLabel("Before").fill("2000-01-01");
  await expect(dialog.getByRole("status")).toHaveText(
    "3 items sent or made before 2000-01-01 will be marked processed: 1 email, 1 message, 1 call.",
  );
  await dialog.getByRole("button", { name: "Mark 3 items processed" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.locator(".toast").filter({ hasText: "Marked 3 items processed" })).toBeVisible();

  for (const n of [mail, sms, call]) expect(await processed(admin, n.id)).toBe(true);
  expect(await processed(admin, later.id)).not.toBe(true);

  // A second time finds nothing.
  await activePane(page).getByRole("button", { name: "Mark older processed…" }).click();
  await dialog.getByLabel("Before").fill("2000-01-01");
  await expect(dialog.getByRole("status")).toHaveText("Nothing sent or made before 2000-01-01 waits to be processed.");
  await expect(dialog.getByRole("button", { name: "Mark 0 items processed" })).toBeDisabled();
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toHaveCount(0);
});
