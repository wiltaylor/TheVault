import type { APIRequestContext, Locator } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// The definition editors (S5): types, templates, taxonomies, and every field kind's editor on a
// note page (D3, D6, D7, D14).

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

async function putType(admin: APIRequestContext, name: string, wcl: string) {
  const res = await admin.put(`/api/types/${name}`, { data: { wcl } });
  expect(res.status(), await res.text()).toBe(200);
}

async function putTaxonomy(admin: APIRequestContext, name: string, wcl: string) {
  const res = await admin.put(`/api/taxonomies/${name}`, { data: { wcl } });
  expect(res.status(), await res.text()).toBe(200);
}

async function createNote(admin: APIRequestContext, type: string, fields: Record<string, unknown>) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string; note: { fields: Record<string, unknown>; blocks: unknown[] } };
}

async function readNote(admin: APIRequestContext, id: string) {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return (await res.json()) as {
    note: { fields: Record<string, unknown>; blocks: { kind: string; labels?: string[]; fields?: Record<string, unknown> }[] };
  };
}

test("creating a type with one field of each kind through the UI produces a type file that validates", async ({
  page,
  admin,
}) => {
  const t = token();
  const taxonomy = `kinds_tax_${t}`;
  await putTaxonomy(admin, taxonomy, `label = "Kinds"\n\nterm "One"\n`);
  const name = `kinds_${t}`;
  const kinds = [
    "string",
    "int",
    "float",
    "bool",
    "date",
    "datetime",
    "date_range",
    "url",
    "select",
    "taxonomy",
    "reference",
    "list",
    "dict",
    "location",
    "content",
    "asset",
    "ink",
    "canvas",
  ];

  await logIn(page, "/settings/types/new");
  await page.getByLabel("Name", { exact: true }).fill(name);
  await page.getByLabel("Label", { exact: true }).fill("Every kind");
  for (const [i, kind] of kinds.entries()) {
    await page.getByRole("button", { name: "Add field" }).click();
    const card = page.getByTestId(`field-row-${i}`);
    await card.getByLabel("Name", { exact: true }).fill(`f_${kind}`);
    await card.getByLabel("Kind", { exact: true }).selectOption(kind);
    if (kind === "select") await card.getByLabel("Options, one per line").fill("low\nhigh");
    if (kind === "taxonomy") await card.getByLabel("Taxonomy", { exact: true }).selectOption(taxonomy);
    if (kind === "reference") await card.getByLabel("Target type", { exact: true }).selectOption("task");
  }
  await expect(page.getByRole("list", { name: "Problems" })).toHaveCount(0);
  await page.getByRole("button", { name: "Create type" }).click();
  await expect(page).toHaveURL(new RegExp(`/types/${name}$`));

  // The server accepted the file, so it parsed and checked; schema inspection reads it back.
  const wcl = await admin.get(`/api/types/${name}`, { headers: { Accept: "text/plain" } });
  expect(wcl.status()).toBe(200);
  const text = await wcl.text();
  for (const kind of kinds) expect(text).toMatch(new RegExp(`field f_${kind} \\{\\n\\s+kind\\s+= "${kind}"`));
  const schema = (await (await admin.get("/api/schema")).json()) as {
    types: { name: string; fields: { name: string; kind: string }[] }[];
  };
  const stored = schema.types.find((x) => x.name === name);
  expect(stored?.fields.map((f) => f.kind)).toEqual(kinds);
  // The note page lays a new note out from the generated default template.
  const note = await createNote(admin, name, { title: `Kinds ${t}` });
  await page.goto(`/notes/${note.id}`);
  for (const kind of kinds) await expect(page.getByTestId(`field-f_${kind}`)).toBeVisible();
});

test("adding a required field shows the preview listing affected notes and refuses to apply without a default", async ({
  page,
  admin,
}) => {
  const t = token();
  const name = `req_${t}`;
  await putType(
    admin,
    name,
    `label = "Req"\n\nfield note {\n  kind = "string"\n}\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n  }\n}\n`,
  );
  const a = await createNote(admin, name, { title: `Alpha ${t}` });
  const b = await createNote(admin, name, { title: `Beta ${t}` });

  await logIn(page, `/settings/types/${name}/edit`);
  await page.getByRole("button", { name: "Add field" }).click();
  const card = page.getByTestId("field-row-1");
  await card.getByLabel("Name", { exact: true }).fill("owner");
  await card.getByLabel("Required").check();
  await page.getByRole("button", { name: "Preview changes" }).click();

  const preview = page.getByRole("region", { name: "Change preview" });
  await expect(preview.getByRole("alert")).toContainText("needs a default");
  const affected = preview.getByRole("list", { name: "Affected notes" });
  await expect(affected.getByRole("link", { name: `Alpha ${t}` })).toBeVisible();
  await expect(affected.getByRole("link", { name: `Beta ${t}` })).toBeVisible();
  await expect(preview.getByRole("button", { name: "Apply changes" })).toHaveCount(0);
  expect((await readNote(admin, a.id)).note.fields.owner).toBeUndefined();

  await card.getByLabel("Default for owner").fill("me");
  await expect(preview).toHaveCount(0);
  await page.getByRole("button", { name: "Preview changes" }).click();
  await expect(affected.getByRole("listitem")).toHaveCount(2);
  await expect(affected).toContainText('set owner to "me"');
  await preview.getByRole("button", { name: "Apply changes" }).click();
  await expect(page).toHaveURL(new RegExp(`/types/${name}$`));
  expect((await readNote(admin, a.id)).note.fields.owner).toBe("me");
  expect((await readNote(admin, b.id)).note.fields.owner).toBe("me");
});

test("moving a field to another tab in the template editor changes where it renders on the note page", async ({
  page,
  admin,
}) => {
  const t = token();
  const name = `tpl_${t}`;
  await putType(
    admin,
    name,
    `label = "Tpl"

field estimate {
  kind  = "int"
  label = "Estimate"
}

template default {
  tab "Overview" {
    slot "title"
  }

  tab "Details" {
    slot "estimate"
  }
}
`,
  );
  const note = await createNote(admin, name, { title: `Tpl ${t}`, estimate: 3 });
  const overview = () => page.getByRole("tab", { name: "Overview" });
  const details = () => page.getByRole("tab", { name: "Details" });

  await logIn(page, `/notes/${note.id}`);
  await expect(page.getByTestId("field-estimate")).toHaveCount(0);
  await details().click();
  await expect(page.getByTestId("field-estimate")).toBeVisible();

  // With the item's controls.
  await page.goto(`/settings/types/${name}/templates`);
  await details().click();
  await page.getByLabel("Move estimate to tab").selectOption({ label: "Overview" });
  await expect(page.getByTestId("template-item-estimate")).toHaveCount(0);
  await overview().click();
  await expect(page.getByTestId("template-item-estimate")).toBeVisible();
  await page.getByRole("button", { name: "Save template" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved the default template.");

  await page.goto(`/notes/${note.id}`);
  await expect(overview()).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("field-estimate").getByLabel("Estimate")).toHaveValue("3");
  await details().click();
  await expect(page.getByTestId("field-estimate")).toHaveCount(0);

  // By dragging the property block's handle onto the other tab.
  await page.goto(`/settings/types/${name}/templates`);
  await page.getByTestId("template-item-estimate").hover();
  const handle = page.getByTestId("block-handle");
  await expect(handle).toBeVisible();
  const from = (await handle.boundingBox())!;
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(from.x + 20, from.y, { steps: 4 });
  const to = (await details().boundingBox())!;
  await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, { steps: 10 });
  await page.mouse.up();
  await expect(details()).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("template-item-estimate")).toBeVisible();
  await page.getByRole("button", { name: "Save template" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved the default template.");

  await page.goto(`/notes/${note.id}`);
  await expect(page.getByTestId("field-estimate")).toHaveCount(0);
  await details().click();
  await expect(page.getByTestId("field-estimate").getByLabel("Estimate")).toHaveValue("3");
});

/** Open a chip picker's + box, type `typed` and pick the suggestion named `option`. */
async function pick(scope: Locator, label: string, typed: string, option: string) {
  const plus = scope.getByRole("button", { name: `Add to ${label}` });
  const box = scope.getByRole("combobox", { name: `Add to ${label}` });
  if (!(await box.isVisible())) await plus.click();
  await box.fill(typed);
  await scope.getByRole("option", { name: option, exact: true }).click();
}

/** A type with a field of every kind a note page edits, plus a computed one. */
async function everyKind(admin: APIRequestContext, t: string) {
  const taxonomy = `tax_${t}`;
  await putTaxonomy(
    admin,
    taxonomy,
    `label = "Tax"\n\nterm "Active" {\n  term "Blocked"\n}\n\nterm "Done"\n`,
  );
  const name = `every_${t}`;
  const fields: [string, string, string][] = [
    ["s", "Text", `kind = "string"`],
    ["i", "Count", `kind = "int"`],
    ["fl", "Score", `kind = "float"`],
    ["b", "Flag", `kind = "bool"`],
    ["d", "Due", `kind = "date"`],
    ["dt", "Starts", `kind = "datetime"`],
    ["span", "Span", `kind = "date_range"\n  of = "date"`],
    ["u", "Link", `kind = "url"`],
    ["sel", "Priority", `kind = "select"\n  options = ["low", "high"]`],
    ["chan", "Channels", `kind = "select"\n  multiple = true\n  options = ["email", "chat", "phone"]`],
    ["status", "Status", `kind = "taxonomy"\n  taxonomy = "${taxonomy}"`],
    ["topics", "Topics", `kind = "taxonomy"\n  multiple = true\n  taxonomy = "${taxonomy}"`],
    ["owner", "Owner", `kind = "reference"\n  target = "task"`],
    ["refs", "Related", `kind = "reference"\n  multiple = true\n  target = "task"`],
    ["aliases", "Aliases", `kind = "list"\n  items = "string"`],
    ["extra", "Extra", `kind = "dict"\n  values = "string"`],
    ["place", "Place", `kind = "location"\n  map = "geo"`],
    ["cover", "Cover", `kind = "asset"`],
    ["words", "Words", `kind = "int"\n  computed = true`],
  ];
  const defs = fields.map(([f, label, body]) => `field ${f} {\n  ${body}\n  label = "${label}"\n}\n`).join("\n");
  const slots = fields.map(([f]) => `    slot "${f}"\n`).join("");
  await putType(admin, name, `label = "Every"\n\n${defs}\ntemplate default {\n  tab "All" {\n    slot "title"\n${slots}  }\n}\n`);
  return name;
}

test("editing every field kind on a note page saves a value that validates", async ({ page, admin }) => {
  const t = token();
  const name = await everyKind(admin, t);
  const upload = await admin.post(`/api/assets?filename=cover-${t}.txt`, { data: "cover" });
  expect(upload.status(), await upload.text()).toBe(201);
  const asset = ((await upload.json()) as { id: string }).id;
  const first = await createNote(admin, "task", { title: `Target ${t} first` });
  const second = await createNote(admin, "task", { title: `Target ${t} second` });
  const note = await createNote(admin, name, { title: `Every ${t}` });

  await logIn(page, `/notes/${note.id}`);
  // Every field is edited in place, and the note saves itself.
  const field = (f: string) => page.getByTestId(`field-${f}`);
  await field("s").getByLabel("Text", { exact: true }).fill("hello");
  await field("i").getByLabel("Count", { exact: true }).fill("7");
  await field("fl").getByLabel("Score", { exact: true }).fill("2.5");
  await field("b").getByLabel("Flag", { exact: true }).check();
  await field("d").getByLabel("Due", { exact: true }).fill("2026-11-01");
  await field("dt").getByLabel("Starts", { exact: true }).fill("2026-10-20T09:30:15");
  await field("span").getByLabel("Start").fill("2026-10-20");
  await field("span").getByLabel("End").fill("2026-11-01");
  await field("u").getByLabel("Link", { exact: true }).fill("https://example.com/x");
  await field("sel").getByLabel("Priority", { exact: true }).selectOption("high");
  await pick(field("chan"), "Channels", "em", "email");
  await pick(field("chan"), "Channels", "ch", "chat");
  await field("status").getByLabel("Status", { exact: true }).selectOption("Active/Blocked");
  await pick(field("topics"), "Topics", "act", "Active");
  await pick(field("topics"), "Topics", "do", "Done");
  await pick(field("owner"), "Owner", t, `Target ${t} first`);
  await expect(field("owner").getByRole("link", { name: `Target ${t} first` })).toBeVisible();
  // One reference at a time: with it set, there is no + to add another.
  await expect(field("owner").getByRole("button", { name: "Add to Owner" })).toHaveCount(0);
  await pick(field("refs"), "Related", t, `Target ${t} second`);
  await pick(field("refs"), "Related", t, `Target ${t} first`);
  await field("aliases").getByRole("button", { name: "Add to Aliases" }).click();
  await field("aliases").getByRole("combobox", { name: "Add to Aliases" }).fill("one");
  await field("aliases").getByRole("combobox", { name: "Add to Aliases" }).press("Enter");
  await field("aliases").getByRole("combobox", { name: "Add to Aliases" }).fill("two");
  await field("aliases").getByRole("combobox", { name: "Add to Aliases" }).press("Enter");
  await expect(field("aliases").locator(".chip")).toHaveText(["one×", "two×"]);
  await field("extra").getByRole("button", { name: "Add entry" }).click();
  await field("extra").getByLabel("Extra key 1", { exact: true }).fill("source");
  await field("extra").getByLabel("Extra value 1", { exact: true }).fill("capacities");
  await field("place").getByLabel("Latitude").fill("-37.8136");
  await field("place").getByLabel("Longitude").fill("144.9631");
  await field("cover").getByLabel("Cover", { exact: true }).selectOption(asset);

  await page.getByLabel("Title", { exact: true }).focus();
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
  await expect(page.getByRole("alert")).toHaveCount(0);

  const saved = await readNote(admin, note.id);
  const f = saved.note.fields;
  expect(f).toMatchObject({
    s: "hello",
    i: 7,
    fl: 2.5,
    b: true,
    d: "2026-11-01",
    span: { start: "2026-10-20", end: "2026-11-01" },
    u: "https://example.com/x",
    sel: "high",
    chan: ["email", "chat"],
    status: "Active/Blocked",
    topics: ["Active", "Done"],
    owner: first.id,
    refs: [second.id, first.id],
    aliases: ["one", "two"],
    place: { lat: -37.8136, lng: 144.9631 },
    cover: asset,
  });
  expect(f.dt).toMatch(/^2026-10-20T09:30:15(Z|[+-]\d\d:\d\d)$/);
  expect(saved.note.blocks).toContainEqual({ kind: "dict", labels: ["extra"], fields: { source: "capacities" } });

  // The saved values read back through the page, references by title.
  await page.reload();
  await expect(field("owner")).toContainText(`Target ${t} first`);
  await expect(field("status").getByLabel("Status", { exact: true })).toHaveValue("Active/Blocked");
  await expect(field("extra").getByLabel("Extra key 1", { exact: true })).toHaveValue("source");
  await expect(field("extra").getByLabel("Extra value 1", { exact: true })).toHaveValue("capacities");
});

test("a computed field cannot be edited in the UI", async ({ page, admin }) => {
  const t = token();
  const name = await everyKind(admin, t);
  const note = await createNote(admin, name, { title: `Computed ${t}` });

  await logIn(page, `/notes/${note.id}`);
  const words = page.getByTestId("field-words");
  await expect(words).toContainText("computed");
  await expect(words.locator("input, select, textarea, [contenteditable]")).toHaveCount(0);
  // Every other field of the note has an editor at the same time.
  await expect(page.getByTestId("field-i").getByLabel("Count", { exact: true })).toBeEditable();

  // The API refuses a client write to it too (D3).
  const res = await admin.put(`/api/notes/${note.id}`, {
    data: { note: { fields: { title: `Computed ${t}`, words: 5 }, blocks: [] }, base_version: (await readNoteVersion(admin, note.id)) },
  });
  expect(res.status()).toBe(400);
});

async function readNoteVersion(admin: APIRequestContext, id: string): Promise<string> {
  const res = await admin.get(`/api/notes/${id}`);
  return ((await res.json()) as { version: string }).version;
}

test("the taxonomy editor builds a hierarchy and renaming a term rewrites the notes holding it", async ({
  page,
  admin,
}) => {
  const t = token();
  const taxonomy = `stage_${t}`;
  await logIn(page, "/taxonomies");
  await page.getByRole("link", { name: "New taxonomy" }).click();
  await page.getByLabel("Name", { exact: true }).fill(taxonomy);
  await page.getByLabel("Label", { exact: true }).fill("Stage");
  await page.getByLabel("Name of First term").fill("Active");
  await page.getByRole("button", { name: "Add a term under Active" }).click();
  await page.getByLabel("Name of Active/New term").fill("Blocked");
  await page.getByRole("button", { name: "Add term" }).click();
  await page.getByLabel("Name of New term").fill("Waiting");
  // Nest Waiting under Active, after Blocked, then give it a colour.
  await page.getByRole("button", { name: "Nest Waiting under the term above" }).click();
  await page.getByLabel("Colour of Active/Waiting").selectOption("red");
  await page.getByRole("button", { name: "Add term" }).click();
  await page.getByLabel("Name of New term").fill("Done");
  await page.getByRole("button", { name: "Create taxonomy" }).click();
  await expect(page).toHaveURL(new RegExp(`/taxonomies/${taxonomy}$`));

  const tree = async () =>
    ((await (await admin.get(`/api/taxonomies/${taxonomy}`)).json()) as {
      terms: { path: string; colour: string | null; children: { path: string; colour: string | null }[] }[];
    }).terms;
  expect(await tree()).toMatchObject([
    {
      path: "Active",
      children: [
        { path: "Active/Blocked", colour: null },
        { path: "Active/Waiting", colour: "red" },
      ],
    },
    { path: "Done", children: [] },
  ]);

  const type = `staged_${t}`;
  await putType(
    admin,
    type,
    `label = "Staged"\n\nfield stage {\n  kind = "taxonomy"\n  taxonomy = "${taxonomy}"\n}\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n    slot "stage"\n  }\n}\n`,
  );
  const note = await createNote(admin, type, { title: `Staged ${t}`, stage: "Active/Blocked" });

  await page.reload();
  await page.getByLabel("Name of Active", { exact: true }).fill("Doing");
  await page.getByRole("button", { name: "Save taxonomy" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved. 1 note was updated.");
  expect((await readNote(admin, note.id)).note.fields.stage).toBe("Doing/Blocked");

  // The note page offers the terms as a hierarchy.
  await page.goto(`/notes/${note.id}`);
  await expect(page.getByTestId("field-stage")).toBeVisible();
  const options = page.getByTestId("field-stage").getByRole("combobox").locator("option");
  expect(await options.evaluateAll((os) => os.map((o) => o.textContent))).toEqual([
    "—",
    "Doing",
    "\u00a0\u00a0Blocked",
    "\u00a0\u00a0Waiting",
    "Done",
  ]);
});
