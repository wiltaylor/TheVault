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
  return (ink?.blocks ?? []).map((s) => s.fields?.points as number[]);
}

const pressures = (points: number[]) => points.filter((_, i) => i % 3 === 2);

/** Leave what was edited, which saves at once, and wait until the note is saved. */
async function saveAndWait(page: Page) {
  await page.getByLabel("Title", { exact: true }).focus();
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
}

/** Client coordinates of a point given in the surface's ink units. */
async function client(surface: Locator, x: number, y: number): Promise<[number, number]> {
  await surface.scrollIntoViewIfNeeded();
  const box = (await surface.boundingBox())!;
  const { w, h } = await surface.evaluate((el) => ({
    w: (el as SVGSVGElement).viewBox.baseVal.width,
    h: (el as SVGSVGElement).viewBox.baseVal.height,
  }));
  return [box.x + (x * box.width) / w, box.y + (y * box.height) / h];
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

test("three mouse strokes on an ink block save as three strokes and reload as the same drawing", async ({
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
  for (const y of [100, 200, 300]) await mouseLine(page, surface, y, 100, 700);
  await expect(surface.locator("path.ink-stroke")).toHaveCount(3);
  const drawn = await paths(surface);
  await saveAndWait(page);

  const saved = await stored(admin, note.id);
  const ink = bodyInk(saved);
  expect(ink).toHaveLength(1);
  expect(ink[0]!.fields).toEqual({ width: 800, height: 400 });
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

  await mouseLine(page, surface, 200, 100, 700);
  await expect(surface.locator("path.ink-stroke")).toHaveCount(1);
  await saveAndWait(page);

  const ink = (await stored(admin, note.id)).note.blocks.find((b) => b.kind === "ink");
  expect(ink?.labels).toEqual(["drawing"]);
  expect(ink?.fields).toEqual({ width: 800, height: 400 });
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
