import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// The always-editable note page and the inline template editor: fields and content edited in
// place and saved by themselves, template blocks read-only on notes with `{{name}}` showing
// values, the properties sidebar, the `/` menu, and moving blocks.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

/** A type of its own per test, so template changes touch no other test. */
function typeWcl(extra = ""): string {
  return `label = "Inline"
plural = "Inlines"

field status {
  kind    = "select"
  label   = "Status"
  options = ["todo", "doing", "done"]
}

field due {
  kind  = "date"
  label = "Due"
}

field estimate {
  kind  = "int"
  label = "Estimate"
}

field owner {
  kind  = "string"
  label = "Owner"
}

field body {
  kind  = "content"
  label = "Body"
}

template default {
  tab "Main" {
    slot "status"
    paragraph {
      text = "**{{title}}** is {{status}}, due {{due}}."
    }
    slot "due"
    slot "body"
  }
${extra}}
`;
}

async function setup(admin: APIRequestContext, fields: Record<string, unknown> = {}, body: unknown[] = []) {
  const name = `inl_${token()}`;
  const put = await admin.put(`/api/types/${name}`, { data: { wcl: typeWcl() } });
  expect(put.status(), await put.text()).toBe(200);
  const blocks = body.length ? [{ kind: "content", labels: ["body"], blocks: body }] : [];
  const res = await admin.post(`/api/notes?type=${name}`, {
    data: { note: { fields: { title: `Inline ${token()}`, status: "todo", ...fields }, blocks } },
  });
  expect(res.status(), await res.text()).toBe(201);
  const note = (await res.json()) as { id: string; version: string; note: { fields: Record<string, unknown> } };
  return { name, note, title: note.note.fields.title as string };
}

async function stored(admin: APIRequestContext, id: string) {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return (await res.json()) as {
    version: string;
    note: { fields: Record<string, unknown>; blocks: { kind: string; labels?: string[]; blocks?: { kind: string; fields?: Record<string, unknown> }[] }[] };
  };
}

/** The body's blocks as stored: their text, or their kind. */
async function storedBody(admin: APIRequestContext, id: string): Promise<string[]> {
  const n = await stored(admin, id);
  const body = n.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === "body");
  return (body?.blocks ?? []).map((b) => String(b.fields?.text ?? b.kind));
}

async function saved(page: Page) {
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
}

/** Drag `handle` onto the top edge of `target` with a moving pointer, as a person would. */
async function dragBy(page: Page, handle: Locator, target: Locator) {
  const from = (await handle.boundingBox())!;
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(from.x + from.width / 2 + 10, from.y + from.height / 2 - 5, { steps: 4 });
  const to = (await target.boundingBox())!;
  await page.mouse.move(to.x + 12, to.y + 3, { steps: 10 });
  await page.mouse.up();
}

const body = (page: Page): Locator => page.getByRole("textbox", { name: "Body" });

test("a field edited in place saves itself and survives a reload", async ({ page, admin }) => {
  const { note, title } = await setup(admin);
  await logIn(page, `/notes/${note.id}`);
  await expect(page.getByRole("heading", { level: 1, name: title })).toBeVisible();
  await expect(page.getByRole("button", { name: "Edit", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Save", exact: true })).toHaveCount(0);

  await page.getByTestId("field-status").getByLabel("Status").selectOption("doing");
  await page.getByTestId("field-due").getByLabel("Due").fill("2026-11-05");
  await expect(page.getByTestId("save-status")).toHaveText(/Unsaved changes|Saving…|Saved/);
  await saved(page);
  expect((await stored(admin, note.id)).note.fields).toMatchObject({ status: "doing", due: "2026-11-05" });

  // The title is edited in place too.
  await page.getByLabel("Title").fill(`${title} renamed`);
  await saved(page);
  await expect.poll(async () => (await stored(admin, note.id)).note.fields.title).toBe(`${title} renamed`);

  await page.reload();
  await expect(page.getByLabel("Title")).toHaveValue(`${title} renamed`);
  await expect(page.getByTestId("field-status").getByLabel("Status")).toHaveValue("doing");
  await expect(page.getByTestId("field-due").getByLabel("Due")).toHaveValue("2026-11-05");
});

test("a content block edited inline saves itself", async ({ page, admin }) => {
  const { note } = await setup(admin, {}, [{ kind: "paragraph", fields: { text: "First line" } }]);
  await logIn(page, `/notes/${note.id}`);
  await body(page).getByText("First line").click();
  await page.keyboard.press("End");
  await page.keyboard.type(" and more");
  await saved(page);
  expect(await storedBody(admin, note.id)).toEqual(["First line and more"]);
  await page.reload();
  await expect(body(page)).toContainText("First line and more");
});

test("an autosave over a stale version shows the conflict view instead of overwriting", async ({ page, admin }) => {
  const { note, title } = await setup(admin);
  await logIn(page, `/notes/${note.id}`);
  await expect(page.getByLabel("Title")).toHaveValue(title);

  // Someone else saves first.
  const other = await admin.put(`/api/notes/${note.id}`, {
    data: { note: { fields: { title: `${title} elsewhere`, status: "done" }, blocks: [] }, base_version: note.version },
  });
  expect(other.status(), await other.text()).toBe(200);

  await page.getByTestId("field-status").getByLabel("Status").selectOption("doing");
  const conflict = page.getByTestId("conflict-view");
  await expect(conflict).toBeVisible();
  await expect(conflict.locator('tr[data-field="status"] [data-testid="conflict-mine"]')).toHaveText("doing");
  await expect(conflict.locator('tr[data-field="status"] [data-testid="conflict-stored"]')).toHaveText("done");
  expect((await stored(admin, note.id)).note.fields.status).toBe("done");

  await conflict.getByRole("button", { name: "Keep the stored version" }).click();
  await expect(page.getByLabel("Title")).toHaveValue(`${title} elsewhere`);
  await expect(page.getByTestId("field-status").getByLabel("Status")).toHaveValue("done");
});

test("template blocks are read-only on a note, and {{field}} shows the value as it changes", async ({ page, admin }) => {
  const { note, title } = await setup(admin, { due: "2026-12-01" });
  await logIn(page, `/notes/${note.id}`);
  const block = page.getByTestId("template-block");
  await expect(block).toHaveText(`${title} is todo, due 2026-12-01.`);
  await expect(block.locator("strong")).toHaveText(title);
  await expect(block.locator('[contenteditable="true"], input, textarea, select')).toHaveCount(0);
  await block.click();
  await page.keyboard.type("typed");
  await expect(block).toHaveText(`${title} is todo, due 2026-12-01.`);

  // The value follows the field as it is edited, before it is even saved.
  await page.getByTestId("field-status").getByLabel("Status").selectOption("done");
  await expect(block).toHaveText(`${title} is done, due 2026-12-01.`);
  await page.getByLabel("Title").fill("Renamed");
  await expect(block).toHaveText("Renamed is done, due 2026-12-01.");
  await saved(page);
});

test("fields the template does not place are edited in the properties sidebar", async ({ page, admin }) => {
  const { note } = await setup(admin);
  await logIn(page, `/notes/${note.id}`);
  const sidebar = page.getByRole("complementary", { name: "Properties" });
  await expect(sidebar).toHaveCount(0);
  const toggle = page.getByRole("button", { name: "Properties", exact: true });
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await toggle.click();
  await expect(sidebar).toBeVisible();

  // Only unplaced fields: estimate and owner. {{status}} and {{due}} are read-only, and slotted.
  await expect(sidebar.locator("[data-field]")).toHaveCount(2);
  await expect(sidebar.getByTestId("field-estimate")).toBeVisible();
  await expect(sidebar.getByTestId("field-owner")).toBeVisible();
  for (const placed of ["status", "due", "body", "title"]) {
    await expect(sidebar.getByTestId(`field-${placed}`)).toHaveCount(0);
  }

  await sidebar.getByLabel("Estimate").fill("8");
  await sidebar.getByLabel("Owner").fill("Ana");
  await saved(page);
  expect((await stored(admin, note.id)).note.fields).toMatchObject({ estimate: 8, owner: "Ana" });

  // The browser remembers the sidebar open.
  await page.reload();
  await expect(sidebar).toBeVisible();
  await expect(sidebar.getByLabel("Estimate")).toHaveValue("8");
  await page.getByRole("button", { name: "Hide right sidebar" }).click();
  await expect(sidebar).toHaveCount(0);
  await page.reload();
  await expect(page.getByLabel("Title")).toBeVisible();
  await expect(sidebar).toHaveCount(0);
});

test("the / menu inserts a block, filters as you type and closes on Escape", async ({ page, admin }) => {
  const { note } = await setup(admin, {}, [{ kind: "paragraph", fields: { text: "Intro" } }]);
  await logIn(page, `/notes/${note.id}`);
  await body(page).getByText("Intro").click();
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await page.keyboard.type("/");
  const menu = page.getByRole("listbox", { name: "Insert block" });
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("option", { name: "Code" })).toBeVisible();
  await expect(menu.getByRole("option", { name: "Heading 1" })).toBeVisible();

  await page.keyboard.type("head");
  await expect(menu.getByRole("option")).toHaveText([/Heading 1/, /Heading 2/, /Heading 3/]);
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  // Escape leaves the text as typed.
  await expect(body(page)).toContainText("/head");

  for (let i = 0; i < 5; i++) await page.keyboard.press("Backspace");
  await page.keyboard.type("/head");
  await expect(menu).toBeVisible();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(menu).toHaveCount(0);
  await page.keyboard.type("Second part");
  await expect(body(page).locator("h2")).toHaveText("Second part");

  await page.keyboard.press("Enter");
  await page.keyboard.type("/terminal");
  await expect(menu.getByRole("option")).toHaveText([/Terminal/]);
  await page.keyboard.press("Enter");
  await expect(menu).toHaveCount(0);
  await saved(page);
  const n = await stored(admin, note.id);
  const blocks = n.note.blocks.find((b) => b.labels?.[0] === "body")?.blocks ?? [];
  expect(blocks.map((b) => b.kind)).toEqual(["paragraph", "heading", "terminal"]);
  expect(blocks[1]?.fields).toEqual({ level: 2, text: "Second part" });
});

test("blocks move by dragging their handle and with Alt+Arrow, and the order is saved", async ({ page, admin }) => {
  const { note } = await setup(admin, {}, ["alpha", "beta", "gamma"].map((text) => ({ kind: "paragraph", fields: { text } })));
  await logIn(page, `/notes/${note.id}`);
  const para = (t: string) => body(page).locator("p", { hasText: t });
  await expect(para("gamma")).toBeVisible();

  // Drag gamma's handle to the top.
  await para("gamma").hover();
  const handle = page.getByTestId("field-body").getByTestId("block-handle");
  await expect(handle).toBeVisible();
  await dragBy(page, handle, para("alpha"));
  await expect(body(page).locator("p")).toHaveText(["gamma", "alpha", "beta"]);
  await saved(page);
  expect(await storedBody(admin, note.id)).toEqual(["gamma", "alpha", "beta"]);

  // The keyboard moves the block holding the caret.
  await para("beta").click();
  await page.keyboard.press("Alt+ArrowUp");
  await page.keyboard.press("Alt+ArrowUp");
  await expect(body(page).locator("p")).toHaveText(["beta", "gamma", "alpha"]);
  await page.keyboard.press("Alt+ArrowDown");
  await expect(body(page).locator("p")).toHaveText(["gamma", "beta", "alpha"]);
  await saved(page);
  expect(await storedBody(admin, note.id)).toEqual(["gamma", "beta", "alpha"]);
});

test("the template editor places a property with / and a field value with {{", async ({ page, admin }) => {
  const { name, note } = await setup(admin, { estimate: 5, owner: "Kim" });
  await logIn(page, `/settings/types/${name}/templates`);
  const editor = page.getByRole("textbox", { name: "Main content" });
  await expect(editor.getByTestId("template-item-status")).toBeVisible();
  // The fixed paragraph is typed in place, with its references as chips.
  await expect(editor.getByTestId("field-ref")).toHaveText(["{{title}}", "{{status}}", "{{due}}"]);

  await editor.locator("p").first().click();
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await page.keyboard.type("/");
  const menu = page.getByRole("listbox", { name: "Insert block" });
  // The Property section lists the fields not placed yet.
  await expect(menu.getByRole("option", { name: "Owner" })).toBeVisible();
  await expect(menu.getByRole("option", { name: "Estimate" })).toBeVisible();
  await expect(menu.getByRole("option", { name: "Status" })).toHaveCount(0);
  await page.keyboard.type("owner");
  await expect(menu.getByRole("option")).toHaveText([/Owner/]);
  await page.keyboard.press("Enter");
  await expect(editor.getByTestId("template-item-owner")).toBeVisible();

  await page.keyboard.type("Estimated at {{esti");
  const fields = page.getByRole("listbox", { name: "Insert field" });
  await expect(fields.getByRole("option")).toHaveText([/Estimate/]);
  await page.keyboard.press("Enter");
  await expect(fields).toHaveCount(0);
  await page.keyboard.type(" hours");
  await expect(editor.getByTestId("field-ref")).toHaveText(["{{title}}", "{{status}}", "{{due}}", "{{estimate}}"]);

  await page.getByRole("button", { name: "Save template" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved the default template.");
  const wcl = await (await admin.get(`/api/types/${name}/templates/default`)).json();
  expect(wcl.wcl).toContain('slot "owner"');
  expect(wcl.wcl).toContain("Estimated at {{estimate}} hours");

  // The note page now has owner in place, and shows the estimate in text; neither is in the sidebar.
  await page.goto(`/notes/${note.id}`);
  await expect(page.getByTestId("field-owner").getByLabel("Owner")).toHaveValue("Kim");
  await expect(page.getByText("Estimated at 5 hours")).toBeVisible();
  await page.getByRole("button", { name: "Properties", exact: true }).click();
  const sidebar = page.getByRole("complementary", { name: "Properties" });
  await expect(sidebar.getByTestId("field-estimate")).toBeVisible();
  await expect(sidebar.getByTestId("field-owner")).toHaveCount(0);
});

test("the server refuses a template whose {{name}} names no field", async ({ admin }) => {
  const { name } = await setup(admin);
  const res = await admin.put(`/api/types/${name}/templates/card`, {
    data: { wcl: 'template card {\n  tab "C" {\n    paragraph {\n      text = "Due {{nothing}}"\n    }\n  }\n}\n' },
  });
  expect(res.status()).toBe(400);
  expect(await res.text()).toContain("`{{nothing}}` names no field of the type");
});

test("every property on the page leaves the sidebar saying so", async ({ page, admin }) => {
  const name = `all_${token()}`;
  const put = await admin.put(`/api/types/${name}`, {
    data: { wcl: `label = "All"\n\nfield a {\n  kind = "string"\n}\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n    slot "a"\n  }\n}\n` },
  });
  expect(put.status(), await put.text()).toBe(200);
  const res = await admin.post(`/api/notes?type=${name}`, { data: { note: { fields: { title: "All placed" } } } });
  const id = (await res.json()).id;
  await logIn(page, `/notes/${id}`);
  await page.getByRole("button", { name: "Properties", exact: true }).click();
  await expect(page.getByRole("complementary", { name: "Properties" })).toContainText("Every property is on the page.");
  await page.getByRole("button", { name: "Properties", exact: true }).click();
});

test("cards render {{field}} from the card template, read-only", async ({ page, admin }) => {
  const { name, title } = await setup(admin, { status: "doing" });
  const card = `template card {\n  tab "Card" {\n    slot "title"\n    paragraph {\n      text = "Now {{status}}, owned by {{owner}}"\n    }\n  }\n}\n`;
  const put = await admin.put(`/api/types/${name}/templates/card`, { data: { wcl: card } });
  expect(put.status(), await put.text()).toBe(200);
  const res = await admin.post("/api/notes?type=page", {
    data: {
      note: {
        fields: { title: `Cards of ${name}` },
        blocks: [
          {
            kind: "content",
            labels: ["body"],
            blocks: [
              {
                kind: "query_view",
                fields: { query: `from ${name} sort title asc` },
                blocks: [{ kind: "view", labels: ["cards"], fields: { kind: "cards" } }],
              },
            ],
          },
        ],
      },
    },
  });
  expect(res.status(), await res.text()).toBe(201);
  await logIn(page, `/notes/${(await res.json()).id}`);
  const cards = page.getByTestId("query-card");
  await expect(cards).toHaveCount(1);
  await expect(cards.first()).toContainText(title);
  await expect(cards.first().locator("[data-field-ref]")).toHaveText(["doing", "—"]);
  await expect(cards.first()).toContainText("Now doing, owned by —");
});
