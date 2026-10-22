import type { APIRequestContext } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

const BOOK_TYPE = `label  = "Book"
plural = "Books"
icon   = "book"

field author {
  kind  = "string"
  label = "Author"
}

field pages {
  kind  = "int"
  label = "Pages"
}

field body {
  kind  = "content"
  label = "Body"
}

template default {
  tab "Main" {
    slot "title"
    slot "author"
    slot "pages"
    slot "body"
  }
}

template card {
  tab "Card" {
    slot "title"
    slot "author"
    slot "pages"
  }
}

list_page {
  sort = "title asc"
}
`;

async function putType(admin: APIRequestContext, name: string, wcl: string) {
  const res = await admin.put(`/api/types/${name}`, { data: { wcl } });
  expect(res.status(), await res.text()).toBe(200);
}

/** Create notes of `type` in one commit. */
async function seed(admin: APIRequestContext, type: string, notes: Record<string, unknown>[]) {
  const res = await admin.post("/api/batch", {
    data: { ops: notes.map((fields) => ({ op: "create", type, note: { fields, blocks: [] } })) },
  });
  expect(res.status(), await res.text()).toBe(200);
}

test("a type's notes show as cards by default, a remembered List view per type, and a search", async ({
  page,
  admin,
}) => {
  const t = token();
  const books = `book_${t}`;
  const other = `shelf_${t}`;
  await putType(admin, books, BOOK_TYPE);
  await putType(admin, other, BOOK_TYPE.replace('"Books"', '"Shelves"'));
  await seed(admin, books, [
    { title: "Dune", author: "Herbert", pages: 412 },
    { title: "Emma", author: "Austen", pages: 1036 },
    { title: "Ubik", author: "Dick", pages: 202 },
  ]);
  await seed(admin, other, [{ title: "Top shelf" }]);

  await logIn(page, `/types/${books}`);
  const pane = activePane(page);
  await expect(pane.getByRole("heading", { name: "Books" })).toBeVisible();
  const views = pane.getByRole("group", { name: "View" });
  await expect(views.getByRole("button", { name: "Cards" })).toHaveAttribute("aria-pressed", "true");
  await expect(views.getByRole("button", { name: "List" })).toHaveAttribute("aria-pressed", "false");

  // Cards, drawn with the card template, sorted by title.
  const cards = pane.getByTestId("note-card");
  await expect(cards).toHaveCount(3);
  await expect(cards.getByTestId("query-title")).toHaveText(["Dune", "Emma", "Ubik"]);
  await expect(cards.first()).toContainText("Herbert");
  await expect(pane.getByTestId("type-list-status")).toHaveText("3 notes");

  // The List view has the card's fields and Updated as columns, and sorts by a header.
  await views.getByRole("button", { name: "List" }).click();
  await expect(views.getByRole("button", { name: "List" })).toHaveAttribute("aria-pressed", "true");
  await expect(cards).toHaveCount(0);
  const table = pane.getByTestId("note-table");
  await expect(table.getByRole("columnheader")).toHaveText(["Title↑", "Author", "Pages", "Updated"]);
  const rows = pane.getByTestId("note-row");
  await expect(rows).toHaveCount(3);
  await table.getByRole("button", { name: "Pages" }).click();
  await expect(table.getByRole("columnheader", { name: "Pages" })).toHaveAttribute("aria-sort", "ascending");
  await expect(rows.locator("td:first-child")).toHaveText(["Ubik", "Dune", "Emma"]);
  await table.getByRole("button", { name: "Pages" }).click();
  await expect(rows.locator("td:first-child")).toHaveText(["Emma", "Dune", "Ubik"]);

  // The choice survives a reload, for this type only.
  await page.reload();
  await expect(activePane(page).getByTestId("note-row")).toHaveCount(3);
  await expect(activePane(page).getByRole("group", { name: "View" }).getByRole("button", { name: "List" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await page.goto(`/types/${other}`);
  await expect(activePane(page).getByRole("heading", { name: "Shelves" })).toBeVisible();
  await expect(activePane(page).getByTestId("note-card")).toHaveCount(1);
  await page.goto(`/types/${books}`);
  await expect(activePane(page).getByTestId("note-row")).toHaveCount(3);

  // The search narrows the list as it is typed, and Escape clears it.
  const search = activePane(page).getByRole("searchbox", { name: "Search books" });
  await search.fill("Herb");
  await expect(activePane(page).getByTestId("note-row")).toHaveCount(1);
  await expect(activePane(page).getByTestId("note-row")).toContainText("Dune");
  await expect(activePane(page).getByTestId("type-list-status")).toHaveText("1 match");
  await search.press("Escape");
  await expect(search).toHaveValue("");
  await expect(activePane(page).getByTestId("note-row")).toHaveCount(3);

  // A middle-click anywhere on a row opens its note in a background tab; a click opens it here.
  const tabs = page.getByRole("tablist", { name: "Open tabs" }).getByRole("tab");
  await expect(tabs).toHaveCount(1);
  await activePane(page).getByTestId("note-row").filter({ hasText: "Ubik" }).locator("td").nth(2).click({ button: "middle" });
  await expect(tabs).toHaveCount(2);
  await expect(page).toHaveURL(new RegExp(`/types/${books}$`));
  await activePane(page).getByTestId("note-row").filter({ hasText: "Emma" }).click();
  await expect(page).toHaveURL(/\/notes\/[0-9A-Z]+$/);
  await expect(page.getByLabel("Title", { exact: true })).toHaveText("Emma");
});

test("a big type lists a page at a time, and a search narrows the same pages (lists L2)", async ({ page, admin }) => {
  const name = `many_${token()}`;
  await putType(admin, name, BOOK_TYPE.replace('"Books"', '"Many books"'));
  await seed(
    admin,
    name,
    Array.from({ length: 120 }, (_, i) => ({ title: `Volume ${String(i).padStart(3, "0")}`, pages: i })),
  );

  await logIn(page, `/types/${name}`);
  const pane = activePane(page);
  const status = pane.getByTestId("type-list-status");
  const titles = pane.getByTestId("note-card").getByTestId("query-title");
  const pager = pane.getByRole("navigation", { name: "Pages" });
  await expect(status).toHaveText("1–50 of 120");
  await expect(titles).toHaveCount(50);
  await expect(titles.first()).toHaveText("Volume 000");
  await expect(pager.getByRole("button", { name: "Previous" })).toBeDisabled();

  await pager.getByRole("button", { name: "Next" }).click();
  await expect(status).toHaveText("51–100 of 120");
  await expect(titles.first()).toHaveText("Volume 050");
  await pager.getByRole("button", { name: "Next" }).click();
  await expect(status).toHaveText("101–120 of 120");
  await expect(titles).toHaveCount(20);
  await expect(pager.getByRole("button", { name: "Next" })).toBeDisabled();
  await pager.getByRole("button", { name: "Previous" }).click();
  await expect(status).toHaveText("51–100 of 120");
  await pager.getByRole("button", { name: "First" }).click();
  await expect(status).toHaveText("1–50 of 120");
  await expect(titles.first()).toHaveText("Volume 000");

  // A search starts again at page 1 and pages the same way.
  await pager.getByRole("button", { name: "Next" }).click();
  await expect(status).toHaveText("51–100 of 120");
  const search = pane.getByRole("searchbox", { name: "Search many books" });
  await search.fill("042");
  await expect(titles).toHaveText(["Volume 042"]);
  await expect(status).toHaveText("1 match");
  await search.fill("Volume");
  await expect(status).toHaveText("1–50 of 120 matches");
  await expect(titles).toHaveCount(50);

  await search.press("Escape");
  await expect(status).toHaveText("1–50 of 120");
});

test("the email type opens in the details view, newest sent first, 50 to a page (lists L5)", async ({ page, admin }) => {
  const t = token();
  // Sent dates after any other email in the shared vault, a second apart, newest first.
  const base = Date.now() + 100 * 365 * 86_400_000;
  const sent = (i: number) => new Date(base - i * 1000).toISOString().replace(/\.\d{3}Z$/, "Z");
  // Created oldest first, so neither the creation order nor the titles give the sent order.
  await seed(
    admin,
    "email",
    Array.from({ length: 60 }, (_, k) => 59 - k).map((i) => ({
      title: `Mail ${t} ${String((i * 37) % 60).padStart(2, "0")}`,
      sent: sent(i),
      snippet: `Snippet ${i}`,
    })),
  );
  const title = (i: number) => `Mail ${t} ${String((i * 37) % 60).padStart(2, "0")}`;

  await logIn(page, "/types/email");
  const pane = activePane(page);
  await expect(pane.getByRole("heading", { name: "Emails", level: 1 })).toBeVisible();
  const views = pane.getByRole("group", { name: "View" });
  await expect(views.getByRole("button", { name: "Details" })).toHaveAttribute("aria-pressed", "true");
  const details = pane.getByTestId("type-detail");
  await expect(details).toHaveCount(50);
  await expect(pane.getByTestId("note-card")).toHaveCount(0);
  const titles = details.getByTestId("query-title");
  await expect(titles.nth(0)).toHaveText(title(0));
  await expect(titles.nth(1)).toHaveText(title(1));
  await expect(titles.nth(49)).toHaveText(title(49));
  // Each note shows its default template: the Email tab with its Sent field.
  await expect(details.first()).toContainText("Sent");
  await expect(pane.getByTestId("type-list-status")).toContainText(/^1–50 of /);

  const pager = pane.getByRole("navigation", { name: "Pages" });
  await pager.getByRole("button", { name: "Next" }).click();
  await expect(pane.getByTestId("type-list-status")).toContainText(/^51–/);
  await expect(titles.nth(0)).toHaveText(title(50));
  await pager.getByRole("button", { name: "Previous" }).click();
  await expect(titles.nth(0)).toHaveText(title(0));

  // The List view sorts by Sent, newest first, until a header changes it.
  await views.getByRole("button", { name: "List" }).click();
  const table = pane.getByTestId("note-table");
  await expect(table.getByRole("columnheader", { name: "Sent", exact: true })).toHaveAttribute("aria-sort", "descending");
  await expect(pane.getByTestId("note-row").first()).toContainText(title(0));
  await table.getByRole("button", { name: "Sent", exact: true }).click();
  await expect(table.getByRole("columnheader", { name: "Sent", exact: true })).toHaveAttribute("aria-sort", "ascending");
  await expect(pane.getByTestId("note-row").first()).not.toContainText(title(0));

  // Reset to type default brings back the details view by sent date.
  await pane.getByRole("button", { name: "Reset to type default" }).click();
  await expect(views.getByRole("button", { name: "Details" })).toHaveAttribute("aria-pressed", "true");
  await expect(titles.nth(0)).toHaveText(title(0));
});
