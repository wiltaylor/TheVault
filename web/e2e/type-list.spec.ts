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
  await expect(table.getByRole("columnheader")).toHaveText(["Title", "Author", "Pages", "Updated"]);
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

test("a type with more than 200 notes lists none until a search narrows it", async ({ page, admin }) => {
  const name = `many_${token()}`;
  await putType(admin, name, BOOK_TYPE.replace('"Books"', '"Many books"'));
  await seed(
    admin,
    name,
    Array.from({ length: 201 }, (_, i) => ({ title: `Volume ${String(i).padStart(3, "0")}`, pages: i })),
  );

  await logIn(page, `/types/${name}`);
  const pane = activePane(page);
  await expect(pane.getByTestId("type-list-status")).toHaveText("201 notes. Search to find one.");
  await expect(pane.getByTestId("note-card")).toHaveCount(0);
  await expect(pane.getByTestId("note-row")).toHaveCount(0);

  const search = pane.getByRole("searchbox", { name: "Search many books" });
  await search.fill("042");
  await expect(pane.getByTestId("note-card").getByTestId("query-title")).toHaveText(["Volume 042"]);

  // A search matching more than the most a list shows says so.
  await search.fill("Volume");
  await expect(pane.getByTestId("type-list-status")).toContainText("Showing the first 200 of 201 matches");
  await expect(pane.getByTestId("note-card")).toHaveCount(200);

  await search.press("Escape");
  await expect(pane.getByTestId("type-list-status")).toHaveText("201 notes. Search to find one.");
  await expect(pane.getByTestId("note-card")).toHaveCount(0);
});
