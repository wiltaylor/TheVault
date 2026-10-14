import { expect, logIn, test } from "./fixtures";

test("the dark mode switch in the sidebar changes the theme and is remembered", async ({ page }) => {
  await logIn(page);
  const html = page.locator("html");
  const toggle = page.getByRole("switch", { name: "Dark mode" });
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await expect(html).toHaveAttribute("data-theme", "light");
  const lightBg = await page.evaluate(() => getComputedStyle(document.body.parentElement!).backgroundColor);

  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await expect(html).toHaveAttribute("data-theme", "dark");
  const darkBg = await page.evaluate(() => getComputedStyle(document.body.parentElement!).backgroundColor);
  expect(darkBg).not.toEqual(lightBg);

  await page.reload();
  await expect(html).toHaveAttribute("data-theme", "dark");
  await expect(page.getByRole("switch", { name: "Dark mode" })).toHaveAttribute("aria-checked", "true");

  await page.getByRole("switch", { name: "Dark mode" }).click();
  await expect(html).toHaveAttribute("data-theme", "light");
});

test.describe("with a dark system preference", () => {
  test.use({ colorScheme: "dark" });

  test("a browser with no saved choice starts dark", async ({ page }) => {
    await logIn(page);
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  });
});

test("the sidebar holds only Daily notes and Search, with trash and settings beside the theme switch", async ({ page }) => {
  await logIn(page);
  const main = page.getByRole("navigation", { name: "Main" });
  await expect(main.getByRole("link")).toHaveText(["Daily notes", "Search"]);

  const trash = page.locator(".sidebar-footer").getByRole("link", { name: "Trash" });
  await trash.click();
  await expect(page).toHaveURL(/\/trash$/);
  await expect(page.getByRole("heading", { name: "Trash" })).toBeVisible();
  await expect(trash).toHaveClass(/active/);
});

test("types, taxonomies, tasks, secrets and API tokens live under settings, and scripts are not a section", async ({ page }) => {
  await logIn(page);
  const main = page.getByRole("navigation", { name: "Main" });
  for (const moved of ["Types", "Taxonomies", "Tasks", "Secrets", "API tokens"]) {
    await expect(main.getByRole("link", { name: moved, exact: true })).toHaveCount(0);
  }

  await page.getByRole("link", { name: "Settings" }).click();
  await expect(page).toHaveURL(/\/settings$/);
  const tabs = page.getByRole("navigation", { name: "Settings sections" });
  await expect(page.getByRole("heading", { name: "General settings" })).toBeVisible();

  for (const [tab, heading, path] of [
    ["Types", "Types", "/settings/types"],
    ["Taxonomies", "Taxonomies", "/settings/taxonomies"],
    ["Tasks", "Tasks", "/settings/tasks"],
    ["Secrets", "Secrets", "/settings/secrets"],
    ["API tokens", "API tokens", "/settings/tokens"],
  ] as const) {
    await tabs.getByRole("link", { name: tab }).click();
    await expect(page).toHaveURL(new RegExp(`${path}$`));
    await expect(page.getByRole("heading", { name: heading, exact: true })).toBeVisible();
  }
  await expect(tabs.getByRole("link")).toHaveText(["General", "Types", "Taxonomies", "Tasks", "Secrets", "API tokens"]);
});

test("old admin addresses go to their place under settings", async ({ page }) => {
  await logIn(page, "/tokens");
  await expect(page).toHaveURL(/\/settings\/tokens$/);
  await expect(page.getByRole("heading", { name: "API tokens" })).toBeVisible();
  await page.goto("/scripts");
  await expect(page).toHaveURL(/\/settings\/types$/);
  await page.goto("/taxonomies");
  await expect(page).toHaveURL(/\/settings\/taxonomies$/);
});

test("the Types tab in settings is a gallery with a card for every type", async ({ page, admin }) => {
  // A type of this test's own, so notes other tests create in parallel cannot change its count.
  const name = `gallery_${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  const plural = `Gallery ${name.slice(8)}`;
  const wcl = `label = "${name}"\nplural = "${plural}"\ndescription = "Shown as a card."\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n  }\n}\n`;
  const put = await admin.put(`/api/types/${name}`, { data: { wcl } });
  expect(put.status(), await put.text()).toBe(200);
  for (const title of ["One", "Two"]) {
    const res = await admin.post(`/api/notes?type=${name}`, { data: { note: { fields: { title }, blocks: [] } } });
    expect(res.status(), await res.text()).toBe(201);
  }

  // Read the types before the page does, so types other tests add meanwhile only add cards.
  const types = ((await (await admin.get("/api/types")).json()) as { types: unknown[] }).types;
  await logIn(page, "/settings");
  await page.getByRole("navigation", { name: "Settings sections" }).getByRole("link", { name: "Types" }).click();
  await expect(page).toHaveURL(/\/settings\/types$/);
  await expect(page.getByRole("heading", { name: "Types", exact: true })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Settings sections" }).getByRole("link", { name: "Types" })).toHaveClass(/active/);

  const gallery = page.getByRole("list", { name: "Types" });
  await expect(gallery.getByRole("listitem").filter({ hasText: plural })).toBeVisible();
  // One card per type, plus the assets card and the card that creates a new type. Other tests may
  // add types meanwhile.
  expect(await gallery.getByRole("listitem").count()).toBeGreaterThanOrEqual(types.length + 2);
  await expect(gallery.getByRole("listitem").filter({ hasText: "Tasks" }).first()).toBeVisible();

  const card = gallery.getByRole("listitem").filter({ hasText: plural });
  await expect(card).toContainText("Shown as a card.");
  await expect(card.getByTestId("type-count")).toHaveText("2 notes");
  await expect(card).toContainText(/\d+ fields?/);

  await expect(gallery.getByRole("link", { name: /New type/ })).toHaveAttribute("href", "/settings/types/new");
  await expect(card.getByRole("link", { name: "Edit type" })).toHaveAttribute("href", `/settings/types/${name}/edit`);
  // The card's main area opens the type's note list, outside settings.
  await card.getByRole("link", { name: new RegExp(`^${plural}`) }).click();
  await expect(page).toHaveURL(new RegExp(`/types/${name}$`));
  await expect(page.getByRole("heading", { name: plural })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Settings sections" })).toHaveCount(0);
});

test("the type and template editors open under the settings tabs, and their old addresses redirect", async ({ page, admin }) => {
  const name = `under_${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  const wcl = `label = "${name}"\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n  }\n}\n`;
  const put = await admin.put(`/api/types/${name}`, { data: { wcl } });
  expect(put.status(), await put.text()).toBe(200);

  await logIn(page, "/settings/types");
  const tabs = page.getByRole("navigation", { name: "Settings sections" });
  const card = page.getByRole("list", { name: "Types" }).getByRole("listitem").filter({ hasText: name });
  await card.getByRole("link", { name: "Edit type" }).click();
  await expect(page).toHaveURL(new RegExp(`/settings/types/${name}/edit$`));
  await expect(tabs.getByRole("link", { name: "Types" })).toHaveClass(/active/);
  await expect(page.getByRole("link", { name: "Templates" })).toHaveAttribute("href", `/settings/types/${name}/templates`);
  await page.getByRole("link", { name: "Templates" }).click();
  await expect(page).toHaveURL(new RegExp(`/settings/types/${name}/templates$`));
  await expect(tabs).toBeVisible();
  await expect(page.getByRole("link", { name: "Edit fields" })).toHaveAttribute("href", `/settings/types/${name}/edit`);

  await page.goto(`/types/${name}/edit?from=old`);
  await expect(page).toHaveURL(new RegExp(`/settings/types/${name}/edit\\?from=old$`));
  await expect(tabs).toBeVisible();
  await page.goto(`/types/${name}/templates`);
  await expect(page).toHaveURL(new RegExp(`/settings/types/${name}/templates$`));
  await page.goto("/types/new");
  await expect(page).toHaveURL(/\/settings\/types\/new$/);
  await expect(tabs.getByRole("link", { name: "Types" })).toHaveClass(/active/);
  await page.goto("/types");
  await expect(page).toHaveURL(/\/settings\/types$/);
  await expect(page.getByRole("list", { name: "Types" })).toBeVisible();
  // The note list keeps its address, outside settings.
  await page.goto(`/types/${name}`);
  await expect(page).toHaveURL(new RegExp(`/types/${name}$`));
  await expect(tabs).toHaveCount(0);
  await expect(page.getByRole("link", { name: "Edit type" })).toHaveAttribute("href", `/settings/types/${name}/edit`);
});

test("assets open from their card in the types gallery", async ({ page, admin }) => {
  const assets = ((await (await admin.get("/api/assets")).json()) as { assets: unknown[] }).assets;
  await logIn(page, "/settings/types");
  const card = page.getByRole("list", { name: "Types" }).getByRole("listitem").filter({ hasText: "Images and files" });
  await expect(card.getByTestId("asset-count")).toHaveText(/^\d+ files?$/);
  const shown = Number((await card.getByTestId("asset-count").textContent())!.split(" ")[0]);
  expect(shown).toBeGreaterThanOrEqual(assets.length);
  await card.getByRole("link", { name: /^Assets/ }).click();
  await expect(page).toHaveURL(/\/assets$/);
  await expect(page.getByRole("heading", { name: "Assets" })).toBeVisible();
});
