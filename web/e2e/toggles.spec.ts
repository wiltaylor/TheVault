import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// Toggles and callouts: a toggle with a heading summary and a nested list, made with `>> ` and
// the `/` menu, and a warning callout from `/callout`; both stored as blocks, and after a reload
// the toggle as this device left it, open, though opening it saved nothing.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

const TYPE_WCL = `label = "Folder"
plural = "Folders"

field body {
  kind  = "content"
  label = "Body"
}

template default {
  tab "Main" {
    slot "body"
  }
}
`;

type Json = { kind: string; fields?: Record<string, unknown>; blocks?: Json[]; labels?: string[] };

async function setup(admin: APIRequestContext, blocks: Json[] = []) {
  const name = `fold_${token()}`;
  const put = await admin.put(`/api/types/${name}`, { data: { wcl: TYPE_WCL } });
  expect(put.status(), await put.text()).toBe(200);
  const content = blocks.length ? [{ kind: "content", labels: ["body"], blocks }] : [];
  const res = await admin.post(`/api/notes?type=${name}`, { data: { note: { fields: { title: `Toggles ${token()}` }, blocks: content } } });
  expect(res.status(), await res.text()).toBe(201);
  return ((await res.json()) as { id: string }).id;
}

async function stored(admin: APIRequestContext, id: string): Promise<{ version: string; wcl: string; body: Json[] }> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  const n = (await res.json()) as { version: string; wcl: string; note: { blocks: Json[] } };
  const body = n.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === "body");
  return { version: n.version, wcl: n.wcl, body: body?.blocks ?? [] };
}

const body = (page: Page): Locator => page.getByRole("textbox", { name: "Body" });

async function saved(page: Page) {
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
}

/** Screenshots in the light and dark themes, to `$E2E_SHOTS/ticket-toggle-<name>-<theme>.png`. */
async function shots(page: Page, name: string) {
  const dir = process.env.E2E_SHOTS;
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  for (const theme of ["light", "dark"]) {
    await page.evaluate((t) => (document.documentElement.dataset.theme = t), theme);
    await page.waitForTimeout(150);
    await page.screenshot({ path: join(dir, `ticket-toggle-${name}-${theme}.png`) });
  }
  await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
}

test("a heading toggle with a nested list and a warning callout, kept and reopened after a reload", async ({ page, admin }) => {
  const id = await setup(admin);
  await logIn(page, `/notes/${id}`);
  await body(page).click();
  const menu = page.getByRole("listbox", { name: "Insert block" });

  // A heading toggle from the menu; its summary takes the caret.
  await page.keyboard.type("/toggle");
  await menu.getByRole("option", { name: "Toggle heading 2" }).click();
  await expect(body(page).getByTestId("toggle")).toHaveCount(1);
  const toggle = body(page).getByTestId("toggle").first();
  await page.keyboard.type("Packing list");
  await expect(toggle.locator('[data-toggle-summary][data-level="2"]')).toHaveText("Packing list");
  // Enter at the end of a closed summary adds a line after it; open, it starts the nested blocks.
  await toggle.getByTestId("toggle-button").click();
  await expect(toggle).toHaveAttribute("data-open", "true");
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await page.keyboard.type("- Books");
  await page.keyboard.press("Enter");
  await page.keyboard.type("Plates");
  await expect(toggle.locator("ul li")).toHaveText(["Books", "Plates"]);

  // `>> ` makes a plain toggle; a callout from the menu after it, of the kind chosen.
  await body(page).locator("p").last().click();
  await page.keyboard.press("Control+End");
  await page.keyboard.type(">> Later");
  await expect(body(page).getByTestId("toggle")).toHaveCount(2);
  await body(page).getByTestId("toggle").nth(1).locator("[data-toggle-summary]").click();
  await page.keyboard.press("End");
  // Closed: Enter goes on after the toggle.
  await page.keyboard.press("Enter");
  await page.keyboard.type("/callout");
  await menu.getByRole("option", { name: "Callout" }).click();
  await menu.getByRole("option", { name: "Warning" }).click();
  const callout = body(page).getByTestId("admonition");
  await expect(callout).toHaveAttribute("data-colour", "orange");
  await expect(callout).toHaveAttribute("data-icon", "triangle-alert");
  await page.keyboard.type("Mind the stairs");
  await saved(page);

  const first = await stored(admin, id);
  expect(first.body).toEqual([
    {
      kind: "toggle",
      fields: { level: 2, text: "Packing list" },
      blocks: [
        {
          kind: "list",
          fields: { style: "bullet" },
          blocks: [
            { kind: "item", fields: { text: "Books" } },
            { kind: "item", fields: { text: "Plates" } },
          ],
        },
      ],
    },
    { kind: "toggle", fields: { text: "Later" } },
    { kind: "admonition", fields: { kind: "warning" }, blocks: [{ kind: "paragraph", fields: { text: "Mind the stairs" } }] },
  ]);
  expect(first.wcl).toContain("toggle {");
  expect(first.wcl).toContain('kind = "warning"');
  await shots(page, "editor");

  // Closing and opening the toggle is not an edit: the stored version does not move.
  await toggle.getByTestId("toggle-button").click();
  await expect(toggle).toHaveAttribute("data-open", "false");
  await expect(toggle.locator("ul")).toBeHidden();
  await toggle.getByTestId("toggle-button").click();
  await expect(toggle.locator("ul")).toBeVisible();
  await page.waitForTimeout(1500);
  expect((await stored(admin, id)).version).toBe(first.version);

  // After a reload the toggle is as this device left it, open; the one never opened is closed.
  await page.reload();
  const again = body(page).getByTestId("toggle");
  await expect(again).toHaveCount(2);
  await expect(again.first()).toHaveAttribute("data-open", "true");
  await expect(again.first().locator("ul li")).toHaveText(["Books", "Plates"]);
  await expect(again.nth(1)).toHaveAttribute("data-open", "false");
  await expect(body(page).getByTestId("admonition")).toContainText("Mind the stairs");
  await expect(body(page).getByTestId("admonition").locator("[data-admonition-title]")).toHaveAttribute("data-placeholder", "Warning");
  await shots(page, "reloaded");

  // Closed, it stays closed through the next reload.
  await again.first().getByTestId("toggle-button").click();
  await page.reload();
  await expect(body(page).getByTestId("toggle").first()).toHaveAttribute("data-open", "false");
  await expect(body(page).getByTestId("toggle").first().locator("ul")).toBeHidden();
  await shots(page, "closed");
});

test("Tab moves a block into the callout above it and Shift+Tab takes it out again", async ({ page, admin }) => {
  const id = await setup(admin);
  await logIn(page, `/notes/${id}`);
  await body(page).click();
  const menu = page.getByRole("listbox", { name: "Insert block" });
  await page.keyboard.type("/callout");
  await menu.getByRole("option", { name: "Callout" }).click();
  await menu.getByRole("option", { name: "Tip" }).click();
  await page.keyboard.type("inside");
  await page.keyboard.press("Control+End");
  await page.keyboard.type("outside");
  await page.keyboard.press("Tab");
  await saved(page);
  expect((await stored(admin, id)).body).toEqual([
    {
      kind: "admonition",
      fields: { kind: "tip" },
      blocks: [
        { kind: "paragraph", fields: { text: "inside" } },
        { kind: "paragraph", fields: { text: "outside" } },
      ],
    },
  ]);
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.type("!");
  await saved(page);
  expect((await stored(admin, id)).body).toEqual([
    { kind: "admonition", fields: { kind: "tip" }, blocks: [{ kind: "paragraph", fields: { text: "inside" } }] },
    { kind: "paragraph", fields: { text: "outside!" } },
  ]);
});

test("a note opened from a search opens the toggle holding the hit", async ({ page, admin }) => {
  const word = `wombat${token()}`;
  const p = (text: string): Json => ({ kind: "paragraph", fields: { text } });
  const id = await setup(admin, [
    { kind: "toggle", fields: { text: "Nothing" }, blocks: [p("plain words")] },
    { kind: "toggle", fields: { text: "Found" }, blocks: [p(`the ${word} is here`)] },
  ]);
  await logIn(page, "/search");
  await activePane(page).getByRole("searchbox", { name: "Search notes" }).pressSequentially(word, { delay: 10 });
  await expect(page).toHaveURL(new RegExp(`/search\\?q=${word}$`));
  await expect(activePane(page).getByTestId("note-row")).toHaveCount(1);
  await activePane(page).getByTestId("note-row").first().click();
  await expect(page).toHaveURL(new RegExp(`/notes/${id}\\?find=`));
  const toggles = body(page).getByTestId("toggle");
  await expect(toggles).toHaveCount(2);
  await expect(toggles.nth(1)).toHaveAttribute("data-open", "true");
  await expect(body(page).getByText(`the ${word} is here`)).toBeVisible();
  await expect(toggles.nth(0)).toHaveAttribute("data-open", "false");
});
