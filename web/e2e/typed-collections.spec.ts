import type { APIRequestContext, Locator } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// Typed lists and dicts (types T8): built in the type editor, then edited on a note page.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function putType(admin: APIRequestContext, name: string, wcl: string) {
  const res = await admin.put(`/api/types/${name}`, { data: { wcl } });
  expect(res.status(), await res.text()).toBe(200);
}

async function typeFile(admin: APIRequestContext, name: string): Promise<string> {
  return (await admin.get(`/api/types/${name}`, { headers: { Accept: "text/plain" } })).text();
}

async function createNote(admin: APIRequestContext, type: string, fields: Record<string, unknown>) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string };
}

interface StoredNote {
  note: { fields: Record<string, unknown>; blocks: { kind: string; labels?: string[]; fields?: Record<string, unknown> }[] };
}

async function readNote(admin: APIRequestContext, id: string): Promise<StoredNote> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return (await res.json()) as StoredNote;
}

async function pick(scope: Locator, label: string, typed: string, option: string) {
  const plus = scope.getByRole("button", { name: `Add to ${label}` });
  const box = scope.getByRole("combobox", { name: `Add to ${label}` });
  if (!(await box.isVisible())) await plus.click();
  await box.fill(typed);
  await scope.getByRole("option", { name: option, exact: true }).click();
}

test("a type built with a list of references, a list of date ranges and a dict of typed keys edits and saves on a note", async ({
  page,
  admin,
}) => {
  const t = token();
  const tool = `tool_${t}`;
  const crew = `crew_${t}`;
  await putType(admin, tool, `label = "Tool"\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n  }\n}\n`);
  const hammer = await createNote(admin, tool, { title: `Hammer ${t}` });
  const saw = await createNote(admin, tool, { title: `Saw ${t}` });

  await logIn(page, "/settings/types/new");
  await page.getByLabel("Name", { exact: true }).fill(crew);
  await page.getByLabel("Label", { exact: true }).fill("Crew");

  // A list of references, two-way.
  await page.getByRole("button", { name: "Add field" }).click();
  const tools = page.getByTestId("field-row-0");
  await tools.getByLabel("Name", { exact: true }).fill("tools");
  await tools.getByLabel("Label", { exact: true }).fill("Tools");
  await tools.getByLabel("Kind", { exact: true }).selectOption("list");
  await tools.getByLabel("Item kind").selectOption("reference");
  const toolItem = tools.getByTestId("item-settings");
  await toolItem.getByLabel("Target type").selectOption(tool);
  await toolItem.getByLabel("Relation").selectOption("two_way");
  await toolItem.getByLabel("Inverse field").fill("crews");

  // A list of date ranges.
  await page.getByRole("button", { name: "Add field" }).click();
  const spans = page.getByTestId("field-row-1");
  await spans.getByLabel("Name", { exact: true }).fill("spans");
  await spans.getByLabel("Label", { exact: true }).fill("Spans");
  await spans.getByLabel("Kind", { exact: true }).selectOption("list");
  await spans.getByLabel("Item kind").selectOption("date_range");
  await expect(spans.getByTestId("item-settings").getByLabel("Ends are")).toHaveValue("date");

  // A dict with an owner reference, a list of text and a dict of its own, and other keys of any kind.
  await page.getByRole("button", { name: "Add field" }).click();
  const details = page.getByTestId("field-row-2");
  await details.getByLabel("Name", { exact: true }).fill("details");
  await details.getByLabel("Label", { exact: true }).fill("Details");
  await details.getByLabel("Kind", { exact: true }).selectOption("dict");
  await details.getByLabel("Other keys", { exact: true }).selectOption("any");
  await details.getByRole("button", { name: "Add key" }).click();
  const owner = details.getByTestId("key-row-0");
  await owner.getByLabel("Key name").fill("owner");
  await owner.getByLabel("Key label").fill("Owner");
  await owner.getByLabel("Key kind").selectOption("reference");
  // Inside a dict, a reference is one-way only.
  await expect(owner.getByLabel("Relation").locator("option")).toHaveText(["One-way"]);
  await owner.getByLabel("Target type").selectOption(tool);
  await details.getByRole("button", { name: "Add key" }).click();
  const tags = details.getByTestId("key-row-1");
  await tags.getByLabel("Key name").fill("tags");
  await tags.getByLabel("Key label").fill("Tags");
  await tags.getByLabel("Key kind").selectOption("list");
  await details.getByRole("button", { name: "Add key" }).click();
  const size = details.getByTestId("key-row-2");
  await size.getByLabel("Key name").fill("size");
  await size.getByLabel("Key label").fill("Size");
  await size.getByLabel("Key kind").selectOption("dict");
  await size.getByLabel("Other keys", { exact: true }).selectOption("none");
  await expect(page.getByRole("alert", { name: "Problems" })).toContainText("details.size: a dictionary needs keys, other keys, or both.");
  await size.getByRole("button", { name: "Add key" }).click();
  const width = size.getByTestId("key-row-0");
  await width.getByLabel("Key name").fill("width");
  await width.getByLabel("Key label").fill("Width");
  await width.getByLabel("Key kind").selectOption("int");
  // A key name is an identifier.
  await width.getByLabel("Key name").fill("9width");
  await expect(page.getByRole("alert", { name: "Problems" })).toContainText('"9width" is not a key name');
  await width.getByLabel("Key name").fill("width");
  await expect(page.getByRole("alert", { name: "Problems" })).toHaveCount(0);

  await page.getByRole("button", { name: "Create type" }).click();
  await expect(page).toHaveURL(new RegExp(`/types/${crew}$`));

  const file = await typeFile(admin, crew);
  expect(file).not.toMatch(/multiple/);
  expect(file).toMatch(/field tools \{\n\s+kind\s+= "list"\n\s+label\s+= "Tools"\n\s+items\s+= "reference"\n\s+target\s+= "tool_/);
  expect(file).toMatch(/field spans \{\n\s+kind\s+= "list"\n\s+label\s+= "Spans"\n\s+items\s+= "date_range"\n\s+of\s+= "date"\n\}/);
  expect(file).toMatch(/values\s+= "any"/);
  expect(file).toMatch(/key owner \{\n\s+kind\s+= "reference"\n\s+label\s+= "Owner"\n\s+target\s+= "tool_/);
  expect(file).toMatch(/key size \{\n\s+kind\s+= "dict"\n\s+label\s+= "Size"\n\s+key width \{\n\s+kind\s+= "int"/);

  // The implied inverse appears on the target type.
  const types = (await (await admin.get("/api/types")).json()) as { types: { name: string; fields: Record<string, unknown>[] }[] };
  const fieldOf = (type: string, name: string) => types.types.find((x) => x.name === type)!.fields.find((f) => f.name === name);
  expect(fieldOf(crew, "tools")).toMatchObject({ kind: "reference", multiple: true, listed: true, relation: "two_way", inverse: "crews" });
  expect(fieldOf(tool, "crews")).toMatchObject({ kind: "reference", multiple: true, implied: true, inverse: "tools" });
  expect(fieldOf(crew, "spans")).toMatchObject({ kind: "list", items: "date_range", item: { kind: "date_range" } });
  expect(fieldOf(crew, "details")).toMatchObject({ kind: "dict", values: "any", keys: [{ name: "owner" }, { name: "tags" }, { name: "size" }] });

  // The editor reads it all back.
  await page.goto(`/settings/types/${crew}/edit`);
  const back = page.getByTestId("field-row-2");
  await back.getByRole("button", { name: "Settings for details" }).click();
  await expect(back.getByLabel("Other keys", { exact: true }).last()).toHaveValue("any");
  await expect(back.getByTestId("key-row-0").getByLabel("Key name")).toHaveValue("owner");
  await expect(back.getByTestId("key-row-2").getByLabel("Key kind")).toHaveValue("dict");

  // A note of it: list items added and moved, dict entries of several kinds.
  const note = await createNote(admin, crew, { title: `Crew ${t}` });
  await page.goto(`/notes/${note.id}`);
  const field = (f: string) => page.getByTestId(`field-${f}`);
  await pick(field("tools"), "Tools", "Hammer", `Hammer ${t}`);
  await pick(field("tools"), "Tools", "Saw", `Saw ${t}`);

  await field("spans").getByRole("button", { name: "Add item" }).click();
  const first = field("spans").getByRole("group", { name: "Spans item 1" });
  await first.getByLabel("Start").fill("2026-03-01");
  await first.getByLabel("End").fill("2026-03-05");
  await field("spans").getByRole("button", { name: "Add item" }).click();
  const second = field("spans").getByRole("group", { name: "Spans item 2" });
  await second.getByLabel("Start").fill("2026-01-10");
  await second.getByLabel("End").fill("2026-01-12");
  await field("spans").getByRole("button", { name: "Move Spans item 2 up" }).click();

  const dict = field("details");
  await pick(dict.getByTestId("dict-key-owner"), "Owner", "Hammer", `Hammer ${t}`);
  const tagBox = dict.getByTestId("dict-key-tags");
  await tagBox.getByRole("button", { name: "Add to Tags" }).click();
  await tagBox.getByRole("combobox", { name: "Add to Tags" }).fill("heavy");
  await tagBox.getByRole("combobox", { name: "Add to Tags" }).press("Enter");
  await dict.getByTestId("dict-key-size").getByLabel("Width").fill("12");
  await dict.getByRole("button", { name: "Add entry" }).click();
  await dict.getByLabel("Details key 1", { exact: true }).fill("rating");
  await dict.getByLabel("Details kind 1", { exact: true }).selectOption("int");
  await dict.getByLabel("Details value 1", { exact: true }).fill("5");
  await dict.getByRole("button", { name: "Add entry" }).click();
  await dict.getByLabel("Details key 2", { exact: true }).fill("done");
  await dict.getByLabel("Details kind 2", { exact: true }).selectOption("bool");
  await dict.getByLabel("Details value 2", { exact: true }).check();
  await dict.getByRole("button", { name: "Add entry" }).click();
  await dict.getByLabel("Details key 3", { exact: true }).fill("checked");
  await dict.getByLabel("Details kind 3", { exact: true }).selectOption("date");
  await dict.getByLabel("Details value 3", { exact: true }).fill("2026-02-02");

  await page.getByLabel("Title", { exact: true }).focus();
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
  await expect(page.getByRole("alert")).toHaveCount(0);

  const saved = await readNote(admin, note.id);
  expect(saved.note.fields.tools).toEqual([hammer.id, saw.id]);
  expect(saved.note.fields.spans).toEqual([
    { start: "2026-01-10", end: "2026-01-12" },
    { start: "2026-03-01", end: "2026-03-05" },
  ]);
  const block = saved.note.blocks.find((b) => b.kind === "dict" && b.labels?.[0] === "details");
  expect(block?.fields).toEqual({ owner: hammer.id, tags: ["heavy"], size: { width: 12 }, rating: 5, done: true, checked: "2026-02-02" });
  // The two-way list gave each tool the crew back.
  await expect.poll(async () => (await readNote(admin, hammer.id)).note.fields.crews).toEqual([note.id]);

  // It reads back on the page.
  await page.reload();
  await expect(field("spans").getByRole("group", { name: "Spans item 1" }).getByLabel("Start")).toHaveValue("2026-01-10");
  await expect(dict.getByTestId("dict-key-owner")).toContainText(`Hammer ${t}`);
  await expect(dict.getByLabel("Details key 2", { exact: true })).toHaveValue("done");
  await expect(dict.getByLabel("Details kind 2", { exact: true })).toHaveValue("bool");
  await expect(dict.getByLabel("Details value 2", { exact: true })).toBeChecked();
  // The server keeps a dict's entries in name order, so the other entries come back in it.
  await expect(dict.getByLabel("Details key 1", { exact: true })).toHaveValue("checked");
  await expect(dict.getByLabel("Details kind 1", { exact: true })).toHaveValue("date");
  await expect(dict.getByLabel("Details kind 3", { exact: true })).toHaveValue("int");
  await expect(dict.getByLabel("Details value 3", { exact: true })).toHaveValue("5");
});

test("a list and a dict show read-only in a query view table, references by title and option labels", async ({ page, admin }) => {
  const t = token();
  const thing = `thing_${t}`;
  await putType(
    admin,
    thing,
    `label = "Thing"

field sizes {
  kind    = "list"
  items   = "select"
  options = ["s", "l"]
  option_labels = ["Small", "Large"]
}

field spans {
  kind  = "list"
  items = "date_range"
  of    = "date"
}

field details {
  kind   = "dict"
  values = "any"
  key owner {
    kind   = "reference"
    label  = "Owner"
    target = "${thing}"
  }
}

template default {
  tab "Main" {
    slot "title"
    slot "sizes"
    slot "spans"
    slot "details"
  }
}
`,
  );
  const other = await createNote(admin, thing, { title: `Other ${t}` });
  const res = await admin.post(`/api/notes?type=${thing}`, {
    data: {
      note: {
        fields: { title: `Main ${t}`, sizes: ["s", "l"], spans: [{ start: "2026-01-01", end: "2026-01-02" }] },
        blocks: [{ kind: "dict", labels: ["details"], fields: { owner: other.id, count: 3 } }],
      },
    },
  });
  expect(res.status(), await res.text()).toBe(201);
  const host = await admin.post("/api/notes?type=page", {
    data: {
      note: {
        fields: { title: `Host ${t}` },
        blocks: [
          {
            kind: "content",
            labels: ["body"],
            blocks: [
              {
                kind: "query_view",
                fields: { query: `from ${thing} where title == "Main ${t}"` },
                blocks: [{ kind: "view", labels: ["table"], fields: { kind: "table", columns: ["title", "sizes", "spans", "details"] } }],
              },
            ],
          },
        ],
      },
    },
  });
  expect(host.status(), await host.text()).toBe(201);
  await logIn(page, `/notes/${((await host.json()) as { id: string }).id}`);
  const row = page.getByTestId("query-view").getByTestId("query-row");
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("Small, Large");
  await expect(row).toContainText("2026-01-01 – 2026-01-02");
  // A reference inside a dict shows its note's title, not its id.
  await expect(row).toContainText(`Owner: Other ${t}; count: 3`);
  await expect(row).not.toContainText(other.id);
});
