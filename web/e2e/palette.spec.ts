// The command palette: Ctrl/Cmd+K from any page, notes found by title as you type, actions after
// `>`, recent notes before anything is typed, background tabs with Ctrl+Enter, and a phone's
// full-width sheet opened from the sidebar.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { APIRequestContext, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function createTask(admin: APIRequestContext, title: string): Promise<{ id: string }> {
  const res = await admin.post("/api/notes?type=task", { data: { note: { fields: { title } } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string };
}

const palette = (page: Page) => page.getByRole("dialog", { name: "Command palette" });
const box = (page: Page) => palette(page).getByRole("combobox", { name: "Search or run a command" });
const options = (page: Page) => palette(page).getByRole("option");
const tabs = (page: Page) => page.getByRole("tablist", { name: "Open tabs" }).getByRole("tab");

async function openPalette(page: Page) {
  await page.keyboard.press("ControlOrMeta+k");
  await expect(palette(page)).toBeVisible();
  await expect(box(page)).toBeFocused();
}

test("Ctrl+K opens the palette on a note and in settings; Escape closes it and gives focus back", async ({ page, admin }) => {
  const note = await createTask(admin, `Palette open ${token()}`);
  await logIn(page, `/notes/${note.id}`);
  await expect(activePane(page).getByRole("heading", { name: /Palette open/ })).toBeVisible();
  await openPalette(page);
  // An accessible combobox over a listbox, with the first option active.
  await expect(box(page)).toHaveAttribute("aria-controls", "palette-list");
  await expect(palette(page).getByRole("listbox", { name: "Results" })).toBeVisible();
  await expect(box(page)).toHaveAttribute("aria-activedescendant", "palette-option-0");
  // Ctrl+K again closes it.
  await page.keyboard.press("ControlOrMeta+k");
  await expect(palette(page)).toHaveCount(0);

  await page.goto("/settings/types");
  await expect(activePane(page).getByRole("navigation", { name: "Settings sections" })).toBeVisible();
  await openPalette(page);
  // Tab stays in the dialog.
  await page.keyboard.press("Tab");
  await expect(box(page)).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(palette(page)).toHaveCount(0);

  // Focus goes back to where it was: the Search page's box.
  await page.goto("/search");
  const search = activePane(page).getByRole("searchbox", { name: "Search notes" });
  await search.click();
  await openPalette(page);
  await page.keyboard.press("Escape");
  await expect(palette(page)).toHaveCount(0);
  await expect(search).toBeFocused();
});

test("typing finds a note by title; Enter opens it, Ctrl+Enter opens it in a background tab", async ({ page, admin }) => {
  const word = `zephyr${token()}`;
  const note = await createTask(admin, `${word} planning`);
  const other = await createTask(admin, `Another ${word} note`);
  await logIn(page, "/daily");
  await openPalette(page);
  await box(page).pressSequentially(word.slice(0, -2), { delay: 10 });
  // A prefix finds both; the one whose title starts with it is first, highlighted.
  await expect(options(page).first()).toContainText(`${word} planning`);
  await expect(options(page).nth(1)).toContainText(`Another ${word} note`);
  await expect(options(page).first().locator("mark").first()).toHaveText(word.slice(0, -2));
  await expect(options(page).first()).toContainText("Task");

  // Ctrl+Enter: a background tab, and the page stays.
  await page.keyboard.press("ArrowDown");
  await expect(options(page).nth(1)).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("ControlOrMeta+Enter");
  await expect(palette(page)).toHaveCount(0);
  await expect(tabs(page)).toHaveCount(2);
  await expect(tabs(page).nth(0)).toHaveAttribute("aria-selected", "true");
  await expect(tabs(page).nth(1)).toHaveAccessibleName(new RegExp(`Another ${word}`));
  await expect(page).toHaveURL(/\/planner\/day/);

  // Enter: in the active pane.
  await openPalette(page);
  await box(page).fill(word);
  await expect(options(page).first()).toContainText(`${word} planning`);
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(new RegExp(`/notes/${note.id}$`));
  await expect(activePane(page).getByRole("heading", { name: `${word} planning` })).toBeVisible();
  expect(other.id).not.toBe(note.id);
});

test("recent notes show before anything is typed, most recent first", async ({ page, admin }) => {
  const t = token();
  const first = await createTask(admin, `Recent one ${t}`);
  const second = await createTask(admin, `Recent two ${t}`);
  await logIn(page, `/notes/${first.id}`);
  await expect(activePane(page).getByRole("heading", { name: `Recent one ${t}` })).toBeVisible();
  await page.goto(`/notes/${second.id}`);
  await expect(activePane(page).getByRole("heading", { name: `Recent two ${t}` })).toBeVisible();
  await openPalette(page);
  const recent = palette(page).getByRole("group", { name: "Recent notes" }).getByRole("option");
  await expect(recent.nth(0)).toContainText(`Recent two ${t}`);
  await expect(recent.nth(1)).toContainText(`Recent one ${t}`);
  // Actions follow, with their shortcuts.
  const capture = palette(page).getByRole("option", { name: /Capture/ });
  await expect(capture.locator("kbd")).toHaveText("Ctrl+Shift+K");
  // A click opens one.
  await recent.nth(1).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${first.id}$`));
});

test("> lists actions: New Task opens the new task form, Go to Inbox the inbox, Capture its dialog", async ({ page }) => {
  await logIn(page, "/daily");
  await openPalette(page);
  await box(page).pressSequentially(">new task", { delay: 10 });
  await expect(palette(page).getByText("Commands only")).toBeVisible();
  await expect(options(page).first()).toContainText("New Task");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/\/notes\/new\?type=task$/);
  await expect(activePane(page).getByText("New Task", { exact: true })).toBeVisible();

  await openPalette(page);
  await box(page).pressSequentially("inbox", { delay: 10 });
  await expect(options(page).first()).toContainText("Go to Inbox");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/\/inbox$/);

  await openPalette(page);
  await box(page).pressSequentially(">capt", { delay: 10 });
  await expect(options(page).first()).toContainText("Capture");
  await page.keyboard.press("Enter");
  await expect(palette(page)).toHaveCount(0);
  const capture = page.getByRole("dialog", { name: "Capture" });
  await expect(capture).toBeVisible();
  await expect(capture.getByRole("textbox", { name: "Capture text" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(capture).toHaveCount(0);

  // Settings sections and a type's notes.
  await openPalette(page);
  await box(page).pressSequentially(">settings taxo", { delay: 10 });
  await expect(options(page).first()).toContainText("Settings: Taxonomies");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/\/settings\/taxonomies$/);
  await openPalette(page);
  await box(page).pressSequentially("go to tasks", { delay: 10 });
  await expect(options(page).first()).toContainText("Go to Tasks");
  await options(page).first().click();
  await expect(page).toHaveURL(/\/types\/task$/);
});

test("#type narrows the notes to one type", async ({ page, admin }) => {
  const word = `quillon${token()}`;
  await createTask(admin, `${word} task`);
  const res = await admin.post("/api/notes?type=note", { data: { note: { fields: { title: `${word} note` } } } });
  expect(res.status(), await res.text()).toBe(201);
  await logIn(page, "/daily");
  await openPalette(page);
  await box(page).pressSequentially(`#task ${word}`, { delay: 5 });
  await expect(palette(page).getByText("Notes of type Task")).toBeVisible();
  await expect(options(page).first()).toContainText(`${word} task`);
  await expect(palette(page).getByRole("option", { name: new RegExp(`${word} note`) })).toHaveCount(0);
});

test("on a phone the palette opens from the sidebar as a full-width sheet", async ({ page, admin }) => {
  const word = `phonepal${token()}`;
  await createTask(admin, `${word} errand`);
  await page.setViewportSize({ width: 390, height: 844 });
  await logIn(page, "/daily");
  await page.getByRole("button", { name: "Menu" }).click();
  await page.getByRole("navigation", { name: "Main" }).getByRole("button", { name: "Search or run a command" }).click();
  await expect(palette(page)).toBeVisible();
  await expect(box(page)).toBeFocused();
  const rect = await palette(page).boundingBox();
  expect(rect!.width).toBeGreaterThanOrEqual(389);
  expect(rect!.x).toBeLessThanOrEqual(1);
  // Split actions are not offered on a phone.
  await box(page).fill(">split");
  await expect(palette(page).getByText("Commands only")).toBeVisible();
  await expect(options(page).filter({ hasText: /Split right|Split down/ })).toHaveCount(0);
  await expect(palette(page).getByRole("option", { name: /Close pane/ })).toHaveCount(0);
  await box(page).fill(">close pane");
  await expect(options(page).first()).toContainText("Close pane");
  await box(page).fill(word);
  await expect(options(page).first()).toContainText(`${word} errand`);
  await options(page).first().click();
  await expect(palette(page)).toHaveCount(0);
  await expect(activePane(page).getByRole("heading", { name: `${word} errand` })).toBeVisible();
  // Cancel closes it.
  await page.getByRole("button", { name: "Menu" }).click();
  await page.getByRole("navigation", { name: "Main" }).getByRole("button", { name: "Search or run a command" }).click();
  await palette(page).getByRole("button", { name: "Cancel" }).click();
  await expect(palette(page)).toHaveCount(0);
});

test("a button at the top of the window opens the palette: the top bar on a phone, the tab bar on a desktop", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await logIn(page, "/daily");
  const top = page.locator("header.topbar").getByRole("button", { name: "Search or run a command" });
  await expect(top).toBeVisible();
  const r = (await top.boundingBox())!;
  expect(r.y).toBeLessThan(60);
  await top.click();
  await expect(palette(page)).toBeVisible();
  await expect(box(page)).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(palette(page)).toHaveCount(0);

  await page.setViewportSize({ width: 1280, height: 800 });
  await expect(top).toBeHidden();
  const tabbar = page.getByRole("tablist", { name: "Open tabs" }).getByRole("button", { name: "Search or run a command" });
  await expect(tabbar).toBeVisible();
  await tabbar.click();
  await expect(palette(page)).toBeVisible();
  await expect(box(page)).toBeFocused();
});

test("screenshots of the palette with notes and actions", async ({ page, admin }) => {
  const dir = process.env.E2E_SHOTS;
  test.skip(!dir, "E2E_SHOTS is not set");
  const t = token();
  for (const title of [`Newsletter draft ${t}`, `New hire onboarding ${t}`, `News digest ${t}`]) await createTask(admin, title);
  mkdirSync(dir!, { recursive: true });
  await logIn(page, "/daily");
  for (const [name, size] of [
    ["desktop", { width: 1280, height: 800 }],
    ["phone", { width: 390, height: 844 }],
  ] as const) {
    await page.setViewportSize(size);
    await page.goto("/daily");
    await expect(page.getByRole("navigation", { name: "Main" })).toBeAttached();
    await openPaletteFrom(page, name === "phone");
    await box(page).pressSequentially("new", { delay: 20 });
    await expect(options(page).filter({ hasText: t })).toHaveCount(3);
    await expect(palette(page).getByText("Searching…")).toHaveCount(0);
    for (const theme of ["light", "dark"]) {
      await page.evaluate((x) => (document.documentElement.dataset.theme = x), theme);
      await page.screenshot({ path: join(dir!, `palette-${name}-${theme}.png`) });
    }
    await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
    await page.keyboard.press("Escape");
  }
});

async function openPaletteFrom(page: Page, phone: boolean) {
  if (!phone) return openPalette(page);
  await page.getByRole("button", { name: "Menu" }).click();
  await page.getByRole("navigation", { name: "Main" }).getByRole("button", { name: "Search or run a command" }).click();
  await expect(box(page)).toBeFocused();
}
