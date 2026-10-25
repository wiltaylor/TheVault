import type { APIRequestContext, APIResponse } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// A taxonomy's terms carry palette colours, set in the taxonomy editor. A term without one takes
// its nearest coloured ancestor's, and a note's taxonomy chips show that colour.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

async function ok(res: Promise<APIResponse>, want: number) {
  const r = await res;
  expect(r.status(), await r.text()).toBe(want);
  return r;
}

/** A three-level taxonomy with no colours, a type with a multi-term field over it, and a note holding two terms. */
async function setup(admin: APIRequestContext) {
  const t = token();
  const tax = `area_${t}`;
  await ok(
    admin.put(`/api/taxonomies/${tax}`, {
      data: { wcl: `label = "Areas"\n\nterm "Plant" {\n  term "Electrical" {\n    term "Wiring"\n  }\n}\n\nterm "Office"\n` },
    }),
    200,
  );
  const type = `coloured_${t}`;
  const wcl =
    `label = "Coloured"\n\nfield topics {\n  kind = "list"\n  items = "taxonomy"\n  taxonomy = "${tax}"\n  label = "Topics"\n}\n\n` +
    `template default {\n  tab "Main" {\n    slot "title"\n    slot "topics"\n  }\n}\n`;
  await ok(admin.put(`/api/types/${type}`, { data: { wcl } }), 200);
  const res = await ok(
    admin.post(`/api/notes?type=${type}`, {
      data: { note: { fields: { title: `Coloured ${t}`, topics: ["Plant/Electrical/Wiring", "Office"] }, blocks: [] } },
    }),
    201,
  );
  const note = ((await res.json()) as { id: string }).id;
  return { tax, note };
}

test("a term's colour set in the taxonomy editor colours its chips and its children's", async ({ page, admin }) => {
  const s = await setup(admin);
  await logIn(page, `/taxonomies/${s.tax}`);

  // Plant gets green from the palette; its children show they inherit it.
  await page.getByRole("button", { name: "Colour of Plant", exact: true }).click();
  const palette = page.getByRole("group", { name: "Colours for Plant" });
  await expect(palette.getByRole("button", { name: "No colour" })).toHaveAttribute("aria-pressed", "true");
  await palette.getByRole("button", { name: "Green" }).click();
  await expect(palette).toBeHidden();
  await expect(page.getByRole("button", { name: "Colour of Plant", exact: true })).toHaveAttribute("data-colour", "green");
  const electrical = page.getByTestId("term-Plant/Electrical");
  await expect(electrical.getByRole("button", { name: "Colour of Plant/Electrical", exact: true })).toHaveAttribute("data-inherited", "");
  await expect(electrical.locator(".term-inherits").first()).toHaveText("inherits green");

  // Office gets gray, which the hashed chip colours never use.
  await page.getByRole("button", { name: "Colour of Office", exact: true }).click();
  await page.getByRole("group", { name: "Colours for Office" }).getByRole("button", { name: "Gray" }).click();
  await page.getByRole("button", { name: "Save taxonomy" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved.");

  // The file holds only the colours set; Electrical and Wiring have none of their own.
  const file = ((await (await admin.get(`/api/taxonomies/${s.tax}`)).json()) as {
    terms: { colour: string | null; children: { colour: string | null; children: { colour: string | null }[] }[] }[];
  }).terms;
  expect(file.map((t) => t.colour)).toEqual(["green", "gray"]);
  expect(file[0]!.children[0]!.colour).toBeNull();
  expect(file[0]!.children[0]!.children[0]!.colour).toBeNull();

  // The note's chips: Wiring inherits green from Plant, two levels up; Office is gray.
  await page.goto(`/notes/${s.note}`);
  const topics = page.getByTestId("field-topics");
  await expect(topics.locator(".chip")).toHaveText(["Plant/Electrical/Wiring×", "Office×"]);
  await expect(topics.locator('.chip[data-value="Plant/Electrical/Wiring"]')).toHaveAttribute("data-colour", "green");
  await expect(topics.locator('.chip[data-value="Office"]')).toHaveAttribute("data-colour", "gray");
  const [chip, gray] = await topics.locator('.chip[data-value="Office"]').evaluate((el) => [
    getComputedStyle(el).getPropertyValue("--chip-colour").trim(),
    getComputedStyle(document.documentElement).getPropertyValue("--palette-gray").trim(),
  ]);
  expect(chip).toBe(gray);

  // The suggestion list shows each term's colour as a dot.
  await topics.getByRole("button", { name: "Add to Topics" }).click();
  await topics.getByRole("combobox", { name: "Add to Topics" }).fill("elec");
  await expect(topics.getByRole("option", { name: /^Electrical/ }).locator(".combo-dot")).toHaveAttribute("data-colour", "green");

  // Wiring set to red in the editor overrides what it inherits.
  await page.goto(`/taxonomies/${s.tax}`);
  await page.getByRole("button", { name: "Colour of Plant/Electrical/Wiring", exact: true }).click();
  await page.getByRole("group", { name: "Colours for Plant/Electrical/Wiring" }).getByRole("button", { name: "Red" }).click();
  await page.getByRole("button", { name: "Save taxonomy" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved.");
  await page.goto(`/notes/${s.note}`);
  await expect(page.getByTestId("field-topics").locator('.chip[data-value="Plant/Electrical/Wiring"]')).toHaveAttribute("data-colour", "red");
});
