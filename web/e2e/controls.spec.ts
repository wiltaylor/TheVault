import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// Form controls bound to note fields: inserted from the `/` menu in a template, changed on a note
// with every change saved to its field (where a `{{field}}` and a slot of the same field follow),
// a required field that cannot be cleared, a read-only control on a computed field, controls in
// columns, in a note's own content and in a canvas card.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

const pad = (n: number) => String(n).padStart(2, "0");
const today = new Date();
const ym = `${today.getFullYear()}-${pad(today.getMonth() + 1)}`;

/** A type with a field of every kind a control fits, and some it does not. */
function fieldsWcl(name: string): string {
  return `label = "Form ${name}"
plural = "Forms ${name}"

field name {
  kind     = "string"
  label    = "Name"
  required = true
}

field notes {
  kind  = "string"
  label = "Notes"
}

field site {
  kind  = "url"
  label = "Site"
}

field count {
  kind  = "int"
  label = "Count"
}

field ratio {
  kind  = "float"
  label = "Ratio"
}

field done {
  kind  = "bool"
  label = "Done"
}

field due {
  kind  = "date"
  label = "Due"
}

field at {
  kind  = "datetime"
  label = "At"
}

field span {
  kind  = "date_range"
  label = "Span"
}

field size {
  kind    = "select"
  label   = "Size"
  options = ["small", "medium", "large"]
}

field sizes {
  kind     = "list"
  items    = "select"
  label    = "Sizes"
  options  = ["small", "medium", "large"]
}

field area {
  kind     = "taxonomy"
  label    = "Area"
  taxonomy = "area"
}

field areas {
  kind     = "list"
  items    = "taxonomy"
  label    = "Areas"
  taxonomy = "area"
}

field tags {
  kind  = "list"
  label = "Tags"
  items = "string"
}

field parent {
  kind   = "reference"
  label  = "Parent"
  target = "${name}"
}

field links {
  kind   = "list"
  items  = "reference"
  label  = "Links"
  target = "${name}"
}

field stars {
  kind  = "int"
  label = "Stars"
}

field words {
  kind     = "int"
  label    = "Words"
  computed = true
}

field body {
  kind  = "content"
  label = "Body"
}

field plan {
  kind  = "canvas"
  label = "Plan"
}
`;
}

const ctl = (field: string, kind: string, extra = "") =>
  `control {\n  field = "${field}"\n  kind  = "${kind}"${extra}\n}\n`;

/** The form template: every control kind, in two columns, with references and slots to compare. */
const FORM_TAB = `    paragraph {
      text = "Name={{name}} Count={{count}} Done={{done}} Size={{size}} Due={{due}} Stars={{stars}} Tags={{tags}}"
    }
    columns {
      column {
        width = 1
${[
  ctl("name", "text", '\n  placeholder = "Your name"'),
  ctl("notes", "textarea"),
  ctl("count", "number", "\n  min = 0\n  max = 10"),
  ctl("ratio", "slider", "\n  min = 0\n  max = 1\n  step = 0.1"),
  ctl("done", "toggle"),
  ctl("done", "checkbox", '\n  label = "Done (box)"'),
  ctl("size", "dropdown"),
  ctl("size", "radio", '\n  label = "Size (radio)"'),
  ctl("area", "dropdown"),
  ctl("words", "number"),
].join("")}      }
      column {
        width = 1
${[
  ctl("parent", "dropdown"),
  ctl("tags", "chips"),
  ctl("sizes", "chips"),
  ctl("areas", "chips"),
  ctl("links", "chips"),
  ctl("due", "calendar"),
  ctl("span", "calendar"),
  ctl("at", "datetime"),
  ctl("at", "time", '\n  label = "At (time)"'),
  ctl("stars", "rating", "\n  max = 5"),
].join("")}      }
    }
    slot "count"
    slot "size"
    slot "due"
    slot "tags"
    slot "body"
`;

async function putType(admin: APIRequestContext, name: string, tab: string) {
  const wcl = `${fieldsWcl(name)}\ntemplate default {\n  tab "Main" {\n${tab}  }\n}\n`;
  const put = await admin.put(`/api/types/${name}`, { data: { wcl } });
  expect(put.status(), await put.text()).toBe(200);
}

type Json = { kind: string; fields?: Record<string, unknown>; blocks?: Json[]; labels?: string[] };

async function createNote(admin: APIRequestContext, type: string, fields: Record<string, unknown>, blocks: Json[] = []) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return ((await res.json()) as { id: string }).id;
}

async function fields(admin: APIRequestContext, id: string): Promise<Record<string, unknown>> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return ((await res.json()) as { note: { fields: Record<string, unknown> } }).note.fields;
}

async function saved(page: Page) {
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
}

/** The control bound to `field` of kind `kind`. */
function control(scope: Page | Locator, field: string, kind: string): Locator {
  return scope.locator(`[data-testid="control"][data-field="${field}"][data-kind="${kind}"]`);
}

async function properties(page: Page): Promise<Locator> {
  await page.getByRole("button", { name: "Properties", exact: true }).click();
  return page.getByRole("complementary", { name: "Properties" });
}

test("each control kind is inserted from / in a template, offered only fitting fields", async ({ page, admin }) => {
  const name = `form_${token()}`;
  await putType(admin, name, '    slot "title"\n');
  await logIn(page, `/settings/types/${name}/templates`);
  const editor = page.getByRole("textbox", { name: "Main content" });
  await expect(editor.getByTestId("template-item-title")).toBeVisible();
  const menu = page.getByRole("listbox", { name: "Insert block" });

  // Each kind, the field chosen for it, fields it must offer, and fields it must not.
  const cases: [string, string, string[], string[]][] = [
    ["Text box", "Name", ["Site", "Count", "Ratio", "Title"], ["Done", "Due", "Size", "Tags"]],
    ["Text area", "Notes", ["Name"], ["Site", "Count"]],
    ["Number stepper", "Count", ["Ratio", "Stars", "Words"], ["Name", "Done"]],
    ["Slider", "Ratio", ["Count"], ["Name", "Due"]],
    ["Toggle", "Done", [], ["Name", "Count"]],
    ["Checkbox", "Done", [], ["Size"]],
    ["Dropdown", "Size", ["Area", "Parent"], ["Sizes", "Areas", "Links", "Name"]],
    ["Radio buttons", "Size", [], ["Sizes", "Area", "Parent"]],
    ["Chip list", "Tags", ["Sizes", "Areas", "Links"], ["Size", "Area", "Parent", "Name"]],
    ["Calendar", "Due", ["Span"], ["At", "Name"]],
    ["Date and time", "At", [], ["Due", "Span"]],
    ["Time", "At", [], ["Due"]],
    ["Rating", "Stars", ["Count"], ["Ratio", "Done"]],
  ];
  for (const [kind, field, also, never] of cases) {
    await editor.locator("p").last().click();
    await page.keyboard.type("/");
    await expect(menu.getByText("Controls", { exact: true })).toBeVisible();
    await menu.getByRole("option", { name: kind, exact: true }).click();
    // The follow-up list is the fields this kind fits, with their kinds.
    await expect(menu.getByText(`${kind}: choose a field`)).toBeVisible();
    const offered = (f: string) => menu.getByRole("option").filter({ has: page.locator("span", { hasText: new RegExp(`^${f}$`) }) });
    for (const f of [field, ...also]) await expect(offered(f)).toHaveCount(1);
    for (const f of never) await expect(offered(f)).toHaveCount(0);
    await offered(field).click();
  }
  const controls = editor.getByTestId("control");
  await expect(controls).toHaveCount(cases.length);

  // The settings strip: a label, and for a slider its ends; only fitting fields are offered.
  const slider = editor.locator('[data-testid="control"][data-kind="slider"]');
  await slider.hover();
  const strip = page.getByTestId("control-strip").filter({ hasText: "Slider" });
  await expect(strip).toBeVisible();
  await expect(strip.getByLabel("Bound field").locator("option")).toHaveText(["Count (int)", "Ratio (float)", "Stars (int)", "Words (int, computed)"]);
  await strip.getByLabel("Control label").fill("Mix");
  await strip.getByLabel("Maximum").fill("5");
  await strip.getByLabel("Maximum").press("Tab");
  await strip.getByLabel("Bound field").selectOption("count");
  await expect(slider.locator(".control-label")).toContainText("Mix");

  await page.getByRole("button", { name: "Save template" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved the default template.");
  const wcl = (await (await admin.get(`/api/types/${name}/templates/default`)).json()).wcl as string;
  for (const kind of ["text", "textarea", "number", "slider", "toggle", "checkbox", "dropdown", "radio", "chips", "calendar", "datetime", "time", "rating"]) {
    expect(wcl).toContain(`kind  = "${kind}"`);
  }
  expect(wcl).toMatch(/field = "count"\n\s+kind {2}= "slider"\n\s+label = "Mix"\n\s+min {3}= 0\n\s+max {3}= 5\n\s+step {2}= 1/);

  // On a note, the controls edit the fields.
  const id = await createNote(admin, name, { title: "Inserted", name: "Ada" });
  await page.goto(`/notes/${id}`);
  await control(page, "name", "text").getByRole("textbox").fill("Grace");
  await control(page, "stars", "rating").getByRole("radio", { name: "3 stars" }).click();
  await saved(page);
  expect(await fields(admin, id)).toMatchObject({ name: "Grace", stars: 3 });
});

test("every control writes its field, and a {{field}} and a slot of the same field follow", async ({ page, admin }) => {
  const name = `form_${token()}`;
  await putType(admin, name, FORM_TAB);
  const target = await createNote(admin, name, { title: "Target note", name: "T" });
  const id = await createNote(admin, name, { title: "Filled in", name: "Ada" });
  await logIn(page, `/notes/${id}`);
  const refs = page.getByTestId("template-block").filter({ hasText: "Name=" });
  await expect(refs).toContainText("Name=Ada");
  const layout = page.getByTestId("template-columns");

  // Text and text area.
  await control(layout, "name", "text").getByRole("textbox", { name: "Name" }).fill("Grace");
  await control(layout, "notes", "textarea").getByRole("textbox", { name: "Notes" }).fill("Line one\nLine two");
  await expect(refs).toContainText("Name=Grace");

  // Number stepper: from empty, Increase starts at the minimum.
  const count = control(layout, "count", "number");
  for (let i = 0; i < 3; i++) await count.getByRole("button", { name: "Increase Count" }).click();
  await expect(count.getByRole("spinbutton")).toHaveValue("2");
  await expect(refs).toContainText("Count=2");
  await expect(page.getByTestId("field-count").getByLabel("Count")).toHaveValue("2");
  // The slot's edit reaches the control.
  await page.getByTestId("field-count").getByLabel("Count").fill("7");
  await expect(count.getByRole("spinbutton")).toHaveValue("7");

  // Slider, by keyboard.
  const ratio = control(layout, "ratio", "slider").getByRole("slider");
  await ratio.focus();
  await page.keyboard.press("End");
  await expect(ratio).toHaveAttribute("aria-valuenow", "1");
  await page.keyboard.press("ArrowLeft");
  await expect(ratio).toHaveAttribute("aria-valuenow", "0.9");

  // A toggle and a checkbox on one field agree.
  const toggle = control(layout, "done", "toggle").getByRole("switch", { name: "Done" });
  const box = control(layout, "done", "checkbox").getByRole("checkbox", { name: "Done (box)" });
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await expect(box).toBeChecked();
  await expect(refs).toContainText("Done=Yes");
  await box.uncheck();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await box.check();

  // A dropdown and radio buttons on one field agree, and the slot follows.
  await control(layout, "size", "dropdown").getByRole("combobox", { name: "Size" }).selectOption("medium");
  const radios = control(layout, "size", "radio").getByRole("radiogroup", { name: "Size (radio)" });
  await expect(radios.getByRole("radio", { name: "medium" })).toBeChecked();
  await radios.getByRole("radio", { name: "large" }).check();
  await expect(control(layout, "size", "dropdown").getByRole("combobox", { name: "Size" })).toHaveValue("large");
  await expect(page.getByTestId("field-size").getByLabel("Size")).toHaveValue("large");
  await expect(refs).toContainText("Size=large");

  // A taxonomy dropdown shows the hierarchy.
  const area = control(layout, "area", "dropdown").getByRole("combobox", { name: "Area" });
  await expect(area.locator("option")).toHaveText(["—", "Plant", /› Electrical/, /› Plumbing/, "Structure"]);
  await area.selectOption("Plant/Plumbing");

  // A reference dropdown searches notes.
  const parent = control(layout, "parent", "dropdown").getByRole("combobox", { name: "Parent" });
  await parent.click();
  await parent.fill("Target");
  await control(layout, "parent", "dropdown").getByRole("option", { name: "Target note" }).click();
  await expect(parent).toHaveValue("Target note");

  // Chips: free text for a list, options for a select, terms for a taxonomy, notes for references.
  const tags = control(layout, "tags", "chips");
  // No box around the chips; a + opens the add box, and Escape puts the + back.
  expect(await tags.locator(".control-chips").evaluate((el) => getComputedStyle(el).borderTopStyle)).toBe("none");
  await expect(tags.getByRole("combobox", { name: "Add to Tags" })).toHaveCount(0);
  await tags.getByRole("button", { name: "Add to Tags" }).click();
  const addTag = tags.getByRole("combobox", { name: "Add to Tags" });
  await expect(addTag).toBeFocused();
  await addTag.press("Escape");
  await expect(addTag).toHaveCount(0);
  await expect(tags.getByRole("button", { name: "Add to Tags" })).toBeFocused();
  await tags.getByRole("button", { name: "Add to Tags" }).click();
  await addTag.fill("alpha");
  await addTag.press("Enter");
  await addTag.fill("beta");
  await addTag.press("Enter");
  await expect(tags.locator(".chip")).toHaveText(["alpha×", "beta×"]);
  await tags.getByRole("button", { name: "Remove alpha" }).click();
  await expect(tags.locator(".chip")).toHaveText(["beta×"]);
  await expect(refs).toContainText("Tags=beta");
  await expect(page.getByTestId("field-tags").locator(".chip")).toHaveText(["beta×"]);
  const sizes = control(layout, "sizes", "chips");
  await sizes.getByRole("button", { name: "Add to Sizes" }).click();
  await sizes.getByRole("combobox", { name: "Add to Sizes" }).fill("sm");
  await sizes.getByRole("option", { name: "small" }).click();
  await expect(sizes.locator(".chip")).toHaveText(["small×"]);
  const areas = control(layout, "areas", "chips");
  await areas.getByRole("button", { name: "Add to Areas" }).click();
  await areas.getByRole("combobox", { name: "Add to Areas" }).fill("Elec");
  await areas.getByRole("option", { name: "Electrical" }).click();
  const links = control(layout, "links", "chips");
  await links.getByRole("button", { name: "Add to Links" }).click();
  await links.getByRole("combobox", { name: "Add to Links" }).fill("Target");
  await links.getByRole("option", { name: "Target note" }).click();
  await expect(links.locator(".chip")).toHaveText(["Target note×"]);

  // Calendars: a date takes one click, a range two.
  const due = control(layout, "due", "calendar");
  await due.locator(`button[data-day="${ym}-15"]`).click();
  await expect(due.locator(`button[data-day="${ym}-15"]`)).toHaveAttribute("aria-pressed", "true");
  await expect(refs).toContainText(`Due=${ym}-15`);
  await expect(page.getByTestId("field-due").getByLabel("Due")).toHaveValue(`${ym}-15`);
  const span = control(layout, "span", "calendar");
  await span.locator(`button[data-day="${ym}-20"]`).click();
  await expect(span).toContainText("pick the end date");
  await span.locator(`button[data-day="${ym}-10"]`).click();
  await expect(span.locator(".day-inside")).toHaveCount(9);

  // Date and time, then the time alone, which keeps the date.
  await control(layout, "at", "datetime").locator("input").fill("2026-10-20T09:30:15");
  const time = control(layout, "at", "time").locator('input[type="time"]');
  await expect(time).toHaveValue("09:30");
  await time.fill("14:45");
  await expect(control(layout, "at", "datetime").locator("input")).toHaveValue("2026-10-20T14:45");

  // Rating: stars by click and keyboard.
  const stars = control(layout, "stars", "rating").getByRole("radiogroup", { name: "Stars" });
  await stars.getByRole("radio", { name: "2 stars" }).click();
  await page.keyboard.press("ArrowRight");
  await expect(stars.getByRole("radio", { name: "3 stars" })).toHaveAttribute("aria-checked", "true");
  await expect(refs).toContainText("Stars=3");

  await saved(page);
  const f = await fields(admin, id);
  expect(f).toMatchObject({
    name: "Grace",
    notes: "Line one\nLine two",
    count: 7,
    ratio: 0.9,
    done: true,
    size: "large",
    area: "Plant/Plumbing",
    parent: target,
    tags: ["beta"],
    sizes: ["small"],
    areas: ["Plant/Electrical"],
    links: [target],
    due: `${ym}-15`,
    span: { start: `${ym}-10`, end: `${ym}-20` },
    stars: 3,
  });
  expect(String(f.at)).toMatch(/^2026-10-20T14:45:00(Z|[+-]\d\d:\d\d)$/);

  // The Properties panel lists every field, the ones controls in columns place too, with the
  // values the controls wrote. The body has its editor on the page; the canvas no place.
  const panel = await properties(page);
  for (const field of ["name", "notes", "ratio", "done", "area", "parent", "sizes", "areas", "links", "span", "at", "stars", "words", "plan"]) {
    await expect(panel.getByTestId(`field-${field}`)).toBeVisible();
  }
  await expect(panel.getByTestId("field-name").getByLabel("Name")).toHaveValue("Grace");
  await expect(panel.getByTestId("field-body")).toHaveCount(0);

  // After a reload the controls show the stored values.
  await page.reload();
  await expect(control(page, "name", "text").getByRole("textbox")).toHaveValue("Grace");
  await expect(control(page, "stars", "rating").getByRole("radio", { name: "3 stars" })).toHaveAttribute("aria-checked", "true");
  await expect(control(page, "tags", "chips").locator(".chip")).toHaveText(["beta×"]);
});

test("a required field cannot be cleared, and a computed field's control is read-only", async ({ page, admin }) => {
  const name = `form_${token()}`;
  await putType(admin, name, FORM_TAB);
  const id = await createNote(admin, name, { title: "Required", name: "Ada" });
  await logIn(page, `/notes/${id}`);
  const box = control(page, "name", "text").getByRole("textbox", { name: "Name" });
  await box.fill("");
  await expect(control(page, "name", "text").getByRole("alert")).toHaveText("Name is required, so it cannot be empty.");
  await expect(box).toHaveAttribute("aria-describedby", /error/);
  await box.blur();
  await expect(box).toHaveValue("Ada");
  await control(page, "notes", "textarea").getByRole("textbox").fill("Edited");
  await saved(page);
  expect(await fields(admin, id)).toMatchObject({ name: "Ada", notes: "Edited" });

  // The computed field: shown, disabled, with the reason.
  const words = control(page, "words", "number");
  await expect(words.getByRole("spinbutton")).toBeDisabled();
  await expect(words.getByRole("button", { name: "Increase Words" })).toBeDisabled();
  await expect(words).toContainText("computed");
  await expect(words).toContainText("only the type's handlers write this field");
});

test("a control in a note's own content edits the note and places its field", async ({ page, admin }) => {
  const name = `form_${token()}`;
  await putType(admin, name, '    slot "body"\n');
  const id = await createNote(admin, name, { title: "Own content", name: "Ada" });
  await logIn(page, `/notes/${id}`);
  const panel = await properties(page);
  await expect(panel.getByTestId("field-stars")).toBeVisible();
  await page.getByRole("button", { name: "Properties", exact: true }).click();

  const body = page.getByRole("textbox", { name: "Body" });
  await body.click();
  await page.keyboard.type("/rating");
  const menu = page.getByRole("listbox", { name: "Insert block" });
  await menu.getByRole("option", { name: "Rating", exact: true }).click();
  await expect(menu.getByRole("option", { name: /^Ratio/ })).toHaveCount(0);
  await menu.getByRole("option", { name: /^Stars/ }).click();
  const rating = control(body, "stars", "rating");
  await expect(rating).toBeVisible();
  await rating.getByRole("radio", { name: "4 stars" }).click();
  await saved(page);
  const res = await admin.get(`/api/notes/${id}`);
  const stored = (await res.json()) as { note: { fields: Record<string, unknown>; blocks: Json[] } };
  expect(stored.note.fields.stars).toBe(4);
  expect(stored.note.blocks.find((b) => b.labels?.[0] === "body")?.blocks).toMatchObject([{ kind: "control", fields: { field: "stars", kind: "rating" } }]);

  // Placed by the control, the field stays in the Properties panel, with the control's value.
  const after = await properties(page);
  await expect(after.getByTestId("field-name")).toBeVisible();
  await expect(after.getByTestId("field-stars")).toBeVisible();
  await expect(after.getByTestId("field-stars").getByLabel("Stars")).toHaveValue("4");
});

test("a control in a canvas card edits the note that holds the canvas", async ({ page, admin }) => {
  const name = `form_${token()}`;
  await putType(admin, name, '    slot "plan"\n');
  const canvas: Json = {
    kind: "canvas",
    labels: ["plan"],
    blocks: [
      { kind: "viewport", fields: { x: 0, y: 0, zoom: 1 } },
      {
        kind: "node",
        labels: ["c"],
        fields: { kind: "card", x: 20, y: 20, width: 320, height: 200 },
        blocks: [
          { kind: "paragraph", fields: { text: "On the board" } },
          { kind: "control", fields: { field: "done", kind: "toggle" } },
          { kind: "control", fields: { field: "size", kind: "radio" } },
        ],
      },
    ],
  };
  const id = await createNote(admin, name, { title: "Board", name: "Ada" }, [canvas]);
  await logIn(page, `/notes/${id}`);
  const card = page.locator('[data-testid="canvas-node"][data-id="c"]');
  const toggle = card.getByRole("switch", { name: "Done" });
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await card.getByRole("radio", { name: "small" }).check();
  await saved(page);
  expect(await fields(admin, id)).toMatchObject({ done: true, size: "small" });
  // The card did not move.
  const res = await admin.get(`/api/notes/${id}`);
  const stored = (await res.json()) as { note: { blocks: Json[] } };
  const node = stored.note.blocks[0]?.blocks?.find((b) => b.labels?.[0] === "c");
  expect(node?.fields).toMatchObject({ x: 20, y: 20 });
});
