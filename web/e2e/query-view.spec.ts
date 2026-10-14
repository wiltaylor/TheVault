import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";
import { MASS_NOTES } from "./vault";

function unique(prefix: string): string {
  return `${prefix} ${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function create(admin: APIRequestContext, type: string, fields: Record<string, unknown>, blocks: unknown[] = []) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string; version: string };
}

/** A page whose body holds `blocks`. */
function page(admin: APIRequestContext, title: string, blocks: unknown[]) {
  return create(admin, "page", { title }, blocks.length ? [{ kind: "content", labels: ["body"], blocks }] : []);
}

/** The titles shown by a query view's rows or cards, in order. */
async function titles(items: Locator): Promise<string[]> {
  return (await items.getByTestId("query-title").allTextContents()).map((t) => t.trim());
}

function viewTab(view: Locator, name: string) {
  return view.getByRole("tablist", { name: "Views" }).getByRole("tab", { name, exact: true });
}

async function openNote(p: Page, id: string, title: string) {
  await logIn(p, `/notes/${id}`);
  await expect(p.getByRole("heading", { level: 1, name: title })).toBeVisible();
}

test("a project's template query view shows only that project's tasks", async ({ page: p, admin }) => {
  const mineTitle = unique("Mine");
  const mine = await create(admin, "project", { title: mineTitle });
  const other = await create(admin, "project", { title: unique("Other") });
  const tag = unique("Task");
  for (const n of [1, 2, 3]) await create(admin, "task", { title: `${tag} mine ${n}`, project: mine.id });
  for (const n of [1, 2]) await create(admin, "task", { title: `${tag} other ${n}`, project: other.id });
  await create(admin, "task", { title: `${tag} loose` });

  await openNote(p, mine.id, mineTitle);
  const view = p.getByTestId("query-view");
  await expect(view.getByRole("heading", { name: "Tasks" })).toBeVisible();
  const rows = view.getByTestId("query-row");
  await expect(rows).toHaveCount(3);
  expect(await titles(rows)).toEqual([`${tag} mine 1`, `${tag} mine 2`, `${tag} mine 3`]);

  await p.goto(`/notes/${other.id}`);
  await expect(view.getByTestId("query-row")).toHaveCount(2);
  expect(await titles(view.getByTestId("query-row"))).toEqual([`${tag} other 1`, `${tag} other 2`]);
});

test("a table view and a cards view over the same query switch and show the same notes", async ({
  page: p,
  admin,
}) => {
  const title = unique("Switch");
  const project = await create(admin, "project", { title });
  const tag = unique("Card");
  await create(admin, "task", { title: `${tag} a`, status: "todo", estimate: 2, project: project.id });
  await create(admin, "task", { title: `${tag} b`, status: "doing", estimate: 5, project: project.id });

  await openNote(p, project.id, title);
  const view = p.getByTestId("query-view");
  await expect(viewTab(view, "Table")).toHaveAttribute("aria-selected", "true");
  const rows = view.getByTestId("query-row");
  await expect(rows).toHaveCount(2);
  const tableTitles = await titles(rows);
  await expect(view.getByTestId("query-columns")).toHaveText(["Title", "Status", "Estimate"]);
  await expect(rows.nth(1)).toContainText("doing");
  await expect(rows.nth(1)).toContainText("5");

  await viewTab(view, "Cards").click();
  await expect(viewTab(view, "Cards")).toHaveAttribute("aria-selected", "true");
  await expect(rows).toHaveCount(0);
  const cards = view.getByTestId("query-card");
  await expect(cards).toHaveCount(2);
  expect(await titles(cards)).toEqual(tableTitles);
  // The task type's card template: the title, and the status under the card's own label.
  await expect(cards.nth(1)).toContainText("State");
  await expect(cards.nth(1)).toContainText("doing");
  await expect(cards.nth(1)).not.toContainText("Estimate");

  await viewTab(view, "Table").click();
  await expect(rows).toHaveCount(2);
  expect(await titles(rows)).toEqual(tableTitles);
});

test("a query returning 50,000 notes renders fewer than 300 rows or cards in every view", async ({
  page: p,
  admin,
}) => {
  const title = unique("Everything");
  const note = await page(admin, title, [
    {
      kind: "query_view",
      fields: { query: "from mass sort title asc" },
      blocks: [
        { kind: "view", labels: ["table"], fields: { kind: "table", columns: ["title", "created"] } },
        { kind: "view", labels: ["cards"], fields: { kind: "cards" } },
        { kind: "view", labels: ["list"], fields: { kind: "list" } },
        { kind: "view", labels: ["canvas"], fields: { kind: "canvas" } },
        { kind: "view", labels: ["details"], fields: { kind: "details" } },
      ],
    },
  ]);

  await openNote(p, note.id, title);
  const view = p.getByTestId("query-view");
  await expect(view.getByTestId("query-count")).toHaveText(`${MASS_NOTES} notes`);
  const items = view.locator("[data-testid=query-row], [data-testid=query-card], [data-testid=query-detail]");

  for (const kind of ["table", "cards", "list", "canvas", "details"]) {
    await viewTab(view, kind).click();
    await expect(view.locator(`[data-view-kind=${kind}]`)).toBeVisible();
    await expect(items.first()).toContainText("Mass 00001");
    const count = await items.count();
    expect(count, `${kind} renders ${count} items`).toBeGreaterThan(0);
    expect(count, `${kind} renders ${count} items`).toBeLessThan(300);
  }

  // Paging on keeps the DOM small. Each further page is one more cursor request, so the test
  // goes a few pages in rather than to the end of 50,000.
  await view.getByRole("button", { name: "Next page" }).click();
  await view.getByRole("button", { name: "Next page" }).click();
  await expect(view.getByText("Page 3 of 2500")).toBeVisible();
  await expect(items.first()).toContainText("Mass 00041");
  expect(await items.count()).toBeLessThan(300);

  for (const kind of ["table", "cards", "list"]) {
    await viewTab(view, kind).click();
    const scroller = view.locator(`[data-view-kind=${kind}] .virtual-list`);
    await scroller.evaluate((el) => el.scrollTo(0, el.scrollHeight / 20));
    await expect(items.getByText(/^Mass 02\d{3}$/).first()).toBeVisible({ timeout: 15_000 });
    expect(await items.count()).toBeLessThan(300);
  }
});

test("a query view switched to canvas shows the query's notes as cards on a canvas, placed ones where saved", async ({
  page: p,
  admin,
}) => {
  const title = unique("Canvas view");
  const project = await create(admin, "project", { title });
  const tag = unique("Node");
  const a = await create(admin, "task", { title: `${tag} a`, status: "todo", project: project.id });
  await create(admin, "task", { title: `${tag} b`, status: "doing", project: project.id });
  await create(admin, "task", { title: `${tag} c`, status: "done", project: project.id });
  const note = await page(admin, title, [
    {
      kind: "query_view",
      fields: { query: `from task where project == "${project.id}" sort title asc` },
      blocks: [
        { kind: "view", labels: ["table"], fields: { kind: "table" } },
        {
          kind: "view",
          labels: ["board"],
          fields: { kind: "canvas", title: "Board" },
          blocks: [{ kind: "place", labels: [a.id], fields: { x: 600.0, y: 40.0, width: 300.0, height: 200.0 } }],
        },
      ],
    },
  ]);

  await openNote(p, note.id, title);
  const view = p.getByTestId("query-view");
  await expect(view.getByTestId("query-row")).toHaveCount(3);
  await viewTab(view, "Board").click();
  await expect(view.locator("[data-view-kind=canvas]")).toBeVisible();
  await expect(view.getByText(/not available yet/)).toHaveCount(0);

  const canvas = view.getByTestId("canvas");
  const nodes = canvas.getByTestId("canvas-node");
  await expect(nodes).toHaveCount(3);
  const cards = canvas.getByTestId("query-card");
  expect(await titles(cards)).toEqual([`${tag} a`, `${tag} b`, `${tag} c`]);
  // The task type's card template draws each card.
  await expect(cards.nth(1)).toContainText("State");
  await expect(cards.nth(1)).toContainText("doing");

  // The placed note sits at its place; the others are laid out below it, left to right.
  const boxes = await nodes.evaluateAll((els) =>
    els.map((el) => {
      const s = (el as HTMLElement).style;
      return [el.getAttribute("data-id"), s.left, s.top, s.width, s.height].join(" ");
    }),
  );
  expect(boxes[0]).toBe(`${a.id} 600px 40px 300px 200px`);
  expect(boxes[1]).toMatch(/ 600px 264px 240px 176px$/);
  expect(boxes[2]).toMatch(/ 864px 264px 240px 176px$/);

  // A card's title opens its note.
  await cards.nth(1).getByTestId("query-title").click();
  await expect(p.getByRole("heading", { level: 1, name: `${tag} b` })).toBeVisible();
});

test("views saved in the editor persist across reload", async ({ page: p, admin }) => {
  const project = await create(admin, "project", { title: unique("Saved") });
  const tag = unique("Persisted");
  await create(admin, "task", { title: `${tag} 1`, project: project.id });
  await create(admin, "task", { title: `${tag} 2`, project: project.id });
  const title = unique("Views page");
  const note = await page(admin, title, []);

  await openNote(p, note.id, title);
  await p.getByRole("textbox", { name: "Body" }).click();
  await p.keyboard.type("/query");
  await expect(p.getByRole("listbox", { name: "Insert block" }).getByRole("option")).toHaveText([/Query view/]);
  await p.keyboard.press("Enter");
  const settings = p.getByRole("group", { name: "Query view settings" });
  await expect(settings).toBeVisible();
  await settings.getByLabel("Heading").fill("Persisted tasks");
  await settings.getByRole("button", { name: "Text" }).click();
  await settings.getByLabel("Query", { exact: true }).fill(`from task where project == "${project.id}" sort title asc`);
  const views = settings.getByTestId("view-settings");
  await expect(views).toHaveCount(1);
  await views.nth(0).getByLabel("Name").fill("rows");
  await settings.getByRole("button", { name: "Add view" }).click();
  await expect(views).toHaveCount(2);
  await views.nth(1).getByLabel("Name").fill("grid");
  await views.nth(1).getByLabel("Kind").selectOption("cards");
  await settings.getByLabel("Show first").selectOption("grid");
  // Leaving the settings saves at once.
  await p.getByLabel("Title", { exact: true }).focus();
  await expect(p.getByTestId("save-status")).toHaveText("Saved");

  const stored = await (await admin.get(`/api/notes/${note.id}`)).json();
  expect(stored.wcl).toContain("view rows {");
  expect(stored.wcl).toContain("view grid {");
  expect(stored.wcl).toContain('default_view = "grid"');

  await p.reload();
  const view = p.getByTestId("query-view");
  await expect(view.getByRole("heading", { name: "Persisted tasks" })).toBeVisible();
  await expect(view.getByRole("tablist", { name: "Views" }).getByRole("tab")).toHaveText(["rows", "grid"]);
  await expect(viewTab(view, "grid")).toHaveAttribute("aria-selected", "true");
  await expect(view.getByTestId("query-card")).toHaveCount(2);
  await viewTab(view, "rows").click();
  expect(await titles(view.getByTestId("query-row"))).toEqual([`${tag} 1`, `${tag} 2`]);

  // Editing again starts from the saved views.
  await p.getByRole("button", { name: "Edit query view" }).click();
  await expect(p.getByRole("group", { name: "Query view settings" }).getByTestId("view-settings")).toHaveCount(2);
});
