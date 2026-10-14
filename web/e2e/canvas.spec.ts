import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { activePane, expect, logIn, test, vaultDir } from "./fixtures";

interface Block {
  kind: string;
  labels?: string[];
  fields?: Record<string, unknown>;
  blocks?: Block[];
}

interface Stored {
  id: string;
  version: string;
  note: { fields: Record<string, unknown>; blocks: Block[] };
}

function unique(prefix: string): string {
  return `${prefix} ${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function create(
  admin: APIRequestContext,
  type: string,
  fields: Record<string, unknown>,
  blocks: Block[] = [],
): Promise<Stored> {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as Stored;
}

async function stored(admin: APIRequestContext, id: string): Promise<Stored> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return (await res.json()) as Stored;
}

function boardCanvas(note: Stored): Block | undefined {
  return note.note.blocks.find((b) => b.kind === "canvas" && b.labels?.[0] === "board");
}

const savedItem = async (admin: APIRequestContext, id: string, item: string) =>
  (boardCanvas(await stored(admin, id))?.blocks ?? []).find((b) => b.labels?.[0] === item);

async function openNote(p: Page, id: string, title: string) {
  await logIn(p, `/notes/${id}`);
  await expect(p.getByRole("heading", { level: 1, name: title })).toBeVisible();
}

/** Leave what was edited, which saves at once, and wait until the note is saved. */
async function saveAndWait(p: Page) {
  await p.getByLabel("Title", { exact: true }).focus();
  await expect(p.getByTestId("save-status")).toHaveText("Saved");
}

/** A card node: its box and its content blocks. */
function card(id: string, x: number, y: number, blocks: Block[], extra: Record<string, unknown> = {}): Block {
  return { kind: "node", labels: [id], fields: { kind: "card", x, y, width: 200, height: 100, ...extra }, blocks };
}

const para = (text: string): Block => ({ kind: "paragraph", fields: { text } });

/** A board whose canvas holds `items` after a view at the origin, at 100%. */
function board(items: Block[], fields: Record<string, unknown> = {}): Block[] {
  return [{ kind: "canvas", labels: ["board"], fields, blocks: [{ kind: "viewport", fields: { x: 0, y: 0, zoom: 1 } }, ...items] }];
}

/** What the canvas shows: each node's id, kind and box, and each edge's ends. */
async function drawn(canvas: Locator) {
  const nodes = await canvas.getByTestId("canvas-node").evaluateAll((els) =>
    els.map((el) => {
      const s = (el as HTMLElement).style;
      return [el.getAttribute("data-id"), el.getAttribute("data-kind"), s.left, s.top, s.width, s.height].join(" ");
    }),
  );
  const edges = await canvas
    .getByTestId("canvas-edge")
    .evaluateAll((els) =>
      els.map((el) => [...el.querySelectorAll("path:not(.canvas-edge-halo), polygon")].map((l) => l.outerHTML).join("")),
    );
  return { nodes, edges };
}

/** Open the canvas's "+" popup and choose `what`: Card or Note, or Group under More. */
async function addToCanvas(canvas: Locator, what: string) {
  await canvas.getByRole("button", { name: "Add to canvas" }).click();
  const popup = canvas.getByRole("dialog", { name: "Add to canvas" });
  await expect(popup).toBeVisible();
  await popup.getByRole("button", { name: new RegExp(`^${what}\\b`) }).click();
}

/** Drag from port `side` of node `from` and drop on node `to`, as a person would. */
async function connect(p: Page, canvas: Locator, from: string, side: string, to: string) {
  const fromNode = canvas.locator(`[data-testid="canvas-node"][data-id="${from}"]`);
  await fromNode.hover();
  const port = canvas.locator(`[data-testid="canvas-port"][data-node="${from}"][data-side="${side}"]`);
  await expect(port).toBeVisible();
  const a = (await port.boundingBox())!;
  const b = (await canvas.locator(`[data-testid="canvas-node"][data-id="${to}"]`).boundingBox())!;
  await p.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
  await p.mouse.down();
  await p.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 8 });
  await expect(canvas.getByTestId("canvas-edge-preview")).toBeVisible();
  await p.mouse.up();
}

/** The points of each drawn edge, from its `data-points`. */
async function edgePoints(canvas: Locator): Promise<{ x: number; y: number }[][]> {
  const attrs = await canvas.getByTestId("canvas-edge").evaluateAll((els) => els.map((el) => el.getAttribute("data-points") ?? ""));
  return attrs.map((a) => a.split(" ").map((xy) => ({ x: Number(xy.split(",")[0]), y: Number(xy.split(",")[1]) })));
}

/** The view as the canvas element reports it. */
async function viewOf(canvas: Locator) {
  const v = canvas.getByTestId("canvas-view");
  return {
    x: Number(await v.getAttribute("data-x")),
    y: Number(await v.getAttribute("data-y")),
    zoom: Number(await v.getAttribute("data-zoom")),
  };
}

/** The accessible names of the buttons in `l`, in order. */
async function buttonNames(l: Locator) {
  await expect(l).toBeVisible();
  return l.getByRole("button").evaluateAll((els) => els.map((e) => e.getAttribute("aria-label") ?? e.textContent?.trim()));
}

const nodeOf = (canvas: Locator, id: string) => canvas.locator(`[data-testid="canvas-node"][data-id="${id}"]`);

/** Click edge `id` a quarter of the way along its longest segment, clear of its caption. */
async function clickEdge(p: Page, canvas: Locator, id: string) {
  const edge = canvas.locator(`[data-testid="canvas-edge"][data-id="${id}"]`);
  const pts = (await edge.getAttribute("data-points"))!.split(" ").map((xy) => xy.split(",").map(Number) as [number, number]);
  let i = 0;
  for (let k = 1; k < pts.length - 1; k++) {
    const len = (j: number) => Math.hypot(pts[j + 1]![0] - pts[j]![0], pts[j + 1]![1] - pts[j]![1]);
    if (len(k) > len(i)) i = k;
  }
  const [a, b] = [pts[i]!, pts[i + 1]!];
  const view = (await canvas.getByTestId("canvas-view").boundingBox())!;
  const v = await viewOf(canvas);
  // The view's border is 1 px.
  const x = view.x + 1 + (a[0] + (b[0] - a[0]) / 4 - v.x) * v.zoom;
  const y = view.y + 1 + (a[1] + (b[1] - a[1]) / 4 - v.y) * v.zoom;
  await p.mouse.click(x, y);
}

/** A ULID made here, for a note file written straight into the vault. */
function ulid(): string {
  const alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
  let t = Date.now();
  let time = "";
  for (let i = 0; i < 10; i++) {
    time = alphabet[t % 32] + time;
    t = Math.floor(t / 32);
  }
  let rand = "";
  for (let i = 0; i < 16; i++) rand += alphabet[Math.floor(Math.random() * 32)];
  return time + rand;
}

test("+ offers Card, Note and Group; a card's blocks are edited in place with / and saved", async ({ page: p, admin }) => {
  const title = unique("Board");
  const note = await create(admin, "board", { title });
  await openNote(p, note.id, title);
  const canvas = p.getByTestId("field-board").getByTestId("canvas");
  await expect(canvas.getByTestId("canvas-node")).toHaveCount(0);

  // The popup holds Card and Note, and Group as the only thing under More.
  await canvas.getByRole("button", { name: "Add to canvas" }).click();
  const popup = canvas.getByRole("dialog", { name: "Add to canvas" });
  await expect(popup.getByRole("button")).toHaveText([/^Card/, /^Note/, "Group"]);
  for (const gone of ["Image", "File", "Link", "Query", "Text"]) await expect(popup.getByRole("button", { name: gone, exact: true })).toHaveCount(0);
  await popup.getByRole("button", { name: /^Card\b/ }).click();

  // A new card opens ready to write in: `/` inserts blocks as in a content field.
  const nodes = canvas.getByTestId("canvas-node");
  await expect(nodes).toHaveCount(1);
  const editor = canvas.getByRole("textbox", { name: "Card content" });
  await expect(editor).toBeFocused();
  await p.keyboard.type("/h2");
  await expect(p.getByRole("listbox", { name: "Insert block" }).getByRole("option")).toHaveText([/Heading 2/]);
  await p.keyboard.press("Enter");
  await p.keyboard.type("Plan");
  await p.keyboard.press("Enter");
  await p.keyboard.type("/code");
  // A card cannot hold a canvas, so the menu never offers one.
  await expect(p.getByRole("listbox", { name: "Insert block" }).getByRole("option")).toHaveText([/Code/]);
  await p.keyboard.press("Enter");
  await expect(nodes.first().locator(".cm-editor")).toBeVisible();
  await p.keyboard.type("/canvas");
  await expect(p.getByRole("listbox", { name: "Insert block" }).getByRole("option")).toHaveCount(0);
  for (let i = 0; i < 7; i++) await p.keyboard.press("Backspace");

  // Escape leaves the card; the blocks show read-only, and the canvas pans from the card again.
  await p.keyboard.press("Escape");
  await expect(editor).toHaveCount(0);
  await expect(nodes.first()).toHaveClass(/selected/);
  await expect(nodes.first().getByRole("heading", { name: "Plan" })).toBeVisible();
  await saveAndWait(p);
  const blocks = () => savedItem(admin, note.id, "card").then((n) => n?.blocks);
  expect(await blocks()).toEqual([
    { kind: "heading", fields: { level: 2, text: "Plan" } },
    { kind: "code", fields: { source: "" } },
  ]);

  // A double-click enters editing again where it was clicked, and Escape leaves it.
  await nodes.first().getByRole("heading", { name: "Plan" }).dblclick();
  await expect(editor).toBeFocused();
  await p.keyboard.press("End");
  await p.keyboard.type(" ahead");
  // While editing, dragging inside the card selects text rather than moving the card.
  const before = await nodes.first().boundingBox();
  const h = (await editor.locator("h3, h2").first().boundingBox())!;
  await p.mouse.move(h.x + 4, h.y + h.height / 2);
  await p.mouse.down();
  await p.mouse.move(h.x + 60, h.y + h.height / 2 + 30, { steps: 5 });
  await p.mouse.up();
  expect(await nodes.first().boundingBox()).toEqual(before);
  expect(await viewOf(canvas)).toEqual({ x: 0, y: 0, zoom: 1 });
  await p.keyboard.press("Escape");
  await expect(editor).toHaveCount(0);

  // Enter on the selected card edits it too; a click outside the card stops.
  await canvas.getByTestId("canvas-view").focus();
  await p.keyboard.press("Enter");
  await expect(editor).toBeFocused();
  await p.getByLabel("Title", { exact: true }).click();
  await expect(editor).toHaveCount(0);
  await expect(p.getByTestId("save-status")).toHaveText("Saved");
  expect((await blocks())?.[0]).toEqual({ kind: "heading", fields: { level: 2, text: "Plan ahead" } });

  // A second card joined to the first saves a canvas that validates and reloads identically.
  await addToCanvas(canvas, "Card");
  await expect(editor).toBeFocused();
  await p.keyboard.type("Second");
  await p.keyboard.press("Escape");
  const second = nodes.nth(1);
  const box = (await second.boundingBox())!;
  await p.mouse.move(box.x + 20, box.y + 20);
  await p.mouse.down();
  await p.mouse.move(box.x + 160, box.y + 90, { steps: 5 });
  await p.mouse.up();
  await connect(p, canvas, "card", "right", "card_2");
  await expect(canvas.getByTestId("canvas-edge")).toHaveCount(1);
  await saveAndWait(p);
  const saved = boardCanvas(await stored(admin, note.id))!;
  expect(saved.blocks?.map((b) => `${b.kind} ${b.labels?.[0] ?? ""}`)).toEqual(["viewport ", "node card", "node card_2", "edge card_to_card_2"]);
  expect(saved.blocks?.[2]?.blocks).toEqual([para("Second")]);
  const shown = await drawn(canvas);
  await p.reload();
  await expect(p.getByRole("heading", { level: 1, name: title })).toBeVisible();
  const reloaded = p.getByTestId("field-board").getByTestId("canvas");
  await expect(reloaded.getByTestId("canvas-node")).toHaveCount(2);
  expect(await drawn(reloaded)).toEqual(shown);
  expect(boardCanvas(await stored(admin, note.id))).toEqual(saved);
});

test("selecting a node shows its action cluster, not an inspector; its swatch sets and clears its colour", async ({
  page: p,
  admin,
}) => {
  const title = unique("Colours");
  const note = await create(admin, "board", { title }, board([card("a", 40, 80, [para("Card A")])]));
  await openNote(p, note.id, title);
  const canvas = p.getByTestId("field-board").getByTestId("canvas");
  const a = nodeOf(canvas, "a");
  await a.click();
  await expect(a).toHaveClass(/selected/);
  await expect(canvas.locator(".canvas-inspector")).toHaveCount(0);
  await expect(canvas.getByRole("group", { name: "Selected item" })).toHaveCount(0);

  // The cluster sits at the node's top-right corner.
  const cluster = canvas.getByRole("toolbar", { name: "Card actions" });
  expect(await buttonNames(cluster)).toEqual(["Colour", "Delete"]);
  const cb = (await cluster.boundingBox())!;
  const nb = (await a.boundingBox())!;
  expect(Math.abs(cb.x + cb.width - (nb.x + nb.width))).toBeLessThan(4);
  expect(cb.y + cb.height).toBeLessThanOrEqual(nb.y + 1);

  await cluster.getByRole("button", { name: "Colour" }).click();
  const palette = canvas.getByRole("group", { name: "Palette" });
  expect(await buttonNames(palette)).toEqual([
    "No colour",
    "Gray",
    "Brown",
    "Orange",
    "Yellow",
    "Green",
    "Blue",
    "Purple",
    "Pink",
    "Red",
  ]);
  await palette.getByRole("button", { name: "Purple" }).click();
  await expect(palette).toHaveCount(0);
  await expect(a).toHaveAttribute("data-colour", "purple");
  const purple = await p.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--palette-purple").trim());
  expect(await a.evaluate((el) => getComputedStyle(el).borderTopColor)).toBe(
    await p.evaluate((hex) => {
      const d = document.createElement("div");
      d.style.color = hex;
      document.body.append(d);
      const c = getComputedStyle(d).color;
      d.remove();
      return c;
    }, purple),
  );
  await saveAndWait(p);
  expect((await savedItem(admin, note.id, "a"))?.fields?.colour).toBe("purple");

  // No colour takes the item out again; Delete removes the node.
  await a.click();
  await cluster.getByRole("button", { name: "Colour" }).click();
  await expect(palette.getByRole("button", { name: "Purple" })).toHaveAttribute("aria-pressed", "true");
  await palette.getByRole("button", { name: "No colour" }).click();
  await saveAndWait(p);
  expect((await savedItem(admin, note.id, "a"))?.fields).not.toHaveProperty("colour");
  await a.click();
  await cluster.getByRole("button", { name: "Delete" }).click();
  await expect(canvas.getByTestId("canvas-node")).toHaveCount(0);
  await saveAndWait(p);
  expect(await savedItem(admin, note.id, "a")).toBeUndefined();
});

test("a note node opens its note: Open in a new tab, middle-click and Ctrl+click in background tabs", async ({ page: p, admin }) => {
  const target = await create(admin, "task", { title: unique("Pinned on canvas") });
  const targetTitle = String(target.note.fields.title);
  const title = unique("Opener");
  const note = await create(
    admin,
    "board",
    { title },
    board([{ kind: "node", labels: ["n"], fields: { kind: "note", note: target.id, x: 40, y: 60, width: 260, height: 140 } }]),
  );
  await openNote(p, note.id, title);
  const tabs = p.getByRole("tablist", { name: "Open tabs" }).getByRole("tab");
  await expect(tabs).toHaveCount(1);
  const canvas = p.getByTestId("field-board").getByTestId("canvas");
  const node = nodeOf(canvas, "n");
  await expect(node).toContainText(targetTitle);

  // A middle-click and a Ctrl/Cmd+click each open a tab behind this one.
  await node.click({ button: "middle", position: { x: 120, y: 90 } });
  await expect(tabs).toHaveCount(2);
  await expect(tabs.nth(0)).toHaveAttribute("aria-selected", "true");
  await expect(tabs.nth(1)).toHaveAccessibleName(targetTitle);
  await node.click({ modifiers: ["ControlOrMeta"], position: { x: 120, y: 90 } });
  await expect(tabs).toHaveCount(3);
  await expect(tabs.nth(0)).toHaveAttribute("aria-selected", "true");
  await expect(p).toHaveURL(new RegExp(`/notes/${note.id}$`));
  // Neither click selected or moved the node.
  await expect(node).not.toHaveClass(/selected/);

  // Open, in the selected note's cluster, opens it in a new tab and shows it.
  await node.click();
  const cluster = canvas.getByRole("toolbar", { name: "Note actions" });
  expect(await buttonNames(cluster)).toEqual(["Open", "Colour", "Delete"]);
  await cluster.getByRole("button", { name: "Open" }).click();
  await expect(tabs).toHaveCount(4);
  await expect(tabs.nth(3)).toHaveAttribute("aria-selected", "true");
  await expect(p).toHaveURL(new RegExp(`/notes/${target.id}$`));
  await expect(activePane(p).getByLabel("Title", { exact: true })).toHaveValue(targetTitle);
});

test("a connection's colour, all four arrows and its caption change and save, and its arrowheads take the colour", async ({
  page: p,
  admin,
}) => {
  const title = unique("Edges");
  const note = await create(
    admin,
    "board",
    { title },
    board([
      card("a", 40, 200, [para("Card A")]),
      card("b", 480, 200, [para("Card B")]),
      { kind: "edge", labels: ["ab"], fields: { from: "a", to: "b" } },
    ]),
  );
  await openNote(p, note.id, title);
  const canvas = p.getByTestId("field-board").getByTestId("canvas");
  const edge = canvas.locator('[data-testid="canvas-edge"][data-id="ab"]');
  await expect(edge.getByTestId("canvas-arrowhead")).toHaveCount(1);
  await clickEdge(p, canvas, "ab");
  await expect(edge).toHaveClass(/selected/);
  await expect(canvas.locator(".canvas-inspector")).toHaveCount(0);

  // The cluster sits at the connection's midpoint.
  const cluster = canvas.getByRole("toolbar", { name: "Connection actions" });
  expect(await buttonNames(cluster)).toEqual([
    "Colour",
    "No arrows",
    "Arrow at start",
    "Arrow at end",
    "Arrows at both ends",
    "Edit caption",
    "Delete",
  ]);
  const cb = (await cluster.boundingBox())!;
  const view = (await canvas.getByTestId("canvas-view").boundingBox())!;
  expect(Math.abs(cb.x + cb.width / 2 - (view.x + 360))).toBeLessThan(4);
  await expect(cluster.getByRole("button", { name: "Arrow at end" })).toHaveAttribute("aria-pressed", "true");
  // The arrow choices are drawn along the connection, which runs left to right.
  await expect(cluster.locator('[data-arrow="end"] g')).toHaveAttribute("transform", "rotate(0 12 12)");

  // Colour: the line and its arrowheads both take it.
  await cluster.getByRole("button", { name: "Colour" }).click();
  await canvas.getByRole("group", { name: "Palette" }).getByRole("button", { name: "Red" }).click();
  const colours = () =>
    edge.evaluate((g) => ({
      line: getComputedStyle(g.querySelector(".canvas-edge-line")!).stroke,
      heads: [...g.querySelectorAll(".canvas-arrowhead")].map((h) => getComputedStyle(h).fill),
    }));
  const red = await p.evaluate(() => {
    const d = document.createElement("div");
    d.style.color = "var(--palette-red)";
    document.body.append(d);
    const c = getComputedStyle(d).color;
    d.remove();
    return c;
  });
  await expect.poll(colours).toEqual({ line: red, heads: [red] });
  await saveAndWait(p);
  expect((await savedItem(admin, note.id, "ab"))?.fields?.colour).toBe("red");

  // Each arrow choice saves, and draws its heads at the ends it names, in the edge's colour.
  const arrows = [
    ["Arrow at start", "start", ["start"]],
    ["Arrows at both ends", "both", ["start", "end"]],
    ["No arrows", "none", []],
    ["Arrow at end", undefined, ["end"]],
  ] as const;
  for (const [name, item, heads] of arrows) {
    await clickEdge(p, canvas, "ab");
    await cluster.getByRole("button", { name }).click();
    await expect(cluster.getByRole("button", { name })).toHaveAttribute("aria-pressed", "true");
    await expect(edge.getByTestId("canvas-arrowhead")).toHaveCount(heads.length);
    expect(await edge.getByTestId("canvas-arrowhead").evaluateAll((els) => els.map((e) => e.getAttribute("data-end")))).toEqual(heads);
    expect((await colours()).heads).toEqual(heads.map(() => red));
    await saveAndWait(p);
    expect((await savedItem(admin, note.id, "ab"))?.fields?.arrow).toBe(item);
  }
  // The start arrowhead sits on a, the end one on b.
  await clickEdge(p, canvas, "ab");
  await cluster.getByRole("button", { name: "Arrows at both ends" }).click();
  const tips = await edge.getByTestId("canvas-arrowhead").evaluateAll((els) => els.map((e) => e.getAttribute("points")!.split(" ")[0]));
  expect(tips).toEqual(["240,250", "480,250"]);

  // A caption: written through the cluster, drawn at the midpoint over the line.
  await cluster.getByRole("button", { name: "Edit caption" }).click();
  const input = canvas.getByLabel("Caption", { exact: true });
  await expect(input).toBeFocused();
  await input.fill("depends on");
  await input.press("Enter");
  const caption = canvas.getByTestId("canvas-edge-label");
  await expect(caption).toHaveText("depends on");
  const lb = (await caption.boundingBox())!;
  expect(Math.abs(lb.x + lb.width / 2 - (view.x + 360))).toBeLessThan(3);
  expect(Math.abs(lb.y + lb.height / 2 - (view.y + 250))).toBeLessThan(3);
  expect(await caption.evaluate((el) => getComputedStyle(el).backgroundColor)).not.toBe("rgba(0, 0, 0, 0)");
  await saveAndWait(p);
  expect((await savedItem(admin, note.id, "ab"))?.fields?.label).toBe("depends on");

  // Clicking the caption of the selected connection edits it; clearing it removes it.
  await canvas.getByTestId("canvas-view").click({ position: { x: 20, y: 20 } });
  await expect(edge).not.toHaveClass(/selected/);
  await caption.click();
  await expect(edge).toHaveClass(/selected/);
  await caption.click();
  await expect(input).toHaveValue("depends on");
  await input.fill("");
  await input.press("Enter");
  await expect(caption).toHaveCount(0);
  await saveAndWait(p);
  expect((await savedItem(admin, note.id, "ab"))?.fields).not.toHaveProperty("label");

  // Delete removes the connection.
  await clickEdge(p, canvas, "ab");
  await cluster.getByRole("button", { name: "Delete" }).click();
  await expect(edge).toHaveCount(0);
  await saveAndWait(p);
  expect(await savedItem(admin, note.id, "ab")).toBeUndefined();
});

test("the bottom handle sets the canvas's height, by drag and by arrow keys, and the width follows the pane", async ({
  page: p,
  admin,
}) => {
  const title = unique("Tall");
  const note = await create(admin, "board", { title }, board([card("a", 40, 40, [para("Card A")])]));
  await openNote(p, note.id, title);
  const canvas = p.getByTestId("field-board").getByTestId("canvas");
  const view = canvas.getByTestId("canvas-view");
  const handle = canvas.getByRole("separator", { name: "Canvas height" });
  await expect(handle).toHaveAttribute("aria-valuenow", "480");
  expect((await view.boundingBox())!.height).toBeCloseTo(480, 0);

  const h = (await handle.boundingBox())!;
  await p.mouse.move(h.x + h.width / 2, h.y + h.height / 2);
  await p.mouse.down();
  await p.mouse.move(h.x + h.width / 2, h.y + h.height / 2 + 120, { steps: 6 });
  await p.mouse.up();
  await expect(handle).toHaveAttribute("aria-valuenow", "600");
  expect((await view.boundingBox())!.height).toBeCloseTo(600, 0);
  await saveAndWait(p);
  expect(boardCanvas(await stored(admin, note.id))?.fields).toEqual({ height: 600 });

  // The handle takes focus; Up and Down resize by 20, Shift by 100.
  await handle.focus();
  await p.keyboard.press("ArrowUp");
  await p.keyboard.press("Shift+ArrowUp");
  await expect(handle).toHaveAttribute("aria-valuenow", "480");
  await p.keyboard.press("ArrowDown");
  await expect(handle).toHaveAttribute("aria-valuenow", "500");
  await saveAndWait(p);
  expect(boardCanvas(await stored(admin, note.id))?.fields).toEqual({ height: 500 });

  // A reload keeps the height.
  await p.reload();
  await expect(p.getByTestId("field-board").getByTestId("canvas-view")).toHaveCSS("height", "500px");

  // The width always fills the pane, and follows it when the pane narrows.
  const pane = activePane(p);
  const wide = (await p.getByTestId("field-board").getByTestId("canvas-view").boundingBox())!.width;
  await pane.getByRole("button", { name: "Pane actions" }).click();
  await p.getByRole("menuitem", { name: "Split right" }).click();
  const left = p.locator(".pane").filter({ visible: true }).first();
  const narrowView = left.getByTestId("field-board").getByTestId("canvas-view");
  await expect.poll(async () => (await narrowView.boundingBox())!.width).toBeLessThan(wide * 0.7);
  const content = (await left.getByTestId("field-board").boundingBox())!;
  expect(Math.abs((await narrowView.boundingBox())!.width - content.width)).toBeLessThan(4);
});

test("pan and zoom are saved with the canvas and come back after a reload", async ({ page: p, admin }) => {
  const title = unique("Board");
  const note = await create(admin, "board", { title }, board([card("a", 40, 40, [para("Anchor")])]));
  await openNote(p, note.id, title);
  const canvas = p.getByTestId("field-board").getByTestId("canvas");
  await canvas.getByRole("button", { name: "Zoom controls" }).click();
  await expect(canvas.getByTestId("canvas-zoom")).toHaveText("100%");

  await canvas.getByRole("button", { name: "Zoom in" }).click();
  await canvas.getByRole("button", { name: "Zoom in" }).click();
  await expect(canvas.getByTestId("canvas-zoom")).toHaveText("156%");
  // Drag an empty part of the canvas to pan.
  const view = canvas.getByTestId("canvas-view");
  const vb = (await view.boundingBox())!;
  await p.mouse.move(vb.x + vb.width - 40, vb.y + vb.height - 40);
  await p.mouse.down();
  await p.mouse.move(vb.x + vb.width - 140, vb.y + vb.height - 90, { steps: 5 });
  await p.mouse.up();
  const panned = await viewOf(canvas);
  expect(panned.x).toBeGreaterThan(50);
  expect(panned.y).toBeGreaterThan(20);
  await saveAndWait(p);

  const vp = boardCanvas(await stored(admin, note.id))?.blocks?.find((b) => b.kind === "viewport")?.fields;
  expect(vp?.zoom).toBeCloseTo(1.5625, 3);
  expect(vp?.x).toBeCloseTo(panned.x, 1);
  expect(vp?.y).toBeCloseTo(panned.y, 1);

  await p.reload();
  const reloaded = p.getByTestId("field-board").getByTestId("canvas");
  await expect(reloaded.getByTestId("canvas-zoom")).toHaveText("156%");
  const back = await viewOf(reloaded);
  expect(back.x).toBeCloseTo(panned.x, 1);
  expect(back.y).toBeCloseTo(panned.y, 1);
  expect(back.zoom).toBeCloseTo(1.5625, 3);
});

test("a canvas on a type's template shows at the template's height on every note, and its card's query view follows new notes", async ({
  page: p,
  admin,
}) => {
  const tag = unique("Crew");
  const red = await create(admin, "crew", { title: `${tag} red` });
  const blue = await create(admin, "crew", { title: `${tag} blue` });
  await create(admin, "board", { title: `${tag} red board 1`, crew: red.id });
  await create(admin, "board", { title: `${tag} blue board 1`, crew: blue.id });

  // Every crew shows the template's canvas, and the query view in its card sees that crew as `this`.
  for (const [crew, name] of [
    [blue, "blue"],
    [red, "red"],
  ] as const) {
    if (crew === blue) await openNote(p, crew.id, `${tag} ${name}`);
    else {
      await p.goto(`/notes/${crew.id}`);
      await expect(p.getByRole("heading", { level: 1, name: `${tag} ${name}` })).toBeVisible();
    }
    const canvas = p.getByTestId("canvas");
    await expect(canvas).toHaveCount(1);
    await expect(canvas.getByTestId("canvas-node")).toHaveCount(2);
    await expect(canvas.getByTestId("canvas-edge")).toHaveCount(1);
    await expect(canvas.locator("strong", { hasText: "boards" })).toBeVisible();
    const rows = canvas.getByTestId("query-view").getByTestId("query-row");
    await expect(rows).toHaveText([`${tag} ${name} board 1`]);
    // The template's height, and no handle: the canvas is the template's, not the note's.
    await expect(canvas.getByTestId("canvas-view")).toHaveCSS("height", "360px");
    await expect(canvas.getByRole("separator", { name: "Canvas height" })).toHaveCount(0);
  }

  // A board made elsewhere for the red crew appears without a reload; the blue crew's does not.
  const rows = p.getByTestId("canvas").getByTestId("query-view").getByTestId("query-row");
  await create(admin, "board", { title: `${tag} red board 2`, crew: red.id });
  await create(admin, "board", { title: `${tag} blue board 2`, crew: blue.id });
  await expect(rows).toHaveText([`${tag} red board 1`, `${tag} red board 2`], { timeout: 15_000 });
  await expect(p.getByTestId("canvas").getByTestId("query-count")).toHaveText("2 notes");
});

test("a query view block inside a card shows its results", async ({ page: p, admin }) => {
  const tag = unique("Queried");
  await create(admin, "task", { title: `${tag} one` });
  await create(admin, "task", { title: `${tag} two` });
  const title = unique("Board");
  const note = await create(
    admin,
    "board",
    { title },
    board([
      card("q", 40, 40, [
        { kind: "heading", fields: { level: 2, text: "Tasks" } },
        {
          kind: "query_view",
          fields: { query: `from task search "${tag}" sort title asc` },
          blocks: [{ kind: "view", labels: ["cards"], fields: { kind: "cards" } }],
        },
      ], { width: 320, height: 360 }),
    ]),
  );
  await openNote(p, note.id, title);
  const q = nodeOf(p.getByTestId("field-board").getByTestId("canvas"), "q");
  await expect(q.getByRole("heading", { name: "Tasks" })).toBeVisible();
  await expect(q.getByTestId("query-card")).toHaveCount(2);
  await expect(q.getByTestId("query-card").first()).toContainText(`${tag} one`);
});

test("a canvas field placed on a template shows on every note of the type", async ({ page: p, admin }) => {
  const tag = unique("Field");
  const one = await create(admin, "board", { title: `${tag} one` });
  const two = await create(admin, "board", { title: `${tag} two` }, board([card("n", 0, 0, [para("Two's node")])]));
  await openNote(p, one.id, `${tag} one`);
  await expect(p.getByTestId("field-board").getByTestId("canvas")).toBeVisible();
  await expect(p.getByTestId("field-board").getByTestId("canvas-node")).toHaveCount(0);
  await p.goto(`/notes/${two.id}`);
  await expect(p.getByTestId("field-board").getByTestId("canvas-node")).toHaveText(["Two's node"]);
});

test("a canvas inserted into content is edited in place and saved in the content field", async ({ page: p, admin }) => {
  const title = unique("Board");
  const note = await create(admin, "board", { title });
  await openNote(p, note.id, title);
  const body = p.getByTestId("field-body");
  await body.getByRole("textbox", { name: "Body" }).click();
  await p.keyboard.type("/canvas");
  await expect(p.getByRole("listbox", { name: "Insert block" }).getByRole("option")).toHaveText([/Canvas/]);
  await p.keyboard.press("Enter");
  const canvas = body.getByTestId("canvas");
  await expect(canvas).toBeVisible();
  await addToCanvas(canvas, "Card");
  await expect(canvas.getByTestId("canvas-node")).toHaveCount(1);
  // The card's editor takes the keys, not the content field around the canvas.
  await expect(canvas.getByRole("textbox", { name: "Card content" })).toBeFocused();
  await p.keyboard.type("Inside **content**");
  await p.keyboard.press("Escape");
  await expect(canvas.locator("strong", { hasText: "content" })).toBeVisible();
  await saveAndWait(p);

  const content = (await stored(admin, note.id)).note.blocks.find((b) => b.kind === "content");
  const inner = content?.blocks?.find((b) => b.kind === "canvas");
  expect(inner?.labels).toBeUndefined();
  const node = inner?.blocks?.find((b) => b.kind === "node");
  expect(node?.fields?.kind).toBe("card");
  expect(node?.blocks).toEqual([para("Inside **content**")]);
  // The canvas block in the content gets its own height handle too.
  await expect(canvas.getByRole("separator", { name: "Canvas height" })).toBeVisible();
});

test("the template editor adds a canvas that every note of the type then shows", async ({ page: p, admin }) => {
  // A type of its own, so changing its template touches no other test.
  const name = `t${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  const put = await admin.put(`/api/types/${name}`, {
    data: { wcl: `label = "Temp"\nplural = "Temps"\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n  }\n}\n` },
  });
  expect(put.status(), await put.text()).toBe(200);
  const a = await create(admin, name, { title: "Temp A" });
  const b = await create(admin, name, { title: "Temp B" });

  await logIn(p, `/settings/types/${name}/templates`);
  // The template is edited like a note: `/` inserts a canvas.
  await p.getByRole("textbox", { name: "Main content" }).locator("p").last().click();
  await p.keyboard.type("/canvas");
  await p.keyboard.press("Enter");
  const canvas = p.getByTestId("canvas");
  await expect(canvas).toBeVisible();
  await addToCanvas(canvas, "Card");
  await expect(canvas.getByRole("textbox", { name: "Card content" })).toBeFocused();
  await p.keyboard.type("On every temp");
  await p.keyboard.press("Escape");
  await p.getByRole("button", { name: "Save template" }).click();
  await expect(p.getByText("Saved the default template.")).toBeVisible();

  for (const n of [a, b]) {
    await p.goto(`/notes/${n.id}`);
    await expect(p.getByTestId("canvas").getByTestId("canvas-node")).toHaveText(["On every temp"]);
  }
});

test("the corner + adds a Card and a Note in the middle of the view, and the magnifier shows zoom controls", async ({
  page: p,
  admin,
}) => {
  const target = await create(admin, "task", { title: unique("Pinned on canvas") });
  const title = unique("Corner");
  const note = await create(admin, "board", { title });
  await openNote(p, note.id, title);
  const canvas = p.getByTestId("field-board").getByTestId("canvas");
  await expect(canvas.locator(".canvas-toolbar")).toHaveCount(0);
  await expect(canvas.getByRole("button", { name: "Add to canvas" })).toBeVisible();
  await expect(canvas.getByRole("button", { name: "Zoom controls" })).toBeVisible();

  // A Card lands in the middle of the view.
  await addToCanvas(canvas, "Card");
  const newCard = canvas.locator('[data-testid="canvas-node"][data-kind="card"]');
  await expect(newCard).toHaveCount(1);
  const view = (await canvas.getByTestId("canvas-view").boundingBox())!;
  const c = (await newCard.boundingBox())!;
  expect(Math.abs(c.x + c.width / 2 - (view.x + view.width / 2))).toBeLessThan(40);
  expect(Math.abs(c.y + c.height / 2 - (view.y + view.height / 2))).toBeLessThan(40);

  // A Note is found by its title.
  await canvas.getByRole("button", { name: "Add to canvas" }).click();
  const popup = canvas.getByRole("dialog", { name: "Add to canvas" });
  await expect(popup.getByText("More")).toBeVisible();
  await expect(popup.getByRole("button", { name: "Group", exact: true })).toBeVisible();
  await popup.getByRole("button", { name: /^Note\b/ }).click();
  await popup.getByLabel("Find a note").fill(String(target.note.fields.title));
  await popup.getByRole("button", { name: String(target.note.fields.title) }).click();
  await expect(popup).toHaveCount(0);
  const noteNode = canvas.locator('[data-testid="canvas-node"][data-kind="note"]');
  await expect(noteNode).toContainText(String(target.note.fields.title));
  await saveAndWait(p);
  const kinds = (boardCanvas(await stored(admin, note.id))?.blocks ?? []).filter((b) => b.kind === "node").map((b) => b.fields?.kind);
  expect(kinds).toEqual(["card", "note"]);

  // The magnifier toggles the zoom controls; zoom in changes the scale.
  const zoomIn = canvas.getByRole("button", { name: "Zoom in" });
  await expect(zoomIn).toHaveCount(0);
  await canvas.getByRole("button", { name: "Zoom controls" }).click();
  for (const name of ["Zoom out", "Zoom in", "Fit to content", "Zoom to 100%"]) {
    await expect(canvas.getByRole("button", { name })).toBeVisible();
  }
  const before = await viewOf(canvas);
  await zoomIn.click();
  expect((await viewOf(canvas)).zoom).toBeCloseTo(before.zoom * 1.25, 3);
  await expect(canvas.getByTestId("canvas-zoom")).toHaveText("125%");
  await canvas.getByRole("button", { name: "Zoom to 100%" }).click();
  await expect(canvas.getByTestId("canvas-zoom")).toHaveText("100%");
  await canvas.getByRole("button", { name: "Zoom controls" }).click();
  await expect(zoomIn).toHaveCount(0);
});

/** A board whose canvas holds two cards, `a` and `b`, and no edges. */
async function twoCards(admin: APIRequestContext, title: string, b = { x: 420, y: 180 }) {
  return create(admin, "board", { title }, board([card("a", 40, 40, [para("Card A")]), card("b", b.x, b.y, [para("Card B")])]));
}

const savedEdges = async (admin: APIRequestContext, id: string) =>
  (boardCanvas(await stored(admin, id))?.blocks ?? []).filter((b) => b.kind === "edge");

test("dragging from a port to another node draws an orthogonal edge and saves it; empty canvas cancels", async ({
  page: p,
  admin,
}) => {
  const title = unique("Ports");
  const note = await twoCards(admin, title);
  await openNote(p, note.id, title);
  const canvas = p.getByTestId("field-board").getByTestId("canvas");
  await expect(canvas.getByTestId("canvas-node")).toHaveCount(2);

  // Dropped on empty canvas: nothing.
  await nodeOf(canvas, "a").hover();
  const port = canvas.locator('[data-testid="canvas-port"][data-node="a"][data-side="bottom"]');
  const pb = (await port.boundingBox())!;
  const vb = (await canvas.getByTestId("canvas-view").boundingBox())!;
  await p.mouse.move(pb.x + pb.width / 2, pb.y + pb.height / 2);
  await p.mouse.down();
  await p.mouse.move(vb.x + 60, vb.y + vb.height - 40, { steps: 6 });
  await expect(canvas.getByTestId("canvas-edge-preview")).toBeVisible();
  await p.mouse.up();
  await expect(canvas.getByTestId("canvas-edge-preview")).toHaveCount(0);
  await expect(canvas.getByTestId("canvas-edge")).toHaveCount(0);

  // Dropped anywhere on the other node: an edge, snapped to the closest ports.
  await connect(p, canvas, "a", "bottom", "b");
  await expect(canvas.getByTestId("canvas-edge")).toHaveCount(1);
  await saveAndWait(p);
  expect(await savedEdges(admin, note.id)).toEqual([
    { kind: "edge", labels: ["a_to_b"], fields: { from: "a", to: "b", from_side: "right", to_side: "left" } },
  ]);

  // Every segment is horizontal or vertical, leaving a's right side and entering b's left.
  const [pts] = await edgePoints(canvas);
  expect(pts!.length).toBeGreaterThan(2);
  for (let i = 1; i < pts!.length; i++) {
    const [u, v] = [pts![i - 1]!, pts![i]!];
    expect(u.x === v.x || u.y === v.y, JSON.stringify(pts)).toBe(true);
  }
  expect(pts![0]).toEqual({ x: 240, y: 90 });
  expect(pts!.at(-1)).toEqual({ x: 420, y: 230 });
  expect(pts![1]!.y).toBe(90);
  expect(pts!.at(-2)!.y).toBe(230);
});

test("moving a node so other sides are closer re-snaps the edge in the saved file", async ({ page: p, admin }) => {
  const title = unique("Resnap");
  const note = await create(
    admin,
    "board",
    { title },
    board([
      card("a", 40, 40, [para("Card A")]),
      card("b", 420, 40, [para("Card B")]),
      // Stored sides that no longer match: re-snapped on the next move.
      { kind: "edge", labels: ["ab"], fields: { from: "a", to: "b", from_side: "top", to_side: "top" } },
    ]),
  );
  await openNote(p, note.id, title);
  const canvas = p.getByTestId("field-board").getByTestId("canvas");
  const b = nodeOf(canvas, "b");
  const box = (await b.boundingBox())!;
  // Drag b from beside a to below it.
  await p.mouse.move(box.x + 30, box.y + 30);
  await p.mouse.down();
  await p.mouse.move(box.x + 30 - 380, box.y + 30 + 240, { steps: 10 });
  await p.mouse.up();
  await saveAndWait(p);
  expect((await savedEdges(admin, note.id))[0]?.fields).toEqual({ from: "a", to: "b", from_side: "bottom", to_side: "top" });
  const [pts] = await edgePoints(canvas);
  for (let i = 1; i < pts!.length; i++) expect(pts![i - 1]!.x === pts![i]!.x || pts![i - 1]!.y === pts![i]!.y).toBe(true);
  expect(pts![0]).toEqual({ x: 140, y: 140 });
});

test("a group is selected by its frame, renamed by double-clicking its label, coloured, resized and deleted", async ({
  page: p,
  admin,
}) => {
  const title = unique("Grouped");
  const note = await create(
    admin,
    "board",
    { title },
    board([
      { kind: "group", labels: ["g"], fields: { label: "Plan", x: 40, y: 60, width: 400, height: 300 } },
      { kind: "node", labels: ["c"], fields: { kind: "card", x: 80, y: 120, width: 160, height: 80, group: "g" }, blocks: [para("Inside")] },
    ]),
  );
  await openNote(p, note.id, title);
  const canvas = p.getByTestId("field-board").getByTestId("canvas");
  const group = canvas.getByTestId("canvas-group");
  const c = nodeOf(canvas, "c");
  const saved = () => stored(admin, note.id).then((n) => boardCanvas(n)?.blocks?.find((b) => b.kind === "group"));

  // A click on the frame, away from the label and the card, selects the group: no inspector,
  // its cluster instead.
  await group.click({ position: { x: 360, y: 260 } });
  await expect(group).toHaveClass(/selected/);
  await expect(canvas.locator(".canvas-inspector")).toHaveCount(0);
  const cluster = canvas.getByRole("toolbar", { name: "Group actions" });
  expect(await buttonNames(cluster)).toEqual(["Colour", "Delete"]);

  // A double-click on its label renames it: Enter confirms, Escape cancels.
  const label = canvas.getByTestId("canvas-group-label");
  await label.dblclick();
  const rename = canvas.getByLabel("Group label");
  await expect(rename).toBeFocused();
  await expect(rename).toHaveValue("Plan");
  await rename.fill("Launch");
  await rename.press("Enter");
  await expect(label).toHaveText("Launch");
  await label.dblclick();
  await rename.fill("Discarded");
  await rename.press("Escape");
  await expect(label).toHaveText("Launch");
  await saveAndWait(p);
  expect((await saved())?.fields?.label).toBe("Launch");

  // Its swatch colours the frame.
  await group.click({ position: { x: 360, y: 260 } });
  await cluster.getByRole("button", { name: "Colour" }).click();
  await canvas.getByRole("group", { name: "Palette" }).getByRole("button", { name: "Green" }).click();
  await expect(group).toHaveAttribute("data-colour", "green");
  await saveAndWait(p);
  expect((await saved())?.fields?.colour).toBe("green");

  // Its handle resizes it, and the new size is saved.
  await group.click({ position: { x: 360, y: 260 } });
  const handle = group.locator(".canvas-resize");
  await expect(handle).toBeVisible();
  const h = (await handle.boundingBox())!;
  await p.mouse.move(h.x + h.width / 2, h.y + h.height / 2);
  await p.mouse.down();
  await p.mouse.move(h.x + h.width / 2 + 80, h.y + h.height / 2 + 40, { steps: 6 });
  await p.mouse.up();
  await saveAndWait(p);
  expect((await saved())?.fields).toMatchObject({ x: 40, y: 60, width: 480, height: 340 });

  // A card inside the selected group takes its own click.
  await c.click({ position: { x: 20, y: 20 } });
  await expect(c).toHaveClass(/selected/);
  await expect(group).not.toHaveClass(/selected/);

  // Moving the group by its frame moves its card with it.
  await group.click({ position: { x: 360, y: 260 } });
  const g = (await group.boundingBox())!;
  await p.mouse.move(g.x + 360, g.y + 260);
  await p.mouse.down();
  await p.mouse.move(g.x + 360 + 50, g.y + 260 + 30, { steps: 6 });
  await p.mouse.up();
  await saveAndWait(p);
  const moved = boardCanvas(await stored(admin, note.id))?.blocks ?? [];
  expect(moved.find((b) => b.kind === "group")?.fields).toMatchObject({ x: 90, y: 90 });
  expect(moved.find((b) => b.kind === "node")?.fields).toMatchObject({ x: 130, y: 150 });

  // Delete removes the selected group and leaves its card. (Saving above moved focus to the title.)
  await group.click({ position: { x: 360, y: 260 } });
  await expect(group).toHaveClass(/selected/);
  await p.keyboard.press("Delete");
  await expect(group).toHaveCount(0);
  await saveAndWait(p);
  const left = boardCanvas(await stored(admin, note.id))?.blocks ?? [];
  expect(left.filter((b) => b.kind === "group")).toEqual([]);
  expect(left.find((b) => b.kind === "node")?.fields?.group).toBeUndefined();
});

test("a canvas in the old format, written straight to the vault, loads as cards and saves as cards", async ({ page: p, admin }) => {
  const id = ulid();
  const title = unique("Old board");
  // The file below reaches the index only once it is saved, so its query looks for another note.
  const listed = await create(admin, "task", { title: unique("Listed") });
  const listedTitle = String(listed.note.fields.title);
  const file = join(vaultDir(), "notes", "board", `${id}.wcl`);
  writeFileSync(
    file,
    `title = "${title}"

canvas board {
  viewport {
    x    = 0.0
    y    = 0.0
    zoom = 1.0
  }

  node idea {
    kind   = "text"
    x      = 20.0
    y      = 20.0
    width  = 240.0
    height = 100.0
    colour = "yellow"
    text   = "An **old** idea\\n\\nSecond paragraph"
  }

  node site {
    kind   = "link"
    x      = 320.0
    y      = 20.0
    width  = 240.0
    height = 80.0
    url    = "https://example.com/docs"
  }

  node work {
    kind   = "query"
    x      = 20.0
    y      = 180.0
    width  = 320.0
    height = 200.0
    view   = "list"
    query  = "from task where title == \\"${listedTitle}\\""
  }

  edge idea_to_site {
    from  = "idea"
    to    = "site"
    arrow = "both"
  }
}
`,
  );
  await openNote(p, id, title);
  const canvas = p.getByTestId("field-board").getByTestId("canvas");
  await expect(canvas.getByTestId("canvas-node")).toHaveCount(3);
  expect(await canvas.getByTestId("canvas-node").evaluateAll((els) => els.map((e) => e.getAttribute("data-kind")))).toEqual([
    "card",
    "card",
    "card",
  ]);
  const idea = nodeOf(canvas, "idea");
  await expect(idea.locator("p")).toHaveText(["An old idea", "Second paragraph"]);
  await expect(idea.locator("strong")).toHaveText("old");
  await expect(idea).toHaveAttribute("data-colour", "yellow");
  await expect(nodeOf(canvas, "site").getByRole("link", { name: "https://example.com/docs" })).toBeVisible();
  await expect(nodeOf(canvas, "work").getByTestId("query-row")).toHaveText([listedTitle]);
  await expect(canvas.locator('[data-testid="canvas-edge"][data-id="idea_to_site"]').getByTestId("canvas-arrowhead")).toHaveCount(2);

  // The server reads it as cards; the file is rewritten as cards the next time it is saved.
  const read = boardCanvas(await stored(admin, id));
  expect(read?.blocks?.find((b) => b.labels?.[0] === "idea")).toEqual({
    kind: "node",
    labels: ["idea"],
    fields: { kind: "card", x: 20, y: 20, width: 240, height: 100, colour: "yellow" },
    blocks: [para("An **old** idea"), para("Second paragraph")],
  });
  expect(readFileSync(file, "utf8")).toContain('kind   = "text"');
  const b = (await nodeOf(canvas, "site").boundingBox())!;
  await p.mouse.move(b.x + 200, b.y + 10);
  await p.mouse.down();
  await p.mouse.move(b.x + 240, b.y + 30, { steps: 4 });
  await p.mouse.up();
  await saveAndWait(p);
  const text = readFileSync(file, "utf8");
  expect(text).not.toMatch(/kind += "(text|link|query)"/);
  expect(text.match(/kind += "card"/g)).toHaveLength(3);
  expect(text).toContain("query_view {");
});
