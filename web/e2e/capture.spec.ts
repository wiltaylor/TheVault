// The capture dialog's pen and Send to (capture K2, K3): a capture with text, a drawing made with
// the pen and Send to Assistant saves one fleeting note whose body holds the text, the drawing as
// an ink block and the committed call to Assistant, which the server sees.

import type { Page } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

interface Block {
  kind: string;
  labels?: string[];
  fields?: Record<string, unknown>;
  blocks?: Block[];
}

const ULID = "[0-9A-HJKMNP-TV-Z]{26}";

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

const dialog = (page: Page) => page.getByRole("dialog", { name: "Capture" });

/** Drag the mouse across the pen's surface, from and to points as fractions of its box. */
async function stroke(page: Page, from: [number, number], to: [number, number]) {
  const box = (await dialog(page).getByRole("dialog", { name: "Pen" }).getByRole("img", { name: "Drawing" }).boundingBox())!;
  const at = ([fx, fy]: [number, number]): [number, number] => [box.x + box.width * fx, box.y + box.height * fy];
  await page.mouse.move(...at(from));
  await page.mouse.down();
  await page.mouse.move(...at(to), { steps: 10 });
  await page.mouse.up();
}

test("a capture with a drawing sent to Assistant saves the ink block and the call link", async ({ page, admin }) => {
  const title = `Sketch of the shed ${token()}`;
  await logIn(page, "/daily");
  await page.keyboard.press("Control+Shift+K");
  await dialog(page).getByRole("textbox", { name: "Capture text" }).fill(title);

  // The pen fills most of the screen, and the mouse draws on it at once.
  await dialog(page).getByRole("button", { name: "Pen" }).click();
  const pen = dialog(page).getByRole("dialog", { name: "Pen" });
  await expect(pen).toBeVisible();
  const size = (await pen.boundingBox())!;
  const viewport = page.viewportSize()!;
  expect(size.width).toBeGreaterThan(viewport.width * 0.8);
  const surface = (await pen.getByRole("img", { name: "Drawing" }).boundingBox())!;
  expect(surface.height).toBeGreaterThan(viewport.height * 0.6);
  await expect(pen.getByRole("button", { name: "Done" })).toBeDisabled();
  await stroke(page, [0.2, 0.3], [0.7, 0.3]);
  await stroke(page, [0.2, 0.5], [0.7, 0.6]);
  await pen.getByRole("button", { name: "Done" }).click();
  await expect(pen).toHaveCount(0);
  await expect(dialog(page).getByRole("list", { name: "Drawings" }).getByRole("img", { name: "Drawing 1" })).toBeVisible();

  const sendTo = dialog(page).getByRole("combobox", { name: "Send to" });
  await expect(sendTo.locator("option", { hasText: "Assistant" })).toHaveCount(1);
  await sendTo.selectOption({ label: "Assistant" });
  await dialog(page).getByRole("button", { name: "Save" }).click();
  await expect(dialog(page)).toHaveCount(0);
  await expect(page.locator(".toast").filter({ hasText: "Saved and sent to Assistant" })).toBeVisible();

  const found = await admin.post("/api/query", { data: { query: `from fleeting where title == "${title}"`, page_size: 5 } });
  const rows = ((await found.json()) as { rows: { id: string }[] }).rows;
  expect(rows).toHaveLength(1);
  const note = (await (await admin.get(`/api/notes/${rows[0]!.id}`)).json()) as { note: { blocks: Block[] } };
  const body = note.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === "body")!.blocks!;
  expect(body.map((b) => b.kind)).toEqual(["paragraph", "ink", "paragraph"]);
  expect(body[0]!.fields!.text).toBe(title);
  expect(body[1]!.blocks!.filter((b) => b.kind === "stroke")).toHaveLength(2);
  const link = String(body[2]!.fields!.text);
  expect(link).toMatch(new RegExp(`^\\[@Assistant Process this capture\\]\\(agent:assistant\\?call=${ULID}\\)$`));
  const call = new RegExp(`call=(${ULID})`).exec(link)![1];
  await expect.poll(async () => (await admin.get(`/api/agents/calls/${call}`)).status()).toBe(200);

  // The next capture remembers the choice.
  await page.keyboard.press("Control+Shift+K");
  await expect(dialog(page).getByRole("combobox", { name: "Send to" })).toHaveValue("assistant");
  await dialog(page).getByRole("combobox", { name: "Send to" }).selectOption({ label: "No one" });
  await page.keyboard.press("Escape");
  await expect(dialog(page)).toHaveCount(0);
});
