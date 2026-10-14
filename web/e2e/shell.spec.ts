import { type APIRequestContext, devices } from "@playwright/test";
import { activePane, expect, logIn, password, test } from "./fixtures";
import { BULK_NOTES, bulkTitle } from "./vault";

/** Create a task through the API, as an agent would, and return its id and version. */
async function createTask(admin: APIRequestContext, fields: Record<string, unknown>, body?: string) {
  const blocks = body
    ? [{ kind: "content", labels: ["body"], blocks: [{ kind: "paragraph", fields: { text: body } }] }]
    : [];
  const res = await admin.post("/api/notes?type=task", { data: { note: { fields, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string; version: string; note: { fields: Record<string, unknown> } };
}

function unique(prefix: string): string {
  return `${prefix} ${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

test.describe("login", () => {
  test("the right password reaches the app", async ({ page }) => {
    await page.goto("/");
    await expect(page).toHaveURL(/\/login/);
    await page.getByLabel("Password").fill(password());
    await page.getByRole("button", { name: "Log in" }).click();
    await expect(page.getByRole("navigation", { name: "Day" })).toBeVisible();
    await expect(page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: "Daily notes" })).toBeVisible();
    await expect(page).not.toHaveURL(/\/login/);
  });

  test("the wrong password shows an error and stays on the login page", async ({ page }) => {
    await page.goto("/login");
    await page.getByLabel("Password").fill("not the password");
    await page.getByRole("button", { name: "Log in" }).click();
    await expect(page.getByRole("alert")).toHaveText("That password is not right.");
    await expect(page).toHaveURL(/\/login/);
    await expect(page.getByRole("navigation", { name: "Main" })).toHaveCount(0);
  });

  test("logging out returns to the login page", async ({ page }) => {
    await logIn(page);
    // Log out lives in Settings > General.
    await page.getByRole("link", { name: "Settings" }).click();
    await expect(page.getByRole("heading", { name: "General settings" })).toBeVisible();
    await page.getByRole("button", { name: "Log out" }).click();
    await expect(page).toHaveURL(/\/login/);
    await page.goto("/types/task");
    await expect(page).toHaveURL(/\/login\?next=%2Ftypes%2Ftask/);
  });
});

test("a note created through the API appears in its type list and opens with its fields on the right tabs", async ({
  page,
  admin,
}) => {
  // Sorts ahead of the letters, so the row is in the first screen of the virtualised list
  // however many tasks other tests have made.
  const title = unique("0 Tabbed");
  const created = await createTask(admin, { title, status: "doing", estimate: 7 }, "Body text here");

  await logIn(page);
  await page.getByRole("link", { name: "Settings" }).click();
  await page.getByRole("navigation", { name: "Settings sections" }).getByRole("link", { name: "Types" }).click();
  // The card itself opens the type's editor; its Notes link opens the note list.
  const card = page.getByRole("list", { name: "Types" }).getByRole("listitem").filter({ has: page.getByRole("link", { name: /^Tasks/ }) });
  await card.getByRole("link", { name: "Notes", exact: true }).click();
  await expect(page).toHaveURL(/\/types\/task$/);
  await expect(page.getByRole("heading", { name: "Tasks" })).toBeVisible();
  const row = page.getByTestId("note-card").filter({ hasText: title });
  await expect(row).toBeVisible();
  await row.click();

  await expect(page).toHaveURL(new RegExp(`/notes/${created.id}$`));
  await expect(page.getByRole("heading", { level: 1, name: title })).toBeVisible();
  // Scoped to the pane: the workspace's own tab bar holds tabs too.
  const tabs = activePane(page).getByRole("tab");
  await expect(tabs).toHaveText(["Overview", "Details"]);
  await expect(page.getByRole("tab", { name: "Overview" })).toHaveAttribute("aria-selected", "true");

  const panel = page.getByRole("tabpanel");
  // The title is the page's heading, edited in place, not a field below it.
  await expect(page.getByLabel("Title", { exact: true })).toHaveText(title);
  await expect(panel.getByTestId("field-title")).toHaveCount(0);
  await expect(panel.getByTestId("field-status")).toContainText("Status");
  await expect(panel.getByTestId("field-status").getByLabel("Status")).toHaveValue("doing");
  await expect(panel.getByTestId("field-body")).toContainText("Body text here");
  await expect(panel.getByTestId("field-estimate")).toHaveCount(0);

  await page.getByRole("tab", { name: "Details" }).click();
  await expect(page.getByRole("tab", { name: "Details" })).toHaveAttribute("aria-selected", "true");
  await expect(panel.getByTestId("field-estimate")).toContainText("Hours");
  await expect(panel.getByTestId("field-estimate").getByLabel("Hours")).toHaveValue("7");
  await expect(panel.getByTestId("field-status")).toHaveCount(0);
  await expect(panel.getByTestId("field-title")).toHaveCount(0);
});

test("search finds a note and opens it", async ({ page, admin }) => {
  const word = `zebra${Date.now().toString(36)}`;
  const created = await createTask(admin, { title: unique("Searchable") }, `A note about the ${word}.`);
  // The search page has its own box.
  await logIn(page, "/search");
  await page.getByRole("searchbox", { name: "Search notes" }).fill(word);
  await page.getByRole("searchbox", { name: "Search notes" }).press("Enter");
  await expect(page).toHaveURL(/\/search\?q=/);
  const rows = page.getByTestId("note-row");
  await expect(rows).toHaveCount(1);
  await rows.first().click();
  await expect(page).toHaveURL(new RegExp(`/notes/${created.id}$`));
});

test("a concurrent API write then a save from the UI shows the conflict view", async ({ page, admin }) => {
  const title = unique("Contested");
  const created = await createTask(admin, { title, status: "todo", estimate: 1 });

  await logIn(page, `/notes/${created.id}`);
  await expect(page.getByRole("heading", { level: 1, name: title })).toBeVisible();

  // An agent saves the note while the page has it open.
  const theirs = await admin.put(`/api/notes/${created.id}`, {
    data: {
      base_version: created.version,
      note: { fields: { ...created.note.fields, title: `${title} (theirs)`, estimate: 5 }, blocks: [] },
    },
  });
  expect(theirs.status(), await theirs.text()).toBe(200);

  // The page saves its edits over the version it opened.
  await page.getByLabel("Title").fill(`${title} (mine)`);
  await page.getByLabel("Status").selectOption("done");
  const conflict = page.getByTestId("conflict-view");
  await expect(conflict).toBeVisible();
  await expect(conflict.getByRole("heading", { name: "This note changed while you were editing" })).toBeVisible();
  const titleRow = conflict.locator("tr[data-field=title]");
  await expect(titleRow.getByTestId("conflict-mine")).toHaveText(`${title} (mine)`);
  await expect(titleRow.getByTestId("conflict-stored")).toHaveText(`${title} (theirs)`);
  await expect(titleRow).toHaveClass(/differs/);
  await expect(conflict.locator("tr[data-field=status] [data-testid=conflict-mine]")).toHaveText("done");
  await expect(conflict.locator("tr[data-field=status] [data-testid=conflict-stored]")).toHaveText("todo");
  await expect(conflict.locator("tr[data-field=estimate] [data-testid=conflict-stored]")).toHaveText("5");

  // Saving mine over the stored version writes it.
  await conflict.getByRole("button", { name: "Save my version" }).click();
  await expect(conflict).toHaveCount(0);
  await expect(page.getByRole("heading", { level: 1, name: `${title} (mine)` })).toBeVisible();
  const stored = await (await admin.get(`/api/notes/${created.id}`)).json();
  expect(stored.note.fields.title).toBe(`${title} (mine)`);
  expect(stored.note.fields.status).toBe("done");
});

test("keeping the stored version after a conflict discards the edit", async ({ page, admin }) => {
  const title = unique("Kept");
  const created = await createTask(admin, { title });
  await logIn(page, `/notes/${created.id}`);
  await expect(page.getByLabel("Title")).toHaveText(title);
  const theirs = await admin.put(`/api/notes/${created.id}`, {
    data: { base_version: created.version, note: { fields: { title: `${title} (theirs)` }, blocks: [] } },
  });
  expect(theirs.status()).toBe(200);
  await page.getByLabel("Title").fill(`${title} (mine)`);
  await page.getByLabel("Title").blur();
  await page.getByRole("button", { name: "Keep the stored version" }).click();
  await expect(page.getByTestId("conflict-view")).toHaveCount(0);
  await expect(page.getByRole("heading", { level: 1, name: `${title} (theirs)` })).toBeVisible();
});

test("the manifest and the service worker register", async ({ page, request }) => {
  await page.goto("/login");
  const href = await page.locator('link[rel="manifest"]').getAttribute("href");
  expect(href).toBe("/manifest.webmanifest");
  const manifest = await request.get(href!);
  expect(manifest.ok()).toBe(true);
  expect(manifest.headers()["content-type"]).toContain("application/manifest+json");
  const body = await manifest.json();
  expect(body).toMatchObject({ name: "TheVault", start_url: "/", display: "standalone" });
  expect(body.icons.length).toBeGreaterThan(0);

  const sw = await page.evaluate(async () => {
    const reg = await navigator.serviceWorker.ready;
    return { script: reg.active?.scriptURL ?? null, scope: reg.scope };
  });
  expect(sw.script).toMatch(/\/sw\.js$/);
  expect(new URL(sw.scope).pathname).toBe("/");

  // Only the shell is cached: API reads go to the network every time.
  await logIn(page);
  const cached = await page.evaluate(async () => {
    const urls: string[] = [];
    for (const name of await caches.keys()) {
      for (const req of await (await caches.open(name)).keys()) urls.push(new URL(req.url).pathname);
    }
    return urls;
  });
  expect(cached).toContain("/");
  expect(cached.filter((u) => u.startsWith("/api"))).toEqual([]);
});

test(`a type of ${BULK_NOTES} notes lists none until a search finds one`, async ({ page }) => {
  await logIn(page, "/types/bulk");
  await expect(page.getByRole("heading", { name: "Bulk notes" })).toBeVisible();
  await expect(page.getByTestId("type-list-status")).toHaveText(`${BULK_NOTES.toLocaleString("en-US")} notes. Search to find one.`);
  await expect(page.getByTestId("note-card")).toHaveCount(0);
  await page.getByRole("searchbox", { name: "Search bulk notes" }).fill(bulkTitle(BULK_NOTES).split(" ")[1]!);
  await expect(page.getByTestId("note-card").filter({ hasText: bulkTitle(BULK_NOTES) })).toBeVisible();
});

test.describe("at tablet width", () => {
  test.use({ viewport: devices["iPad Mini"].viewport });

  test("the sidebar opens from the menu button", async ({ page }) => {
    await logIn(page);
    const nav = page.getByRole("navigation", { name: "Main" });
    const daily = nav.getByRole("link", { name: "Daily notes" });
    await expect(daily).not.toBeInViewport();
    await page.getByRole("button", { name: "Menu" }).click();
    await expect(daily).toBeInViewport();
    await daily.click();
    await expect(page.getByRole("navigation", { name: "Day" })).toBeVisible();
    await expect(daily).not.toBeInViewport();
  });
});
