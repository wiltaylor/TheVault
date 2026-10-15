// The built-in conversation type: its page lists the thread's messages through the query view in
// its default template (D15).

import type { APIRequestContext } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

function unique(prefix: string): string {
  return `${prefix} ${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function create(admin: APIRequestContext, type: string, fields: Record<string, unknown>) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string };
}

test("a conversation's page shows its own messages, oldest first", async ({ page, admin }) => {
  const name = unique("Sam");
  const sam = await create(admin, "person", { title: name, phones: ["+61400000002"] });
  const title = unique("Thread");
  const thread = await create(admin, "conversation", { title, participants: [sam.id], channel: "sms" });
  const other = await create(admin, "conversation", { title: unique("Other thread"), channel: "sms" });
  const tag = unique("Text");
  const message = (text: string, sent: string, conversation: string) =>
    create(admin, "message", {
      title: text,
      channel: "sms",
      direction: "in",
      from: sam.id,
      from_raw: "+61400000002",
      conversation,
      sent,
    });
  await message(`${tag} second`, "2026-10-15T09:05:00Z", thread.id);
  await message(`${tag} first`, "2026-10-15T09:00:00Z", thread.id);
  await message(`${tag} elsewhere`, "2026-10-15T09:02:00Z", other.id);

  await logIn(page, `/notes/${thread.id}`);
  await expect(page.getByRole("heading", { level: 1, name: title })).toBeVisible();
  const view = page.getByTestId("query-view");
  await expect(view.getByRole("heading", { name: "Messages" })).toBeVisible();
  const rows = view.getByTestId("query-row");
  await expect(rows).toHaveCount(2);
  const titles = (await rows.getByTestId("query-title").allTextContents()).map((t) => t.trim());
  expect(titles).toEqual([`${tag} first`, `${tag} second`]);
  await expect(rows.first()).toContainText("+61400000002");
});
