import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";
import { FINDING_OWNER, FINDINGS } from "./vault";

// The Search page: full text, filters, sort, the address, and Copy as query. Most tests read the
// seeded findings (vault.ts), which nothing else writes, through a type filter on them.

const ID = Object.fromEntries(FINDINGS.map((f) => [f.title, f.id]));

const main = (page: Page) => page.getByRole("navigation", { name: "Main" });
const tabs = (page: Page) => page.getByRole("tablist", { name: "Open tabs" }).getByRole("tab");
const rows = (page: Page) => activePane(page).getByTestId("note-row");
const box = (page: Page) => activePane(page).getByRole("searchbox", { name: "Search notes" });
const filterRow = (page: Page, field: string) => activePane(page).locator(`.filter-row[data-field="${field}"]`);

async function titles(page: Page): Promise<string[]> {
  return rows(page).locator(".note-title").allTextContents();
}

/** Wait for the result to be exactly `want`, in order. */
async function expectTitles(page: Page, want: string[]) {
  await expect.poll(() => titles(page)).toEqual(want);
  await expect(activePane(page).locator(".search-count")).toHaveText(want.length === 1 ? "1 note" : `${want.length} notes`);
}

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

/** Today in the browser's time zone, as a date input holds it. */
function today(): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

async function addFilter(page: Page, section: "date" | "property", option: string | { label: string }) {
  await activePane(page).getByLabel(`Add ${section} filter`).selectOption(option);
}

/** Add a type in the type picker by typing part of its name and picking it. */
async function addType(page: Page, typed: string, label: string) {
  const row = activePane(page).getByRole("group", { name: "Type filter" });
  const box = row.getByRole("combobox", { name: "Add to Types" });
  if (!(await box.isVisible())) await row.getByRole("button", { name: "Add to Types" }).click();
  await box.fill(typed);
  await row.getByRole("option", { name: label, exact: true }).click();
}

/** The Search page with the findings type chosen, sorted by title. */
async function findings(page: Page, extra = "") {
  await logIn(page, `/search?type=finding&sort=title${extra}`);
  await expect(rows(page).first()).toBeVisible();
}

async function createTask(admin: APIRequestContext, title: string, body: string) {
  const blocks = [{ kind: "content", labels: ["body"], blocks: [{ kind: "paragraph", fields: { text: body } }] }];
  const res = await admin.post("/api/notes?type=task", { data: { note: { fields: { title }, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string };
}

async function selectOp(row: Locator, op: string) {
  await row.getByLabel("Operator").selectOption({ label: op });
}

test("Search in the sidebar opens the page in the active pane; a middle-click opens a background tab", async ({ page }) => {
  await logIn(page, "/daily");
  const link = main(page).getByRole("link", { name: "Search" });
  await expect(link.locator("svg")).toBeVisible();
  await link.click();
  await expect(page).toHaveURL(/\/search$/);
  await expect(activePane(page).getByRole("heading", { name: "Search" })).toBeVisible();
  await expect(box(page)).toBeVisible();
  await expect(link).toHaveClass(/active/);

  await main(page).getByRole("link", { name: "Daily notes" }).click();
  await expect(page).toHaveURL(/\/daily/);
  await link.click({ button: "middle" });
  await expect(tabs(page)).toHaveCount(2);
  await expect(tabs(page).nth(0)).toHaveAttribute("aria-selected", "true");
  await expect(tabs(page).nth(1)).toHaveAccessibleName(/Search/);
  await expect(page).toHaveURL(/\/daily/);
});

test("full text finds a note by a word in its content, as you type", async ({ page, admin }) => {
  const word = `quokka${token()}`;
  const title = `Content match ${token()}`;
  const note = await createTask(admin, title, `Somewhere in here is a ${word} hiding.`);
  await logIn(page, "/search");
  await box(page).pressSequentially(word, { delay: 10 });
  await expectTitles(page, [title]);
  await expect(page).toHaveURL(new RegExp(`/search\\?q=${word}$`));
  // Each row shows its type and when it was updated.
  await expect(rows(page).first().locator(".note-type")).toHaveText("Task");
  await expect(rows(page).first().locator("time")).not.toBeEmpty();
  await rows(page).first().click();
  // The note opens with the search's text, so a toggle holding the hit opens.
  await expect(page).toHaveURL(new RegExp(`/notes/${note.id}\\?find=${word}$`));
});

test("a partial word finds the note as you type, and a space after it asks for the whole word", async ({ page, admin }) => {
  const stem = `wombat${token()}`;
  const title = `Prefix match ${token()}`;
  await createTask(admin, title, `Out past the ${stem}warren, nothing else.`);
  await logIn(page, "/search");
  // Only the start of the word is typed.
  await box(page).pressSequentially(stem, { delay: 10 });
  await expectTitles(page, [title]);
  // With a space after it, the word is complete, and no note holds that whole word.
  await box(page).press("Space");
  await expectTitles(page, []);
  await expect(activePane(page).getByText("No notes match.")).toBeVisible();
});

test("the type filter narrows the results", async ({ page }) => {
  await logIn(page, "/search?q=valve");
  // "valve" is in a finding; adding a task-only filter leaves nothing.
  await expect.poll(() => titles(page)).toContain("Leaky valve");
  await addType(page, "find", "Finding");
  await expectTitles(page, ["Leaky valve"]);
  await expect(activePane(page).locator('.filter-row[data-filter="type"]')).toContainText("Finding");
  await addType(page, "TAS", "Task");
  await expectTitles(page, ["Leaky valve"]);
  await expect(page).toHaveURL(/type=finding&type=task/);
  await activePane(page).getByRole("button", { name: "Remove Finding" }).click();
  await expectTitles(page, []);
  await expect(activePane(page).getByText("No notes match.")).toBeVisible();
});

test("date filters on created, updated and a date field", async ({ page }) => {
  await findings(page);
  await expectTitles(page, ["Brighter lamp", "Cracked beam", "Leaky valve", "Loose tile"]);

  // Created between two dates.
  await addFilter(page, "date", { label: "Created" });
  const created = filterRow(page, "created");
  await selectOp(created, "between");
  await created.getByLabel("From", { exact: true }).fill("2026-03-15");
  await created.getByLabel("To", { exact: true }).fill("2026-05-01");
  await expectTitles(page, ["Brighter lamp", "Cracked beam"]);
  await created.getByRole("button", { name: "Remove Created filter" }).click();
  await expectTitles(page, ["Brighter lamp", "Cracked beam", "Leaky valve", "Loose tile"]);

  // Updated after a date.
  await addFilter(page, "date", { label: "Updated" });
  const updated = filterRow(page, "updated");
  await selectOp(updated, "after");
  await updated.getByLabel("Date", { exact: true }).fill("2026-05-31");
  await expectTitles(page, ["Brighter lamp", "Loose tile"]);
  // In the last N days: none of the seeded findings is recent.
  await selectOp(updated, "in the last");
  await expect(updated.getByLabel("Days", { exact: true })).toHaveValue("7");
  await expectTitles(page, []);
  await updated.getByRole("button", { name: "Remove Updated filter" }).click();

  // A date field of the type: found before a date, then on a day.
  await addFilter(page, "date", { label: "Found" });
  const found = filterRow(page, "found");
  await selectOp(found, "before");
  await found.getByLabel("Date", { exact: true }).fill("2026-04-30");
  await expectTitles(page, ["Brighter lamp", "Leaky valve"]);
  await selectOp(found, "on");
  await found.getByLabel("Date", { exact: true }).fill("2026-05-20");
  await expectTitles(page, ["Cracked beam"]);
});

test("created today finds a note made now, and before today does not", async ({ page, admin }) => {
  const word = `wombat${token()}`;
  const title = `Made today ${token()}`;
  await createTask(admin, title, word);
  await logIn(page, `/search?q=${word}`);
  await expectTitles(page, [title]);
  await addFilter(page, "date", { label: "Created" });
  const created = filterRow(page, "created");
  await selectOp(created, "on");
  await created.getByLabel("Date", { exact: true }).fill(today());
  await expectTitles(page, [title]);
  await selectOp(created, "before");
  await created.getByLabel("Date", { exact: true }).fill(today());
  await expectTitles(page, []);
});

test("property filters: select, taxonomy under, number, bool, reference and is empty", async ({ page }) => {
  await findings(page);

  // Select: is, then is any of.
  await addFilter(page, "property", { label: "Category" });
  const category = filterRow(page, "category");
  await category.getByLabel("Value", { exact: true }).selectOption("bug");
  await expectTitles(page, ["Leaky valve", "Loose tile"]);
  await selectOp(category, "is any of");
  await expect(category.getByRole("checkbox", { name: "bug" })).toBeChecked();
  await category.getByRole("checkbox", { name: "bug" }).uncheck();
  await category.getByRole("checkbox", { name: "risk" }).check();
  await category.getByRole("checkbox", { name: "idea" }).check();
  await expectTitles(page, ["Brighter lamp", "Cracked beam"]);
  await category.getByRole("button", { name: "Remove Category filter" }).click();

  // Taxonomy: under a term takes its descendants.
  await addFilter(page, "property", { label: "Area" });
  const area = filterRow(page, "area");
  await selectOp(area, "is under");
  await area.getByLabel("Value", { exact: true }).selectOption("Plant");
  await expectTitles(page, ["Brighter lamp", "Leaky valve"]);
  await selectOp(area, "is");
  await area.getByLabel("Value", { exact: true }).selectOption("Plant");
  await expectTitles(page, []);
  await area.getByLabel("Value", { exact: true }).selectOption("Plant/Plumbing");
  await expectTitles(page, ["Leaky valve"]);
  await area.getByRole("button", { name: "Remove Area filter" }).click();

  // Number comparison.
  await addFilter(page, "property", { label: "Severity" });
  const severity = filterRow(page, "severity");
  await selectOp(severity, "≥");
  await severity.getByLabel("Value", { exact: true }).fill("3");
  await expectTitles(page, ["Cracked beam", "Leaky valve"]);
  await selectOp(severity, "<");
  await expectTitles(page, ["Brighter lamp"]);
  // Is empty works on any kind.
  await selectOp(severity, "is empty");
  await expectTitles(page, ["Loose tile"]);
  await selectOp(severity, "is not empty");
  await expectTitles(page, ["Brighter lamp", "Cracked beam", "Leaky valve"]);
  await severity.getByRole("button", { name: "Remove Severity filter" }).click();

  // Bool.
  await addFilter(page, "property", { label: "Verified" });
  const verified = filterRow(page, "verified");
  await expectTitles(page, ["Cracked beam", "Leaky valve"]);
  await selectOp(verified, "is false");
  await expectTitles(page, ["Brighter lamp", "Loose tile"]);
  await verified.getByRole("button", { name: "Remove Verified filter" }).click();

  // Reference: a note picked by title, then is empty.
  await addFilter(page, "property", { label: "Owner" });
  const owner = filterRow(page, "owner");
  await owner.getByRole("button", { name: "Add to Note" }).click();
  await owner.getByRole("combobox", { name: "Add to Note" }).fill("Seeded owner");
  await owner.getByRole("option", { name: FINDING_OWNER.title }).click();
  await expectTitles(page, ["Cracked beam", "Leaky valve"]);
  await expect(page).toHaveURL(new RegExp(`prop=finding~owner~is~${FINDING_OWNER.id}`));
  await selectOp(owner, "is empty");
  await expectTitles(page, ["Brighter lamp", "Loose tile"]);
});

test("with no type chosen, a property filter offers every type's fields and narrows to its type", async ({ page }) => {
  await logIn(page, "/search");
  const add = activePane(page).getByLabel("Add property filter");
  await expect(add.locator('optgroup[label="Finding"] option', { hasText: "Severity" })).toHaveCount(1);
  await expect(add.locator('optgroup[label="Task"] option', { hasText: "Estimate" })).toHaveCount(1);
  await add.selectOption("finding.severity");
  const severity = filterRow(page, "severity");
  await expect(severity).toContainText("Finding");
  await selectOp(severity, ">");
  await severity.getByLabel("Value", { exact: true }).fill("4");
  await expectTitles(page, ["Cracked beam"]);
});

test("the sort changes the order", async ({ page }) => {
  await logIn(page, "/search?type=finding");
  // Updated, newest first, by default.
  await expectTitles(page, ["Loose tile", "Brighter lamp", "Cracked beam", "Leaky valve"]);
  const sort = activePane(page).getByLabel("Sort by");
  await sort.selectOption("created");
  await expectTitles(page, ["Loose tile", "Cracked beam", "Brighter lamp", "Leaky valve"]);
  await activePane(page).getByRole("button", { name: /sort ascending/ }).click();
  await expectTitles(page, ["Leaky valve", "Brighter lamp", "Cracked beam", "Loose tile"]);
  await sort.selectOption("title");
  await expectTitles(page, ["Brighter lamp", "Cracked beam", "Leaky valve", "Loose tile"]);
  await expect(page).toHaveURL(/sort=title/);
  // Relevance needs text.
  await expect(sort.locator('option[value="relevance"]')).toBeDisabled();
});

test("the search survives a reload and moves with back and forward", async ({ page }) => {
  await findings(page);
  await addFilter(page, "property", { label: "Category" });
  await filterRow(page, "category").getByLabel("Value", { exact: true }).selectOption("bug");
  await expectTitles(page, ["Leaky valve", "Loose tile"]);
  await box(page).fill("valve");
  await expectTitles(page, ["Leaky valve"]);
  await expect(page).toHaveURL(/q=valve/);

  await page.reload();
  await expect(box(page)).toHaveValue("valve");
  await expect(filterRow(page, "category").getByLabel("Value", { exact: true })).toHaveValue("bug");
  await expectTitles(page, ["Leaky valve"]);

  // Back undoes the text, then the filter; forward redoes them.
  await activePane(page).getByRole("button", { name: "Back" }).click();
  await expect(box(page)).toHaveValue("");
  await expectTitles(page, ["Leaky valve", "Loose tile"]);
  await activePane(page).getByRole("button", { name: "Back" }).click();
  await expect(filterRow(page, "category")).toHaveCount(0);
  await expectTitles(page, ["Brighter lamp", "Cracked beam", "Leaky valve", "Loose tile"]);
  await activePane(page).getByRole("button", { name: "Forward" }).click();
  await expect(filterRow(page, "category").getByLabel("Value", { exact: true })).toHaveValue("bug");
  await activePane(page).getByRole("button", { name: "Forward" }).click();
  await expect(box(page)).toHaveValue("valve");
  await expectTitles(page, ["Leaky valve"]);
});

test("Copy as query gives text that returns the same notes from the API", async ({ page, context, admin }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await findings(page, "&prop=~area~under~Plant&prop=~severity~ge~1");
  await addFilter(page, "date", { label: "Found" });
  const found = filterRow(page, "found");
  await selectOp(found, "between");
  await found.getByLabel("From", { exact: true }).fill("2026-01-01");
  await found.getByLabel("To", { exact: true }).fill("2026-12-31");
  await box(page).fill("lamp");
  await expectTitles(page, ["Brighter lamp"]);

  await activePane(page).getByRole("button", { name: "Copy as query" }).click();
  await expect(activePane(page).getByRole("button", { name: "Copied" })).toBeVisible();
  const text = await page.evaluate(() => navigator.clipboard.readText());
  expect(text).toBe(
    'from finding where under(area, "Plant") && severity >= 1 && found >= "2026-01-01" && found <= "2026-12-31" search "lamp" sort title asc',
  );
  const res = await admin.post("/api/query", { data: { query: text } });
  expect(res.status(), await res.text()).toBe(200);
  const body = (await res.json()) as { rows: { id: string }[] };
  expect(body.rows.map((r) => r.id)).toEqual([ID["Brighter lamp"]]);

  // Without the text, the same holds over several notes.
  await box(page).fill("");
  await expectTitles(page, ["Brighter lamp", "Leaky valve"]);
  await activePane(page).getByRole("button", { name: "Copy as query" }).click();
  const all = await page.evaluate(() => navigator.clipboard.readText());
  const res2 = await admin.post("/api/query", { data: { query: all } });
  const ids = ((await res2.json()) as { rows: { id: string }[] }).rows.map((r) => r.id);
  expect(ids).toEqual([ID["Brighter lamp"], ID["Leaky valve"]]);
});

test("a result opens in the same pane; a middle-click opens it in a background tab", async ({ page }) => {
  await findings(page);
  const row = rows(page).filter({ hasText: "Cracked beam" });
  await row.click({ button: "middle" });
  await expect(tabs(page)).toHaveCount(2);
  await expect(tabs(page).nth(0)).toHaveAttribute("aria-selected", "true");
  await expect(tabs(page).nth(1)).toHaveAccessibleName(/Cracked beam/);
  await expect(page).toHaveURL(/\/search\?/);
  await row.click({ modifiers: ["ControlOrMeta"] });
  await expect(tabs(page)).toHaveCount(3);

  await rows(page).filter({ hasText: "Leaky valve" }).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${ID["Leaky valve"]}$`));
  await expect(activePane(page).getByLabel("Title", { exact: true })).toHaveText("Leaky valve");
  await expect(tabs(page)).toHaveCount(3);
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });

  test("filters fold behind a Filters button that counts them", async ({ page }) => {
    await logIn(page, "/search?type=finding&sort=title&prop=~category~is~bug");
    await expectTitles(page, ["Leaky valve", "Loose tile"]);
    const toggle = activePane(page).getByRole("button", { name: "Filters (2)" });
    await expect(toggle).toBeVisible();
    await expect(filterRow(page, "category")).toBeHidden();
    await toggle.click();
    await expect(filterRow(page, "category")).toBeVisible();
    await expect(activePane(page).getByLabel("Add property filter")).toBeVisible();
    // Rows stay on screen and readable.
    const row = await rows(page).first().boundingBox();
    expect(row!.width).toBeLessThanOrEqual(390);
  });
});
