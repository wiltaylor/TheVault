import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// Column layouts: inserting them from the `/` menu, typing in each column, resizing by the gap,
// dragging blocks into, out of and beside each other, the rules that remove empty columns and
// unwrap a lone one, stacking in a narrow pane, and a template laying fields out in columns.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

const TYPE_WCL = `label = "Layout"
plural = "Layouts"

field status {
  kind    = "select"
  label   = "Status"
  options = ["todo", "doing", "done"]
}

field owner {
  kind  = "string"
  label = "Owner"
}

field due {
  kind  = "date"
  label = "Due"
}

field body {
  kind  = "content"
  label = "Body"
}

template default {
  tab "Main" {
    slot "body"
  }
}
`;

interface Stored {
  version: string;
  wcl: string;
  note: { fields: Record<string, unknown>; blocks: { kind: string; labels?: string[]; blocks?: Json[] }[] };
}
type Json = { kind: string; fields?: Record<string, unknown>; blocks?: Json[]; labels?: string[] };

async function setup(admin: APIRequestContext, body: Json[] = [], wcl = TYPE_WCL) {
  const name = `col_${token()}`;
  const put = await admin.put(`/api/types/${name}`, { data: { wcl } });
  expect(put.status(), await put.text()).toBe(200);
  const blocks = body.length ? [{ kind: "content", labels: ["body"], blocks: body }] : [];
  const res = await admin.post(`/api/notes?type=${name}`, {
    data: { note: { fields: { title: `Columns ${token()}`, status: "todo", owner: "Kim" }, blocks } },
  });
  expect(res.status(), await res.text()).toBe(201);
  return { name, id: ((await res.json()) as { id: string }).id };
}

async function stored(admin: APIRequestContext, id: string): Promise<Stored> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return (await res.json()) as Stored;
}

/** The body as stored, each block as its text, a layout as the texts of its columns. */
async function storedBody(admin: APIRequestContext, id: string): Promise<unknown[]> {
  const n = await stored(admin, id);
  const body = n.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === "body");
  const show = (b: Json): unknown =>
    b.kind === "columns" ? (b.blocks ?? []).map((c) => (c.blocks ?? []).map(show)) : String(b.fields?.text ?? b.kind);
  return (body?.blocks ?? []).map(show);
}

const p = (text: string): Json => ({ kind: "paragraph", fields: { text } });
const col = (...blocks: Json[]): Json => ({ kind: "column", fields: { width: 1 }, blocks });

const body = (page: Page): Locator => page.getByRole("textbox", { name: "Body" });

async function saved(page: Page) {
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
}

/** Drag `handle` to a point in `target`'s box, given as offsets from its top left, as a person would. */
async function dragTo(page: Page, handle: Locator, target: Locator, at: (b: { width: number; height: number }) => [number, number]) {
  const from = (await handle.boundingBox())!;
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(from.x + from.width / 2 + 10, from.y + from.height / 2 - 5, { steps: 4 });
  const to = (await target.boundingBox())!;
  const [dx, dy] = at(to);
  await page.mouse.move(to.x + dx, to.y + dy, { steps: 10 });
  await page.mouse.up();
}

/** Hover a block so its gutter is beside it, and return its drag handle. */
async function handleOf(page: Page, block: Locator): Promise<Locator> {
  await block.hover();
  const handle = page.getByTestId("field-body").getByTestId("block-handle");
  await expect(handle).toBeVisible();
  return handle;
}

test("3 columns from the / menu take typing in each column and are there after a reload", async ({ page, admin }) => {
  const { id } = await setup(admin);
  await logIn(page, `/notes/${id}`);
  await body(page).click();
  await page.keyboard.type("/col");
  const menu = page.getByRole("listbox", { name: "Insert block" });
  await expect(menu.getByRole("option", { name: "2 columns" })).toBeVisible();
  await expect(menu.getByRole("option", { name: "4 columns" })).toBeVisible();
  await expect(menu.getByRole("option", { name: /Columns…/ })).toBeVisible();
  await menu.getByRole("option", { name: "3 columns" }).click();
  const cols = body(page).getByTestId("column");
  await expect(cols).toHaveCount(3);

  // The caret is in the first column; each column takes its own text.
  await page.keyboard.type("Left words");
  await cols.nth(1).click();
  await page.keyboard.type("Middle words");
  await cols.nth(2).click();
  await page.keyboard.type("Right words");
  // A heading inside a column, from the / menu there, which offers no layouts.
  await page.keyboard.press("Enter");
  await page.keyboard.type("/");
  await expect(menu.getByRole("option", { name: /^Heading 2/ })).toBeVisible();
  await expect(menu.getByRole("option", { name: "2 columns" })).toHaveCount(0);
  await page.keyboard.type("h2");
  await page.keyboard.press("Enter");
  await page.keyboard.type("Right title");
  await saved(page);
  expect(await storedBody(admin, id)).toEqual([[["Left words"], ["Middle words"], ["Right words", "Right title"]]]);

  // Nor does the menu in another column: layouts do not nest.
  await body(page).locator("p", { hasText: "Left words" }).click();
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await page.keyboard.type("/");
  await expect(menu.getByRole("option", { name: /Columns…/ })).toHaveCount(0);
  await page.keyboard.press("Escape");

  await page.reload();
  await expect(body(page).getByTestId("column")).toHaveCount(3);
  await expect(body(page).getByTestId("column").nth(1)).toHaveText("Middle words");
  await expect(body(page).getByTestId("column").nth(2).locator("h2, h3")).toHaveText("Right title");
  const wcl = (await stored(admin, id)).wcl;
  expect(wcl).toContain("columns {");
  expect(wcl.match(/column \{/g)).toHaveLength(3);
});

test("Columns… picks a count up to six", async ({ page, admin }) => {
  const { id } = await setup(admin);
  await logIn(page, `/notes/${id}`);
  await body(page).click();
  await page.keyboard.type("/columns");
  const menu = page.getByRole("listbox", { name: "Insert block" });
  await menu.getByRole("option", { name: /Columns…/ }).click();
  await expect(menu.getByRole("option")).toHaveText(["2 columns", "3 columns", "4 columns", "5 columns", "6 columns"]);
  await menu.getByRole("option", { name: "6 columns" }).click();
  await expect(body(page).getByTestId("column")).toHaveCount(6);
  // The toolbar cannot add a seventh.
  await body(page).getByTestId("columns").hover();
  await expect(page.getByRole("button", { name: "Add a column" })).toBeDisabled();
});

test("dragging the gap resizes the columns, the keyboard nudges it, and the widths save", async ({ page, admin }) => {
  const { id } = await setup(admin, [{ kind: "columns", blocks: [col(p("alpha")), col(p("beta"))] }]);
  await logIn(page, `/notes/${id}`);
  const cols = body(page).getByTestId("column");
  await expect(cols).toHaveCount(2);
  const before = (await cols.nth(0).boundingBox())!;
  const gap = body(page).getByRole("separator", { name: "Resize columns" });
  await body(page).getByTestId("columns").hover();
  const g = (await gap.boundingBox())!;
  const row = (await body(page).getByTestId("columns").boundingBox())!;
  await page.mouse.move(g.x + g.width / 2, g.y + 10);
  await page.mouse.down();
  await page.mouse.move(row.x + row.width * 0.7, g.y + 10, { steps: 8 });
  await page.mouse.up();
  const after = (await cols.nth(0).boundingBox())!;
  expect(after.width).toBeGreaterThan(before.width * 1.25);
  await saved(page);
  const widths = async () => {
    const n = await stored(admin, id);
    const lay = n.note.blocks.find((b) => b.kind === "content")!.blocks![0]!;
    return lay.blocks!.map((c) => Number(c.fields?.width));
  };
  const [a, b] = await widths();
  expect(a! / (a! + b!)).toBeGreaterThan(0.6);

  // Focused, Left moves the gap left.
  await gap.focus();
  await page.keyboard.press("ArrowLeft");
  await saved(page);
  const [a2, b2] = await widths();
  expect(a2! / (a2! + b2!)).toBeLessThan(a! / (a! + b!));

  // The widths hold over a reload, and Equal widths resets them.
  await page.reload();
  const reloaded = (await body(page).getByTestId("column").nth(0).boundingBox())!;
  expect(reloaded.width).toBeGreaterThan(before.width * 1.2);
  await body(page).getByTestId("columns").hover();
  await page.getByRole("button", { name: "Equal widths" }).click();
  await saved(page);
  expect(await widths()).toEqual([1, 1]);
});

test("a block drags into a column and out again", async ({ page, admin }) => {
  const { id } = await setup(admin, [p("loose"), { kind: "columns", blocks: [col(p("left one")), col(p("right one"))] }, p("tail")]);
  await logIn(page, `/notes/${id}`);
  const para = (t: string) => body(page).locator("p", { hasText: t });
  await expect(para("right one")).toBeVisible();

  // Into the right column, onto the top of its block.
  await dragTo(page, await handleOf(page, para("loose")), para("right one"), () => [12, 3]);
  await expect(body(page).getByTestId("column").nth(1).locator("p")).toHaveText(["loose", "right one"]);
  await saved(page);
  expect(await storedBody(admin, id)).toEqual([[["left one"], ["loose", "right one"]], "tail"]);

  // And out again, above the tail.
  await dragTo(page, await handleOf(page, para("loose")), para("tail"), () => [12, 3]);
  await saved(page);
  expect(await storedBody(admin, id)).toEqual([[["left one"], ["right one"]], "loose", "tail"]);
});

test("a block dropped on another's right edge makes two columns of them", async ({ page, admin }) => {
  const { id } = await setup(admin, [p("first block"), p("second block"), p("third block")]);
  await logIn(page, `/notes/${id}`);
  const para = (t: string) => body(page).locator("p", { hasText: t });
  await expect(para("third block")).toBeVisible();
  await dragTo(page, await handleOf(page, para("third block")), para("first block"), (b) => [b.width - 10, b.height / 2]);
  await expect(body(page).getByTestId("column")).toHaveCount(2);
  await saved(page);
  expect(await storedBody(admin, id)).toEqual([[["first block"], ["third block"]], "second block"]);
});

test("emptying a column removes it, and a layout left with one column unwraps", async ({ page, admin }) => {
  const { id } = await setup(admin, [{ kind: "columns", blocks: [col(p("keep me")), col(p("x")), col(p("y"))] }, p("after")]);
  await logIn(page, `/notes/${id}`);
  const cols = body(page).getByTestId("column");
  await expect(cols).toHaveCount(3);

  // Delete the middle column's only block: its text, then Backspace on the empty line.
  await body(page).locator("p", { hasText: /^x$/ }).click();
  await page.keyboard.press("End");
  await page.keyboard.press("Backspace");
  await page.keyboard.press("Backspace");
  await expect(cols).toHaveCount(2);
  await saved(page);
  expect(await storedBody(admin, id)).toEqual([[["keep me"], ["y"]], "after"]);

  // The last other column goes the same way: one column left, the layout unwraps.
  await body(page).locator("p", { hasText: /^y$/ }).click();
  await page.keyboard.press("End");
  await page.keyboard.press("Backspace");
  await page.keyboard.press("Backspace");
  await expect(body(page).getByTestId("columns")).toHaveCount(0);
  await saved(page);
  expect(await storedBody(admin, id)).toEqual(["keep me", "after"]);
});

test("columns stack in a narrow pane and sit side by side in a wide one", async ({ page, admin }) => {
  const { id } = await setup(admin, [{ kind: "columns", blocks: [col(p("one side")), col(p("other side"))] }]);
  await logIn(page, `/notes/${id}`);
  const sideBySide = async (pane: Locator) => {
    const cols = pane.getByTestId("column");
    const a = (await cols.nth(0).boundingBox())!;
    const b = (await cols.nth(1).boundingBox())!;
    return Math.abs(a.y - b.y) < 2 && b.x > a.x + a.width / 2;
  };
  const pane = activePane(page);
  await expect(pane.getByTestId("column")).toHaveCount(2);
  expect(await sideBySide(pane)).toBe(true);

  await pane.getByRole("button", { name: "Pane actions" }).click();
  await page.getByRole("menuitem", { name: "Split right" }).click();
  const panes = page.locator(".pane").filter({ visible: true });
  await expect(panes).toHaveCount(2);
  for (const i of [0, 1]) {
    const pn = panes.nth(i);
    await expect(pn.getByTestId("column")).toHaveCount(2);
    const a = (await pn.getByTestId("column").nth(0).boundingBox())!;
    const b = (await pn.getByTestId("column").nth(1).boundingBox())!;
    // Stacked, in order, at full width.
    expect(b.y).toBeGreaterThan(a.y + a.height - 1);
    expect(Math.abs(a.width - b.width)).toBeLessThan(2);
  }
  // Editing still works while stacked.
  await panes.nth(0).locator("p", { hasText: "other side" }).click();
  await page.keyboard.press("End");
  await page.keyboard.type(" too");
  await expect(panes.nth(0).getByTestId("save-status")).toHaveText("Saved");
  expect(await storedBody(admin, id)).toEqual([[["one side"], ["other side too"]]]);

  // Closing the split widens the pane again, and the columns sit side by side.
  await panes.nth(1).getByRole("button", { name: "Pane actions" }).click();
  await page.getByRole("menuitem", { name: "Close pane" }).click();
  await expect(panes).toHaveCount(1);
  await expect.poll(() => sideBySide(activePane(page))).toBe(true);
});

const TEMPLATE_WCL = TYPE_WCL.replace(
  `template default {
  tab "Main" {
    slot "body"
  }
}`,
  `template default {
  tab "Main" {
    columns {
      column {
        width = 1
        slot "status"
        slot "due"
      }
      column {
        width = 1
        slot "owner" {
          label = "Who"
        }
        paragraph {
          text = "Owned by {{owner}}."
        }
      }
    }
    slot "body"
  }
}`,
);

test("a template's fields in two columns are edited side by side and left out of the Properties panel", async ({ page, admin }) => {
  const { id } = await setup(admin, [], TEMPLATE_WCL);
  await logIn(page, `/notes/${id}`);
  const layout = page.getByTestId("template-columns");
  const status = layout.getByTestId("field-status");
  const owner = layout.getByTestId("field-owner");
  await expect(status).toBeVisible();
  await expect(owner.getByLabel("Who")).toHaveValue("Kim");
  const a = (await status.boundingBox())!;
  const b = (await owner.boundingBox())!;
  expect(Math.abs(a.y - b.y)).toBeLessThan(4);
  expect(b.x).toBeGreaterThan(a.x + a.width - 4);
  await expect(layout).toContainText("Owned by Kim.");

  // Editing a field in a column saves it, and the fixed text follows.
  await owner.getByLabel("Who").fill("Ana");
  await expect(layout).toContainText("Owned by Ana.");
  await saved(page);
  expect((await stored(admin, id)).note.fields.owner).toBe("Ana");

  // Every field is on the page.
  await page.getByRole("button", { name: "Properties", exact: true }).click();
  const sidebar = page.getByRole("complementary", { name: "Properties" });
  await expect(sidebar).toContainText("Every property is on the page.");
  await expect(sidebar.getByTestId("field-status")).toHaveCount(0);
  await page.getByRole("button", { name: "Properties", exact: true }).click();
});

test("the template editor places properties inside columns and saves them there", async ({ page, admin }) => {
  const { name, id } = await setup(admin);
  await logIn(page, `/settings/types/${name}/templates`);
  const editor = page.getByRole("textbox", { name: "Main content" });
  await expect(editor.getByTestId("template-item-body")).toBeVisible();
  // A layout before the body, with a property in each column.
  await editor.getByTestId("template-item-body").hover();
  await page.getByTestId("block-add").click();
  const menu = page.getByRole("listbox", { name: "Insert block" });
  await page.keyboard.type("col");
  await menu.getByRole("option", { name: "2 columns" }).click();
  await page.keyboard.type("/stat");
  await menu.getByRole("option", { name: "Status" }).click();
  await editor.getByTestId("column").nth(1).click();
  await page.keyboard.type("/owner");
  await menu.getByRole("option", { name: "Owner" }).click();
  await expect(editor.getByTestId("column").nth(0).getByTestId("template-item-status")).toBeVisible();
  await expect(editor.getByTestId("column").nth(1).getByTestId("template-item-owner")).toBeVisible();
  // Placed in a column, a field leaves the / menu's Property section.
  await editor.getByTestId("column").nth(1).click();
  await page.keyboard.type("/");
  await expect(menu.getByRole("option", { name: "Due" })).toBeVisible();
  await expect(menu.getByRole("option", { name: "Status" })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.keyboard.press("Backspace");

  await page.getByRole("button", { name: "Save template" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved the default template.");
  const wcl = (await (await admin.get(`/api/types/${name}/templates/default`)).json()).wcl as string;
  expect(wcl).toMatch(/columns \{\n\s+column \{\n\s+width = 1\n\s+slot "status"\n\s+\}\n\s+column \{\n\s+width = 1\n\s+slot "owner"/);

  await page.goto(`/notes/${id}`);
  await expect(page.getByTestId("template-columns").getByTestId("field-owner").getByLabel("Owner")).toHaveValue("Kim");
});

test("cards and canvas cards show columns read-only", async ({ page, admin }) => {
  const { name } = await setup(admin);
  const card = `template card {\n  tab "Card" {\n    slot "title"\n    columns {\n      column {\n        width = 1\n        slot "status"\n      }\n      column {\n        width = 1\n        slot "owner"\n      }\n    }\n  }\n}\n`;
  const put = await admin.put(`/api/types/${name}/templates/card`, { data: { wcl: card } });
  expect(put.status(), await put.text()).toBe(200);
  const canvas = {
    kind: "canvas",
    blocks: [
      { kind: "viewport", fields: { x: 0, y: 0, zoom: 1 } },
      {
        kind: "node",
        labels: ["c"],
        fields: { kind: "card", x: 20, y: 20, width: 320, height: 160 },
        blocks: [{ kind: "columns", blocks: [col(p("card left")), col(p("card right"))] }],
      },
    ],
  };
  const query = {
    kind: "query_view",
    fields: { query: `from ${name} sort title asc` },
    blocks: [{ kind: "view", labels: ["cards"], fields: { kind: "cards" } }],
  };
  const res = await admin.post("/api/notes?type=page", {
    data: { note: { fields: { title: `Read-only ${name}` }, blocks: [{ kind: "content", labels: ["body"], blocks: [query, canvas] }] } },
  });
  expect(res.status(), await res.text()).toBe(201);
  await logIn(page, `/notes/${(await res.json()).id}`);

  // The card template's fields, in their columns.
  const qcard = page.getByTestId("query-card").first();
  await expect(qcard.getByTestId("column")).toHaveCount(2);
  await expect(qcard.getByTestId("column").nth(0)).toContainText("todo");
  await expect(qcard.getByTestId("column").nth(1)).toContainText("Kim");

  // The canvas card's layout: a narrow card stacks its columns.
  const node = page.locator('[data-testid="canvas-node"][data-id="c"]');
  await expect(node.getByTestId("column")).toHaveText(["card left", "card right"]);
  const a = (await node.getByTestId("column").nth(0).boundingBox())!;
  const b = (await node.getByTestId("column").nth(1).boundingBox())!;
  expect(b.y).toBeGreaterThan(a.y + a.height - 1);
});
