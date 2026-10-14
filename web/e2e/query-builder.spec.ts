import type { APIRequestContext, Page } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// The visual query builder edits the same text an agent writes (D8). These run against the real
// server, so the text is what `query` prints, not a stand-in.

function unique(prefix: string): string {
  return `${prefix} ${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

/** A page whose body holds one query view over `query`. */
async function pageWithQuery(admin: APIRequestContext, title: string, query: string) {
  const blocks = [
    {
      kind: "content",
      labels: ["body"],
      blocks: [
        {
          kind: "query_view",
          fields: { query },
          blocks: [{ kind: "view", labels: ["table"], fields: { kind: "table" } }],
        },
      ],
    },
  ];
  const res = await admin.post("/api/notes?type=page", { data: { note: { fields: { title }, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string; version: string };
}

/** The `query` item of the first query view block in a stored note. */
async function storedQuery(admin: APIRequestContext, id: string): Promise<string> {
  const note = await (await admin.get(`/api/notes/${id}`)).json();
  const find = (blocks: { kind: string; fields?: Record<string, unknown>; blocks?: unknown[] }[]): string | undefined => {
    for (const b of blocks ?? []) {
      if (b.kind === "query_view") return b.fields?.query as string;
      const inner = find((b.blocks ?? []) as typeof blocks);
      if (inner !== undefined) return inner;
    }
  };
  const q = find(note.note.blocks);
  expect(q, JSON.stringify(note.note)).toBeDefined();
  return q!;
}

async function editNote(p: Page, id: string, title: string) {
  await logIn(p, `/notes/${id}`);
  await expect(p.getByRole("heading", { level: 1, name: title })).toBeVisible();
  // The query view shows its results; its settings open in place.
  await p.getByRole("button", { name: "Edit query view" }).click();
  return p.getByRole("group", { name: "Query view settings" });
}

/** Leave the settings, which saves any change at once, and wait for the save to settle. */
async function save(p: Page) {
  await p.getByLabel("Title", { exact: true }).focus();
  await p.waitForTimeout(1800);
  await expect(p.getByTestId("save-status")).not.toHaveText(/Saving|Unsaved/);
}

test("building a query with a type, two conditions, a sort and a limit produces the expected text", async ({
  page: p,
  admin,
}) => {
  const title = unique("Built");
  const note = await pageWithQuery(admin, title, "from * sort title asc");
  const settings = await editNote(p, note.id, title);
  const builder = settings.getByRole("group", { name: "Query builder" });
  await expect(builder).toBeVisible();
  await expect(builder.getByLabel("Sort field")).toHaveValue("title");

  await builder.getByRole("button", { name: "Add to Types" }).click();
  await builder.getByRole("combobox", { name: "Add to Types" }).fill("tas");
  await builder.getByRole("option", { name: "Task", exact: true }).click();
  await builder.getByRole("button", { name: "Add condition" }).click();
  const conditions = builder.getByTestId("condition");
  await conditions.nth(0).getByLabel("Field").fill("status");
  await conditions.nth(0).getByLabel("Value", { exact: true }).fill("todo");
  await builder.getByRole("button", { name: "Add condition" }).click();
  await conditions.nth(1).getByLabel("Field").fill("estimate");
  await conditions.nth(1).getByLabel("Operator").selectOption("gt");
  await conditions.nth(1).getByLabel("Value type").selectOption("number");
  await conditions.nth(1).getByLabel("Value", { exact: true }).fill("2");
  await builder.getByLabel("Order").selectOption("desc");
  await builder.getByLabel("Limit").fill("10");

  const expected = 'from task where status == "todo" && estimate > 2 sort title desc limit 10';
  await expect(settings.getByRole("alert")).toHaveCount(0);
  await settings.getByRole("button", { name: "Text" }).click();
  await expect(settings.getByLabel("Query", { exact: true })).toHaveValue(expected);

  await save(p);
  expect(await storedQuery(admin, note.id)).toBe(expected);
});

test("agent-written query text opened in the builder and saved unchanged stays byte-identical", async ({
  page: p,
  admin,
}) => {
  // Not the canonical layout: extra spaces, newlines, and operators without spaces round them.
  const agent = 'from   task\nwhere status=="todo"  &&estimate>2\n  sort title desc limit 10';
  const title = unique("Agent");
  const note = await pageWithQuery(admin, title, agent);
  const settings = await editNote(p, note.id, title);
  const builder = settings.getByRole("group", { name: "Query builder" });
  await expect(builder).toBeVisible();
  await expect(builder.locator(".query-builder-from .chip")).toHaveText(["Task×"]);
  await expect(builder.getByTestId("condition")).toHaveCount(2);
  await expect(builder.getByTestId("condition").nth(1).getByLabel("Field")).toHaveValue("estimate");
  await expect(builder.getByLabel("Limit")).toHaveValue("10");

  // Looking at it as text and back changes nothing either.
  await settings.getByRole("button", { name: "Text" }).click();
  await expect(settings.getByLabel("Query", { exact: true })).toHaveValue(agent);
  await settings.getByRole("button", { name: "Builder" }).click();
  await expect(builder).toBeVisible();

  await save(p);
  expect(await storedQuery(admin, note.id)).toBe(agent);
});

test("a query the builder cannot show opens in text mode with a notice", async ({ page: p, admin }) => {
  const query = "from task where estimate * 2 > 3";
  const title = unique("Arithmetic");
  const note = await pageWithQuery(admin, title, query);
  const settings = await editNote(p, note.id, title);

  const notice = settings.getByTestId("builder-notice");
  await expect(notice).toBeVisible();
  await expect(notice).toContainText("the builder cannot show this query");
  await expect(settings.getByLabel("Query", { exact: true })).toHaveValue(query);
  await expect(settings.getByRole("button", { name: "Text" })).toHaveAttribute("aria-pressed", "true");
  await expect(settings.getByRole("group", { name: "Query builder" })).toHaveCount(0);

  // Asking for the builder again keeps it as text.
  await settings.getByRole("button", { name: "Builder" }).click();
  await expect(notice).toBeVisible();
  await expect(settings.getByRole("group", { name: "Query builder" })).toHaveCount(0);

  await save(p);
  expect(await storedQuery(admin, note.id)).toBe(query);
});
