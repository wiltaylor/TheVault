import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { APIRequestContext, APIResponse, Locator, Page } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// A list field can hold terms of a taxonomy, and a taxonomy can be open: a term typed on a note
// that the taxonomy lacks is offered as "Add", and saving the note adds it to the taxonomy.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

async function ok(res: Promise<APIResponse>, want: number) {
  const r = await res;
  expect(r.status(), await r.text()).toBe(want);
  return r;
}

/**
 * With E2E_SHOTS set to a folder, save light and dark screenshots of `what` there. Without it,
 * nothing is written.
 */
async function shots(page: Page, name: string, ...what: Locator[]) {
  const dir = process.env.E2E_SHOTS;
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  // The area round every part, which may sit outside one another, such as a popup list.
  const boxes = (await Promise.all(what.map((w) => w.boundingBox()))).filter((b) => b !== null);
  const x = Math.max(0, Math.min(...boxes.map((b) => b.x)) - 8);
  const y = Math.max(0, Math.min(...boxes.map((b) => b.y)) - 8);
  const clip = {
    x,
    y,
    width: Math.max(...boxes.map((b) => b.x + b.width)) + 8 - x,
    height: Math.max(...boxes.map((b) => b.y + b.height)) + 8 - y,
  };
  for (const theme of ["light", "dark"]) {
    await page.evaluate((t) => (document.documentElement.dataset.theme = t), theme);
    await page.screenshot({ path: join(dir, `opentax-${name}-${theme}.png`), clip });
  }
  await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
}

async function taxonomy(admin: APIRequestContext, name: string) {
  return (await (await ok(admin.get(`/api/taxonomies/${name}`), 200)).json()) as {
    allow_new: boolean;
    terms: { path: string; children: { path: string }[] }[];
  };
}

test("the type editor makes a list field hold the terms of a taxonomy", async ({ page, admin }) => {
  const t = token();
  const tax = `topic_${t}`;
  await ok(admin.put(`/api/taxonomies/${tax}`, { data: { wcl: `label = "Topics ${t}"\n\nterm "Plants"\n` } }), 200);
  const name = `listed_${t}`;
  await ok(
    admin.put(`/api/types/${name}`, {
      data: { wcl: `label = "Listed"\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n  }\n}\n` },
    }),
    200,
  );

  await logIn(page, `/settings/types/${name}/edit`);
  await page.getByRole("button", { name: "Add field" }).click();
  const row = page.getByTestId("field-row-0");
  await row.getByLabel("Name", { exact: true }).fill("topics");
  await row.getByLabel("Kind", { exact: true }).selectOption("list");
  // Taxonomy is one of the item kinds, and choosing it asks which taxonomy.
  await expect(row.getByLabel("Taxonomy", { exact: true })).toHaveCount(0);
  await row.getByLabel("Item kind").selectOption("taxonomy");
  await row.getByLabel("Taxonomy", { exact: true }).selectOption(tax);
  await row.scrollIntoViewIfNeeded();
  await shots(page, "type-editor", page.getByRole("table", { name: "Fields" }).locator("thead"), row);
  await page.getByRole("button", { name: "Preview changes" }).click();
  await page.getByRole("button", { name: "Apply changes" }).click();

  await expect
    .poll(async () => (await admin.get(`/api/types/${name}`, { headers: { Accept: "text/plain" } })).text())
    .toMatch(new RegExp(`field topics \\{\\n\\s+kind\\s+= "list"\\n\\s+items\\s+= "taxonomy"\\n\\s+taxonomy = "${tax}"\\n\\}`));

  // It reads back as a list of terms.
  await page.goto(`/settings/types/${name}/edit`);
  const again = page.getByTestId("field-row-0");
  await expect(again.getByLabel("Kind", { exact: true })).toHaveValue("list");
  await again.getByRole("button", { name: "Settings for topics" }).click();
  await expect(again.getByLabel("Item kind")).toHaveValue("taxonomy");
  await expect(again.getByLabel("Taxonomy", { exact: true })).toHaveValue(tax);
});

test("an open taxonomy takes a new term typed on a note, and a closed one offers no Add", async ({ page, admin }) => {
  const t = token();
  const open = `tag_${t}`;
  const closed = `stage_${t}`;
  await ok(
    admin.put(`/api/taxonomies/${open}`, {
      data: { wcl: `label = "Tags ${t}"\n\nterm "Plant" {\n  colour = "green"\n}\n\nterm "Office"\n` },
    }),
    200,
  );
  await ok(admin.put(`/api/taxonomies/${closed}`, { data: { wcl: `label = "Stages ${t}"\n\nterm "Draft"\n` } }), 200);
  const type = `tagged_${t}`;
  const wcl =
    `label = "Tagged"\n\n` +
    `field tags {\n  kind = "list"\n  items = "taxonomy"\n  taxonomy = "${open}"\n  label = "Tags"\n}\n\n` +
    `field stages {\n  kind = "list"\n  items = "taxonomy"\n  taxonomy = "${closed}"\n  label = "Stages"\n}\n\n` +
    `template default {\n  tab "Main" {\n    slot "title"\n    slot "tags"\n    slot "stages"\n  }\n}\n`;
  await ok(admin.put(`/api/types/${type}`, { data: { wcl } }), 200);
  const note = (
    (await (
      await ok(admin.post(`/api/notes?type=${type}`, { data: { note: { fields: { title: `Tagged ${t}` }, blocks: [] } } }), 201)
    ).json()) as { id: string }
  ).id;

  // The taxonomy editor opens it to new terms.
  await logIn(page, `/taxonomies/${open}`);
  const allow = page.getByLabel("Let fields add new terms");
  await expect(allow).not.toBeChecked();
  await allow.check();
  await shots(page, "taxonomy-editor", page.locator(".form-grid"), page.getByRole("tree", { name: "Terms" }));
  await page.getByRole("button", { name: "Save taxonomy" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved.");
  expect((await taxonomy(admin, open)).allow_new).toBe(true);

  await page.goto(`/notes/${note}`);
  const tags = page.getByTestId("field-tags");
  await tags.getByRole("button", { name: "Add to Tags" }).click();
  const box = tags.getByRole("combobox", { name: "Add to Tags" });
  // Text that names no term is offered last, as an Add.
  await box.fill("Off");
  await expect(tags.getByRole("option")).toHaveText(["Office", "Add “Off”"]);
  await box.fill("Plant/Solar");
  const add = tags.getByRole("option", { name: "Add “Plant/Solar”" });
  await expect(add).toBeVisible();
  await expect(add.locator(".combo-dot")).toHaveAttribute("data-colour", "green");
  await shots(page, "picker", tags, tags.getByRole("listbox"));
  await add.click();
  await box.fill("Brand new");
  await box.press("Enter");
  // A name in another case is the existing term, and a missing parent is refused.
  await box.fill("office");
  await box.press("Enter");
  await box.fill("Nowhere/Else");
  await box.press("Enter");
  await expect(tags.getByRole("alert")).toHaveText("There is no term “Nowhere” to add “Else” under.");
  await expect(tags.locator(".chip")).toHaveText(["Plant/Solar×", "Brand new×", "Office×"]);
  await expect(tags.locator('.chip[data-value="Plant/Solar"]')).toHaveAttribute("data-colour", "green");
  await page.getByLabel("Title", { exact: true }).focus();
  await expect(page.getByTestId("save-status")).toHaveText("Saved");

  const stored = ((await (await admin.get(`/api/notes/${note}`)).json()) as { note: { fields: Record<string, unknown> } }).note.fields;
  expect(stored.tags).toEqual(["Plant/Solar", "Brand new", "Office"]);
  const terms = (await taxonomy(admin, open)).terms;
  expect(terms.map((x) => x.path)).toEqual(["Plant", "Office", "Brand new"]);
  expect(terms[0]!.children.map((x) => x.path)).toEqual(["Plant/Solar"]);

  // The client loaded the taxonomy again: the new terms are suggestions now, not Adds.
  await tags.getByRole("button", { name: "Remove Brand new" }).click();
  await tags.getByRole("button", { name: "Add to Tags" }).click();
  await tags.getByRole("combobox", { name: "Add to Tags" }).fill("bran");
  await expect(tags.getByRole("option")).toHaveText(["Brand new", "Add “bran”"]);
  await tags.getByRole("combobox", { name: "Add to Tags" }).press("Escape");

  // A closed taxonomy offers only its terms, and typed text that names none is not added.
  const stages = page.getByTestId("field-stages");
  await stages.getByRole("button", { name: "Add to Stages" }).click();
  const stageBox = stages.getByRole("combobox", { name: "Add to Stages" });
  await stageBox.fill("d");
  await expect(stages.getByRole("option")).toHaveText(["Draft"]);
  await stageBox.fill("Published");
  await expect(stages.getByRole("option")).toHaveCount(0);
  await stageBox.press("Enter");
  await expect(stages.locator(".chip")).toHaveCount(0);

  // The new terms are in the taxonomy editor.
  await page.goto(`/taxonomies/${open}`);
  await expect(page.getByLabel("Name of Brand new")).toHaveValue("Brand new");
  await expect(page.getByLabel("Name of Plant/Solar")).toHaveValue("Solar");
  await expect(page.getByLabel("Let fields add new terms")).toBeChecked();
});
