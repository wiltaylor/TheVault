import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { expect, logIn, test, vaultDir } from "./fixtures";

// Kanban, calendar and timeline views (S5; D7, D12, D29).

function unique(prefix: string): string {
  return `${prefix}${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function create(admin: APIRequestContext, type: string, fields: Record<string, unknown>, blocks: unknown[] = []) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string; version: string };
}

/** A page holding one query view over the stories tagged `tag`, with `views`. */
async function viewPage(admin: APIRequestContext, tag: string, views: Record<string, Record<string, unknown>>) {
  const title = unique("Views ");
  const note = await create(admin, "page", { title }, [
    {
      kind: "content",
      labels: ["body"],
      blocks: [
        {
          kind: "query_view",
          fields: { query: `from story where tag == "${tag}" sort title asc` },
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

/** The story's file as the server wrote it. */
function storyFile(id: string): string {
  return readFileSync(join(vaultDir(), "notes/story", `${id}.wcl`), "utf8");
}

function column(view: Locator, term: string): Locator {
  return view.locator(`[data-testid=kanban-column][data-term="${term}"]`);
}

/** `YYYY-MM-DD` of day `day` in the current month, in the browser's and this process's time zone. */
function day(day: number): string {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(day)}`;
}

test("dragging a card to another column changes the note's taxonomy value in its file", async ({ page: p, admin }) => {
  const tag = unique("drag");
  const a = await create(admin, "story", { title: `${tag} A`, tag, stage: "Todo" });
  await create(admin, "story", { title: `${tag} B`, tag, stage: "Todo" });
  await create(admin, "story", { title: `${tag} C`, tag });
  const page = await viewPage(admin, tag, { board: { kind: "kanban", group: "stage" } });
  expect(storyFile(a.id)).toMatch(/^stage\s+= "Todo"$/m);

  const view = await open(p, page.id, page.title);
  await expect(view.getByTestId("kanban-column").locator("h4")).toHaveText(["Todo", "Doing", "Done", "No stage"]);
  await expect(column(view, "Todo").getByTestId("kanban-card")).toHaveCount(2);
  await expect(column(view, "").getByTestId("kanban-card")).toHaveText([new RegExp(`${tag} C`)]);

  const card = column(view, "Todo").locator(`[data-id="${a.id}"]`);
  await card.dragTo(column(view, "Doing"));

  await expect(column(view, "Doing").getByTestId("kanban-card")).toHaveText([new RegExp(`${tag} A`)]);
  await expect(column(view, "Todo").getByTestId("kanban-card")).toHaveCount(1);
  await expect.poll(() => storyFile(a.id)).toMatch(/^stage\s+= "Doing"$/m);

  // Dropping on the no-value column clears the field.
  await column(view, "Doing").locator(`[data-id="${a.id}"]`).dragTo(column(view, ""));
  await expect(column(view, "").getByTestId("kanban-card")).toHaveCount(2);
  await expect.poll(() => storyFile(a.id)).not.toMatch(/^stage\s/m);
});

test("a kanban drag shows in every other view of the note on the page without a reload", async ({ page: p, admin }) => {
  const tag = unique("live");
  const a = await create(admin, "story", { title: `${tag} A`, tag, stage: "Todo" });
  await create(admin, "story", { title: `${tag} B`, tag, stage: "Todo" });
  const query = `from story where tag == "${tag}" sort title asc`;
  const block = (name: string, fields: Record<string, unknown>) => ({
    kind: "query_view",
    fields: { query },
    blocks: [{ kind: "view", labels: [name], fields }],
  });
  const title = unique("Live ");
  const page = await create(admin, "page", { title }, [
    {
      kind: "content",
      labels: ["body"],
      blocks: [
        block("board", { kind: "kanban", group: "stage" }),
        block("rows", { kind: "table", columns: ["title", "stage"] }),
        block("expanded", { kind: "details" }),
        block("again", { kind: "kanban", group: "stage" }),
      ],
    },
  ]);

  await open(p, page.id, title);
  const views = p.getByTestId("query-view");
  await expect(views).toHaveCount(4);
  const [board, table, details, again] = [views.nth(0), views.nth(1), views.nth(2), views.nth(3)];
  const rowA = table.getByTestId("query-row").filter({ hasText: `${tag} A` });
  const detailA = details.getByTestId("query-detail").filter({ hasText: `${tag} A` });
  await expect(rowA).toContainText("Todo");
  await expect(detailA).toContainText("Todo");
  await expect(column(again, "Todo").getByTestId("kanban-card")).toHaveCount(2);

  // A reload would drop this marker.
  await p.evaluate(() => ((window as unknown as { liveMarker: boolean }).liveMarker = true));
  await column(board, "Todo").locator(`[data-id="${a.id}"]`).dragTo(column(board, "Doing"));
  await expect.poll(() => storyFile(a.id)).toMatch(/^stage\s+= "Doing"$/m);

  await expect(rowA).toContainText("Doing");
  await expect(rowA).not.toContainText("Todo");
  await expect(detailA).toContainText("Doing");
  await expect(column(again, "Doing").getByTestId("kanban-card")).toHaveText([new RegExp(`${tag} A`)]);
  await expect(column(again, "Todo").getByTestId("kanban-card")).toHaveCount(1);
  // The rows changed in place: the views kept their rows and the page never reloaded.
  await expect(table.getByTestId("query-row")).toHaveCount(2);
  expect(await p.evaluate(() => (window as unknown as { liveMarker?: boolean }).liveMarker)).toBe(true);
});

test("kanban columns page through long columns", async ({ page: p, admin }) => {
  const tag = unique("many");
  for (let i = 0; i < 30; i++) {
    await create(admin, "story", { title: `${tag} ${String(i).padStart(2, "0")}`, tag, stage: "Done" });
  }
  const page = await viewPage(admin, tag, { board: { kind: "kanban", group: "stage" } });
  const view = await open(p, page.id, page.title);
  const done = column(view, "Done");
  await expect(done.getByTestId("kanban-count")).toHaveText("30");
  await expect(done.getByTestId("kanban-card")).toHaveCount(25);
  await done.getByRole("button", { name: "Show more (5 left)" }).click();
  await expect(done.getByTestId("kanban-card")).toHaveCount(30);
  await expect(done.getByRole("button", { name: /Show more/ })).toHaveCount(0);
});

test("a kanban over a multi-term field is refused with a clear message", async ({ page: p, admin }) => {
  const tag = unique("multi");
  await create(admin, "story", { title: `${tag} A`, tag, topics: ["Todo", "Done"] });
  const page = await viewPage(admin, tag, { board: { kind: "kanban", group: "topics" } });
  const view = await open(p, page.id, page.title);
  await expect(view.getByTestId("view-problem")).toHaveText(
    "A kanban view needs a single-term taxonomy field, but Topics holds several terms. Choose a field that holds one term.",
  );
  await expect(view.getByTestId("kanban-card")).toHaveCount(0);
});

test("notes with dates appear on the right calendar days and a date range spans its days", async ({
  page: p,
  admin,
}) => {
  const tag = unique("cal");
  // A Monday-to-Wednesday range inside one week row of this month (weeks start on Sunday).
  const now = new Date();
  const firstSunday = 1 + ((7 - new Date(now.getFullYear(), now.getMonth(), 1).getDay()) % 7);
  const mon = firstSunday + 8;
  await create(admin, "story", { title: `${tag} single`, tag, due: day(3), span: { start: day(mon), end: day(mon + 2) } });
  await create(admin, "story", { title: `${tag} other`, tag, due: day(mon + 4) });
  const page = await viewPage(admin, tag, {
    due: { kind: "calendar", date: "due" },
    span: { kind: "calendar", date: "span" },
  });

  const view = await open(p, page.id, page.title);
  const cell = (d: string) => view.locator(`td.fc-daygrid-day[data-date="${d}"]`);
  await expect(cell(day(3)).getByTestId("calendar-event")).toHaveText([new RegExp(`${tag} single`)]);
  await expect(cell(day(mon + 4)).getByTestId("calendar-event")).toHaveText([new RegExp(`${tag} other`)]);
  await expect(cell(day(mon)).getByTestId("calendar-event")).toHaveCount(0);

  await view.getByRole("tablist", { name: "Views" }).getByRole("tab", { name: "span" }).click();
  const range = view.getByTestId("calendar-event");
  await expect(range).toHaveCount(1);
  await expect(range).toContainText(`${tag} single`);
  const box = (await range.boundingBox())!;
  const first = (await cell(day(mon)).boundingBox())!;
  const last = (await cell(day(mon + 2)).boundingBox())!;
  const after = (await cell(day(mon + 3)).boundingBox())!;
  expect(Math.abs(box.x - first.x)).toBeLessThan(6);
  expect(Math.abs(box.x + box.width - (last.x + last.width))).toBeLessThan(6);
  expect(box.x + box.width).toBeLessThan(after.x + 6);
  // The note with only a due date has no span, so it is not on the span calendar.
  await expect(view.getByText(`${tag} other`)).toHaveCount(0);

  // Clicking an event opens its note.
  await range.click();
  await expect(p.getByRole("heading", { level: 1, name: `${tag} single` })).toBeVisible();
});

test("the timeline shows date-range notes as ranges and single dates as points", async ({ page: p, admin }) => {
  const tag = unique("line");
  await create(admin, "story", {
    title: `${tag} ranged`,
    tag,
    due: "2026-03-10",
    span: { start: "2026-03-02", end: "2026-03-20" },
  });
  await create(admin, "story", { title: `${tag} dated`, tag, due: "2026-03-14" });
  const page = await viewPage(admin, tag, {
    span: { kind: "timeline", date: "span" },
    due: { kind: "timeline", date: "due" },
  });

  const view = await open(p, page.id, page.title);
  const timeline = view.getByTestId("timeline");
  await expect(timeline.locator(".vis-item.vis-range")).toHaveText([`${tag} ranged`]);
  await expect(timeline.locator(".vis-item.vis-point")).toHaveCount(0);

  await view.getByRole("tablist", { name: "Views" }).getByRole("tab", { name: "due" }).click();
  // Points are not in date order in the DOM, so compare them as a set.
  await expect
    .poll(async () => (await timeline.locator(".vis-item.vis-point").allInnerTexts()).map((t) => t.trim()).sort())
    .toEqual([`${tag} dated`, `${tag} ranged`]);
  await expect(timeline.locator(".vis-item.vis-range")).toHaveCount(0);
});

test("a 409 on drag shows the conflict view instead of losing the move", async ({ page: p, admin }) => {
  const tag = unique("clash");
  const a = await create(admin, "story", { title: `${tag} A`, tag, stage: "Todo", estimate: 1 });
  const page = await viewPage(admin, tag, { board: { kind: "kanban", group: "stage" } });
  const view = await open(p, page.id, page.title);
  const card = column(view, "Todo").locator(`[data-id="${a.id}"]`);
  await expect(card).toBeVisible();

  // Start the drag, then save the note elsewhere before the drop.
  const loaded = p.waitForResponse((r) => r.url().endsWith(`/api/notes/${a.id}`) && r.request().method() === "GET");
  await card.hover();
  await p.mouse.down();
  const target = (await column(view, "Done").boundingBox())!;
  await p.mouse.move(target.x + 20, target.y + 20, { steps: 4 });
  await loaded;
  const stored = (await (await admin.get(`/api/notes/${a.id}`)).json()) as { version: string; note: { fields: Record<string, unknown>; blocks: unknown[] } };
  const res = await admin.put(`/api/notes/${a.id}`, {
    data: { note: { ...stored.note, fields: { ...stored.note.fields, estimate: 8 } }, base_version: stored.version },
  });
  expect(res.status(), await res.text()).toBe(200);
  await p.mouse.move(target.x + target.width / 2, target.y + target.height / 2, { steps: 4 });
  await p.mouse.up();

  const conflict = p.getByTestId("conflict-view");
  await expect(conflict).toBeVisible();
  const stage = conflict.locator('tr[data-field="stage"]');
  await expect(stage.getByTestId("conflict-mine")).toHaveText("Done");
  await expect(stage.getByTestId("conflict-stored")).toHaveText("Todo");
  await expect(conflict.locator('tr[data-field="estimate"]').getByTestId("conflict-stored")).toHaveText("8");
  // The move is not lost: the file still holds the other save until the user chooses.
  expect(storyFile(a.id)).toMatch(/^stage\s+= "Todo"$/m);

  await conflict.getByRole("button", { name: "Save my version" }).click();
  await expect(conflict).toHaveCount(0);
  await expect.poll(() => storyFile(a.id)).toMatch(/^stage\s+= "Done"$/m);
  await expect(column(view, "Done").getByTestId("kanban-card")).toHaveCount(1);
  await expect(column(view, "Todo").getByTestId("kanban-card")).toHaveCount(0);
});
