// The built-in phone data types: a conversation's page lists the thread's messages through the
// query view in its default template (D15), and a message shows the pictures among its attachments.

import type { APIRequestContext, Locator } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

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

/** A 1 by 1 red PNG. */
const PNG = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC", "base64");

async function upload(admin: APIRequestContext, filename: string, data: Buffer): Promise<string> {
  const res = await admin.post(`/api/assets?filename=${filename}`, { data });
  expect(res.status(), await res.text()).toBe(201);
  return ((await res.json()) as { id: string }).id;
}

const loaded = (img: Locator) =>
  img.evaluate((el) => (el as HTMLImageElement).complete && (el as HTMLImageElement).naturalWidth > 0);

test("a message's image attachment shows as a picture on its page and its card; other files stay files", async ({ page, admin }) => {
  const tag = unique("Picture");
  const photo = await upload(admin, `image000001-${Date.now().toString(36)}.png`, PNG);
  const card = await upload(admin, `contact-${Date.now().toString(36)}.vcf`, Buffer.from("BEGIN:VCARD\nEND:VCARD\n"));
  const message = await create(admin, "message", {
    title: tag,
    channel: "mms",
    direction: "in",
    from_raw: "+61400000009",
    sent: "2026-10-15T10:00:00Z",
    attachments: [photo, card],
  });

  await logIn(page, `/notes/${message.id}`);
  await expect(page.getByLabel("Title", { exact: true })).toHaveText(tag);
  const shown = activePane(page).getByRole("img", { name: "Picture sent with the message" });
  await expect(shown).toHaveCount(1);
  await expect(shown).toHaveAttribute("src", new RegExp(photo));
  await expect.poll(() => loaded(shown)).toBe(true);
  // Every attachment, the picture too, stays in the field's chips; only the picture is drawn.
  await expect(activePane(page).getByTestId("field-attachments").locator(".chip")).toHaveCount(2);

  // The type page shows it too, in the details view the message type opens in (lists L5).
  await page.goto("/types/message");
  await activePane(page).getByRole("searchbox", { name: "Search messages" }).fill(tag);
  const found = activePane(page).getByTestId("type-detail").filter({ hasText: tag });
  await expect(found).toHaveCount(1);
  const cover = found.getByRole("img", { name: "Picture sent with the message" });
  await expect(cover).toHaveCount(1);
  await expect.poll(() => loaded(cover)).toBe(true);
});
