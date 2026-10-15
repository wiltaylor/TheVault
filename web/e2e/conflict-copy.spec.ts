// A sync conflict copy: when two devices changed the same note, the sync engine keeps the
// server's version and saves the device's as a copy whose `conflict_of` field names the original
// (D20). The copy's page says so and links to the original.

import type { APIRequestContext } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

async function create(admin: APIRequestContext, fields: Record<string, unknown>): Promise<string> {
  const res = await admin.post("/api/notes?type=task", { data: { note: { fields, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  return ((await res.json()) as { id: string }).id;
}

test("a conflict copy shows a link to its original", async ({ page, admin }) => {
  const word = token();
  const original = await create(admin, { title: `Plan ${word}` });
  const copy = await create(admin, { title: `Plan ${word} (conflict, Pixel)`, conflict_of: original });

  await logIn(page, `/notes/${copy}`);
  const pane = activePane(page);
  await expect(pane.getByRole("heading", { level: 1, name: `Plan ${word} (conflict, Pixel)` })).toBeVisible();
  const banner = pane.getByRole("region", { name: "Conflict copy" });
  await expect(banner).toBeVisible();
  const link = banner.getByRole("link", { name: `Plan ${word}` });
  await expect(link).toHaveAttribute("href", `/notes/${original}`);

  await link.click();
  await expect(pane.getByRole("heading", { level: 1, name: `Plan ${word}` })).toBeVisible();
  // The original is not a copy.
  await expect(pane.getByRole("region", { name: "Conflict copy" })).toHaveCount(0);
});
