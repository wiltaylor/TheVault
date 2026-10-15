// An image from an asset field that holds nothing yet. Where the template places the field another
// way too (a message's attachments slot), the image shows nothing at all; where it does not (a
// fleeting note's image), a slim "Add image" row offers the upload and grows into a drop target
// while a file is dragged over it.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { APIRequestContext, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

function unique(prefix: string): string {
  return `${prefix} ${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function create(admin: APIRequestContext, type: string, fields: Record<string, unknown>, body?: string) {
  const blocks = body ? [{ kind: "content", labels: ["body"], blocks: [{ kind: "paragraph", fields: { text: body } }] }] : [];
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string };
}

/** A 1 by 1 red PNG. */
const PNG = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC", "base64");

/** Screenshots of `what` in the light and dark themes, into `$E2E_SHOTS` when it is set. */
async function shots(page: Page, name: string) {
  const dir = process.env.E2E_SHOTS;
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  for (const theme of ["light", "dark"]) {
    await page.evaluate((t) => (document.documentElement.dataset.theme = t), theme);
    await page.waitForTimeout(150);
    await page.screenshot({ path: join(dir, `empty-${name}-${theme}.png`) });
  }
  await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
}

test("a plain text message shows no image placeholder and no empty box", async ({ page, admin }) => {
  const name = unique("Sam");
  const sam = await create(admin, "person", { title: name, phones: ["+61400000002"] });
  const thread = await create(admin, "conversation", { title: unique("Thread"), participants: [sam.id], channel: "sms" });
  const text = unique("Running ten minutes late, see you at the cafe");
  const message = await create(
    admin,
    "message",
    {
      title: text,
      channel: "sms",
      direction: "in",
      from: sam.id,
      from_raw: "+61400000002",
      conversation: thread.id,
      sent: "2026-10-15T09:00:00Z",
    },
    text,
  );

  await logIn(page, `/notes/${message.id}`);
  const pane = activePane(page);
  await expect(page.getByLabel("Title", { exact: true })).toHaveText(text);
  await expect(pane.getByTestId("field-attachments")).toBeVisible();
  await expect(pane.getByTestId("field-image-add")).toHaveCount(0);
  await expect(pane.locator("figure.field-image")).toHaveCount(0);
  // No template block is left as an empty, padded box where the image would be.
  const empty = await pane.getByTestId("template-block").evaluateAll((els) => els.filter((e) => e.getBoundingClientRect().height > 0 && !e.textContent?.trim() && !e.querySelector("img,button,input")).length);
  expect(empty).toBe(0);
  await shots(page, "message");
});

test("a fleeting note without an image offers a slim add-image row that uploads", async ({ page, admin }) => {
  const title = unique("Idea");
  const note = await create(admin, "fleeting", { title }, "Look into a better empty state");

  await logIn(page, `/notes/${note.id}`);
  const add = activePane(page).getByTestId("field-image-add");
  await expect(add).toHaveText("Add image");
  await expect(add).toHaveAccessibleName("Add image to Images");
  // A slim row, not a large box.
  const box = (await add.boundingBox())!;
  expect(box.height).toBeLessThanOrEqual(48);
  await shots(page, "fleeting");

  // Reached from the keyboard too.
  await add.focus();
  await expect(add).toBeFocused();

  const chooser = page.waitForEvent("filechooser");
  await page.keyboard.press("Enter");
  await (await chooser).setFiles({ name: `idea-${Date.now().toString(36)}.png`, mimeType: "image/png", buffer: PNG });
  await expect(activePane(page).locator("figure.field-image img")).toHaveCount(1);
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
  await expect(activePane(page).getByTestId("field-image-add")).toHaveCount(0);
  const saved = (await (await admin.get(`/api/notes/${note.id}`)).json()) as { note: { fields: Record<string, unknown> } };
  expect(saved.note.fields.image).toHaveLength(1);
});

test("dragging a file over the slim row makes it a full drop target, and dropping uploads", async ({ page, admin }) => {
  const note = await create(admin, "fleeting", { title: unique("Drop") }, "Drop a picture here");

  await logIn(page, `/notes/${note.id}`);
  const add = activePane(page).getByTestId("field-image-add");
  await expect(add).toBeVisible();
  const slim = (await add.boundingBox())!.height;

  const data = await page.evaluateHandle((bytes) => {
    const dt = new DataTransfer();
    dt.items.add(new File([new Uint8Array(bytes)], "dropped.png", { type: "image/png" }));
    return dt;
  }, [...PNG]);
  await add.dispatchEvent("dragenter", { dataTransfer: data });
  await add.dispatchEvent("dragover", { dataTransfer: data });
  await expect(add).toHaveText(/Drop image/);
  await expect.poll(async () => (await add.boundingBox())!.height).toBeGreaterThan(slim * 2);
  await add.dispatchEvent("drop", { dataTransfer: data });
  await expect(activePane(page).locator("figure.field-image img")).toHaveCount(1);
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
});
