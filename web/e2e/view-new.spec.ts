import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { expect, logIn, test, vaultDir } from "./fixtures";

// Adding notes from a query view (allow_new): each view adds a note of the query's type that
// starts with the values its `where` fixes and what the place it was added at stands for.

function unique(prefix: string): string {
  return `${prefix}${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function create(admin: APIRequestContext, type: string, fields: Record<string, unknown>, blocks: unknown[] = []) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string; version: string };
}

/** A page holding one query view with `allow_new` over the stories tagged `tag`. */
async function viewPage(admin: APIRequestContext, tag: string, views: Record<string, Record<string, unknown>>) {
  const title = unique("Adds ");
  const note = await create(admin, "page", { title }, [
    {
      kind: "content",
      labels: ["body"],
      blocks: [
        {
          kind: "query_view",
          fields: { query: `from story where tag == "${tag}" sort title asc`, allow_new: true },
          blocks: Object.entries(views).map(([name, fields]) => ({ kind: "view", labels: [name], fields })),
        },
      ],
    },
  ]);
  return { ...note, title };
}

async function open(p: Page, id: string, title: string): Promise<Locator> {
  await logIn(p, `/notes/${id}`);
  await expect(p.getByRole("heading", { level: 1, name: title })).toBeVisible();
  return p.getByTestId("query-view");
}

/** The stories tagged `tag`, with their fields. */
async function stories(admin: APIRequestContext, tag: string) {
  const res = await admin.post("/api/query", { data: { query: `from story where tag == "${tag}" sort title asc`, page_size: 50 } });
  return ((await res.json()) as { rows: { id: string; title: string; fields: Record<string, unknown> }[] }).rows;
}

function storyFile(id: string): string {
  return readFileSync(join(vaultDir(), "notes/story", `${id}.wcl`), "utf8");
}

test("a table with no results adds notes from its New row, each with the query's tag, one after another", async ({ page: p, admin }) => {
  const tag = unique("tbl");
  const page = await viewPage(admin, tag, { table: { kind: "table", columns: ["title", "tag"] } });
  const view = await open(p, page.id, page.title);
  await expect(view.getByTestId("query-count")).toHaveText("0 notes");

  await view.getByRole("button", { name: "+ New story" }).click();
  const box = view.getByRole("textbox", { name: "New story title" });
  await expect(box).toBeFocused();
  await box.fill(`${tag} first`);
  await box.press("Enter");
  await expect(view.getByTestId("query-row")).toHaveText([new RegExp(`${tag} first`)]);
  // The box stays for the next one.
  await expect(box).toBeFocused();
  await box.fill(`${tag} second`);
  await box.press("Enter");
  await expect(view.getByTestId("query-row")).toHaveCount(2);
  await box.press("Escape");
  await expect(box).toHaveCount(0);

  const rows = await stories(admin, tag);
  expect(rows.map((r) => r.title)).toEqual([`${tag} first`, `${tag} second`]);
  expect(storyFile(rows[0]!.id)).toMatch(new RegExp(`^tag\\s+= "${tag}"$`, "m"));
});

test("a kanban column adds a card with its own term, and the no-term column with none", async ({ page: p, admin }) => {
  const tag = unique("kb");
  const page = await viewPage(admin, tag, { board: { kind: "kanban", group: "stage" } });
  const view = await open(p, page.id, page.title);
  const column = (term: string) => view.locator(`[data-testid=kanban-column][data-term="${term}"]`);

  await column("Doing").getByRole("button", { name: "+ New" }).click();
  await column("Doing").getByRole("textbox", { name: "New story title" }).fill(`${tag} doing`);
  await column("Doing").getByRole("textbox", { name: "New story title" }).press("Enter");
  await expect(column("Doing").getByTestId("kanban-card")).toHaveText([new RegExp(`${tag} doing`)]);

  await column("").getByRole("button", { name: "+ New" }).click();
  await column("").getByRole("textbox", { name: "New story title" }).fill(`${tag} none`);
  await column("").getByRole("textbox", { name: "New story title" }).press("Enter");
  await expect(column("").getByTestId("kanban-card")).toHaveText([new RegExp(`${tag} none`)]);

  const rows = await stories(admin, tag);
  expect(rows.find((r) => r.title === `${tag} doing`)?.fields.stage).toBe("Doing");
  expect(rows.find((r) => r.title === `${tag} none`)?.fields.stage ?? null).toBeNull();
});

test("a calendar day's + adds a note on that day", async ({ page: p, admin }) => {
  const tag = unique("cal");
  const page = await viewPage(admin, tag, { cal: { kind: "calendar", date: "due" } });
  const view = await open(p, page.id, page.title);
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  const day = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-15`;

  await view.locator(`[data-testid=calendar-new][data-date="${day}"]`).click();
  const dialog = p.getByRole("dialog", { name: "New story" });
  await dialog.getByRole("textbox", { name: "New story title" }).fill(`${tag} on the 15th`);
  await dialog.getByRole("textbox", { name: "New story title" }).press("Enter");
  await expect(dialog).toHaveCount(0);
  await expect(view.getByTestId("calendar-event").filter({ hasText: `${tag} on the 15th` })).toBeVisible();

  const rows = await stories(admin, tag);
  expect(rows[0]?.fields.due).toBe(day);
});

test("a view without allow_new adds nothing, and its empty state says so", async ({ page: p, admin }) => {
  const tag = unique("none");
  const title = unique("Plain ");
  const note = await create(admin, "page", { title }, [
    {
      kind: "content",
      labels: ["body"],
      blocks: [
        {
          kind: "query_view",
          fields: { query: `from story where tag == "${tag}"` },
          blocks: [{ kind: "view", labels: ["table"], fields: { kind: "table" } }],
        },
      ],
    },
  ]);
  const view = await open(p, note.id, title);
  await expect(view.getByText("No notes match.")).toBeVisible();
  await expect(view.getByTestId("new-note-button")).toHaveCount(0);
});

test("a double-click on a timeline adds a note at that moment", async ({ page: p, admin }) => {
  const tag = unique("tl");
  const page = await viewPage(admin, tag, { line: { kind: "timeline", date: "due" } });
  const view = await open(p, page.id, page.title);
  const timeline = view.getByTestId("timeline");
  await expect(timeline.locator(".vis-timeline")).toBeVisible();
  const box = (await timeline.boundingBox())!;
  await p.mouse.dblclick(box.x + box.width / 2, box.y + box.height / 2);
  const dialog = p.getByRole("dialog", { name: "New story" });
  await dialog.getByRole("textbox", { name: "New story title" }).fill(`${tag} then`);
  await dialog.getByRole("textbox", { name: "New story title" }).press("Enter");
  await expect(dialog).toHaveCount(0);

  await expect.poll(async () => (await stories(admin, tag))[0]?.fields.due).toMatch(/^\d{4}-\d{2}-\d{2}$/);
});
