import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

async function createTask(admin: APIRequestContext, title: string) {
  const res = await admin.post("/api/notes?type=task", { data: { note: { fields: { title }, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string; version: string };
}

interface Item {
  kind: "note" | "folder";
  id: string;
  title?: string;
  name?: string;
  children: Item[];
}

async function tree(admin: APIRequestContext): Promise<Item[]> {
  return ((await (await admin.get("/api/pins")).json()) as { pins: Item[] }).pins;
}

/** Remove every pinned note and folder. */
async function unpinAll(admin: APIRequestContext) {
  for (let items = await tree(admin); items.length > 0; items = await tree(admin)) {
    for (const p of items) {
      const res = await admin.delete(p.kind === "folder" ? `/api/pins/folders/${p.id}` : `/api/pins/${p.id}`);
      expect(res.status(), await res.text()).toBe(204);
    }
  }
}

/** The tree as `label(child child)` text: notes by title, folders by name. */
function shape(items: Item[]): string {
  return items
    .map((i) => {
      const label = i.kind === "folder" ? i.name! : i.title!;
      return i.children.length ? `${label}(${shape(i.children)})` : label;
    })
    .join(" ");
}

/** The tree as the sidebar shows it, collapsed entries without their children. */
async function shownShape(page: Page): Promise<string> {
  return page.locator(".pinned .pin-tree").evaluate((root) => {
    const walk = (ul: Element): string =>
      [...ul.children]
        .map((li) => {
          const row = li.querySelector(":scope > .pin-row")!;
          const label = row.querySelector(".pin-note span, .pin-folder span:last-child")!.textContent;
          const sub = li.querySelector(":scope > ul");
          return sub ? `${label}(${walk(sub)})` : label;
        })
        .join(" ");
    return walk(root);
  });
}

async function pinNote(admin: APIRequestContext, id: string) {
  const res = await admin.put(`/api/pins/${id}`);
  expect(res.status(), await res.text()).toBe(200);
}

async function makeFolder(admin: APIRequestContext, name: string): Promise<string> {
  const res = await admin.post("/api/pins/folders", { data: { name } });
  expect(res.status(), await res.text()).toBe(201);
  return ((await res.json()) as { id: string }).id;
}

const row = (page: Page, id: string) => page.locator(`.pinned [data-pin-id="${id}"]`);
const entry = (page: Page, id: string) => page.locator(`.pinned li.pin-entry:has(> [data-pin-id="${id}"])`);

/** Where `l` is on screen, once it is there: a row the sidebar is drawing anew has no box for a moment. */
async function boxOf(l: Locator) {
  let box: Awaited<ReturnType<Locator["boundingBox"]>> = null;
  await expect.poll(async () => (box = await l.boundingBox())).not.toBeNull();
  return box!;
}

/**
 * Press on entry `from`, drag it to entry `to` (`top`, `middle` or `bottom` of its row), check the
 * drop indicator with `during`, and let go.
 */
async function dragTo(
  page: Page,
  from: string,
  to: Locator,
  where: "top" | "middle" | "bottom",
  during?: () => Promise<void>,
) {
  const a = await boxOf(row(page, from));
  const b = await boxOf(to);
  const y = where === "top" ? b.y + 3 : where === "bottom" ? b.y + b.height - 3 : b.y + b.height / 2;
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
  await page.mouse.down();
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2 + 8, { steps: 2 });
  await page.mouse.move(b.x + b.width / 2, y, { steps: 8 });
  if (during) await during();
  await page.mouse.up();
}

test.describe.configure({ mode: "serial" });

test("pinning a note puts it in the sidebar, on every device, until it is unpinned", async ({ page, admin, browser }) => {
  await unpinAll(admin);
  const title = `Pinned ${token()}`;
  const note = await createTask(admin, title);

  await logIn(page, `/notes/${note.id}`);
  // The Pinned section shows even when empty, so a folder can be made there.
  await expect(page.getByRole("navigation", { name: "Pinned" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Pinned" }).getByRole("link")).toHaveCount(0);
  const pin = page.getByRole("button", { name: "Pin", exact: true });
  await expect(pin).toHaveAttribute("aria-pressed", "false");
  await pin.click();

  const pinned = page.getByRole("navigation", { name: "Pinned" });
  await expect(pinned.getByRole("link", { name: title })).toBeVisible();
  await expect(page.getByRole("button", { name: "Unpin" })).toHaveAttribute("aria-pressed", "true");

  // The pin is on the server: a fresh browser, as another device would be, sees it.
  const other = await browser.newContext();
  const otherPage = await other.newPage();
  await logIn(otherPage, "/daily");
  await otherPage.getByRole("navigation", { name: "Pinned" }).getByRole("link", { name: title }).click();
  await expect(otherPage).toHaveURL(new RegExp(`/notes/${note.id}$`));
  await other.close();

  await page.goto("/daily");
  await pinned.getByRole("link", { name: title }).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${note.id}$`));
  await page.getByRole("button", { name: "Unpin" }).click();
  await expect(pinned.getByRole("link")).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole("button", { name: "Pin", exact: true })).toBeVisible();
  await expect(pinned.getByRole("link")).toHaveCount(0);
  expect(await tree(admin)).toEqual([]);
});

test("a pinned note's new title shows in the sidebar after it is saved", async ({ page, admin }) => {
  await unpinAll(admin);
  const title = `Rename me ${token()}`;
  const note = await createTask(admin, title);
  const res = await admin.put(`/api/pins/${note.id}`);
  expect(res.status(), await res.text()).toBe(200);

  await logIn(page, `/notes/${note.id}`);
  const pinned = page.getByRole("navigation", { name: "Pinned" });
  await expect(pinned.getByRole("link", { name: title })).toBeVisible();

  const renamed = `Renamed ${token()}`;
  await page.getByLabel("Title").fill(renamed);
  await page.getByLabel("Title").press("Enter");
  await expect(pinned.getByRole("link", { name: renamed })).toBeVisible();
  await expect(pinned.getByRole("link", { name: title })).toHaveCount(0);
  await unpinAll(admin);
});

test("a pinned folder is created, renamed and deleted, and its notes move up when it goes", async ({ page, admin }) => {
  await unpinAll(admin);
  const title = `Filed ${token()}`;
  const note = await createTask(admin, title);
  await pinNote(admin, note.id);

  await logIn(page, "/daily");
  const pinned = page.getByRole("navigation", { name: "Pinned" });
  const input = pinned.getByLabel("New folder name");

  // Escape cancels.
  await pinned.getByRole("button", { name: "New folder" }).click();
  await input.fill("Scratch");
  await input.press("Escape");
  await expect(input).toHaveCount(0);
  await expect(pinned.getByRole("button", { name: "Scratch", exact: true })).toHaveCount(0);

  // Enter creates.
  await pinned.getByRole("button", { name: "New folder" }).click();
  await input.fill("Work");
  await input.press("Enter");
  await expect(pinned.getByRole("button", { name: "Work", exact: true })).toBeVisible();
  await expect.poll(async () => shape(await tree(admin))).toBe(`${title} Work`);

  // Double-click renames.
  await pinned.getByRole("button", { name: "Work", exact: true }).dblclick();
  const rename = pinned.getByLabel("Folder name");
  await expect(rename).toHaveValue("Work");
  await rename.fill("Projects");
  await rename.press("Enter");
  await expect(pinned.getByRole("button", { name: "Projects", exact: true })).toBeVisible();

  // So does the menu.
  await pinned.getByRole("button", { name: "Actions for Projects" }).click();
  await page.getByRole("menuitem", { name: "Rename" }).click();
  await rename.fill("Later");
  await rename.press("Enter");
  await expect(pinned.getByRole("button", { name: "Later", exact: true })).toBeVisible();
  await expect.poll(async () => shape(await tree(admin))).toBe(`${title} Later`);

  // Move the note into the folder from the menu, by keyboard.
  await pinned.getByRole("button", { name: `Actions for ${title}` }).click();
  await page.getByRole("menuitem", { name: "Move to…" }).press("Enter");
  await page.getByRole("menuitem", { name: "Later" }).press("Enter");
  await expect.poll(async () => shape(await tree(admin))).toBe(`Later(${title})`);
  await expect.poll(() => shownShape(page)).toBe(`Later(${title})`);

  // Deleting the folder keeps the note, in the folder's place.
  await pinned.getByRole("button", { name: "Actions for Later" }).click();
  await page.getByRole("menuitem", { name: "Delete folder" }).click();
  await expect(pinned.getByRole("button", { name: "Later", exact: true })).toHaveCount(0);
  await expect(pinned.getByRole("link", { name: title })).toBeVisible();
  await expect.poll(async () => shape(await tree(admin))).toBe(title);
  await page.reload();
  await expect.poll(() => shownShape(page)).toBe(title);
  await unpinAll(admin);
});

test("dragging nests a note in a folder and reorders entries, and the order holds after reload and on another device", async ({
  page,
  admin,
  browser,
}) => {
  await unpinAll(admin);
  const t = token();
  const [a, b, c] = [`A ${t}`, `B ${t}`, `C ${t}`];
  const na = await createTask(admin, a);
  const nb = await createTask(admin, b);
  const nc = await createTask(admin, c);
  for (const n of [na, nb, nc]) await pinNote(admin, n.id);
  const folder = await makeFolder(admin, "Box");

  await logIn(page, "/daily");
  const pinned = page.getByRole("navigation", { name: "Pinned" });
  await expect.poll(() => shownShape(page)).toBe(`${a} ${b} ${c} Box`);

  // Onto the middle of the folder: inside it, highlighted while dragging.
  await dragTo(page, na.id, row(page, folder), "middle", async () => {
    await expect(row(page, folder)).toHaveClass(/drop-inside/);
    await expect(page.locator(".pin-drag-ghost")).toHaveText(a);
  });
  await expect(page).toHaveURL(/\/planner\/day/);
  await expect.poll(() => shownShape(page)).toBe(`${b} ${c} Box(${a})`);
  await expect.poll(async () => shape(await tree(admin))).toBe(`${b} ${c} Box(${a})`);

  // Onto the top edge of another entry: before it, shown by a line.
  await dragTo(page, nc.id, row(page, nb.id), "top", async () => {
    await expect(entry(page, nb.id)).toHaveClass(/drop-before/);
    await expect(row(page, nb.id)).not.toHaveClass(/drop-inside/);
  });
  await expect.poll(async () => shape(await tree(admin))).toBe(`${c} ${b} Box(${a})`);

  // Onto the bottom edge: after it.
  await dragTo(page, nc.id, row(page, folder), "bottom", async () => {
    await expect(entry(page, folder)).toHaveClass(/drop-after/);
  });
  await expect.poll(async () => shape(await tree(admin))).toBe(`${b} Box(${a}) ${c}`);

  // Onto the Pinned header: to the end of the top level.
  await dragTo(page, na.id, pinned.locator(".pinned-header"), "middle", async () => {
    await expect(pinned.locator(".pinned-header")).toHaveClass(/drop-inside/);
  });
  await expect.poll(async () => shape(await tree(admin))).toBe(`${b} Box ${c} ${a}`);
  await dragTo(page, na.id, row(page, folder), "middle");
  await expect.poll(async () => shape(await tree(admin))).toBe(`${b} Box(${a}) ${c}`);

  // A folder cannot go into its own descendant: no indicator, and nothing changes.
  await dragTo(page, folder, row(page, na.id), "middle", async () => {
    await expect(page.locator(".pinned .drop-inside, .pinned .drop-before, .pinned .drop-after")).toHaveCount(0);
  });
  await expect.poll(() => shownShape(page)).toBe(`${b} Box(${a}) ${c}`);
  expect(shape(await tree(admin))).toBe(`${b} Box(${a}) ${c}`);

  // Escape cancels a drag.
  const from = await boxOf(row(page, nb.id));
  const to = await boxOf(row(page, nc.id));
  await page.mouse.move(from.x + 20, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(to.x + 20, to.y + to.height / 2, { steps: 6 });
  await expect(row(page, nc.id)).toHaveClass(/drop-inside/);
  await page.keyboard.press("Escape");
  await page.mouse.up();
  await expect(page.locator(".pin-drag-ghost")).toHaveCount(0);
  expect(shape(await tree(admin))).toBe(`${b} Box(${a}) ${c}`);

  // The order and the open folder hold after a reload.
  await page.reload();
  await expect.poll(() => shownShape(page)).toBe(`${b} Box(${a}) ${c}`);

  // Another device sees the same tree. Its folder starts closed, and clicking it opens it
  // without leaving the page.
  const other = await browser.newContext();
  const otherPage = await other.newPage();
  await logIn(otherPage, "/daily");
  const box = otherPage.getByRole("navigation", { name: "Pinned" }).getByRole("button", { name: "Box", exact: true });
  await expect.poll(() => shownShape(otherPage)).toBe(`${b} Box ${c}`);
  await expect(box).toHaveAttribute("aria-expanded", "false");
  const before = otherPage.url();
  await box.click();
  await expect(box).toHaveAttribute("aria-expanded", "true");
  await expect.poll(() => shownShape(otherPage)).toBe(`${b} Box(${a}) ${c}`);
  expect(otherPage.url()).toBe(before);
  await box.click();
  await expect.poll(() => shownShape(otherPage)).toBe(`${b} Box ${c}`);
  expect(otherPage.url()).toBe(before);
  await other.close();
  await unpinAll(admin);
});

test("a note nests under another pinned note, which opens with a click and expands with its arrow", async ({ page, admin }) => {
  await unpinAll(admin);
  const t = token();
  const [parent, child] = [`Parent ${t}`, `Child ${t}`];
  const p = await createTask(admin, parent);
  const c = await createTask(admin, child);
  await pinNote(admin, p.id);
  await pinNote(admin, c.id);

  await logIn(page, "/daily");
  const pinned = page.getByRole("navigation", { name: "Pinned" });
  await expect(pinned.getByRole("button", { name: `Expand ${parent}` })).toHaveCount(0);
  await dragTo(page, c.id, row(page, p.id), "middle", async () => {
    await expect(row(page, p.id)).toHaveClass(/drop-inside/);
  });
  await expect.poll(async () => shape(await tree(admin))).toBe(`${parent}(${child})`);
  await expect.poll(() => shownShape(page)).toBe(`${parent}(${child})`);

  const arrow = pinned.getByRole("button", { name: `Collapse ${parent}` });
  await expect(arrow).toHaveAttribute("aria-expanded", "true");
  await arrow.click();
  await expect(pinned.getByRole("link", { name: child })).toHaveCount(0);
  await expect(page).toHaveURL(/\/planner\/day/);
  await pinned.getByRole("button", { name: `Expand ${parent}` }).click();
  await expect(pinned.getByRole("link", { name: child })).toBeVisible();

  // The parent note's own link still opens it.
  await pinned.getByRole("link", { name: parent }).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${p.id}$`));
  await pinned.getByRole("link", { name: child }).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${c.id}$`));

  // Unpinning the parent from its menu moves the child up into its place.
  await pinned.getByRole("button", { name: `Actions for ${parent}` }).click();
  await page.getByRole("menuitem", { name: "Unpin" }).click();
  await expect.poll(async () => shape(await tree(admin))).toBe(child);
  await expect.poll(() => shownShape(page)).toBe(child);
  await unpinAll(admin);
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });

  test("the drawer's Move to… menu files a note in a folder", async ({ page, admin }) => {
    await unpinAll(admin);
    const title = `Phone ${token()}`;
    const n = await createTask(admin, title);
    await pinNote(admin, n.id);
    await makeFolder(admin, "Pocket");

    await logIn(page, "/daily");
    await page.getByRole("button", { name: "Menu" }).click();
    const pinned = page.getByRole("navigation", { name: "Pinned" });
    await expect(pinned.getByRole("link", { name: title })).toBeInViewport();
    await pinned.getByRole("button", { name: `Actions for ${title}` }).tap();
    await page.getByRole("menuitem", { name: "Move to…" }).tap();
    await page.getByRole("menuitem", { name: "Pocket" }).tap();
    await expect.poll(async () => shape(await tree(admin))).toBe(`Pocket(${title})`);
    await expect.poll(() => shownShape(page)).toBe(`Pocket(${title})`);
    await unpinAll(admin);
  });
});
