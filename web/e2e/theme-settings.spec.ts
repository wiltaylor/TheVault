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

test("the sidebar holds only Home and Types, with trash and settings beside the theme switch", async ({ page }) => {
  await logIn(page);
  const main = page.getByRole("navigation", { name: "Main" });
  await expect(main.getByRole("link")).toHaveText(["Home", "Types"]);

  const trash = page.locator(".sidebar-footer").getByRole("link", { name: "Trash" });
  await trash.click();
  await expect(page).toHaveURL(/\/trash$/);
  await expect(page.getByRole("heading", { name: "Trash" })).toBeVisible();
  await expect(trash).toHaveClass(/active/);
});

test("taxonomies, scripts, tasks, secrets and API tokens live under settings", async ({ page }) => {
  await logIn(page);
  const main = page.getByRole("navigation", { name: "Main" });
  for (const moved of ["Taxonomies", "Scripts", "Tasks", "Secrets", "API tokens"]) {
    await expect(main.getByRole("link", { name: moved, exact: true })).toHaveCount(0);
  }

  await page.getByRole("link", { name: "Settings" }).click();
  await expect(page).toHaveURL(/\/settings$/);
  const tabs = page.getByRole("navigation", { name: "Settings sections" });
  await expect(page.getByRole("heading", { name: "General settings" })).toBeVisible();

  for (const [tab, heading, path] of [
    ["Taxonomies", "Taxonomies", "/settings/taxonomies"],
    ["Scripts", "Scripts", "/settings/scripts"],
    ["Tasks", "Tasks", "/settings/tasks"],
    ["Secrets", "Secrets", "/settings/secrets"],
    ["API tokens", "API tokens", "/settings/tokens"],
  ] as const) {
    await tabs.getByRole("link", { name: tab }).click();
    await expect(page).toHaveURL(new RegExp(`${path}$`));
    await expect(page.getByRole("heading", { name: heading, exact: true })).toBeVisible();
  }
});

test("old admin addresses go to their place under settings", async ({ page }) => {
  await logIn(page, "/tokens");
  await expect(page).toHaveURL(/\/settings\/tokens$/);
  await expect(page.getByRole("heading", { name: "API tokens" })).toBeVisible();
  await page.goto("/scripts");
  await expect(page).toHaveURL(/\/settings\/scripts$/);
  await page.goto("/taxonomies");
  await expect(page).toHaveURL(/\/settings\/taxonomies$/);
});

test("the types button opens a gallery with a card for every type", async ({ page, admin }) => {
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

  await logIn(page);
  await page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: "Types" }).click();
  await expect(page).toHaveURL(/\/types$/);
  await expect(page.getByRole("heading", { name: "Types", exact: true })).toBeVisible();

  const gallery = page.getByRole("list", { name: "Types" });
  const types = ((await (await admin.get("/api/types")).json()) as { types: unknown[] }).types;
  // One card per type, plus the assets card and the card that creates a new type. Other tests may
  // add types meanwhile.
  expect(await gallery.getByRole("listitem").count()).toBeGreaterThanOrEqual(types.length + 2);
  await expect(gallery.getByRole("listitem").filter({ hasText: "Tasks" }).first()).toBeVisible();

  const card = gallery.getByRole("listitem").filter({ hasText: plural });
  await expect(card).toContainText("Shown as a card.");
  await expect(card.getByTestId("type-count")).toHaveText("2 notes");
  await expect(card).toContainText(/\d+ fields?/);

  await expect(gallery.getByRole("link", { name: /New type/ })).toHaveAttribute("href", "/types/new");
  await card.getByRole("link", { name: new RegExp(`^${plural}`) }).click();
  await expect(page).toHaveURL(new RegExp(`/types/${name}$`));
  await expect(page.getByRole("heading", { name: plural })).toBeVisible();
});

test("assets open from their card in the types gallery", async ({ page, admin }) => {
  const assets = ((await (await admin.get("/api/assets")).json()) as { assets: unknown[] }).assets;
  await logIn(page, "/types");
  const card = page.getByRole("list", { name: "Types" }).getByRole("listitem").filter({ hasText: "Images and files" });
  await expect(card.getByTestId("asset-count")).toHaveText(/^\d+ files?$/);
  const shown = Number((await card.getByTestId("asset-count").textContent())!.split(" ")[0]);
  expect(shown).toBeGreaterThanOrEqual(assets.length);
  await card.getByRole("link", { name: /^Assets/ }).click();
  await expect(page).toHaveURL(/\/assets$/);
  await expect(page.getByRole("heading", { name: "Assets" })).toBeVisible();
});
