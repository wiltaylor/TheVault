// A person's phones and emails are one chip per value with its label, copied and edited from the
// chip, and the photo is an image tile that opens a picker (D16).

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

function unique(prefix: string): string {
  return `${prefix} ${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function fieldsOf(admin: APIRequestContext, id: string) {
  return ((await (await admin.get(`/api/notes/${id}`)).json()) as { note: { fields: Record<string, unknown> } }).note.fields;
}

/** With E2E_SHOTS set to a folder, a screenshot of `what` there as `ticket-93-<name>.png`. */
async function shot(page: Page, name: string, what: Locator) {
  const dir = process.env.E2E_SHOTS;
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  await page.waitForTimeout(200);
  await what.screenshot({ path: join(dir, `ticket-93-${name}.png`) });
}

/** A small PNG drawn in the page: a face on a coloured ground. */
async function facePng(page: Page): Promise<Buffer> {
  const url = await page.evaluate(() => {
    const c = document.createElement("canvas");
    c.width = c.height = 160;
    const g = c.getContext("2d")!;
    g.fillStyle = "#7c9cc9";
    g.fillRect(0, 0, 160, 160);
    g.fillStyle = "#f2d3b3";
    g.beginPath();
    g.arc(80, 70, 38, 0, Math.PI * 2);
    g.fill();
    g.fillStyle = "#3d5a80";
    g.fillRect(30, 118, 100, 42);
    return c.toDataURL("image/png");
  });
  return Buffer.from(url.split(",")[1]!, "base64");
}

test("a person's phones are labelled chips that copy and edit, and the photo tile opens the picker", async ({
  page,
  admin,
  context,
}) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const name = unique("Sam");
  // A label another person's phone uses, offered when Sam's are edited.
  const pager = `pager${Date.now().toString(36)}`;
  const other = await admin.post("/api/notes?type=person", {
    data: { note: { fields: { title: unique("Kim"), phones: [{ number: "+61400000029", label: pager }] }, blocks: [] } },
  });
  expect(other.status(), await other.text()).toBe(201);
  const res = await admin.post("/api/notes?type=person", {
    data: {
      note: {
        fields: {
          title: name,
          phones: [
            { number: "+61400000020", label: "mobile" },
            { number: "+61400000021", label: "home" },
          ],
          emails: [{ address: "sam@example.com", label: "work" }],
        },
        blocks: [],
      },
    },
  });
  expect(res.status(), await res.text()).toBe(201);
  const { id } = (await res.json()) as { id: string };
  await logIn(page, `/notes/${id}`);
  const pane = activePane(page);
  const phones = pane.getByTestId("field-phones");

  // One chip per value, the label beside the number.
  await expect(phones.locator(".labelled-chip")).toHaveCount(2);
  const mobile = phones.locator(".labelled-chip").first();
  await expect(mobile.locator(".chip-label")).toHaveText("mobile");
  await expect(mobile.locator(".chip-value")).toHaveText("+61400000020");
  await expect(phones.locator(".labelled-chip").nth(1)).toContainText("home+61400000021");
  await expect(pane.getByTestId("field-emails").locator(".chip-label")).toHaveText("work");

  // The empty photo tile, with the chips, for the user.
  const tile = pane.getByRole("button", { name: "Add photo" });
  await expect(tile).toBeVisible();
  await shot(page, "person-empty-photo", pane.locator(".note-view"));

  // Copy puts the number, not the label, on the clipboard and says so.
  await mobile.hover();
  await shot(page, "chip-hover", phones);
  // The actions show while the pointer is over the chip.
  await mobile.hover();
  await expect(mobile.getByRole("button", { name: "Copy mobile +61400000020" })).toBeVisible();
  await mobile.getByRole("button", { name: "Copy mobile +61400000020" }).click();
  await expect(mobile.getByRole("button", { name: "Copied +61400000020" })).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("+61400000020");

  // Clicking the chip opens Edit; Enter saves the label and value of that chip only.
  await phones.getByRole("button", { name: "home +61400000021", exact: true }).click();
  const popover = phones.getByRole("dialog", { name: "Edit home +61400000021" });
  await expect(popover).toBeVisible();
  await expect(phones.locator(`datalist option[value="${pager}"]`)).toHaveCount(1);
  await shot(page, "chip-edit-popover", phones.locator("xpath=.."));
  await popover.getByRole("combobox", { name: "Label" }).fill("work");
  await popover.getByRole("textbox", { name: "Value" }).fill("+61299990000");
  await popover.getByRole("textbox", { name: "Value" }).press("Enter");
  await expect(popover).toHaveCount(0);
  await expect(phones.locator(".labelled-chip").nth(1)).toContainText("work+61299990000");
  await expect
    .poll(async () => (await fieldsOf(admin, id)).phones)
    .toEqual([
      { number: "+61400000020", label: "mobile" },
      { number: "+61299990000", label: "work" },
    ]);

  // The photo tile opens the picker; an upload there sets the field.
  await tile.click();
  const dialog = page.getByRole("dialog", { name: "Choose Photo" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("searchbox", { name: "Search images" })).toBeVisible();
  await shot(page, "photo-picker", dialog);
  const png = await facePng(page);
  const chooser = page.waitForEvent("filechooser");
  await dialog.getByRole("button", { name: "Upload an image to Photo" }).click();
  await (await chooser).setFiles({ name: "sam.png", mimeType: "image/png", buffer: png });
  await expect(dialog).toHaveCount(0);
  await expect.poll(async () => (await fieldsOf(admin, id)).photo).toMatch(/^[0-9A-Z]{26}$/);
  const set = pane.getByRole("button", { name: "Change Photo" });
  await expect(set.locator("img")).toBeVisible();
  await expect(pane.getByRole("link", { name: "Open full size" })).toBeVisible();
  await shot(page, "person-set-photo", pane.locator(".note-view"));

  // The picker lists the uploaded image among the vault's images.
  await set.click();
  await expect(page.getByRole("dialog", { name: "Choose Photo" }).getByRole("button", { name: "sam.png" })).toBeVisible();
  await shot(page, "photo-picker-images", page.getByRole("dialog", { name: "Choose Photo" }));
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "Choose Photo" })).toHaveCount(0);
});
