import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// The tabbed, split-pane workspace, and the sidebars around it.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

/** A type of its own: `owner` is placed nowhere, so only the properties sidebar edits it, and a paragraph shows it. */
const TYPE_WCL = `label = "Desk"
plural = "Desks"

field owner {
  kind  = "string"
  label = "Owner"
}

template default {
  tab "Main" {
    slot "title"
    paragraph {
      text = "Owned by {{owner}}."
    }
  }
}
`;

async function deskType(admin: APIRequestContext): Promise<string> {
  const name = `desk_${token()}`;
  const put = await admin.put(`/api/types/${name}`, { data: { wcl: TYPE_WCL } });
  expect(put.status(), await put.text()).toBe(200);
  return name;
}

async function desk(admin: APIRequestContext, type: string, title: string, owner = "Kim") {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields: { title, owner } } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string };
}

const tabBar = (page: Page) => page.getByRole("tablist", { name: "Open tabs" });
const tabs = (page: Page) => tabBar(page).getByRole("tab");
const tabNames = (page: Page) => tabs(page).evaluateAll((els) => els.map((e) => e.getAttribute("aria-label")));
/** The panes on screen, in layout order. */
const panes = (page: Page) => page.locator(".pane").filter({ visible: true });
const title = (pane: Locator) => pane.getByTestId("pane-title");

async function paneAction(page: Page, pane: Locator, action: string) {
  await pane.getByRole("button", { name: "Pane actions" }).click();
  await page.getByRole("menuitem", { name: action }).click();
}

async function box(l: Locator) {
  const b = await l.boundingBox();
  expect(b).not.toBeNull();
  return b!;
}

/** Drag from the middle of `from` to a point given as fractions of `to`'s box, in steps. */
async function drag(page: Page, from: Locator, to: Locator, fx = 0.5, fy = 0.5) {
  const a = await box(from);
  const b = await box(to);
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
  await page.mouse.down();
  await page.mouse.move(b.x + b.width * fx, b.y + b.height * fy, { steps: 8 });
  await page.mouse.up();
}

test("a link opens in the pane it is in; Ctrl+click and middle-click open background tabs", async ({ page, admin }) => {
  const type = await deskType(admin);
  const a = await desk(admin, type, `Alpha ${token()}`);
  const b = await desk(admin, type, `Beta ${token()}`);
  await logIn(page, `/types/${type}`);
  await paneAction(page, activePane(page), "Split right");
  await expect(panes(page)).toHaveCount(2);
  const [left, right] = [panes(page).nth(0), panes(page).nth(1)];
  // The new pane shows the list too; open the planner in the left one.
  await left.click();
  await page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: "Planner" }).click();
  await expect(left.getByRole("navigation", { name: "Day" })).toBeVisible();

  // A row in the right pane opens there; the left pane keeps its page.
  const rowA = right.getByTestId("note-card").filter({ hasText: "Alpha" });
  await rowA.click();
  await expect(right.getByLabel("Title", { exact: true })).toHaveText(/^Alpha /);
  await expect(left.getByRole("navigation", { name: "Day" })).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`/notes/${a.id}$`));

  // Back in the right pane, Ctrl+click and middle-click open background tabs.
  await right.getByRole("button", { name: "Back" }).click();
  await expect(right.getByTestId("note-card").first()).toBeVisible();
  await right.getByTestId("note-card").filter({ hasText: "Beta" }).click({ modifiers: ["ControlOrMeta"] });
  await expect(tabs(page)).toHaveCount(2);
  await right.getByTestId("note-card").filter({ hasText: "Alpha" }).click({ button: "middle" });
  await expect(tabs(page)).toHaveCount(3);
  // Neither left the current tab or pane.
  await expect(tabs(page).nth(0)).toHaveAttribute("aria-selected", "true");
  await expect(page).toHaveURL(new RegExp(`/types/${type}$`));
  await expect(tabs(page).nth(1)).toHaveAccessibleName(/^Beta /);
  await expect(tabs(page).nth(2)).toHaveAccessibleName(/^Alpha /);

  await tabs(page).nth(1).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${b.id}$`));
  await expect(activePane(page).getByLabel("Title", { exact: true })).toHaveText(/^Beta /);
});

test("split right and down give panes that navigate on their own, and closing collapses the splits", async ({ page }) => {
  await logIn(page, "/settings");
  await paneAction(page, activePane(page), "Split right");
  await paneAction(page, activePane(page), "Split down");
  await expect(panes(page)).toHaveCount(3);
  const [p1, p2, p3] = [panes(page).nth(0), panes(page).nth(1), panes(page).nth(2)];
  const b1 = await box(p1);
  const b2 = await box(p2);
  const b3 = await box(p3);
  expect(b2.x).toBeGreaterThan(b1.x + b1.width / 2);
  expect(Math.abs(b3.x - b2.x)).toBeLessThan(2);
  expect(b3.y).toBeGreaterThan(b2.y + b2.height / 2);

  // Each navigates by itself.
  await p3.getByRole("navigation", { name: "Settings sections" }).getByRole("link", { name: "Types" }).click();
  await expect(p3.getByRole("heading", { name: "Types", exact: true })).toBeVisible();
  await expect(p2.getByRole("heading", { name: "General settings" })).toBeVisible();
  await expect(p1.getByRole("heading", { name: "General settings" })).toBeVisible();
  await p3.getByRole("button", { name: "Back" }).click();
  await expect(p3.getByRole("heading", { name: "General settings" })).toBeVisible();
  await p3.getByRole("button", { name: "Forward" }).click();
  await expect(p3.getByRole("heading", { name: "Types", exact: true })).toBeVisible();

  // Closing the bottom pane gives its space back to its sibling, then the row closes up too.
  await paneAction(page, p3, "Close pane");
  await expect(panes(page)).toHaveCount(2);
  const after = await box(panes(page).nth(1));
  expect(after.height).toBeGreaterThan(b2.height * 1.6);
  await paneAction(page, panes(page).nth(1), "Close pane");
  await expect(panes(page)).toHaveCount(1);
  expect((await box(panes(page).nth(0))).width).toBeGreaterThan(b1.width * 1.6);
});

test("dragging a divider changes the split, and a note page widens with its pane", async ({ page, admin }) => {
  const type = await deskType(admin);
  const n = await desk(admin, type, `Wide ${token()}`);
  await logIn(page, `/notes/${n.id}`);
  await paneAction(page, activePane(page), "Split right");
  const left = panes(page).nth(0);
  const divider = page.getByRole("separator", { name: "Resize panes" });
  await expect(divider).toHaveAttribute("aria-valuenow", "50");
  const before = await box(left.locator(".note-view"));

  const d = await box(divider);
  const area = await box(page.locator(".ws-area"));
  await page.mouse.move(d.x + d.width / 2, d.y + d.height / 2);
  await page.mouse.down();
  await page.mouse.move(area.x + area.width * 0.75, d.y + d.height / 2, { steps: 6 });
  await page.mouse.up();
  await expect(divider).toHaveAttribute("aria-valuenow", "75");
  const after = await box(left.locator(".note-view"));
  expect(after.width).toBeGreaterThan(before.width * 1.3);

  // The keyboard moves it too.
  await divider.focus();
  await page.keyboard.press("ArrowLeft");
  await expect(divider).toHaveAttribute("aria-valuenow", "73");
});

test("dragging a pane onto another's edge docks it there, and onto a tab moves it into that tab", async ({ page }) => {
  await logIn(page, "/settings");
  await paneAction(page, activePane(page), "Split right");
  const [p1, p2] = [panes(page).nth(0), panes(page).nth(1)];
  await p2.getByRole("navigation", { name: "Settings sections" }).getByRole("link", { name: "Types" }).click();

  // The Types pane onto the bottom edge of the other: they stack.
  await drag(page, p2.locator(".pane-header"), p1.locator(".pane-body"), 0.5, 0.95);
  await expect(panes(page)).toHaveCount(2);
  const top = await box(panes(page).nth(0));
  const bottom = await box(panes(page).nth(1));
  expect(bottom.y).toBeGreaterThan(top.y + top.height / 2);
  expect(Math.abs(bottom.x - top.x)).toBeLessThan(2);
  await expect(panes(page).nth(1).getByRole("heading", { name: "Types", exact: true })).toBeVisible();

  // A second tab; then the Types pane onto it.
  await page.getByRole("button", { name: "New tab" }).click();
  await expect(tabs(page)).toHaveCount(2);
  await tabs(page).nth(0).click();
  await expect(panes(page)).toHaveCount(2);
  await drag(page, panes(page).nth(1).locator(".pane-header"), tabs(page).nth(1));
  await expect(tabs(page).nth(1)).toHaveAttribute("aria-selected", "true");
  await expect(panes(page)).toHaveCount(2);
  // It sits to the right of that tab's pane.
  const types = panes(page).filter({ has: page.getByRole("heading", { name: "Types", exact: true }) });
  const daily = panes(page).filter({ has: page.getByRole("navigation", { name: "Day" }) });
  await expect(types).toBeVisible();
  await expect(daily).toBeVisible();
  expect((await box(types)).x).toBeGreaterThan((await box(daily)).x + 100);
  await tabs(page).nth(0).click();
  await expect(panes(page)).toHaveCount(1);
  await expect(panes(page).nth(0).getByRole("heading", { name: "General settings" })).toBeVisible();

  // Onto the empty tab bar: a tab of its own.
  await tabs(page).nth(1).click();
  const barBox = await box(tabBar(page));
  const h = await box(types.locator(".pane-header"));
  await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2);
  await page.mouse.down();
  await page.mouse.move(barBox.x + barBox.width - 40, barBox.y + barBox.height / 2, { steps: 8 });
  await expect(page.locator(".ws-tab-ghost")).toBeVisible();
  await page.mouse.up();
  await expect(tabs(page)).toHaveCount(3);
  await expect(tabs(page).nth(2)).toHaveAttribute("aria-selected", "true");
  await expect(panes(page)).toHaveCount(1);

  // Escape cancels a drag.
  await tabs(page).nth(1).click();
  await paneAction(page, activePane(page), "Split right");
  const src = await box(panes(page).nth(1).locator(".pane-header"));
  const dst = await box(panes(page).nth(0).locator(".pane-body"));
  await page.mouse.move(src.x + src.width / 2, src.y + src.height / 2);
  await page.mouse.down();
  await page.mouse.move(dst.x + 10, dst.y + dst.height / 2, { steps: 6 });
  await expect(page.locator(".ws-drop-preview")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".ws-drop-preview")).toHaveCount(0);
  await page.mouse.up();
  const l = await box(panes(page).nth(0));
  const r = await box(panes(page).nth(1));
  expect(r.x).toBeGreaterThan(l.x + l.width / 2);
});

test("the address follows the active pane, browser back works in it, and reload restores the workspace", async ({ page }) => {
  await logIn(page, "/settings");
  await paneAction(page, activePane(page), "Split right");
  const [p1, p2] = [panes(page).nth(0), panes(page).nth(1)];
  await p2.getByRole("navigation", { name: "Settings sections" }).getByRole("link", { name: "Types" }).click();
  await expect(page).toHaveURL(/\/settings\/types$/);
  await p2.getByRole("navigation", { name: "Settings sections" }).getByRole("link", { name: "Scheduled" }).click();
  await expect(page).toHaveURL(/\/settings\/scheduled$/);

  await p1.locator(".pane-header").click();
  await expect(page).toHaveURL(/\/settings$/);
  await p2.locator(".pane-header").click();
  await expect(page).toHaveURL(/\/settings\/scheduled$/);

  // The browser's back button steps the active pane back; the other pane stays.
  await page.goBack();
  await expect(page).toHaveURL(/\/settings\/types$/);
  await expect(p2.getByRole("heading", { name: "Types", exact: true })).toBeVisible();
  await expect(p1.getByRole("heading", { name: "General settings" })).toBeVisible();
  // Alt+Left and Alt+Right act on the active pane.
  await page.keyboard.press("Alt+ArrowLeft");
  await expect(p2.getByRole("heading", { name: "General settings" })).toBeVisible();
  await page.keyboard.press("Alt+ArrowRight");
  await expect(p2.getByRole("heading", { name: "Types", exact: true })).toBeVisible();

  // A second tab, then reload: both tabs, the split and each pane's page come back.
  await page.getByRole("button", { name: "New tab" }).click();
  await expect(page).toHaveURL(/\/planner$/);
  await tabs(page).nth(0).click();
  await page.waitForTimeout(300);
  await page.reload();
  await expect(tabs(page)).toHaveCount(2);
  await expect(panes(page)).toHaveCount(2);
  await expect(page).toHaveURL(/\/settings\/types$/);
  await expect(panes(page).nth(0).getByRole("heading", { name: "General settings" })).toBeVisible();
  await expect(panes(page).nth(1).getByRole("heading", { name: "Types", exact: true })).toBeVisible();
  await expect(panes(page).nth(1)).toHaveAttribute("data-active", "true");
  // Its history came back too.
  await panes(page).nth(1).getByRole("button", { name: "Forward" }).click();
  await expect(panes(page).nth(1).getByRole("heading", { name: "Scheduled" })).toBeVisible();
  await tabs(page).nth(1).click();
  await expect(panes(page).nth(0).getByRole("navigation", { name: "Day" })).toBeVisible();

  // Alt+W closes the active pane; Alt+PageUp switches tab.
  await page.keyboard.press("Alt+PageUp");
  await expect(tabs(page).nth(0)).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Alt+w");
  await expect(panes(page)).toHaveCount(1);
});

test("tabs reorder by dragging, close with a middle-click or their ×, and closing the last leaves the area empty", async ({ page }) => {
  await logIn(page, "/settings");
  await page.getByRole("button", { name: "New tab" }).click();
  await page.getByRole("button", { name: "New tab" }).click();
  await page.getByRole("link", { name: "Trash" }).click();
  await expect.poll(() => tabNames(page)).toEqual(["Settings", "Planner", "Trash"]);
  await expect(page).toHaveTitle(/^Trash/);
  await drag(page, tabs(page).nth(2), tabs(page).nth(0), 0.1, 0.5);
  await expect.poll(() => tabNames(page)).toEqual(["Trash", "Settings", "Planner"]);
  await tabs(page).nth(1).click({ button: "middle" });
  await expect.poll(() => tabNames(page)).toEqual(["Trash", "Planner"]);
  // Every tab shows its ×, not only the active one.
  await expect(tabs(page).nth(1).getByRole("button", { name: "Close tab" })).toBeVisible();
  expect(Number(await tabs(page).nth(1).getByRole("button", { name: "Close tab" }).evaluate((el) => getComputedStyle(el).opacity))).toBeGreaterThan(0.3);
  await tabs(page).nth(1).getByRole("button", { name: "Close tab" }).click();
  await tabs(page).nth(0).getByRole("button", { name: "Close tab" }).click();

  // No tabs: an empty area, the bare root address, and it stays that way through a reload.
  await expect(tabs(page)).toHaveCount(0);
  await expect(page.locator("[data-pane-id]")).toHaveCount(0);
  await expect(page.locator(".ws-area")).toHaveClass(/ws-empty/);
  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByRole("button", { name: "New tab" })).toBeVisible();
  await page.reload();
  await expect(page.getByRole("button", { name: "New tab" })).toBeVisible();
  await expect(tabs(page)).toHaveCount(0);

  // Opening something from the sidebar gives it a tab of its own.
  await page.getByRole("link", { name: "Planner" }).click();
  await expect.poll(() => tabNames(page)).toEqual(["Planner"]);
  await expect(activePane(page).getByRole("navigation", { name: "Day" })).toBeVisible();
});

test("a pane on a missing note says so, and the rest of the workspace restores", async ({ page }) => {
  await logIn(page, "/notes/01ZZZZZZZZZZZZZZZZZZZZZZZZ");
  await expect(activePane(page).getByRole("alert")).toHaveText("This note does not exist.");
  await paneAction(page, activePane(page), "Split right");
  await page.waitForTimeout(300);
  await page.reload();
  await expect(panes(page)).toHaveCount(2);
  await expect(panes(page).nth(0).getByRole("alert")).toHaveText("This note does not exist.");
});

test("the same note in two panes: an edit in one shows in the other", async ({ page, admin }) => {
  const type = await deskType(admin);
  const n = await desk(admin, type, `Twice ${token()}`);
  await logIn(page, `/notes/${n.id}`);
  await paneAction(page, activePane(page), "Split right");
  const [p1, p2] = [panes(page).nth(0), panes(page).nth(1)];
  await expect(p2.getByLabel("Title", { exact: true })).toHaveText(/^Twice /);
  const renamed = `Renamed ${token()}`;
  await p1.getByLabel("Title", { exact: true }).fill(renamed);
  await expect(p1.getByTestId("save-status")).toHaveText("Saved");
  await expect(p2.getByLabel("Title", { exact: true })).toHaveText(renamed);
  // The tab and both pane headers follow the title.
  await expect(title(p2)).toHaveText(renamed);
  await expect(tabs(page).nth(0)).toHaveAccessibleName(renamed);
  // And the other pane saves over the new version without a conflict.
  await p2.getByLabel("Title", { exact: true }).fill(`${renamed} again`);
  await expect(p2.getByTestId("save-status")).toHaveText("Saved");
  await expect(p1.getByLabel("Title", { exact: true })).toHaveText(`${renamed} again`);
});

test.describe("on a phone", () => {
  test("a split made on a wide screen shows one pane at a time, with a switcher", async ({ page }) => {
    await logIn(page, "/settings");
    await paneAction(page, activePane(page), "Split right");
    await panes(page).nth(1).getByRole("navigation", { name: "Settings sections" }).getByRole("link", { name: "Types" }).click();
    await page.setViewportSize({ width: 390, height: 844 });
    await expect(panes(page)).toHaveCount(1);
    await expect(panes(page).nth(0).getByRole("heading", { name: "Types", exact: true })).toBeVisible();
    await expect(page.getByRole("separator", { name: "Resize panes" })).toHaveCount(0);

    const switcher = page.getByRole("button", { name: "Switch pane" });
    await switcher.click();
    const list = page.getByRole("listbox", { name: "Panes in this tab" });
    await expect(list.getByRole("option")).toHaveCount(2);
    await expect(list.getByRole("option", { selected: true })).toHaveText("Settings");
    await list.getByRole("option").nth(0).click();
    await expect(panes(page)).toHaveCount(1);
    await expect(panes(page).nth(0).getByRole("heading", { name: "General settings" })).toBeVisible();
    await expect(page).toHaveURL(/\/settings$/);
    // No splitting on a phone; tabs still work.
    await page.getByRole("button", { name: "Pane actions" }).click();
    await expect(page.getByRole("menuitem", { name: "Split right" })).toHaveCount(0);
    await page.keyboard.press("Escape");
    await page.getByRole("button", { name: "New tab" }).click();
    await expect(tabs(page)).toHaveCount(2);
  });
});

test.describe("sidebars", () => {
  test("each sidebar collapses and expands, and stays so across a reload", async ({ page }) => {
    await logIn(page, "/settings");
    const nav = page.getByRole("navigation", { name: "Main" });
    const props = page.getByRole("complementary", { name: "Properties" });
    await expect(nav).toBeVisible();
    await expect(props).toHaveCount(0);
    const paneWidth = (await box(activePane(page))).width;

    await page.getByRole("button", { name: "Hide sidebar" }).click();
    await expect(nav).toBeHidden();
    expect((await box(activePane(page))).width).toBeGreaterThan(paneWidth + 100);
    await page.reload();
    await expect(page.getByRole("button", { name: "Show sidebar" })).toBeVisible();
    await expect(nav).toBeHidden();
    await page.getByRole("button", { name: "Show sidebar" }).click();
    await expect(nav).toBeVisible();

    await page.getByRole("button", { name: "Show right sidebar" }).click();
    await expect(props).toBeVisible();
    await expect(props).toHaveText("Open a note to see its properties.");
    await page.reload();
    await expect(props).toBeVisible();
    await expect(nav).toBeVisible();
    await page.getByRole("button", { name: "Hide right sidebar" }).click();
    await expect(props).toHaveCount(0);
    await page.reload();
    await expect(nav).toBeVisible();
    await expect(props).toHaveCount(0);

    // The keyboard does the same.
    await page.keyboard.press("Control+Backslash");
    await expect(nav).toBeHidden();
    await page.keyboard.press("Control+Backslash");
    await expect(nav).toBeVisible();
    await page.keyboard.press("Control+Shift+Backslash");
    await expect(props).toBeVisible();

    // The width follows the dragged edge, and is kept.
    const edge = page.getByRole("separator", { name: "Resize sidebar" });
    const before = (await box(nav)).width;
    await edge.focus();
    await page.keyboard.press("Shift+ArrowRight");
    await page.keyboard.press("Shift+ArrowRight");
    await expect.poll(async () => (await box(nav)).width).toBeGreaterThan(before + 60);
    await page.reload();
    await expect.poll(async () => (await box(nav)).width).toBeGreaterThan(before + 60);
  });

  test("the properties button opens the right sidebar on the active note, and edits there save into the pane", async ({
    page,
    admin,
  }) => {
    const type = await deskType(admin);
    const one = await desk(admin, type, `First ${token()}`, "Kim");
    const two = await desk(admin, type, `Second ${token()}`, "Lee");
    await logIn(page, `/notes/${one.id}`);
    const props = page.getByRole("complementary", { name: "Properties" });
    const button = activePane(page).getByRole("button", { name: "Properties", exact: true });
    await expect(button).toHaveAttribute("aria-expanded", "false");
    await button.click();
    await expect(props).toBeVisible();
    await expect(props).toBeFocused();
    await expect(button).toHaveAttribute("aria-expanded", "true");
    await expect(props.getByLabel("Owner")).toHaveValue("Kim");

    // An edit there saves through the pane's note, and its {{owner}} shows it.
    await props.getByLabel("Owner").fill("Ana");
    await expect(activePane(page).getByText("Owned by Ana.")).toBeVisible();
    await expect(activePane(page).getByTestId("save-status")).toHaveText("Saved");
    const stored = await (await admin.get(`/api/notes/${one.id}`)).json();
    expect(stored.note.fields.owner).toBe("Ana");

    // The panel follows the active pane.
    await paneAction(page, activePane(page), "Split right");
    await panes(page).nth(1).locator(".pane-body").click();
    await page.waitForTimeout(300);
    await page.goto(`/notes/${two.id}`);
    await expect(panes(page)).toHaveCount(2);
    await expect(panes(page).nth(1).getByLabel("Title", { exact: true })).toHaveText(/^Second /);
    await expect(props.getByLabel("Owner")).toHaveValue("Lee");
    await panes(page).nth(0).locator(".pane-header").click();
    await expect(props.getByLabel("Owner")).toHaveValue("Ana");

    // A pane that shows no note: the panel says so.
    await page.getByRole("link", { name: "Settings" }).click();
    await expect(panes(page).nth(0).getByRole("heading", { name: "General settings" })).toBeVisible();
    await expect(props).toHaveText("Open a note to see its properties.");

    // The button closes it again.
    await panes(page).nth(1).locator(".pane-header").click();
    await expect(props.getByLabel("Owner")).toHaveValue("Lee");
    await panes(page).nth(1).getByRole("button", { name: "Properties", exact: true }).click();
    await expect(props).toHaveCount(0);
  });

  test("on a phone, both sidebars are drawers, closed at first", async ({ page, admin }) => {
    const type = await deskType(admin);
    const n = await desk(admin, type, `Phone ${token()}`, "Kim");
    await page.setViewportSize({ width: 390, height: 844 });
    await logIn(page, `/notes/${n.id}`);
    const nav = page.getByRole("navigation", { name: "Main" });
    const props = page.getByRole("complementary", { name: "Properties" });
    await expect(nav).not.toBeInViewport();
    await expect(props).toBeHidden();
    await activePane(page).getByRole("button", { name: "Properties", exact: true }).click();
    await expect(props).toBeInViewport();
    await expect(props.getByLabel("Owner")).toHaveValue("Kim");
    await page.locator(".scrim").click({ position: { x: 10, y: 300 } });
    await expect(props).toBeHidden();
    await page.getByRole("button", { name: "Menu" }).click();
    await expect(nav).toBeInViewport();
  });
});
