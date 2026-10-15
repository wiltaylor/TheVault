// Archiving a note instead of deleting it: the note page's Archive button, its banner and chip,
// archived notes left out of search, the type list, the command palette and the @ picker, the
// Search page's archived filter, the Archive page, links to an archived note, and a query view
// that says `include archived`.

import type { APIRequestContext, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

interface Stored {
  id: string;
  version: string;
  note: { fields: Record<string, unknown> };
}

const para = (text: string) => ({ kind: "paragraph", fields: { text } });

async function create(admin: APIRequestContext, type: string, title: string, body: unknown[] = []): Promise<Stored> {
  const blocks = body.length ? [{ kind: "content", labels: ["body"], blocks: body }] : [];
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields: { title }, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as Stored;
}

async function stored(admin: APIRequestContext, id: string): Promise<Stored> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return (await res.json()) as Stored;
}

async function archive(admin: APIRequestContext, note: Stored): Promise<Stored> {
  const res = await admin.post(`/api/notes/${note.id}/archive?base_version=${note.version}`);
  expect(res.status(), await res.text()).toBe(200);
  return (await res.json()) as Stored;
}

const palette = (page: Page) => page.getByRole("dialog", { name: "Command palette" });

async function openPalette(page: Page) {
  await page.keyboard.press("ControlOrMeta+k");
  await expect(palette(page)).toBeVisible();
}

test("archiving a note from its page hides it from every listing until it is unarchived", async ({ page, admin }) => {
  const word = `shelf${token()}`;
  const note = await create(admin, "task", `Shelved ${word}`);
  // A note that stays, so each listing has something to show while the other is left out.
  await create(admin, "task", `Kept ${word}`);

  await logIn(page, `/notes/${note.id}`);
  const pane = activePane(page);
  await expect(pane.getByRole("heading", { level: 1, name: `Shelved ${word}` })).toBeVisible();
  await expect(pane.getByRole("region", { name: "Archived" })).toHaveCount(0);
  const button = pane.getByRole("button", { name: "Archive", exact: true });
  await expect(button).toHaveAttribute("title", "Archive note");
  await button.click();

  // The page says so, and the button now takes it back out.
  const banner = pane.getByRole("region", { name: "Archived" });
  await expect(banner).toBeVisible();
  await expect(banner).toContainText("Archived");
  await expect(pane.getByTestId("archived-chip")).toHaveText("Archived");
  await expect(pane.getByTitle("Unarchive note")).toHaveAttribute("aria-pressed", "true");
  expect((await stored(admin, note.id)).note.fields.archived).toBe(true);
  // It stays editable.
  await expect(pane.getByLabel("Title", { exact: true })).toHaveAttribute("contenteditable", "plaintext-only");

  // The Search page leaves it out by default, and its archived filter, kept in the address,
  // brings it back.
  await page.goto(`/search?q=${word}`);
  const rows = pane.getByTestId("note-row");
  await expect(rows.filter({ hasText: `Kept ${word}` })).toBeVisible();
  await expect(rows.filter({ hasText: `Shelved ${word}` })).toHaveCount(0);
  await pane.getByLabel("Archived notes").selectOption("only");
  await expect(page).toHaveURL(new RegExp(`/search\\?q=${word}&archived=only$`));
  await expect(rows.filter({ hasText: `Shelved ${word}` })).toBeVisible();
  await expect(rows.filter({ hasText: `Shelved ${word}` }).locator(".archived-chip")).toHaveText("Archived");
  await expect(rows.filter({ hasText: `Kept ${word}` })).toHaveCount(0);
  await expect(pane.getByRole("button", { name: "Copy as query" })).toHaveAttribute(
    "title",
    `from * where archived search "${word}"`,
  );
  await pane.getByLabel("Archived notes").selectOption("include");
  await expect(page).toHaveURL(/archived=include/);
  await expect(rows).toHaveCount(2);
  await expect(pane.getByRole("button", { name: "Copy as query" })).toHaveAttribute(
    "title",
    `from * include archived search "${word}"`,
  );

  // The type's list leaves it out unless Show archived is on.
  await page.goto("/types/task");
  await pane.getByRole("searchbox", { name: "Search tasks" }).fill(word);
  const cards = pane.getByTestId("note-card");
  await expect(cards.filter({ hasText: `Kept ${word}` })).toBeVisible();
  await expect(cards.filter({ hasText: `Shelved ${word}` })).toHaveCount(0);
  const show = pane.getByRole("button", { name: "Show archived" });
  await show.click();
  await expect(show).toHaveAttribute("aria-pressed", "true");
  await expect(cards.filter({ hasText: `Shelved ${word}` })).toBeVisible();
  await expect(cards.filter({ hasText: `Shelved ${word}` }).locator(".archived-chip")).toHaveText("Archived");

  // The command palette's note search leaves it out.
  await openPalette(page);
  await page.keyboard.type(word);
  const options = palette(page).getByRole("option");
  await expect(options.filter({ hasText: `Kept ${word}` })).toBeVisible();
  await expect(options.filter({ hasText: `Shelved ${word}` })).toHaveCount(0);
  await page.keyboard.press("Escape");

  // The Archive page lists it, and Unarchive puts it back everywhere.
  await page.goto("/archive");
  await pane.getByRole("searchbox", { name: "Search archived notes" }).fill(word);
  const archived = pane.getByTestId("archive-row");
  await expect(archived).toHaveCount(1);
  await expect(archived.first()).toContainText(`Shelved ${word}`);
  await archived.first().getByRole("button", { name: "Unarchive" }).click();
  await expect(archived).toHaveCount(0);
  await expect(pane.getByText("No archived notes match.")).toBeVisible();
  expect((await stored(admin, note.id)).note.fields.archived).toBeUndefined();
  await page.goto(`/search?q=${word}`);
  await expect(rows).toHaveCount(2);
});

test("the banner's Unarchive and the palette's actions archive and unarchive the open note", async ({ page, admin }) => {
  const note = await create(admin, "task", `Palette archive ${token()}`);
  await logIn(page, `/notes/${note.id}`);
  const pane = activePane(page);
  await expect(pane.getByRole("heading", { level: 1, name: /Palette archive/ })).toBeVisible();

  await openPalette(page);
  await page.keyboard.type(">archive this");
  await palette(page).getByRole("option", { name: /Archive this note/ }).click();
  await expect(pane.getByRole("region", { name: "Archived" })).toBeVisible();

  await openPalette(page);
  await page.keyboard.type(">unarchive");
  await expect(palette(page).getByRole("option", { name: /Unarchive this note/ })).toBeVisible();
  await page.keyboard.press("Escape");

  await pane.getByRole("region", { name: "Archived" }).getByRole("button", { name: "Unarchive" }).click();
  await expect(pane.getByRole("region", { name: "Archived" })).toHaveCount(0);
  await expect(pane.getByRole("button", { name: "Archive", exact: true })).toBeVisible();
  expect((await stored(admin, note.id)).note.fields.archived).toBeUndefined();

  // Go to Archive opens the page.
  await openPalette(page);
  await page.keyboard.type(">go to archive");
  await page.keyboard.press("Enter");
  await expect(pane.getByRole("heading", { level: 1, name: "Archive" })).toBeVisible();
  // So does the sidebar's archive box.
  await page.goto("/daily");
  await page.getByRole("link", { name: "Archive", exact: true }).click();
  await expect(page).toHaveURL(/\/archive$/);
});

test("the @ picker leaves archived notes out, and a mention of one still opens it, drawn archived", async ({
  page,
  admin,
}) => {
  const word = `atarch${token()}`;
  const target = await archive(admin, await create(admin, "task", `Old ${word}`));
  await create(admin, "task", `New ${word}`);
  const source = await create(admin, "page", `Linking ${token()}`, [para(`See [[${target.id}]].`)]);

  await logIn(page, `/notes/${source.id}`);
  const pane = activePane(page);
  const mention = pane.getByTestId("mention");
  await expect(mention).toHaveText(`Old ${word}`);
  await expect(mention).toHaveClass(/archived/);
  await expect(mention).toHaveAttribute("title", "Archived");

  // The picker finds the live note only.
  const doc = pane.getByRole("textbox", { name: "Body" });
  await doc.click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type(" @");
  await page.keyboard.type(word);
  const results = page.getByTestId("mention-result");
  await expect(results.filter({ hasText: `New ${word}` })).toBeVisible();
  await expect(results.filter({ hasText: `Old ${word}` })).toHaveCount(0);
  await page.keyboard.press("Escape");

  await mention.click();
  await expect(pane.getByRole("heading", { level: 1, name: `Old ${word}` })).toBeVisible();
  await expect(pane.getByRole("region", { name: "Archived" })).toBeVisible();
});

test("a query view leaves archived notes out unless its query says include archived", async ({ page, admin }) => {
  const word = `qvarch${token()}`;
  await archive(admin, await create(admin, "task", `${word} archived`));
  await create(admin, "task", `${word} live`);
  const view = (query: string) => ({
    kind: "query_view",
    fields: { query },
    blocks: [{ kind: "view", labels: ["table"], fields: { kind: "table" } }],
  });
  const host = await create(admin, "page", `Views ${word}`, [
    view(`from task where contains(title, "${word}") sort title asc`),
    view(`from task include archived where contains(title, "${word}") sort title asc`),
  ]);

  await logIn(page, `/notes/${host.id}`);
  const views = activePane(page).getByTestId("query-view");
  await expect(views).toHaveCount(2);
  const titles = (i: number) => views.nth(i).getByTestId("query-row").getByTestId("query-title");
  await expect(titles(0)).toHaveText([`${word} live`]);
  await expect(titles(1)).toHaveText([`${word} archived`, `${word} live`]);
});

test("the Archive page filters by type and moves a note to the trash", async ({ page, admin }) => {
  const word = `arcpage${token()}`;
  await archive(admin, await create(admin, "task", `Task ${word}`));
  const pageNote = await archive(admin, await create(admin, "page", `Page ${word}`));

  await logIn(page, "/archive");
  const pane = activePane(page);
  await pane.getByRole("searchbox", { name: "Search archived notes" }).fill(word);
  const rows = pane.getByTestId("archive-row");
  await expect(rows).toHaveCount(2);
  await pane.getByLabel("Type").selectOption("page");
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText(`Page ${word}`);

  page.once("dialog", (d) => void d.accept());
  await rows.first().getByRole("button", { name: "Delete" }).click();
  await expect(rows).toHaveCount(0);
  await expect
    .poll(async () => {
      const trash = await admin.get("/api/trash");
      return ((await trash.json()) as { notes: { id: string }[] }).notes.map((n) => n.id);
    })
    .toContain(pageNote.id);
});
