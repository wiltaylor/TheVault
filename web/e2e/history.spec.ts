import type { APIRequestContext, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// A note's history in the right sidebar: versions with authors and what changed, a version
// previewed read-only in the note's own pane, restoring one, and the old history address (D13).

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

/** A type whose template places status and body, shows {{status}} in a fixed block, and leaves owner a property. */
const TYPE_WCL = `label = "Memo"
plural = "Memos"

field status {
  kind    = "select"
  label   = "Status"
  options = ["idea", "active", "done"]
}

field owner {
  kind  = "string"
  label = "Owner"
}

field body {
  kind  = "content"
  label = "Body"
}

template default {
  tab "Main" {
    slot "title"
    paragraph {
      text = "Status is {{status}}."
    }
    slot "status"
    slot "body"
  }
}
`;

const para = (text: string) => ({ kind: "paragraph", fields: { text } });

function memo(title: string, status: string, body: string) {
  return { fields: { title, status, owner: "Kim" }, blocks: [{ kind: "content", labels: ["body"], blocks: [para(body)] }] };
}

/** A note with three versions, written by the admin token so they never merge. */
async function threeVersions(admin: APIRequestContext) {
  const type = `memo_${token()}`;
  const put = await admin.put(`/api/types/${type}`, { data: { wcl: TYPE_WCL } });
  expect(put.status(), await put.text()).toBe(200);
  const t = token();
  const created = await admin.post(`/api/notes?type=${type}`, { data: { note: memo(`Alpha ${t}`, "idea", "First body") } });
  expect(created.status(), await created.text()).toBe(201);
  let note = (await created.json()) as { id: string; version: string };
  for (const doc of [memo(`Alpha ${t}`, "active", "Second body"), memo(`Gamma ${t}`, "done", "Third body")]) {
    const res = await admin.put(`/api/notes/${note.id}`, { data: { note: doc, base_version: note.version } });
    expect(res.status(), await res.text()).toBe(200);
    note = (await res.json()) as { id: string; version: string };
  }
  return { id: note.id, type, t };
}

async function stored(admin: APIRequestContext, id: string) {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return (await res.json()) as { version: string; note: { fields: Record<string, unknown>; blocks: { blocks?: { fields?: { text?: string } }[] }[] } };
}

const historyPanel = (page: Page) => page.getByRole("complementary", { name: "History" });
const versions = (page: Page) => historyPanel(page).getByRole("list", { name: "Versions" }).getByRole("listitem");
const banner = (page: Page) => page.getByRole("region", { name: "Version preview" });
const title = (page: Page) => activePane(page).getByLabel("Title", { exact: true });

test("the History icon opens the panel, which lists versions with authors and what changed", async ({ page, admin }) => {
  const { id, t } = await threeVersions(admin);
  await logIn(page, `/notes/${id}`);
  await expect(historyPanel(page)).toHaveCount(0);
  const button = activePane(page).getByRole("button", { name: "History", exact: true });
  await expect(button).toHaveAttribute("aria-expanded", "false");
  await button.click();
  await expect(historyPanel(page)).toBeVisible();
  await expect(button).toHaveAttribute("aria-expanded", "true");
  await expect(page.getByRole("tab", { name: "History" })).toHaveAttribute("aria-selected", "true");
  // The pane did not move.
  await expect(page).toHaveURL(new RegExp(`/notes/${id}$`));

  await expect(versions(page)).toHaveCount(3);
  await expect(versions(page).first()).toContainText("current");
  for (let i = 0; i < 3; i++) await expect(versions(page).nth(i)).toContainText("by token admin");
  await expect(versions(page).nth(0).getByTestId("change-summary")).toHaveText("changed: title, status, body");
  await expect(versions(page).nth(1).getByTestId("change-summary")).toHaveText("changed: status, body");
  await expect(versions(page).nth(2).getByTestId("change-summary")).toHaveText("created");
  await expect(versions(page).nth(0)).toContainText("just now");
  await expect(title(page)).toHaveText(`Gamma ${t}`);

  // The icon closes it again.
  await button.click();
  await expect(historyPanel(page)).toHaveCount(0);
});

test("a chosen version shows read-only in the pane, Arrow Down steps to an older one, and Back to current resumes editing", async ({
  page,
  admin,
}) => {
  const { id, t } = await threeVersions(admin);
  await logIn(page, `/notes/${id}`);
  // An unsaved edit is saved before the preview starts.
  await title(page).fill(`Gamma edited ${t}`);
  await activePane(page).getByRole("button", { name: "History", exact: true }).click();
  await expect(versions(page)).toHaveCount(4);
  expect((await stored(admin, id)).note.fields.title).toBe(`Gamma edited ${t}`);
  await expect(versions(page).nth(0).getByTestId("change-summary")).toHaveText("changed: title");
  await expect(versions(page).nth(0)).toContainText("by you");

  await versions(page).nth(2).getByRole("button").first().click();
  await expect(banner(page)).toBeVisible();
  await expect(banner(page)).toContainText("Viewing the version from");
  await expect(banner(page)).toContainText("by token admin");
  const pane = activePane(page);
  await expect(title(page)).toHaveText(`Alpha ${t}`);
  await expect(pane.getByText("Status is active.")).toBeVisible();
  await expect(pane.getByTestId("field-status").getByLabel("Status")).toHaveValue("active");
  await expect(pane.getByTestId("field-body")).toContainText("Second body");

  // Nothing in the pane edits.
  await expect(title(page)).not.toBeEditable();
  await expect(pane.getByTestId("field-status").getByLabel("Status")).toBeDisabled();
  await expect(pane.getByTestId("field-body").locator("[contenteditable='true']")).toHaveCount(0);
  await title(page).press("End");
  await page.keyboard.type("typed");
  await expect(title(page)).toHaveText(`Alpha ${t}`);

  // Arrow Down in the list shows the next older version.
  await versions(page).nth(2).getByRole("button").first().focus();
  await page.keyboard.press("ArrowDown");
  await expect(versions(page).nth(3).getByRole("button").first()).toBeFocused();
  await expect(pane.getByText("Status is idea.")).toBeVisible();
  await expect(pane.getByTestId("field-body")).toContainText("First body");
  await expect(versions(page).nth(3)).toHaveClass(/selected/);
  await page.keyboard.press("ArrowUp");
  await expect(pane.getByText("Status is active.")).toBeVisible();

  // Back to current: the live note, editable, saving again.
  await banner(page).getByRole("button", { name: "Back to current" }).click();
  await expect(banner(page)).toHaveCount(0);
  await expect(title(page)).toHaveText(`Gamma edited ${t}`);
  await expect(title(page)).toBeEditable();
  await expect(pane.getByText("Status is done.")).toBeVisible();
  await title(page).fill(`Gamma again ${t}`);
  await title(page).press("Enter");
  await expect(pane.getByTestId("save-status")).toHaveText("Saved");
  expect((await stored(admin, id)).note.fields.title).toBe(`Gamma again ${t}`);
  // Nothing the preview showed was saved over the note.
  expect((await stored(admin, id)).note.fields.status).toBe("done");

  // Escape ends a preview too, and so does switching the panel.
  await versions(page).nth(1).getByRole("button").first().click();
  await expect(banner(page)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(banner(page)).toHaveCount(0);
  await versions(page).nth(1).getByRole("button").first().click();
  await expect(banner(page)).toBeVisible();
  await page.getByRole("tab", { name: "Properties" }).click();
  await expect(banner(page)).toHaveCount(0);
  await expect(page.getByRole("complementary", { name: "Properties" }).getByLabel("Owner")).toBeEditable();
  await page.getByRole("tab", { name: "History" }).click();
  await versions(page).nth(1).getByRole("button").first().click();
  await expect(banner(page)).toBeVisible();
  await page.getByRole("button", { name: "Hide right sidebar" }).click();
  await expect(banner(page)).toHaveCount(0);
  await expect(title(page)).toBeEditable();
});

test("restoring a version makes its content current and ends the preview", async ({ page, admin }) => {
  const { id, t } = await threeVersions(admin);
  await logIn(page, `/notes/${id}`);
  await activePane(page).getByRole("button", { name: "History", exact: true }).click();
  await expect(versions(page)).toHaveCount(3);
  await versions(page).nth(2).getByRole("button").first().click();
  await expect(banner(page)).toBeVisible();

  // Declining the confirmation changes nothing.
  page.once("dialog", (d) => void d.dismiss());
  await banner(page).getByRole("button", { name: "Restore this version" }).click();
  await expect(banner(page)).toBeVisible();
  expect((await stored(admin, id)).note.fields.title).toBe(`Gamma ${t}`);

  page.once("dialog", (d) => void d.accept());
  await banner(page).getByRole("button", { name: "Restore this version" }).click();
  await expect(banner(page)).toHaveCount(0);
  await expect(title(page)).toHaveText(`Alpha ${t}`);
  await expect(title(page)).toBeEditable();
  await expect(activePane(page).getByText("Status is idea.")).toBeVisible();
  const now = await stored(admin, id);
  expect(now.note.fields.title).toBe(`Alpha ${t}`);
  expect(now.note.fields.status).toBe("idea");
  expect(now.note.blocks[0]?.blocks?.[0]?.fields?.text).toBe("First body");
  // A new version, on top of the history.
  await expect(versions(page)).toHaveCount(4);
  await expect(versions(page).first()).toContainText("current");
  await expect(versions(page).first()).toContainText("by you");
  await expect(versions(page).first().getByTestId("change-summary")).toHaveText("changed: title, status, body");

  // From the list's own button too.
  await versions(page).nth(1).getByRole("button").first().click();
  await expect(banner(page)).toBeVisible();
  page.once("dialog", (d) => void d.accept());
  await versions(page).nth(1).getByRole("button", { name: "Restore", exact: true }).click();
  await expect(banner(page)).toHaveCount(0);
  await expect(title(page)).toHaveText(`Gamma ${t}`);
  expect((await stored(admin, id)).note.fields.status).toBe("done");
});

test("the old history address opens the note with the History panel", async ({ page, admin }) => {
  const { id, t } = await threeVersions(admin);
  await logIn(page, `/notes/${id}/history`);
  await expect(page).toHaveURL(new RegExp(`/notes/${id}$`));
  await expect(title(page)).toHaveText(`Gamma ${t}`);
  await expect(historyPanel(page)).toBeVisible();
  await expect(versions(page)).toHaveCount(3);
});

test("the History panel follows the active pane", async ({ page, admin }) => {
  const one = await threeVersions(admin);
  const two = await threeVersions(admin);
  await logIn(page, `/notes/${one.id}`);
  await activePane(page).getByRole("button", { name: "History", exact: true }).click();
  await expect(versions(page)).toHaveCount(3);
  const first = await versions(page).first().getAttribute("data-version");

  await activePane(page).getByRole("button", { name: "Pane actions" }).click();
  await page.getByRole("menuitem", { name: "Split right" }).click();
  const panes = page.locator(".pane").filter({ visible: true });
  await expect(panes).toHaveCount(2);
  await panes.nth(1).locator(".pane-body").click();
  await page.waitForTimeout(300);
  await page.goto(`/notes/${two.id}`);
  await expect(panes.nth(1).getByLabel("Title", { exact: true })).toHaveText(`Gamma ${two.t}`);
  await expect(versions(page)).toHaveCount(3);
  await expect(versions(page).first()).not.toHaveAttribute("data-version", first!);

  // A preview in one pane ends when the other becomes active.
  await versions(page).nth(2).getByRole("button").first().click();
  await expect(panes.nth(1).getByRole("region", { name: "Version preview" })).toBeVisible();
  await panes.nth(0).locator(".pane-header").click();
  await expect(versions(page).first()).toHaveAttribute("data-version", first!);
  await expect(page.getByRole("region", { name: "Version preview" })).toHaveCount(0);
  await expect(panes.nth(1).getByLabel("Title", { exact: true })).toHaveText(`Gamma ${two.t}`);

  // A pane that shows no note.
  await page.getByRole("link", { name: "Settings" }).click();
  await expect(panes.nth(0).getByRole("heading", { name: "General settings" })).toBeVisible();
  await expect(historyPanel(page)).toHaveText("Open a note to see its history.");
});

test("on a phone, choosing a version closes the drawer, and the banner reopens it", async ({ page, admin }) => {
  const { id, t } = await threeVersions(admin);
  await page.setViewportSize({ width: 390, height: 844 });
  await logIn(page, `/notes/${id}`);
  await activePane(page).getByRole("button", { name: "History", exact: true }).click();
  await expect(historyPanel(page)).toBeInViewport();
  await versions(page).nth(1).getByRole("button").first().click();
  await expect(historyPanel(page)).toBeHidden();
  await expect(banner(page)).toBeVisible();
  await expect(title(page)).toHaveText(`Alpha ${t}`);
  await banner(page).getByRole("button", { name: "Versions" }).click();
  await expect(historyPanel(page)).toBeInViewport();
  await expect(versions(page).nth(1)).toHaveClass(/selected/);
});
