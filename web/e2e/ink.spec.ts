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

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
}

async function create(admin: APIRequestContext, type: string, title: string, blocks: Block[] = []): Promise<Stored> {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields: { title }, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as Stored;
}

async function stored(admin: APIRequestContext, id: string): Promise<Stored> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return (await res.json()) as Stored;
}

/** The ink blocks in a task's body. */
function bodyInk(note: Stored): Block[] {
  const body = note.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === "body");
  return (body?.blocks ?? []).filter((b) => b.kind === "ink");
}

function strokes(ink: Block | undefined): number[][] {
  return (ink?.blocks ?? []).filter((b) => b.kind === "stroke").map((s) => s.fields?.points as number[]);
}

const pressures = (points: number[]) => points.filter((_, i) => i % 3 === 2);

/** Leave what was edited, which saves at once, and wait until the note is saved. */
async function saveAndWait(page: Page) {
  await page.getByLabel("Title", { exact: true }).focus();
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
}

/** Client coordinates of a point given in ink units, through the surface's current view. */
async function client(surface: Locator, x: number, y: number): Promise<[number, number]> {
  await surface.scrollIntoViewIfNeeded();
  return surface.evaluate(
    (el, [x, y]) => {
      const svg = el as SVGSVGElement;
      const r = svg.getBoundingClientRect();
      const vb = svg.viewBox.baseVal;
      const k = svg.clientWidth / vb.width;
      return [r.left + svg.clientLeft + (x! - vb.x) * k, r.top + svg.clientTop + (y! - vb.y) * k] as [number, number];
    },
    [x, y],
  );
}

/** The surface's view: the ink point at its top-left and the zoom. */
async function viewOf(surface: Locator) {
  return surface.evaluate((el) => {
    const svg = el as SVGSVGElement;
    const vb = svg.viewBox.baseVal;
    return { x: vb.x, y: vb.y, zoom: svg.clientWidth / vb.width };
  });
}

/** Let the mouse and fingers draw: the toggle in the tools cluster, kept for the device. */
async function drawWithMouse(page: Page, on = true) {
  const toggle = page.getByRole("toolbar", { name: "Drawing tools" }).first().getByRole("button", { name: "Draw with mouse and touch" });
  if ((await toggle.getAttribute("aria-pressed")) !== String(on)) await toggle.click();
  await expect(toggle).toHaveAttribute("aria-pressed", String(on));
}

/** Drag the mouse along a horizontal line in ink units. */
async function mouseLine(page: Page, surface: Locator, y: number, x0: number, x1: number) {
  await page.mouse.move(...(await client(surface, x0, y)));
  await page.mouse.down();
  for (let i = 1; i <= 8; i++) await page.mouse.move(...(await client(surface, x0 + ((x1 - x0) * i) / 8, y)));
  await page.mouse.up();
}

/** A stylus stroke from synthetic pointer events, along a line, with the given pressure at each point. */
async function penLine(surface: Locator, y: number, x0: number, x1: number, pressure: number[], pointerId: number) {
  const n = pressure.length - 1;
  const at = async (i: number, buttons: number) => {
    const [clientX, clientY] = await client(surface, x0 + ((x1 - x0) * i) / n, y);
    return { pointerId, pointerType: "pen", isPrimary: true, pressure: pressure[i], button: 0, buttons, clientX, clientY };
  };
  await surface.dispatchEvent("pointerdown", await at(0, 1));
  for (let i = 1; i <= n; i++) await surface.dispatchEvent("pointermove", await at(i, 1));
  await surface.dispatchEvent("pointerup", { ...(await at(n, 0)), button: 0 });
}

const paths = (surface: Locator) =>
  surface.locator("path.ink-stroke").evaluateAll((els) => els.map((el) => el.getAttribute("d")));

const threeLines: Block = {
  kind: "ink",
  fields: { width: 800, height: 400 },
  blocks: [100, 200, 300].map((y) => ({
    kind: "stroke",
    fields: { tool: "pen", colour: "#1f2937", size: 4.0, points: [100, y, 0.5, 400, y, 0.5, 700, y, 0.5] },
  })),
};

test("three mouse strokes, with drawing by mouse on, save as three strokes and reload as the same drawing", async ({
  page,
  admin,
}) => {
  const note = await create(admin, "task", `Ink ${token()}`);
  await logIn(page, `/notes/${note.id}`);
  const doc = page.getByRole("textbox", { name: "Body" });
  await doc.click();
  // The / menu inserts blocks.
  await page.keyboard.type("/ink");
  await page.getByRole("listbox", { name: "Insert block" }).getByRole("option", { name: "Ink drawing" }).click();

  const surface = doc.getByRole("img", { name: "Drawing" });
  await expect(surface).toBeVisible();
  await drawWithMouse(page);
  for (const y of [100, 200, 300]) await mouseLine(page, surface, y, 100, 700);
  await expect(surface.locator("path.ink-stroke")).toHaveCount(3);
  const drawn = await paths(surface);
  await saveAndWait(page);

  const saved = await stored(admin, note.id);
  const ink = bodyInk(saved);
  expect(ink).toHaveLength(1);
  // A new drawing has no page width: it is an endless plane, and nothing panned, so no view.
  expect(ink[0]!.fields).toEqual({ height: 400 });
  expect(ink[0]!.blocks?.map((b) => b.kind)).toEqual(["stroke", "stroke", "stroke"]);
  const lines = strokes(ink[0]);
  expect(lines).toHaveLength(3);
  for (const [i, points] of lines.entries()) {
    expect(points.length % 3).toBe(0);
    expect(points.length).toBeGreaterThanOrEqual(6);
    expect(new Set(pressures(points))).toEqual(new Set([0.5]));
    // Every y lies on the line it was drawn along, give or take a pixel of rounding.
    for (let j = 1; j < points.length; j += 3) expect(Math.abs(points[j]! - (i + 1) * 100)).toBeLessThan(2);
  }
  expect(saved.wcl).toMatch(/ink \{[\s\S]*stroke \{[\s\S]*tool\s*= "pen"/);

  await page.reload();
  const shown = page.getByRole("img", { name: "Drawing" });
  await expect(shown.locator("path.ink-stroke")).toHaveCount(3);
  expect(await paths(shown)).toEqual(drawn);
});

test("erasing one stroke of three saves two", async ({ page, admin }) => {
  const note = await create(admin, "task", `Erase ${token()}`, [
    { kind: "content", labels: ["body"], blocks: [threeLines] },
  ]);
  await logIn(page, `/notes/${note.id}`);
  const surface = page.getByRole("textbox", { name: "Body" }).getByRole("img", { name: "Drawing" });
  await expect(surface.locator("path.ink-stroke")).toHaveCount(3);

  await page.getByRole("toolbar", { name: "Drawing tools" }).getByRole("button", { name: "Eraser" }).click();
  await drawWithMouse(page);
  await page.mouse.click(...(await client(surface, 400, 200)));
  await expect(surface.locator("path.ink-stroke")).toHaveCount(2);
  await saveAndWait(page);

  const lines = strokes(bodyInk(await stored(admin, note.id))[0]);
  expect(lines.map((p) => p[1])).toEqual([100, 300]);
  await page.reload();
  await expect(page.getByRole("img", { name: "Drawing" }).locator("path.ink-stroke")).toHaveCount(2);
});

test("stylus pressure from synthetic pointer events varies the stroke's width", async ({ page, admin }) => {
  const empty: Block = { kind: "ink", fields: { width: 800, height: 400 }, blocks: [] };
  const note = await create(admin, "task", `Pressure ${token()}`, [
    { kind: "content", labels: ["body"], blocks: [empty] },
  ]);
  await logIn(page, `/notes/${note.id}`);
  const surface = page.getByRole("textbox", { name: "Body" }).getByRole("img", { name: "Drawing" });
  const tools = page.getByRole("toolbar", { name: "Drawing tools" });
  await tools.getByRole("button", { name: "Stroke size" }).click();
  await tools.getByRole("button", { name: "Size 16" }).click();
  await expect(tools.getByRole("button", { name: "Stroke size" })).toHaveAttribute("title", "Stroke size: 16");

  const ramp = Array.from({ length: 10 }, (_, i) => 0.1 + i * 0.1);
  await penLine(surface, 100, 100, 700, ramp, 11);
  await penLine(surface, 200, 100, 700, Array(10).fill(0.1), 12);
  await penLine(surface, 300, 100, 700, Array(10).fill(1), 13);
  await expect(surface.locator("path.ink-stroke")).toHaveCount(3);
  await saveAndWait(page);

  const [rampPoints, light, heavy] = strokes(bodyInk(await stored(admin, note.id))[0]);
  expect(pressures(rampPoints!)).toEqual(ramp.map((p) => Math.round(p * 1000) / 1000));
  expect(new Set(pressures(light!))).toEqual(new Set([0.1]));
  expect(new Set(pressures(heavy!))).toEqual(new Set([1]));

  // After a reload, the outline of each stroke is as thick as its pressure.
  await page.reload();
  const shown = page.getByRole("img", { name: "Drawing" }).locator("path.ink-stroke");
  await expect(shown).toHaveCount(3);
  const heights = await shown.evaluateAll((els) => els.map((el) => (el as SVGPathElement).getBBox().height));
  expect(heights[2]!).toBeGreaterThan(heights[1]! * 2);
  // Across the ramp the outline thickens: compare the width near its start and near its end.
  const thickness = await shown.first().evaluate((el) => {
    const pts = [...(el.getAttribute("d") ?? "").matchAll(/[ML](-?[\d.]+),(-?[\d.]+)/g)].map((m) => [
      Number(m[1]),
      Number(m[2]),
    ]);
    const span = (lo: number, hi: number) => {
      const ys = pts.filter(([x]) => x! >= lo && x! <= hi).map(([, y]) => y!);
      return Math.max(...ys) - Math.min(...ys);
    };
    return { start: span(150, 250), end: span(550, 650) };
  });
  expect(thickness.end).toBeGreaterThan(thickness.start * 1.5);
});

test("an ink field renders its drawing surface on its tab and saves what is drawn", async ({ page, admin }) => {
  const note = await create(admin, "sketch", `Sketch ${token()}`);
  await logIn(page, `/notes/${note.id}`);
  const surface = page.getByRole("img", { name: "Drawing" });
  await expect(page.getByRole("tab", { name: "Main" })).toBeVisible();
  await expect(surface).toHaveCount(0);

  await page.getByRole("tab", { name: "Drawing" }).click();
  await expect(page.getByTestId("field-drawing").getByRole("img", { name: "Drawing" })).toBeVisible();

  await drawWithMouse(page);
  await mouseLine(page, surface, 200, 100, 700);
  await expect(surface.locator("path.ink-stroke")).toHaveCount(1);
  await saveAndWait(page);

  const ink = (await stored(admin, note.id)).note.blocks.find((b) => b.kind === "ink");
  expect(ink?.labels).toEqual(["drawing"]);
  expect(ink?.fields).toEqual({ height: 400 });
  expect(strokes(ink)).toHaveLength(1);

  await page.reload();
  await page.getByRole("tab", { name: "Drawing" }).click();
  await expect(surface.locator("path.ink-stroke")).toHaveCount(1);
});

test("the drawing's controls float over its surface, and a read-only drawing has none", async ({ page, admin }) => {
  const note = await create(admin, "task", `Controls ${token()}`, [
    { kind: "content", labels: ["body"], blocks: [threeLines] },
  ]);
  await logIn(page, `/notes/${note.id}`);
  const ink = page.getByRole("textbox", { name: "Body" }).getByTestId("ink");
  const surface = ink.getByRole("img", { name: "Drawing" });
  await expect(surface.locator("path.ink-stroke")).toHaveCount(3);
  const area = (await surface.boundingBox())!;
  const tools = ink.getByRole("toolbar", { name: "Drawing tools" });
  const actions = ink.getByRole("toolbar", { name: "Drawing actions" });
  // Both clusters lie within the surface's box: tools at its top left, actions at its top right.
  for (const [cluster, side] of [
    [tools, "left"],
    [actions, "right"],
  ] as const) {
    const b = (await cluster.boundingBox())!;
    expect(b.x).toBeGreaterThanOrEqual(area.x);
    expect(b.y).toBeGreaterThanOrEqual(area.y);
    expect(b.x + b.width).toBeLessThanOrEqual(area.x + area.width);
    expect(b.y + b.height).toBeLessThan(area.y + area.height / 4);
    if (side === "left") expect(b.x - area.x).toBeLessThan(16);
    else expect(area.x + area.width - (b.x + b.width)).toBeLessThan(16);
  }

  // A colour from the swatch popover draws the next stroke; Escape closes the popover, back on its button.
  await tools.getByRole("button", { name: "Colour" }).click();
  const palette = ink.getByRole("group", { name: "Ink colours" });
  await expect(palette).toBeVisible();
  await palette.getByRole("button", { name: "Red" }).click();
  await expect(palette).toHaveCount(0);
  await tools.getByRole("button", { name: "Colour" }).click();
  await expect(palette.getByRole("button", { name: "Red" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(palette).toHaveCount(0);
  await expect(tools.getByRole("button", { name: "Colour" })).toBeFocused();
  // The background, from its popover by keyboard.
  await actions.getByRole("button", { name: "Background" }).focus();
  await page.keyboard.press("Enter");
  await ink.getByRole("group", { name: "Backgrounds" }).getByRole("button", { name: "Lines" }).click();
  await drawWithMouse(page);
  await mouseLine(page, surface, 350, 100, 700);
  await expect(surface.locator("path.ink-stroke")).toHaveCount(4);
  await saveAndWait(page);
  const saved = bodyInk(await stored(admin, note.id))[0]!;
  expect(saved.fields).toEqual({ width: 800, height: 400, background: "lines" });
  expect(saved.blocks?.[3]?.fields?.colour).toBe("#dc2626");

  // Undo from the actions cluster takes the stroke back off.
  await actions.getByRole("button", { name: "Undo" }).click();
  await expect(surface.locator("path.ink-stroke")).toHaveCount(3);

  // A version previewed from the history panel shows the drawing alone, with no controls on it.
  const drawing = { ...threeLines, labels: ["drawing"] };
  const sketch = await create(admin, "sketch", `Preview ${token()}`, [drawing]);
  const { version } = (await stored(admin, sketch.id)) as Stored & { version: string };
  const put = await admin.put(`/api/notes/${sketch.id}`, {
    data: { note: { fields: { title: `Preview later ${token()}` }, blocks: [drawing] }, base_version: version },
  });
  expect(put.status(), await put.text()).toBe(200);
  await page.goto(`/notes/${sketch.id}`);
  await page.getByRole("tab", { name: "Drawing" }).click();
  const live = page.getByTestId("field-drawing").getByTestId("ink");
  await expect(live.getByRole("toolbar", { name: "Drawing tools" })).toBeVisible();
  await page.getByRole("button", { name: "History", exact: true }).click();
  const list = page.getByRole("complementary", { name: "History" }).getByRole("list", { name: "Versions" });
  await expect(list.getByRole("listitem")).toHaveCount(2);
  // The older version, which has the same drawing.
  await list.getByRole("listitem").nth(1).getByRole("button").first().click();
  await expect(page.getByRole("region", { name: "Version preview" })).toBeVisible();
  await page.getByRole("tab", { name: "Drawing" }).click();
  const shown = page.getByTestId("field-drawing").getByTestId("ink");
  await expect(shown.locator("path.ink-stroke")).toHaveCount(3);
  await expect(shown.getByRole("toolbar")).toHaveCount(0);
  await expect(shown.getByRole("button")).toHaveCount(0);
  await expect(shown.getByTestId("ink-controls")).toHaveCount(0);
});

// ---- the endless plane -------------------------------------------------------------------

/** The saved view of a task's first body drawing, or null while it has none. */
async function storedView(admin: APIRequestContext, id: string) {
  const ink = bodyInk(await stored(admin, id))[0];
  return (ink?.blocks ?? []).find((b) => b.kind === "viewport")?.fields ?? null;
}

/** Drag the mouse by (dx, dy) screen pixels from a point given in ink units. */
async function mouseDrag(page: Page, surface: Locator, from: [number, number], dx: number, dy: number) {
  const [x, y] = await client(surface, ...from);
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx, y + dy, { steps: 8 });
  await page.mouse.up();
}

/**
 * With E2E_SHOTS set to a folder, light and dark screenshots of `what` there, as
 * `inkpan-<name>-<theme>.png`. Without it, nothing is written.
 */
async function shots(page: Page, name: string, what: Locator) {
  const dir = process.env.E2E_SHOTS;
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  await page.mouse.move(0, 0);
  for (const theme of ["light", "dark"]) {
    await page.evaluate((t) => (document.documentElement.dataset.theme = t), theme);
    await page.waitForTimeout(250);
    await what.screenshot({ path: join(dir, `inkpan-${name}-${theme}.png`) });
  }
  await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
}

test("by default a mouse drag pans the drawing and adds no stroke, while a stylus draws", async ({ page, admin }) => {
  const note = await create(admin, "task", `Pan ${token()}`, [
    { kind: "content", labels: ["body"], blocks: [{ ...threeLines, fields: { ...threeLines.fields, background: "grid" } }] },
  ]);
  await logIn(page, `/notes/${note.id}`);
  const ink = page.getByRole("textbox", { name: "Body" }).getByTestId("ink");
  const surface = ink.getByRole("img", { name: "Drawing" });
  await expect(surface.locator("path.ink-stroke")).toHaveCount(3);
  const toggle = ink.getByRole("button", { name: "Draw with mouse and touch" });
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
  await expect(surface).toHaveCSS("cursor", "grab");
  await expect(ink.getByTestId("ink-zoom")).toHaveText(/^\d+%$/);

  const before = await viewOf(surface);
  await mouseDrag(page, surface, [400, 200], -240, -60);
  const after = await viewOf(surface);
  expect(after.x - before.x).toBeCloseTo(240 / before.zoom, 0);
  expect(after.y - before.y).toBeCloseTo(60 / before.zoom, 0);
  await expect(surface.locator("path.ink-stroke")).toHaveCount(3);

  // A stylus draws with the pen wherever the plane now is under it.
  await penLine(surface, 300, 500, 900, [0.5, 0.6, 0.7, 0.6, 0.5], 21);
  await expect(surface.locator("path.ink-stroke")).toHaveCount(4);
  await expect.poll(async () => strokes(bodyInk(await stored(admin, note.id))[0]).length, { timeout: 10_000 }).toBe(4);
  const saved = bodyInk(await stored(admin, note.id))[0]!;
  expect(saved.blocks?.[0]?.kind).toBe("viewport");
  const drawn = strokes(saved)[3]!;
  expect(Math.abs(drawn[0]! - 500)).toBeLessThan(2);
  expect(Math.abs(drawn.at(-3)! - 900)).toBeLessThan(2);
  await shots(page, "panned", ink);
});

test("drawing with mouse and touch makes the mouse draw, and stays on for this device", async ({ page, admin }) => {
  const note = await create(admin, "task", `Mouse draws ${token()}`, [
    { kind: "content", labels: ["body"], blocks: [threeLines] },
  ]);
  await logIn(page, `/notes/${note.id}`);
  const surface = page.getByRole("textbox", { name: "Body" }).getByRole("img", { name: "Drawing" });
  await expect(surface.locator("path.ink-stroke")).toHaveCount(3);
  await drawWithMouse(page);
  await expect(surface).toHaveCSS("cursor", "crosshair");
  const before = await viewOf(surface);
  await mouseLine(page, surface, 350, 100, 700);
  await expect(surface.locator("path.ink-stroke")).toHaveCount(4);
  expect(await viewOf(surface)).toEqual(before);
  await saveAndWait(page);
  const points = strokes(bodyInk(await stored(admin, note.id))[0])[3]!;
  for (let j = 1; j < points.length; j += 3) expect(Math.abs(points[j]! - 350)).toBeLessThan(2);

  await page.reload();
  const again = page.getByRole("textbox", { name: "Body" });
  await expect(again.getByRole("button", { name: "Draw with mouse and touch" })).toHaveAttribute("aria-pressed", "true");
  // Off again, the mouse pans.
  await drawWithMouse(page, false);
  const shown = again.getByRole("img", { name: "Drawing" });
  await mouseDrag(page, shown, [400, 100], 100, 0);
  await expect(shown.locator("path.ink-stroke")).toHaveCount(4);
});

test("strokes drawn after panning far away lie off the old page, and the view is kept after a reload as one version", async ({
  page,
  admin,
}) => {
  const note = await create(admin, "task", `Far ${token()}`, [{ kind: "content", labels: ["body"], blocks: [threeLines] }]);
  await logIn(page, `/notes/${note.id}`);
  const surface = page.getByRole("textbox", { name: "Body" }).getByRole("img", { name: "Drawing" });
  await expect(surface.locator("path.ink-stroke")).toHaveCount(3);
  const zoom = (await viewOf(surface)).zoom;

  // Three pans, each saved before the next: the wheel right, the wheel up, then a drag.
  const [cx, cy] = await client(surface, 400, 200);
  await page.mouse.move(cx, cy);
  await page.mouse.wheel(3000, 0);
  await expect.poll(async () => (await storedView(admin, note.id))?.x as number | undefined).toBeCloseTo(3000 / zoom, 0);
  await page.mouse.wheel(0, -1500);
  await expect.poll(async () => (await storedView(admin, note.id))?.y as number | undefined).toBeCloseTo(-1500 / zoom, 0);
  const far = await viewOf(surface);
  await mouseDrag(page, surface, [far.x + 300 / zoom, far.y + 200 / zoom], -200, 0);
  await expect.poll(async () => (await storedView(admin, note.id))?.x as number | undefined).toBeCloseTo((3000 + 200) / zoom, 0);
  // The old page's strokes are out of view, and not drawn.
  await expect(surface.locator("path.ink-stroke")).toHaveCount(0);

  const view = await viewOf(surface);
  await penLine(surface, view.y + 100, view.x + 50, view.x + 300, [0.5, 0.5, 0.5, 0.5], 31);
  await expect(surface.locator("path.ink-stroke")).toHaveCount(1);
  await saveAndWait(page);
  const saved = bodyInk(await stored(admin, note.id))[0]!;
  const drawn = strokes(saved)[3]!;
  expect(drawn[0]!).toBeGreaterThan(3000);
  expect(drawn[1]!).toBeLessThan(-1000);
  expect(saved.fields).toEqual({ width: 800, height: 400 });

  // One version for all of it on top of the one the note was created with.
  const history = await admin.get(`/api/notes/${note.id}/history`);
  expect(((await history.json()) as { versions: unknown[] }).versions).toHaveLength(2);

  await page.reload();
  const again = page.getByRole("textbox", { name: "Body" }).getByRole("img", { name: "Drawing" });
  await expect(again.locator("path.ink-stroke")).toHaveCount(1);
  const back = await viewOf(again);
  const want = (await storedView(admin, note.id))!;
  expect(back.x).toBeCloseTo(want.x as number, 1);
  expect(back.y).toBeCloseTo(want.y as number, 1);
  expect(back.zoom).toBeCloseTo(want.zoom as number, 2);

  // Fit drawing brings every stroke back into view.
  await page.getByRole("toolbar", { name: "Drawing actions" }).getByRole("button", { name: "Fit drawing" }).click();
  await expect(again.locator("path.ink-stroke")).toHaveCount(4);
});

test("the highlighter draws yellow, translucent and much wider than the pen", async ({ page, admin }) => {
  const empty: Block = { kind: "ink", fields: { height: 400, background: "lines" }, blocks: [] };
  const note = await create(admin, "task", `Highlight ${token()}`, [{ kind: "content", labels: ["body"], blocks: [empty] }]);
  await logIn(page, `/notes/${note.id}`);
  const ink = page.getByRole("textbox", { name: "Body" }).getByTestId("ink");
  const surface = ink.getByRole("img", { name: "Drawing" });
  const tools = ink.getByRole("toolbar", { name: "Drawing tools" });
  await penLine(surface, 100, 60, 460, [0.5, 0.6, 0.7, 0.7, 0.6], 41);
  await tools.getByRole("button", { name: "Highlighter" }).click();
  await expect(tools.getByRole("button", { name: "Colour" })).toHaveAttribute("title", "Colour: Yellow");
  await expect(tools.getByRole("button", { name: "Stroke size" })).toHaveAttribute("title", "Stroke size: 24");
  await penLine(surface, 100, 40, 300, [0.2, 0.5, 1, 0.5, 0.2], 42);
  await expect(surface.locator("path.ink-stroke")).toHaveCount(2);
  const hl = surface.locator("path.ink-highlighter");
  await expect(hl).toHaveAttribute("fill", "#facc15");
  expect(Number(await hl.evaluate((el) => getComputedStyle(el).opacity))).toBeLessThan(1);
  const heights = await surface
    .locator("path.ink-stroke")
    .evaluateAll((els) => els.map((el) => ({ hl: el.classList.contains("ink-highlighter"), h: (el as SVGPathElement).getBBox().height })));
  const hlHeight = heights.find((h) => h.hl)!.h;
  const penHeight = heights.find((h) => !h.hl)!.h;
  expect(hlHeight).toBeGreaterThan(penHeight * 3);
  await saveAndWait(page);
  const [pen, highlight] = (bodyInk(await stored(admin, note.id))[0]!.blocks ?? []).map((b) => b.fields!);
  expect(pen).toMatchObject({ tool: "pen", colour: "#1f2937", size: 4 });
  expect(highlight).toMatchObject({ tool: "highlighter", colour: "#facc15", size: 24 });
  await shots(page, "highlighter", ink);
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });

  type Touch = { x: number; y: number; id: number };
  async function touches(page: Page, steps: { type: "touchStart" | "touchMove" | "touchEnd"; points: Touch[] }[]) {
    const cdp = await page.context().newCDPSession(page);
    for (const s of steps) await cdp.send("Input.dispatchTouchEvent", { type: s.type, touchPoints: s.points });
    await cdp.detach();
  }

  test("one finger pans and two fingers pinch to zoom; neither draws", async ({ page, admin }) => {
    const note = await create(admin, "task", `Phone pan ${token()}`, [
      { kind: "content", labels: ["body"], blocks: [threeLines] },
    ]);
    await logIn(page, `/notes/${note.id}`);
    const ink = page.getByRole("textbox", { name: "Body" }).getByTestId("ink");
    const surface = ink.getByRole("img", { name: "Drawing" });
    await expect(surface.locator("path.ink-stroke")).toHaveCount(3);
    await surface.scrollIntoViewIfNeeded();
    const box = (await surface.boundingBox())!;
    const cx = box.x + box.width / 2;
    const cy = box.y + box.height / 2;
    const before = await viewOf(surface);

    // One finger, 80 px to the left.
    await touches(page, [
      { type: "touchStart", points: [{ x: cx, y: cy, id: 1 }] },
      ...Array.from({ length: 8 }, (_, i) => ({ type: "touchMove" as const, points: [{ x: cx - (80 * (i + 1)) / 8, y: cy, id: 1 }] })),
      { type: "touchEnd", points: [] },
    ]);
    const panned = await viewOf(surface);
    expect(panned.x - before.x).toBeCloseTo(80 / before.zoom, 0);
    expect(panned.zoom).toBeCloseTo(before.zoom, 5);
    await shots(page, "phone", ink);
    await expect(surface.locator("path.ink-stroke")).toHaveCount(3);

    // Two fingers spreading from 60 px apart to 120: twice the zoom.
    await touches(page, [
      { type: "touchStart", points: [{ x: cx - 30, y: cy, id: 1 }, { x: cx + 30, y: cy, id: 2 }] },
      ...Array.from({ length: 8 }, (_, i) => {
        const d = 30 + (30 * (i + 1)) / 8;
        return { type: "touchMove" as const, points: [{ x: cx - d, y: cy, id: 1 }, { x: cx + d, y: cy, id: 2 }] };
      }),
      { type: "touchEnd", points: [] },
    ]);
    const pinched = await viewOf(surface);
    expect(pinched.zoom / panned.zoom).toBeCloseTo(2, 1);
    await expect(ink.getByTestId("ink-zoom")).toHaveText(`${Math.round(pinched.zoom * 100)}%`);
    // Zoomed in, the stroke now below the view is not drawn; none was added.
    await expect(surface.locator("path.ink-stroke")).not.toHaveCount(0);
    await expect.poll(async () => (await storedView(admin, note.id))?.zoom as number | undefined).toBeCloseTo(pinched.zoom, 2);
    expect(strokes(bodyInk(await stored(admin, note.id))[0])).toHaveLength(3);
  });
});
