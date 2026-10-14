// Fleeting notes: the capture dialog opened from anywhere (the + button, the sidebar, Ctrl+Shift+K),
// the inbox it saves to, the sidebar's count of waiting notes, and processing a note from the
// inbox: done (with undo), convert to another type, delete.

import { deflateSync } from "node:zlib";
import type { APIRequestContext, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

function crc32(buf: Buffer): number {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff]! ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(kind: string, data: Buffer): Buffer {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(kind, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

/** A `w` by `h` PNG shaded from colour `a` to `b`. */
function gradientPng(w: number, h: number, a: [number, number, number], b: [number, number, number]): Buffer {
  const rows: Buffer[] = [];
  for (let y = 0; y < h; y++) {
    const row = Buffer.alloc(1 + w * 3);
    for (let x = 0; x < w; x++) {
      const t = (x / w + y / h) / 2;
      for (let i = 0; i < 3; i++) row[1 + x * 3 + i] = Math.round(a[i]! + (b[i]! - a[i]!) * t);
    }
    rows.push(row);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(Buffer.concat(rows))),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

const dialog = (page: Page) => page.getByRole("dialog", { name: "Capture" });
const captureText = (page: Page) => dialog(page).getByRole("textbox", { name: "Capture text" });
const inboxLink = (page: Page) => page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: /^Inbox/ });
const inboxItem = (page: Page, title: string) =>
  activePane(page).getByRole("list", { name: "Fleeting notes" }).getByRole("listitem").filter({ has: page.getByRole("link", { name: title, exact: true }) });

/** Paste `png` into the capture dialog as the clipboard's image. */
async function pasteImage(page: Page, png: Buffer, name = "") {
  await captureText(page).evaluate(
    (el, { bytes, name }) => {
      const file = new File([new Uint8Array(bytes)], name, { type: "image/png" });
      const data = new DataTransfer();
      data.items.add(file);
      el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
    },
    { bytes: [...png], name },
  );
}

async function inboxTotal(admin: APIRequestContext): Promise<number> {
  const res = await admin.post("/api/query", { data: { query: "from fleeting where !processed", page_size: 1 } });
  return ((await res.json()) as { total: number }).total;
}

async function fleeting(admin: APIRequestContext, title: string, body: string, image?: string): Promise<{ id: string; version: string }> {
  const res = await admin.post("/api/notes?type=fleeting", {
    data: {
      note: {
        fields: { title, ...(image ? { image: [image] } : {}) },
        blocks: [{ kind: "content", labels: ["body"], blocks: [{ kind: "paragraph", fields: { text: body } }] }],
      },
    },
  });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string; version: string };
}

test("Ctrl+Shift+K, the + button and the sidebar open the capture dialog from any page", async ({ page }) => {
  await logIn(page, "/daily");
  for (const path of ["/daily", "/search", "/trash", "/settings/types", "/types/task"]) {
    await page.goto(path);
    await expect(page.getByRole("button", { name: "Quick capture" })).toBeVisible();
    await page.keyboard.press("Control+Shift+K");
    await expect(dialog(page)).toBeVisible();
    await expect(captureText(page)).toBeFocused();
    await expect(dialog(page).getByRole("button", { name: "Save" })).toBeDisabled();
    // The + hides while the dialog shows.
    await expect(page.getByRole("button", { name: "Quick capture" })).toBeHidden();
    await page.keyboard.press("Escape");
    await expect(dialog(page)).toHaveCount(0);

    await page.getByRole("button", { name: "Quick capture" }).click();
    await expect(captureText(page)).toBeFocused();
    await dialog(page).getByRole("button", { name: "Close" }).click();
    await expect(dialog(page)).toHaveCount(0);
    // Focus goes back where it was.
    await expect(page.getByRole("button", { name: "Quick capture" })).toBeFocused();
  }
  await page.getByRole("navigation", { name: "Main" }).getByRole("button", { name: "Capture" }).click();
  await expect(captureText(page)).toBeFocused();
  // Tab stays inside the dialog.
  for (let i = 0; i < 6; i++) await page.keyboard.press("Tab");
  expect(await page.evaluate(() => !!document.activeElement?.closest("[role=dialog]"))).toBe(true);
  // A click on the backdrop closes an empty capture.
  await page.mouse.click(5, 5);
  await expect(dialog(page)).toHaveCount(0);
});

test("a text capture is saved to the inbox, counted in the sidebar, and opens from the toast", async ({ page, admin }) => {
  const t = token();
  const title = `Call the plumber ${t}`;
  await logIn(page, "/daily");
  await page.keyboard.press("Control+Shift+K");
  await captureText(page).fill(`  ${title}  \nabout the leak under the sink`);
  await page.keyboard.press("Control+Enter");
  await expect(dialog(page)).toHaveCount(0);
  const toast = page.locator(".toast").filter({ hasText: "Saved to inbox" });
  await expect(toast).toBeVisible();

  const found = await admin.post("/api/query", { data: { query: `from fleeting where title == "${title}"`, page_size: 5 } });
  const rows = ((await found.json()) as { rows: { id: string; fields: Record<string, unknown> }[] }).rows;
  expect(rows).toHaveLength(1);
  expect(rows[0]!.fields.processed).toBe(false);
  const note = await (await admin.get(`/api/notes/${rows[0]!.id}`)).json();
  expect(note.note.blocks[0].blocks[0].fields.text).toBe(`  ${title}  \nabout the leak under the sink`);

  // The badge shows the inbox's count.
  await expect.poll(async () => (await inboxLink(page).textContent())?.replace("Inbox", "").trim()).toBe(String(await inboxTotal(admin)));

  await toast.getByRole("link", { name: "Open" }).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${rows[0]!.id}$`));
  await expect(activePane(page).getByText("about the leak under the sink")).toBeVisible();

  await inboxLink(page).click();
  await expect(page).toHaveURL(/\/inbox$/);
  const item = inboxItem(page, title);
  await expect(item).toBeVisible();
  await expect(item.getByText("about the leak under the sink")).toBeVisible();
});

test("a pasted and an attached image are saved with a title from the date", async ({ page, admin }) => {
  await logIn(page, "/inbox");
  const before = await inboxTotal(admin);
  await page.getByRole("button", { name: "Quick capture" }).click();
  await pasteImage(page, gradientPng(120, 80, [37, 99, 235], [236, 72, 153]));
  const thumbs = dialog(page).getByRole("list", { name: "Attached images" }).getByRole("img");
  await expect(thumbs).toHaveCount(1);
  await dialog(page).locator("input[type=file]").setInputFiles({
    name: "camera.png",
    mimeType: "image/png",
    buffer: gradientPng(90, 120, [22, 163, 74], [250, 204, 21]),
  });
  await expect(thumbs).toHaveCount(2);
  await dialog(page).getByRole("button", { name: "Remove image 2" }).click();
  await expect(thumbs).toHaveCount(1);
  await dialog(page).locator("input[type=file]").setInputFiles({
    name: "camera.png",
    mimeType: "image/png",
    buffer: gradientPng(90, 120, [22, 163, 74], [250, 204, 21]),
  });
  await expect(thumbs).toHaveCount(2);
  await expect(dialog(page).getByRole("button", { name: "Save" })).toBeEnabled();
  await dialog(page).getByRole("button", { name: "Save" }).click();
  await expect(dialog(page)).toHaveCount(0);
  await expect.poll(() => inboxTotal(admin)).toBe(before + 1);

  const res = await admin.post("/api/query", { data: { query: "from fleeting sort created desc", page_size: 1 } });
  const row = ((await res.json()) as { rows: { id: string; title: string; fields: Record<string, unknown> }[] }).rows[0]!;
  expect(row.title).toMatch(/^Image \d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);
  expect(row.fields.image).toHaveLength(2);
  const item = activePane(page).locator(`[data-note="${row.id}"]`);
  await expect(item.locator(".inbox-thumbs img")).toHaveCount(2);
  await expect
    .poll(() => item.locator(".inbox-thumbs img").first().evaluate((img: HTMLImageElement) => img.complete && img.naturalWidth > 0))
    .toBe(true);
});

test("Done takes a note out of the inbox and Undo puts it back", async ({ page, admin }) => {
  const title = `Read the lease ${token()}`;
  const { id } = await fleeting(admin, title, "Clause 7 on pets");
  await logIn(page, "/inbox");
  const item = inboxItem(page, title);
  await expect(item).toBeVisible();
  await item.getByRole("button", { name: "Done" }).click();
  await expect(item).toHaveCount(0);
  await expect.poll(async () => (await (await admin.get(`/api/notes/${id}`)).json()).note.fields.processed).toBe(true);

  // Shown again among the processed.
  await page.getByLabel("Show processed").check();
  await expect(inboxItem(page, title).getByText("Processed", { exact: true })).toBeVisible();
  await page.getByLabel("Show processed").uncheck();
  await expect(inboxItem(page, title)).toHaveCount(0);

  await page.locator(".toast").getByRole("button", { name: "Undo" }).click();
  await expect(inboxItem(page, title)).toBeVisible();
  await expect.poll(async () => (await (await admin.get(`/api/notes/${id}`)).json()).note.fields.processed).toBe(false);
});

test("Convert to a task keeps the title and body, opens the task and trashes the fleeting note", async ({ page, admin }) => {
  const title = `Fix the gate ${token()}`;
  const { id } = await fleeting(admin, title, "It squeaks when it opens");
  await logIn(page, "/inbox");
  const item = inboxItem(page, title);
  await item.getByRole("button", { name: "Convert to…" }).click();
  const box = item.getByRole("combobox", { name: "Add to Type" });
  await expect(box).toBeFocused();
  await box.fill("Task");
  await item.getByRole("option", { name: "Task" }).click();
  await expect(page).toHaveURL(/\/notes\/[0-9A-HJKMNP-TV-Z]{26}$/);
  const taskId = page.url().split("/").pop()!;
  expect(taskId).not.toBe(id);
  const task = await (await admin.get(`/api/notes/${taskId}`)).json();
  expect(task.type).toBe("task");
  expect(task.note.fields.title).toBe(title);
  expect(task.note.blocks[0].labels).toEqual(["body"]);
  expect(task.note.blocks[0].blocks[0].fields.text).toBe("It squeaks when it opens");
  await expect(activePane(page).getByText("It squeaks when it opens")).toBeVisible();

  expect((await admin.get(`/api/notes/${id}`)).status()).toBe(404);
  const trash = (await (await admin.get("/api/trash")).json()) as { notes: { id: string }[] };
  expect(trash.notes.some((n) => n.id === id)).toBe(true);
});

test("Delete moves a fleeting note to the trash, and Undo restores it", async ({ page, admin }) => {
  const title = `Old idea ${token()}`;
  const { id } = await fleeting(admin, title, "Not worth keeping");
  await logIn(page, "/inbox");
  await inboxItem(page, title).getByRole("button", { name: "Delete" }).click();
  await expect(inboxItem(page, title)).toHaveCount(0);
  await expect.poll(async () => (await admin.get(`/api/notes/${id}`)).status()).toBe(404);
  await page.locator(".toast").getByRole("button", { name: "Undo" }).click();
  await expect(inboxItem(page, title)).toBeVisible();
});

test("Escape with something typed asks before discarding it", async ({ page }) => {
  await logIn(page, "/daily");
  await page.keyboard.press("Control+Shift+K");
  await captureText(page).fill("half a thought");
  await page.keyboard.press("Escape");
  const confirm = dialog(page).getByRole("group", { name: "Discard this capture?" });
  await expect(confirm).toBeVisible();
  await confirm.getByRole("button", { name: "Keep editing" }).click();
  await expect(captureText(page)).toHaveValue("half a thought");
  await expect(captureText(page)).toBeFocused();

  // The backdrop asks too.
  await page.mouse.click(5, 5);
  await expect(confirm).toBeVisible();
  await confirm.getByRole("button", { name: "Discard" }).click();
  await expect(dialog(page)).toHaveCount(0);
  await page.keyboard.press("Control+Shift+K");
  await expect(captureText(page)).toHaveValue("");
});

test("the fleeting note type can be edited but has no delete button", async ({ page }) => {
  await logIn(page, "/settings/types/fleeting/edit");
  await expect(page.getByRole("link", { name: "Templates" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Delete type" })).toHaveCount(0);
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true });

  test("the + stays clear of the drawers and the dialog is a full-width sheet", async ({ page, admin }) => {
    await logIn(page, "/daily");
    const plus = page.getByRole("button", { name: "Quick capture" });
    await expect(plus).toBeVisible();
    const box = (await plus.boundingBox())!;
    expect(box.x + box.width).toBeLessThanOrEqual(390);
    expect(box.y + box.height).toBeLessThanOrEqual(844);

    await page.getByRole("button", { name: "Menu" }).click();
    await expect(plus).toBeHidden();
    await expect(inboxLink(page)).toBeVisible();
    await page.getByRole("navigation", { name: "Main" }).getByRole("button", { name: "Capture" }).click();
    await expect(dialog(page)).toBeVisible();
    const sheet = (await dialog(page).boundingBox())!;
    expect(sheet.width).toBeGreaterThan(385);
    expect(Math.round(sheet.y + sheet.height)).toBeGreaterThanOrEqual(843);

    const title = `Phone note ${token()}`;
    await captureText(page).fill(title);
    await dialog(page).getByRole("button", { name: "Save" }).click();
    await expect(dialog(page)).toHaveCount(0);
    await expect(plus).toBeVisible();
    await page.goto("/inbox");
    await expect(inboxItem(page, title)).toBeVisible();
    const res = await admin.post("/api/query", { data: { query: `from fleeting where title == "${title}"`, page_size: 1 } });
    expect(((await res.json()) as { total: number }).total).toBe(1);
  });
});
