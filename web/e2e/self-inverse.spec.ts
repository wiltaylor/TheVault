import type { APIRequestContext, Locator } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// A two-way list of references that is its own inverse: built in the type editor, then listing
// one note on another lists it back, pair by pair.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function createNote(admin: APIRequestContext, type: string, fields: Record<string, unknown>) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string };
}

async function siblingsOf(admin: APIRequestContext, id: string): Promise<unknown> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return ((await res.json()) as { note: { fields: Record<string, unknown> } }).note.fields.siblings;
}

async function pick(scope: Locator, label: string, typed: string, option: string) {
  const plus = scope.getByRole("button", { name: `Add to ${label}` });
  const box = scope.getByRole("combobox", { name: `Add to ${label}` });
  if (!(await box.isVisible())) await plus.click();
  await box.fill(typed);
  await scope.getByRole("option", { name: option, exact: true }).click();
}

test("a list of references that is its own inverse lists a note back on the note it lists", async ({ page, admin }) => {
  const t = token();
  const kin = `kin_${t}`;

  await logIn(page, "/settings/types/new");
  await page.getByLabel("Name", { exact: true }).fill(kin);
  await page.getByLabel("Label", { exact: true }).fill("Kin");
  await page.getByRole("button", { name: "Add field" }).click();
  const row = page.getByTestId("field-row-0");
  await row.getByLabel("Name", { exact: true }).fill("siblings");
  await row.getByLabel("Label", { exact: true }).fill("Siblings");
  await row.getByLabel("Kind", { exact: true }).selectOption("list");
  await row.getByLabel("Item kind").selectOption("reference");
  const item = row.getByTestId("item-settings");
  await item.getByLabel("Target type").selectOption(kin);
  await item.getByLabel("Relation").selectOption("two_way");

  // The picker offers the field itself, and choosing it says what it does.
  await expect(item.locator("datalist option").first()).toHaveAttribute("label", "siblings (this field: links both ways)");
  await expect(item.locator("datalist option").first()).toHaveAttribute("value", "siblings");
  await item.getByLabel("Inverse field").fill("siblings");
  await expect(item).toContainText("This field is its own other side");
  await expect(item).not.toContainText("it gets one on save");
  await expect(page.getByRole("alert", { name: "Problems" })).toHaveCount(0);

  await page.getByRole("button", { name: "Create type" }).click();
  await expect(page).toHaveURL(new RegExp(`/types/${kin}$`));

  // No second field was implied for the other side.
  const types = (await (await admin.get("/api/types")).json()) as { types: { name: string; fields: Record<string, unknown>[] }[] };
  const fields = types.types.find((x) => x.name === kin)!.fields;
  expect(fields.filter((f) => f.name === "siblings")).toHaveLength(1);
  expect(fields.find((f) => f.name === "siblings")).toMatchObject({ relation: "two_way", inverse: "siblings", multiple: true });
  expect(fields.some((f) => f.implied)).toBe(false);

  const ann = await createNote(admin, kin, { title: `Ann ${t}` });
  const bob = await createNote(admin, kin, { title: `Bob ${t}` });
  const cy = await createNote(admin, kin, { title: `Cy ${t}` });

  // Ann lists Bob and Cy on her page.
  await page.goto(`/notes/${ann.id}`);
  const siblings = page.getByTestId("field-siblings");
  await pick(siblings, "Siblings", "Bob", `Bob ${t}`);
  await expect.poll(() => siblingsOf(admin, bob.id)).toEqual([ann.id]);
  await pick(siblings, "Siblings", "Cy", `Cy ${t}`);
  await expect.poll(() => siblingsOf(admin, cy.id)).toEqual([ann.id]);
  // Pairwise: Bob and Cy are not listed on each other.
  expect(await siblingsOf(admin, bob.id)).toEqual([ann.id]);

  // Bob's page lists Ann back.
  await page.goto(`/notes/${bob.id}`);
  await expect(page.getByTestId("field-siblings")).toContainText(`Ann ${t}`);
  await expect(page.getByTestId("field-siblings")).not.toContainText(`Cy ${t}`);
});
