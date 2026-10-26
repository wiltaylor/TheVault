// A list of links: made with the type editor's "List of links" kind, edited on a note one row per
// link (a pasted URL fills the row), and shown as links on the note page and in a table view.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function create(admin: APIRequestContext, type: string, fields: Record<string, unknown>, blocks: unknown[] = []) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string };
}

async function fieldsOf(admin: APIRequestContext, id: string) {
  return ((await (await admin.get(`/api/notes/${id}`)).json()) as { note: { fields: Record<string, unknown> } }).note.fields;
}

/** With E2E_SHOTS set to a folder, a screenshot of `what` there as `link-lists-<name>.png`. */
async function shot(page: Page, name: string, what: Locator) {
  const dir = process.env.E2E_SHOTS;
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  await page.waitForTimeout(200);
  await what.screenshot({ path: join(dir, `link-lists-${name}.png`) });
}

test("a list of links is made in the type editor, edited row by row, and shown as links", async ({ page, admin, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const name = `bookmarks_${token()}`;
  const put = await admin.put(`/api/types/${name}`, {
    data: { wcl: `label = "Bookmarks"\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n  }\n}\n` },
  });
  expect(put.status(), await put.text()).toBe(200);

  // The type editor's List of links kind makes the whole definition.
  await logIn(page, `/settings/types/${name}/edit`);
  await page.getByRole("button", { name: "Add field" }).click();
  const row = page.getByTestId("field-row-0");
  await row.getByLabel("Name", { exact: true }).fill("links");
  await row.getByLabel("Kind", { exact: true }).selectOption({ label: "List of links" });
  await expect(row.getByLabel("Item kind")).toHaveValue("dict");
  await shot(page, "type-editor", row);
  await page.getByRole("button", { name: "Preview changes" }).click();
  await page.getByRole("button", { name: "Apply changes" }).click();
  await expect
    .poll(async () => (await admin.get(`/api/types/${name}`, { headers: { Accept: "text/plain" } })).text())
    .toMatch(/key name \{\s*kind\s*= "string"\s*\}\s*key url \{\s*kind\s*= "url"\s*required\s*= true\s*\}/);
  // Edited again, the field shows as a list of links.
  await page.goto(`/settings/types/${name}/edit`);
  await expect(page.getByTestId("field-row-0").getByLabel("Kind", { exact: true })).toHaveValue("links");

  const tpl = await admin.put(`/api/types/${name}/templates/default`, {
    data: { wcl: `template default {\n  tab "Main" {\n    slot "title"\n    slot "links"\n  }\n}\n` },
  });
  expect(tpl.status(), await tpl.text()).toBe(200);
  const title = `Reading ${token()}`;
  const { id } = await create(admin, name, { title });

  await page.goto(`/notes/${id}`);
  const pane = activePane(page);
  const links = pane.getByTestId("field-links");
  await expect(links.getByRole("button", { name: "Add link" })).toBeVisible();

  // One link typed in, its Name then its URL.
  await links.getByRole("button", { name: "Add link" }).click();
  await expect(links.getByRole("textbox", { name: "Name of Links link 1" })).toBeFocused();
  await page.keyboard.type("Docs");
  await links.getByRole("textbox", { name: "URL of Links link 1" }).fill("https://example.com/docs");

  // One pasted: the URL fills the row and its host the name.
  await links.getByRole("button", { name: "Add link" }).click();
  await page.evaluate(() => navigator.clipboard.writeText("https://github.com/wiltaylor/thevault"));
  await expect(links.getByRole("textbox", { name: "Name of Links link 2" })).toBeFocused();
  await page.keyboard.press("ControlOrMeta+V");
  await expect(links.getByRole("textbox", { name: "Name of Links link 2" })).toHaveValue("github.com");
  await expect(links.getByRole("textbox", { name: "URL of Links link 2" })).toHaveValue("https://github.com/wiltaylor/thevault");
  await expect
    .poll(async () => (await fieldsOf(admin, id)).links)
    .toEqual([
      { name: "Docs", url: "https://example.com/docs" },
      { name: "github.com", url: "https://github.com/wiltaylor/thevault" },
    ]);

  // A URL that is not one says so, and is not stored.
  await links.getByRole("button", { name: "Add link" }).click();
  await links.getByRole("textbox", { name: "URL of Links link 3" }).fill("example.org");
  await links.getByRole("textbox", { name: "Name of Links link 3" }).click();
  await expect(links.getByRole("alert")).toHaveText("“example.org” is not an absolute URL, such as https://example.com.");
  await shot(page, "editor", links);
  await links.getByRole("button", { name: "Remove example.org" }).click();
  await expect(links.getByRole("alert")).toHaveCount(0);

  // Each row opens its link in a new tab.
  const open = links.getByRole("link", { name: "Open Docs" });
  await expect(open).toHaveAttribute("href", "https://example.com/docs");
  await expect(open).toHaveAttribute("target", "_blank");
  await expect(open).toHaveAttribute("rel", "noopener noreferrer");

  // Reordered, each name keeps its URL.
  await links.getByRole("button", { name: "Move github.com up" }).click();
  await expect
    .poll(async () => (await fieldsOf(admin, id)).links)
    .toEqual([
      { name: "github.com", url: "https://github.com/wiltaylor/thevault" },
      { name: "Docs", url: "https://example.com/docs" },
    ]);

  // A table view shows them as links, inline, in the stored order.
  const view = await create(admin, "page", { title: `Links table ${token()}` }, [
    {
      kind: "content",
      labels: ["body"],
      blocks: [
        {
          kind: "query_view",
          fields: { query: `from ${name}` },
          blocks: [{ kind: "view", labels: ["table"], fields: { kind: "table", columns: ["title", "links"] } }],
        },
      ],
    },
  ]);
  await page.goto(`/notes/${view.id}`);
  const tableRow = activePane(page).getByTestId("query-row").filter({ hasText: title });
  await expect(tableRow).toHaveCount(1);
  const cellLinks = tableRow.locator("a.field-link");
  await expect(cellLinks).toHaveText(["github.com", "Docs"]);
  await expect(cellLinks.nth(1)).toHaveAttribute("href", "https://example.com/docs");
  await expect(cellLinks.nth(1)).toHaveAttribute("target", "_blank");
  await expect(tableRow).toContainText("github.com, Docs");
  await shot(page, "table", activePane(page).getByTestId("query-view"));
});
