// Canvas and ink blocks resize from the handle on their bottom edge: a mouse or finger drag shows
// the height as it goes and saves once on release, the arrow keys step it, a double-click resets
// it, and a reload keeps it. An ink drawing's strokes keep their stored coordinates throughout:
// its height is the height of its view onto the plane, in screen pixels.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

interface Block {
  kind: string;
  labels?: string[];
  fields?: Record<string, unknown>;
  blocks?: Block[];
}

interface Stored {
  id: string;
  wcl: string;
  note: { fields: Record<string, unknown>; blocks: Block[] };
}

const token = () => `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;

async function create(admin: APIRequestContext, type: string, title: string, blocks: Block[]): Promise<Stored> {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields: { title }, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as Stored;
}

async function stored(admin: APIRequestContext, id: string): Promise<Stored> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return (await res.json()) as Stored;
}

/** Leave what was edited, which saves at once, and wait until the note is saved. */
async function saveAndWait(page: Page) {
  await page.getByLabel("Title", { exact: true }).focus();
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
}

/** Counts the saves of one note the page sends. */
function countSaves(page: Page, id: string) {
  const saves: string[] = [];
  page.on("request", (r) => {
    if (r.method() === "PUT" && r.url().includes(`/api/notes/${id}`)) saves.push(r.url());
  });
  return saves;
}

/** Drag the handle down by `dy` screen pixels with the mouse, in steps. */
async function mouseDrag(page: Page, handle: Locator, dy: number) {
  await handle.scrollIntoViewIfNeeded();
  const h = (await handle.boundingBox())!;
  await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2);
  await page.mouse.down();
  await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2 + dy, { steps: 8 });
  await page.mouse.up();
}

/** Drag the handle down by `dy` screen pixels with a finger, as touch input from the browser. */
async function touchDrag(page: Page, handle: Locator, dy: number) {
  await handle.scrollIntoViewIfNeeded();
  const h = (await handle.boundingBox())!;
  const x = h.x + h.width / 2;
  const y = h.y + h.height / 2;
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x, y }] });
  for (let i = 1; i <= 8; i++) {
    await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x, y: y + (dy * i) / 8 }] });
  }
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await cdp.detach();
}

const viewport = { kind: "viewport", fields: { x: 0, y: 0, zoom: 1 } };
const card: Block = {
  kind: "node",
  labels: ["a"],
  fields: { kind: "card", x: 40, y: 40, width: 200, height: 100 },
  blocks: [{ kind: "paragraph", fields: { text: "Card A" } }],
};

const strokes: Block[] = [60, 200, 380].map((y) => ({
  kind: "stroke",
  fields: { tool: "pen", colour: "#2563eb", size: 4.0, points: [80, y, 0.5, 400, y, 0.6, 720, y, 0.5] },
}));
const drawing = (height = 400): Block => ({ kind: "ink", fields: { width: 800, height }, blocks: strokes });

function bodyBlocks(note: Stored): Block[] {
  return note.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === "body")?.blocks ?? [];
}

/** Every ink block in the body, columns included. */
function bodyInk(note: Stored): Block[] {
  const all = (bs: Block[]): Block[] => bs.flatMap((b) => (b.kind === "ink" ? [b] : all(b.blocks ?? [])));
  return all(bodyBlocks(note));
}

const height = async (l: Locator) => (await l.boundingBox())!.height;

/** The height of an ink drawing's view, inside its border. */
const viewHeight = (l: Locator) => l.evaluate((el) => el.clientHeight);

/**
 * With E2E_SHOTS set to a folder, light and dark screenshots of `what` there with the handle
 * shown as it is under the pointer. Without it, nothing is written.
 */
async function shots(page: Page, name: string, what: Locator, handle: Locator) {
  const dir = process.env.E2E_SHOTS;
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  const h = (await handle.boundingBox())!;
  await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2);
  for (const theme of ["light", "dark"]) {
    await page.evaluate((t) => (document.documentElement.dataset.theme = t), theme);
    await page.waitForTimeout(250);
    await what.screenshot({ path: join(dir, `resize-${name}-${theme}.png`) });
  }
  await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
}

test("dragging a canvas's handle shows the height, saves it once on release, and a double-click resets it", async ({
  page,
  admin,
}) => {
  const title = `Canvas ${token()}`;
  const note = await create(admin, "board", title, [{ kind: "canvas", labels: ["board"], blocks: [viewport, card] }]);
  await logIn(page, `/notes/${note.id}`);
  const canvas = page.getByTestId("field-board").getByTestId("canvas");
  const view = canvas.getByTestId("canvas-view");
  const handle = canvas.getByRole("separator", { name: "Canvas height" });
  await expect(handle).toHaveAttribute("aria-valuenow", "480");
  const nodeBefore = (await canvas.getByTestId("canvas-node").boundingBox())!;
  await shots(page, "canvas", canvas, handle);

  const saves = countSaves(page, note.id);
  const h = (await handle.boundingBox())!;
  await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2);
  await page.mouse.down();
  await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2 + 150, { steps: 8 });
  // While dragging, the height shows and nothing is saved yet.
  await expect(handle.getByTestId("height-readout")).toHaveText("630 px");
  await page.mouse.up();
  await expect(handle.getByTestId("height-readout")).toHaveCount(0);
  await expect(handle).toHaveAttribute("aria-valuenow", "630");
  expect(await height(view)).toBeCloseTo(630, 0);
  // Only the viewport grew: the node is where it was.
  const nodeAfter = (await canvas.getByTestId("canvas-node").boundingBox())!;
  expect([nodeAfter.x, nodeAfter.y, nodeAfter.width]).toEqual([nodeBefore.x, nodeBefore.y, nodeBefore.width]);
  await saveAndWait(page);
  expect(saves).toHaveLength(1);
  const canvasOf = (n: Stored) => n.note.blocks.find((b) => b.kind === "canvas");
  expect(canvasOf(await stored(admin, note.id))?.fields).toEqual({ height: 630 });

  await page.reload();
  await expect(page.getByTestId("field-board").getByTestId("canvas-view")).toHaveCSS("height", "630px");

  // A double-click puts back the default, which leaves the item out.
  await page.getByTestId("field-board").getByRole("separator", { name: "Canvas height" }).dblclick();
  await expect(page.getByTestId("field-board").getByTestId("canvas-view")).toHaveCSS("height", "480px");
  await saveAndWait(page);
  expect(canvasOf(await stored(admin, note.id))?.fields).toBeUndefined();
});

test("dragging an ink block's handle saves its height once, keeps every stroke, and survives a reload", async ({
  page,
  admin,
}) => {
  const note = await create(admin, "task", `Ink ${token()}`, [{ kind: "content", labels: ["body"], blocks: [drawing()] }]);
  await logIn(page, `/notes/${note.id}`);
  const ink = page.getByRole("textbox", { name: "Body" }).getByTestId("ink");
  const surface = ink.getByRole("img", { name: "Drawing" });
  const handle = ink.getByRole("separator", { name: "Drawing height" });
  await expect(handle).toHaveAttribute("aria-valuenow", "400");
  await expect(surface.locator("path.ink-stroke")).toHaveCount(3);
  const paths = () => surface.locator("path.ink-stroke").evaluateAll((els) => els.map((e) => e.getAttribute("d")));
  const drawn = await paths();
  await shots(page, "ink", ink, handle);

  // Shorter: the stroke at y = 380 lies past the edge and is hidden, not removed.
  const saves = countSaves(page, note.id);
  const before = await height(surface);
  await mouseDrag(page, handle, -200);
  await expect(handle).toHaveAttribute("aria-valuenow", "200");
  await expect.poll(() => viewHeight(surface)).toBe(200);
  // The box includes the 1px border on each side.
  expect(Math.abs((await height(surface)) - 2 - (before - 2) / 2)).toBeLessThan(1.5);
  await saveAndWait(page);
  expect(saves).toHaveLength(1);
  let ink0 = bodyInk(await stored(admin, note.id))[0]!;
  expect(ink0.fields).toEqual({ width: 800, height: 200 });
  expect(ink0.blocks).toEqual(strokes);
  expect(await paths()).toEqual(drawn.slice(0, 2));

  // The keyboard: Shift+Down twice and Down once grows it by 220, and the stroke shows again.
  await handle.focus();
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("ArrowDown");
  await expect(handle).toHaveAttribute("aria-valuenow", "420");
  await saveAndWait(page);
  ink0 = bodyInk(await stored(admin, note.id))[0]!;
  expect(ink0.fields).toEqual({ width: 800, height: 420 });
  expect(ink0.blocks).toEqual(strokes);

  await page.reload();
  const again = page.getByRole("textbox", { name: "Body" }).getByRole("img", { name: "Drawing" });
  await expect.poll(() => viewHeight(again)).toBe(420);
  expect(await again.locator("path.ink-stroke").evaluateAll((els) => els.map((e) => e.getAttribute("d")))).toEqual(drawn);
});

test("an ink block in a narrow column resizes by what the pointer moved, whatever the drawing's zoom", async ({
  page,
  admin,
}) => {
  const col = (...blocks: Block[]): Block => ({ kind: "column", fields: { width: 1 }, blocks });
  const note = await create(admin, "task", `Columns ${token()}`, [
    {
      kind: "content",
      labels: ["body"],
      blocks: [{ kind: "columns", blocks: [col(drawing()), col({ kind: "paragraph", fields: { text: "beside" } })] }],
    },
  ]);
  await logIn(page, `/notes/${note.id}`);
  const surface = page.getByRole("textbox", { name: "Body" }).getByRole("img", { name: "Drawing" });
  const handle = page.getByRole("textbox", { name: "Body" }).getByRole("separator", { name: "Drawing height" });
  await expect(surface).toBeVisible();
  const box = (await surface.boundingBox())!;
  // The old 800-wide page is zoomed out to fit the column.
  expect(box.width / 800).toBeLessThan(0.9);
  await mouseDrag(page, handle, 100);
  const want = 400 + 100;
  await expect.poll(async () => Number(await handle.getAttribute("aria-valuenow"))).toBeGreaterThan(want - 3);
  const got = Number(await handle.getAttribute("aria-valuenow"));
  expect(Math.abs(got - want)).toBeLessThan(3);
  // The surface's bottom edge followed the pointer.
  expect(Math.abs((await height(surface)) - (box.height + 100))).toBeLessThan(3);
  await saveAndWait(page);
  const ink0 = bodyInk(await stored(admin, note.id))[0]!;
  expect(ink0.fields).toEqual({ width: 800, height: got });
  expect(ink0.blocks).toEqual(strokes);
});

test("an ink field resizes from its handle: End, Home and a double-click reset", async ({ page, admin }) => {
  const note = await create(admin, "sketch", `Sketch ${token()}`, [{ ...drawing(300), labels: ["drawing"] }]);
  await logIn(page, `/notes/${note.id}`);
  await page.getByRole("tab", { name: "Drawing" }).click();
  const field = page.getByTestId("field-drawing");
  const handle = field.getByRole("separator", { name: "Drawing height" });
  await expect(handle).toHaveAttribute("aria-valuenow", "300");
  await handle.focus();
  await page.keyboard.press("End");
  await expect(handle).toHaveAttribute("aria-valuenow", "4000");
  await page.keyboard.press("Home");
  await expect(handle).toHaveAttribute("aria-valuenow", "80");
  await handle.dblclick();
  await expect(handle).toHaveAttribute("aria-valuenow", "400");
  await saveAndWait(page);
  const saved = (await stored(admin, note.id)).note.blocks.find((b) => b.kind === "ink")!;
  expect(saved.fields).toEqual({ width: 800, height: 400 });
  expect(saved.blocks).toEqual(strokes);
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });

  test("a finger drag on the handle resizes an ink block and a canvas, and the grip is always shown", async ({
    page,
    admin,
  }) => {
    const inkNote = await create(admin, "task", `Phone ink ${token()}`, [
      { kind: "content", labels: ["body"], blocks: [drawing()] },
    ]);
    await logIn(page, `/notes/${inkNote.id}`);
    const surface = page.getByRole("textbox", { name: "Body" }).getByRole("img", { name: "Drawing" });
    const handle = page.getByRole("textbox", { name: "Body" }).getByRole("separator", { name: "Drawing height" });
    await expect(surface).toBeVisible();
    await expect(handle.locator(".canvas-height-grip")).toHaveCSS("opacity", "1");
    const saves = countSaves(page, inkNote.id);
    await touchDrag(page, handle, 80);
    const want = 400 + 80;
    await expect.poll(async () => Math.abs(Number(await handle.getAttribute("aria-valuenow")) - want)).toBeLessThan(3);
    await saveAndWait(page);
    expect(saves).toHaveLength(1);
    const ink0 = bodyInk(await stored(admin, inkNote.id))[0]!;
    expect(Math.abs((ink0.fields!.height as number) - want)).toBeLessThan(3);
    expect(ink0.blocks).toEqual(strokes);

    const board = await create(admin, "board", `Phone board ${token()}`, [
      { kind: "canvas", labels: ["board"], blocks: [viewport, card] },
    ]);
    await page.goto(`/notes/${board.id}`);
    const canvasHandle = page.getByTestId("field-board").getByRole("separator", { name: "Canvas height" });
    await expect(canvasHandle).toHaveAttribute("aria-valuenow", "480");
    await touchDrag(page, canvasHandle, -120);
    await expect(canvasHandle).toHaveAttribute("aria-valuenow", "360");
    await saveAndWait(page);
    const canvas = (await stored(admin, board.id)).note.blocks.find((b) => b.kind === "canvas");
    expect(canvas?.fields).toEqual({ height: 360 });
  });
});
