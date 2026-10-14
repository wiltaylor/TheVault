import type { APIRequestContext } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

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

const TWO_FIELDS = (icon: string) =>
  `label = "Gadget"\nicon = "${icon}"\n\nfield size {\n  kind = "int"\n  label = "Size"\n}\n\nfield shade {\n  kind = "string"\n}\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n  }\n}\n`;

test("the icon is chosen from a searchable grid of icons", async ({ page, admin }) => {
  const name = `iconic_${token()}`;
  await putType(admin, name, TWO_FIELDS("file-text"));

  await logIn(page, `/settings/types/${name}/edit`);
  const button = page.getByRole("button", { name: "Icon", exact: true });
  await expect(button).toHaveText("file-text");
  await expect(button).toHaveAttribute("aria-expanded", "false");
  await button.click();

  const picker = page.getByRole("dialog", { name: "Choose an icon" });
  await expect(picker.getByRole("option", { name: "file-text" })).toHaveAttribute("aria-selected", "true");
  await picker.getByLabel("Search icons").fill("calendar days");
  await expect(picker.getByRole("option")).not.toHaveCount(0);
  await picker.getByRole("option", { name: "calendar-days", exact: true }).click();
  await expect(picker).toHaveCount(0);
  await expect(button).toHaveText("calendar-days");

  // Escape closes the picker without changing the icon.
  await button.click();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "Choose an icon" })).toHaveCount(0);
  await expect(button).toHaveText("calendar-days");

  await page.getByRole("button", { name: "Preview changes" }).click();
  await page.getByRole("button", { name: "Apply changes" }).click();
  await expect.poll(() => typeFile(admin, name)).toMatch(/^icon\s+= "calendar-days"$/m);
});

test("no icon falls back to the default icon", async ({ page, admin }) => {
  const name = `plain_${token()}`;
  await putType(admin, name, TWO_FIELDS("folder"));
  await logIn(page, `/settings/types/${name}/edit`);
  const button = page.getByRole("button", { name: "Icon", exact: true });
  await expect(button).toHaveText("folder");
  await button.click();
  await page.getByRole("dialog", { name: "Choose an icon" }).getByRole("button", { name: "No icon" }).click();
  await expect(button).toHaveText("Default icon");
  await page.getByRole("button", { name: "Preview changes" }).click();
  await page.getByRole("button", { name: "Apply changes" }).click();
  await expect.poll(() => typeFile(admin, name)).not.toMatch(/^icon\s*=/m);
});

test("fields are a grid with one row per field and settings that open underneath", async ({ page, admin }) => {
  const name = `grid_${token()}`;
  await putType(admin, name, TWO_FIELDS("box"));
  await logIn(page, `/settings/types/${name}/edit`);

  const grid = page.getByRole("table", { name: "Fields" });
  await expect(grid.getByRole("columnheader")).toHaveText(["Order", "Name", "Kind", "Label", "Required", "Computed", "Several", "Actions"]);
  const size = page.getByTestId("field-row-0");
  await expect(size.getByLabel("Name", { exact: true })).toHaveValue("size");
  await expect(size.getByLabel("Kind", { exact: true })).toHaveValue("int");
  await expect(size.getByLabel("Label", { exact: true })).toHaveValue("Size");
  await expect(page.getByTestId("field-row-1").getByLabel("Name", { exact: true })).toHaveValue("shade");

  // Saved fields start with their settings closed; the settings button opens them under the row.
  await expect(size.getByLabel("Description")).toHaveCount(0);
  await size.getByRole("button", { name: "Settings for size" }).click();
  await expect(size.getByRole("button", { name: "Settings for size" })).toHaveAttribute("aria-expanded", "true");
  await size.getByLabel("Description").fill("How big it is");

  // A new field starts open, so its kind's settings are there to fill in.
  await page.getByRole("button", { name: "Add field" }).click();
  const added = page.getByTestId("field-row-2");
  await expect(added.getByLabel("Description")).toBeVisible();
  await added.getByLabel("Name", { exact: true }).fill("tone");
  await added.getByLabel("Kind", { exact: true }).selectOption("select");
  await added.getByLabel("Options, one per line").fill("warm\ncool");

  // The arrows reorder rows.
  await page.getByRole("button", { name: "Move shade up" }).click();
  await expect(page.getByTestId("field-row-0").getByLabel("Name", { exact: true })).toHaveValue("shade");

  await page.getByRole("button", { name: "Remove tone" }).click();
  await expect(page.getByTestId("field-row-2")).toHaveCount(0);
});

test("templates are Default and Card only, and a card template can have tabs", async ({ page, admin }) => {
  const name = `cardy_${token()}`;
  await putType(admin, name, TWO_FIELDS("box"));
  await logIn(page, `/settings/types/${name}/templates`);

  const picker = page.getByRole("group", { name: "Templates" });
  await expect(picker.getByRole("button")).toHaveText(["Default", "Card"]);
  await expect(page.getByLabel("New template name")).toHaveCount(0);

  // The card template starts from the first tab of Default, and takes more tabs.
  await picker.getByRole("button", { name: "Card" }).click();
  await expect(picker.getByRole("button", { name: "Card" })).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "Add tab" }).click();
  await page.getByRole("button", { name: "Save template" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved the card template.");

  const card = await admin.get(`/api/types/${name}/templates/card`, { headers: { Accept: "text/plain" } });
  expect(card.status()).toBe(200);
  expect(((await card.text()).match(/^\s*tab "/gm) ?? []).length).toBe(2);

  // The server refuses any other template name.
  const other = await admin.put(`/api/types/${name}/templates/detail`, {
    data: { wcl: 'tab "Main" {\n  slot "title"\n}\n' },
  });
  expect(other.status()).toBe(400);
});
