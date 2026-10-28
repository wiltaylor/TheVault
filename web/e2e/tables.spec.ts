import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// A `table` content block (docs/vault-format.md, "table"): inserted from the `/` menu as three
// rows of three cells, the first the header; Tab moves through the cells and adds a row from the
// last; a column takes an alignment; it is stored as rows of cells and comes back after a reload.
// Pasted Markdown makes one, copying it gives Markdown back, and a template shows one read-only.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

const TYPE_WCL = `label = "Ledger"

field body {
  kind  = "content"
  label = "Body"
}

template default {
  tab "Main" {
    slot "title"
    table {
      header = true
      align  = ["left", "right"]
      row {
        cells = ["Shown on", "Count"]
      }
      row {
        cells = ["**{{title}}**", "1"]
      }
    }
    slot "body"
  }
}
`;

type Json = { kind: string; fields?: Record<string, unknown>; blocks?: Json[]; labels?: string[] };

async function setup(admin: APIRequestContext) {
  const t = token();
  const type = `ledger_${t}`;
  const put = await admin.put(`/api/types/${type}`, { data: { wcl: TYPE_WCL } });
  expect(put.status(), await put.text()).toBe(200);
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields: { title: `Ledger ${t}` }, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  return { t, note: (await res.json()) as { id: string; version: string } };
}

async function storedBody(admin: APIRequestContext, id: string): Promise<Json[]> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  const n = (await res.json()) as { note: { blocks: Json[] } };
  return n.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === "body")?.blocks ?? [];
}

const body = (page: Page): Locator => activePane(page).getByRole("textbox", { name: "Body" });
const row = (...cells: string[]): Json => ({ kind: "row", fields: { cells } });

/** The text of each cell of `table`, row by row. */
async function cells(table: Locator): Promise<string[][]> {
  return table.locator("tr").evaluateAll((rows) => rows.map((r) => [...r.querySelectorAll("th, td")].map((c) => c.textContent ?? "")));
}

async function saved(page: Page) {
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
}

test("a table from the / menu: typed cell by cell, a row added with Tab, a column aligned, kept after a reload", async ({ page, admin }) => {
  const s = await setup(admin);
  await logIn(page, `/notes/${s.note.id}`);
  await body(page).click();
  await page.keyboard.type("/table");
  await page.getByRole("listbox", { name: "Insert block" }).getByRole("option", { name: "Table" }).click();

  const table = body(page).getByTestId("table-block");
  await expect(table.locator("th")).toHaveCount(3);
  await expect(table.locator("td")).toHaveCount(6);
  // The caret starts in the first header cell; Tab moves on, and from the last cell adds a row.
  for (const [i, text] of ["Item", "Qty", "Notes", "Apples", "3", "ripe", "Pears", "10", ""].entries()) {
    if (i > 0) await page.keyboard.press("Tab");
    if (text) await page.keyboard.type(text);
  }
  await page.keyboard.press("Tab");
  await page.keyboard.type("Plums");
  await expect(table.locator("tr")).toHaveCount(4);

  // The Qty column, aligned right from the table's tools.
  await table.locator("td").nth(1).click();
  await table.getByRole("combobox", { name: "Column alignment" }).selectOption("right");
  await expect(table.locator("td").nth(1)).toHaveCSS("text-align", "right");
  await saved(page);
  const want = [
    {
      kind: "table",
      fields: { align: ["left", "right", "left"], header: true },
      blocks: [row("Item", "Qty", "Notes"), row("Apples", "3", "ripe"), row("Pears", "10", ""), row("Plums", "", "")],
    },
  ];
  await expect.poll(() => storedBody(admin, s.note.id)).toEqual(want);

  await page.reload();
  const again = body(page).getByTestId("table-block");
  await expect(again.locator("th")).toHaveText(["Item", "Qty", "Notes"]);
  expect(await cells(again)).toEqual([
    ["Item", "Qty", "Notes"],
    ["Apples", "3", "ripe"],
    ["Pears", "10", ""],
    ["Plums", "", ""],
  ]);
  await expect(again.locator("th").nth(1)).toHaveCSS("text-align", "right");
});

test("a pasted Markdown table becomes a table, and copied it is Markdown again", async ({ page, admin, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const s = await setup(admin);
  await logIn(page, `/notes/${s.note.id}`);
  await body(page).click();
  const md = "| Fruit | Price | Note |\n| :--- | ---: | :---: |\n| **Apples** | 3 | a \\| b |\n| Pears | 10 | `x` |";
  // As a code editor copies it: the Markdown as text, and HTML that holds no table.
  await body(page).evaluate(
    (el, { md }) => {
      const dt = new DataTransfer();
      dt.setData("text/plain", md);
      dt.setData("text/html", `<div><span>${md.replace(/\n/g, "<br>")}</span></div>`);
      el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
    },
    { md },
  );
  const table = body(page).getByTestId("table-block");
  expect(await cells(table)).toEqual([
    ["Fruit", "Price", "Note"],
    ["Apples", "3", "a | b"],
    ["Pears", "10", "x"],
  ]);
  await expect(table.locator("td strong")).toHaveText("Apples");
  await saved(page);
  await expect
    .poll(() => storedBody(admin, s.note.id))
    .toEqual([
      {
        kind: "table",
        fields: { align: ["left", "right", "center"], header: true },
        blocks: [row("Fruit", "Price", "Note"), row("**Apples**", "3", "a | b"), row("Pears", "10", "`x`")],
      },
    ]);

  // The table's Copy as Markdown.
  await table.locator("td").first().click();
  await table.getByRole("button", { name: "Copy as Markdown" }).click();
  await expect(table.getByRole("button", { name: "Copied" })).toBeVisible();
  // The same table, its left column's delimiter written plain.
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(md.replace(":--- |", "--- |"));

  // A range of cells, selected by dragging across them and copied with the keyboard.
  const from = await table.locator("td").nth(1).boundingBox();
  const to = await table.locator("td").nth(5).boundingBox();
  await page.mouse.move(from!.x + 5, from!.y + from!.height / 2);
  await page.mouse.down();
  await page.mouse.move(to!.x + to!.width / 2, to!.y + to!.height / 2, { steps: 5 });
  await page.mouse.up();
  await expect(table.locator(".selectedCell")).toHaveCount(4);
  await page.keyboard.press("ControlOrMeta+c");
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe("|  |  |\n| ---: | :---: |\n| 3 | a \\| b |\n| 10 | `x` |");
});

test("a template's table shows read-only on the note, its cells' markup and field references filled in", async ({ page, admin }) => {
  const s = await setup(admin);
  await logIn(page, `/notes/${s.note.id}`);
  const fixed = activePane(page).getByTestId("table-block").filter({ hasText: "Shown on" });
  await expect(fixed.locator("th")).toHaveText(["Shown on", "Count"]);
  await expect(fixed.locator("td strong")).toHaveText(`Ledger ${s.t}`);
  await expect(fixed.locator("td").nth(1)).toHaveCSS("text-align", "right");
  // It is not editable: no cell takes the caret, and the table has no tools.
  await expect(fixed.locator("[contenteditable='true']")).toHaveCount(0);
  await expect(fixed.getByRole("toolbar", { name: "Table" })).toHaveCount(0);
});
