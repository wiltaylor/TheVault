import type { APIRequestContext, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function putType(admin: APIRequestContext, name: string, plural: string) {
  const wcl = `label = "${name}"\nplural = "${plural}"\nicon = "rocket"\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n  }\n}\n`;
  const res = await admin.put(`/api/types/${name}`, { data: { wcl } });
  expect(res.status(), await res.text()).toBe(200);
}

function typesNav(page: Page) {
  return page.getByRole("navigation", { name: "Types" });
}

test("the sidebar's Types section starts collapsed, lists every type alphabetically and remembers being open", async ({
  page,
  admin,
}) => {
  const t = token();
  // Lower and upper case, so the order is by label ignoring case.
  const late = { name: `sbz_${t}`, plural: `zz sidebar ${t}` };
  const early = { name: `sba_${t}`, plural: `AA Sidebar ${t}` };
  await putType(admin, late.name, late.plural);
  await putType(admin, early.name, early.plural);

  await logIn(page, "/daily");
  const toggle = typesNav(page).getByRole("button", { name: "Types" });
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(typesNav(page).getByRole("link")).toHaveCount(0);

  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  const links = typesNav(page).getByRole("link");
  await expect(links.filter({ hasText: early.plural })).toBeVisible();
  await expect(links.filter({ hasText: late.plural })).toBeVisible();
  await expect(links.filter({ hasText: early.plural }).locator("svg.icon")).toHaveCount(1);

  const labels = (await links.allTextContents()).map((s) => s.trim());
  const sorted = [...labels].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));
  expect(labels).toEqual(sorted);
  expect(labels.indexOf(early.plural)).toBeLessThan(labels.indexOf(late.plural));
  expect(labels).toContain("Tasks");

  // A click opens the type's note list in the active pane, and the entry shows as active.
  await links.filter({ hasText: late.plural }).click();
  await expect(page).toHaveURL(new RegExp(`/types/${late.name}$`));
  await expect(activePane(page).getByRole("heading", { name: late.plural })).toBeVisible();
  await expect(links.filter({ hasText: late.plural })).toHaveClass(/active/);
  await expect(links.filter({ hasText: early.plural })).not.toHaveClass(/active/);

  // Open survives a reload, and so does closed.
  await page.reload();
  await expect(typesNav(page).getByRole("button", { name: "Types" })).toHaveAttribute("aria-expanded", "true");
  await expect(typesNav(page).getByRole("link").filter({ hasText: early.plural })).toBeVisible();
  await typesNav(page).getByRole("button", { name: "Types" }).click();
  await expect(typesNav(page).getByRole("link")).toHaveCount(0);
  await page.reload();
  await expect(typesNav(page).getByRole("button", { name: "Types" })).toHaveAttribute("aria-expanded", "false");
  await expect(typesNav(page).getByRole("link")).toHaveCount(0);
});

test("the sidebar's Types list follows a type deleted in the editor", async ({ page, admin }) => {
  const t = token();
  const gone = { name: `sbgone_${t}`, plural: `Sidebar gone ${t}` };
  await putType(admin, gone.name, gone.plural);

  await logIn(page, `/settings/types/${gone.name}/edit`);
  await typesNav(page).getByRole("button", { name: "Types" }).click();
  await expect(typesNav(page).getByRole("link").filter({ hasText: gone.plural })).toBeVisible();

  page.once("dialog", (d) => void d.accept());
  await page.getByRole("button", { name: "Delete type" }).click();
  await expect(page).toHaveURL(/\/settings\/types$/);
  await expect(typesNav(page).getByRole("link").filter({ hasText: gone.plural })).toHaveCount(0);
});

test("on a phone the Types section works in the drawer, and a click closes it", async ({ page, admin }) => {
  const t = token();
  const phone = { name: `sbphone_${t}`, plural: `Sidebar phone ${t}` };
  await putType(admin, phone.name, phone.plural);
  await page.setViewportSize({ width: 390, height: 760 });
  await logIn(page, "/daily");

  await page.getByRole("button", { name: "Menu" }).click();
  await typesNav(page).getByRole("button", { name: "Types" }).click();
  const link = typesNav(page).getByRole("link").filter({ hasText: phone.plural });
  await link.scrollIntoViewIfNeeded();
  await link.click();
  await expect(page).toHaveURL(new RegExp(`/types/${phone.name}$`));
  await expect(page.getByRole("button", { name: "Menu" })).toHaveAttribute("aria-expanded", "false");
});

test("uploaded files are listed among the types as Images and files, opening the assets page", async ({ page }) => {
  await logIn(page, "/daily");
  const toggle = typesNav(page).getByRole("button", { name: "Types" });
  if ((await toggle.getAttribute("aria-expanded")) !== "true") await toggle.click();
  const entry = typesNav(page).getByRole("link", { name: "Images and files" });
  await expect(entry).toBeVisible();
  await expect(entry.locator("svg.icon")).toHaveCount(1);
  await entry.click();
  await expect(page).toHaveURL(/\/assets$/);
  await expect(entry).toHaveClass(/active/);
});
