import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { deflateSync } from "node:zlib";
import { activePane, expect, logIn, test, vaultDir } from "./fixtures";

// The images and files library (/assets) shows files as a type's notes are shown, as cards or a
// sortable list; an asset opens in the pane like a note, with the notes using it in the right
// sidebar; and the files nothing uses can be cleaned up, then restored.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

function crc32(buf: Buffer): number {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff]! ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** A `w` by `h` PNG of a diagonal gradient between two colours. */
function gradientPng(w: number, h: number, from: [number, number, number], to: [number, number, number]): Buffer {
  const rows: Buffer[] = [];
  for (let y = 0; y < h; y++) {
    const row = Buffer.alloc(1 + w * 3);
    for (let x = 0; x < w; x++) {
      const t = (x / w + y / h) / 2;
      for (let i = 0; i < 3; i++) row[1 + x * 3 + i] = Math.round(from[i]! + (to[i]! - from[i]!) * t);
    }
    rows.push(row);
  }
  const chunk = (type: string, data: Buffer) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(body));
    return Buffer.concat([len, body, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(Buffer.concat(rows))),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const PDF = () => readFileSync(join(import.meta.dirname, "fixtures/three-pages.pdf"));

async function putType(admin: APIRequestContext, name: string) {
  const wcl = `label = "Album"
plural = "Albums"
icon = "book-image"

field cover {
  kind  = "asset"
  label = "Cover"
}

field body {
  kind  = "content"
  label = "Body"
}

template default {
  tab "Main" {
    slot "title"
    slot "cover"
    slot "body"
  }
}
`;
  const res = await admin.put(`/api/types/${name}`, { data: { wcl } });
  expect(res.status(), await res.text()).toBe(200);
}

async function upload(admin: APIRequestContext, filename: string, data: Buffer) {
  const res = await admin.post(`/api/assets?filename=${encodeURIComponent(filename)}`, { data });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string; filename: string };
}

async function createNote(admin: APIRequestContext, type: string, note: Record<string, unknown>) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string; version: string };
}

/** A library of five files under `t`: a cover a note uses, an image a note's body shows, a PDF, a spare and a text file. */
async function library(admin: APIRequestContext, t: string) {
  const type = `album_${t}`;
  await putType(admin, type);
  const cover = await upload(admin, `${t}-cover.png`, gradientPng(240, 180, [37, 99, 235], [147, 51, 234]));
  const inline = await upload(admin, `${t}-beach.png`, gradientPng(240, 180, [234, 88, 12], [250, 204, 21]));
  const pdf = await upload(admin, `${t}-report.pdf`, PDF());
  const spare = await upload(admin, `${t}-spare.png`, gradientPng(240, 180, [22, 163, 74], [14, 116, 144]));
  const text = await upload(admin, `${t}-notes.txt`, Buffer.from("plain text"));
  const album = await createNote(admin, type, { fields: { title: `Summer ${t}`, cover: cover.id }, blocks: [] });
  const diary = await createNote(admin, type, {
    fields: { title: `Diary ${t}` },
    blocks: [{ kind: "content", labels: ["body"], blocks: [{ kind: "image", fields: { asset: inline.id } }, { kind: "asset", fields: { asset: cover.id } }] }],
  });
  return { type, cover, inline, pdf, spare, text, album, diary };
}

async function search(page: Page, t: string) {
  await activePane(page).getByRole("searchbox", { name: "Search images and files" }).fill(t);
}

/**
 * With E2E_SHOTS set to a folder, save light and dark screenshots of `what` there. Without it,
 * nothing is written.
 */
async function shots(page: Page, name: string, what: Locator | Page) {
  const dir = process.env.E2E_SHOTS;
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  for (const theme of ["light", "dark"]) {
    await page.evaluate((t) => (document.documentElement.dataset.theme = t), theme);
    await page.waitForTimeout(250);
    await what.screenshot({ path: join(dir, `assets-${name}-${theme}.png`) });
  }
  await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
}

test("the library shows files as cards or a remembered list, filtered to the unused ones", async ({ page, admin }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const t = token();
  const lib = await library(admin, t);
  await logIn(page, "/assets");
  const pane = activePane(page);
  await expect(pane.getByRole("heading", { name: "Images and files" })).toBeVisible();
  await search(page, t);

  // Cards by default, one per file, each with what it is and how much uses it.
  const cards = pane.getByTestId("asset-card");
  await expect(cards).toHaveCount(5);
  const cover = cards.filter({ hasText: `${t}-cover.png` });
  await expect(cover.getByTestId("asset-used")).toHaveText("Used by 2 notes");
  await expect(cover.locator("img.asset-thumb-image")).toBeVisible();
  await expect(cards.filter({ hasText: `${t}-spare.png` }).getByTestId("asset-used")).toHaveText("Not used");
  await expect(cards.filter({ hasText: `${t}-report.pdf` }).getByTestId("pdf-thumb")).toBeVisible();
  await expect(cards.filter({ hasText: `${t}-notes.txt` })).toContainText("TXT file");
  await expect(pane.getByTestId("assets-status")).toHaveText("5 matches");
  await expect(pane.getByRole("group", { name: "Show" }).getByRole("button", { name: /^Unused/ })).toHaveText("Unused3");
  await expect(cards.filter({ hasText: `${t}-report.pdf` }).locator(".pdf-thumb img")).toBeVisible();
  await shots(page, "cards", pane);

  // The List view, sortable by any column, is remembered across a reload.
  await pane.getByRole("group", { name: "View" }).getByRole("button", { name: "List" }).click();
  const rows = pane.getByTestId("asset-row");
  await expect(rows).toHaveCount(5);
  await pane.getByRole("button", { name: "Name" }).click();
  await expect(rows.first()).toContainText(`${t}-beach.png`);
  await pane.getByRole("button", { name: "Name" }).click();
  await expect(rows.first()).toContainText(`${t}-spare.png`);
  await pane.getByRole("button", { name: "Used by" }).click();
  await expect(rows.last()).toContainText("Used by 2 notes");
  await shots(page, "list", pane);
  await page.reload();
  await search(page, t);
  await expect(activePane(page).getByTestId("asset-row")).toHaveCount(5);
  await expect(activePane(page).getByTestId("asset-card")).toHaveCount(0);

  // Unused: the three files no note names.
  await activePane(page).getByRole("group", { name: "Show" }).getByRole("button", { name: /^Unused/ }).click();
  await expect(activePane(page).getByTestId("asset-row")).toHaveCount(3);
  await expect(activePane(page).getByTestId("asset-row").filter({ hasText: `${t}-cover.png` })).toHaveCount(0);
  await activePane(page).getByRole("group", { name: "Show" }).getByRole("button", { name: /^PDFs/ }).click();
  await expect(activePane(page).getByTestId("asset-row")).toHaveCount(1);
  await expect(activePane(page).getByTestId("asset-row")).toContainText(`${t}-report.pdf`);

  await activePane(page).getByRole("group", { name: "View" }).getByRole("button", { name: "Cards" }).click();
  await page.reload();
  await search(page, t);
  await expect(activePane(page).getByTestId("asset-card")).toHaveCount(5);
  expect(lib.album.id).toBeTruthy();
});

test("an asset opens in the pane like a note, with the notes using it in the right sidebar", async ({ page, admin }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const t = token();
  const lib = await library(admin, t);
  await logIn(page, "/assets");
  await search(page, t);
  const pane = activePane(page);

  // A Ctrl+click opens a background tab named after the file.
  await pane.getByTestId("asset-card").filter({ hasText: `${t}-cover.png` }).getByRole("link").click({ modifiers: ["ControlOrMeta"] });
  const tabs = page.getByRole("tablist", { name: "Open tabs" }).getByRole("tab");
  await expect(tabs).toHaveCount(2);
  await expect(tabs.nth(1)).toHaveAccessibleName(`${t}-cover.png`);
  await expect(page).toHaveURL(/\/assets$/);

  // A plain click opens it here, with its preview and what it is.
  await pane.getByTestId("asset-card").filter({ hasText: `${t}-cover.png` }).click();
  await expect(page).toHaveURL(new RegExp(`/assets/${lib.cover.id}$`));
  await expect(pane.getByRole("heading", { name: `${t}-cover.png` })).toBeVisible();
  await expect(pane.getByTestId("asset-preview").locator("img")).toBeVisible();
  const meta = pane.getByRole("definition");
  await expect(meta.filter({ hasText: "PNG image" })).toHaveCount(1);
  await expect(pane.getByTestId("pane-title")).toHaveText(`${t}-cover.png`);

  // The right sidebar lists both notes, how each uses it, and opens one.
  await pane.getByRole("button", { name: "Used by 2 notes" }).click();
  const side = page.locator("#right-sidebar");
  await expect(side.getByRole("heading", { name: "Used by" })).toBeVisible();
  const users = side.getByTestId("asset-user");
  await expect(users).toHaveCount(2);
  await expect(side.getByRole("listitem").filter({ hasText: `Summer ${t}` })).toContainText("Cover");
  await expect(side.getByRole("listitem").filter({ hasText: `Diary ${t}` })).toContainText("File in Body");
  await shots(page, "page", page);
  await users.filter({ hasText: `Summer ${t}` }).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${lib.album.id}$`));
  // Beside a note, the sidebar is the note's again.
  await expect(side.getByRole("tab", { name: "Properties" })).toBeVisible();

  // A PDF opens in the inline viewer; an unused file offers its delete in the sidebar.
  await page.goto(`/assets/${lib.pdf.id}`);
  await expect(activePane(page).locator(".pdf-viewer")).toBeVisible();
  await expect(side.getByText("Not used by any note.")).toBeVisible();
  page.once("dialog", (d) => void d.accept());
  await side.getByRole("button", { name: `Delete ${t}-report.pdf` }).click();
  await expect(page).toHaveURL(/\/assets$/);
  expect(existsSync(join(vaultDir(), "assets", lib.pdf.id))).toBe(false);
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByText("Restored 1 file.")).toBeVisible();
  expect(existsSync(join(vaultDir(), "assets", lib.pdf.id))).toBe(true);
});

test("clean up deletes only the unused files after a confirm, and Undo brings them back", async ({ page, admin }) => {
  await page.setViewportSize({ width: 1400, height: 900 });
  const t = token();
  const lib = await library(admin, t);
  // A note in the trash keeps its file too.
  const kept = await upload(admin, `${t}-kept.png`, gradientPng(32, 32, [220, 38, 38], [219, 39, 119]));
  const gone = await createNote(admin, lib.type, { fields: { title: `Binned ${t}`, cover: kept.id }, blocks: [] });
  const del = await admin.delete(`/api/notes/${gone.id}?base_version=${gone.version}`);
  expect(del.status(), await del.text()).toBe(200);
  const mine = new Set([lib.cover.id, lib.inline.id, lib.pdf.id, lib.spare.id, lib.text.id, kept.id]);

  // Other tests share the server: the dry run is narrowed to this test's files, so the confirm
  // deletes only those, by id.
  await page.route("**/api/assets/cleanup", async (route) => {
    const body = route.request().postDataJSON() as { dry_run: boolean; ids?: string[] };
    if (!body.dry_run) {
      expect(body.ids?.every((id) => mine.has(id))).toBe(true);
      return route.continue();
    }
    const res = await route.fetch();
    const plan = (await res.json()) as { assets: { id: string; size: number }[]; count: number; size: number };
    plan.assets = plan.assets.filter((a) => mine.has(a.id));
    plan.count = plan.assets.length;
    plan.size = plan.assets.reduce((n, a) => n + a.size, 0);
    await route.fulfill({ response: res, json: plan });
  });

  await logIn(page, "/assets");
  await search(page, t);
  const pane = activePane(page);
  await expect(pane.getByTestId("asset-card")).toHaveCount(6);
  await pane.getByRole("button", { name: /^Clean up/ }).click();
  const dialog = page.getByRole("alertdialog", { name: "Delete unused files" });
  await expect(dialog.getByTestId("cleanup-summary")).toContainText("Delete 3 files that no note uses");
  const listed = dialog.getByRole("list", { name: "Files to delete" });
  await expect(listed.getByRole("listitem")).toHaveCount(3);
  await expect(listed).not.toContainText(`${t}-kept.png`);
  await expect(listed).not.toContainText(`${t}-cover.png`);
  await shots(page, "cleanup", page);

  // Cancel keeps everything; confirming deletes just those three.
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toHaveCount(0);
  expect(existsSync(join(vaultDir(), "assets", lib.spare.id))).toBe(true);
  await pane.getByRole("button", { name: /^Clean up/ }).click();
  await dialog.getByRole("button", { name: "Delete 3 files" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(pane.getByTestId("asset-card")).toHaveCount(3);
  for (const a of [lib.pdf, lib.spare, lib.text]) expect(existsSync(join(vaultDir(), "assets", a.id))).toBe(false);
  for (const a of [lib.cover, lib.inline, kept]) expect(existsSync(join(vaultDir(), "assets", a.id))).toBe(true);
  // The trashed note's file is counted as used, from the trash.
  await expect(pane.getByTestId("asset-card").filter({ hasText: `${t}-kept.png` }).getByTestId("asset-used")).toHaveText("Used by 1 note");

  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByText("Restored 3 files.")).toBeVisible();
  await expect(pane.getByTestId("asset-card")).toHaveCount(6);
  expect(readFileSync(join(vaultDir(), "assets", lib.text.id, `${t}-notes.txt`), "utf8")).toBe("plain text");

  // Deleted again, a file can be restored from Recently deleted as well.
  await page.unroute("**/api/assets/cleanup");
  page.once("dialog", (d) => void d.accept());
  await page.goto(`/assets/${lib.spare.id}`);
  await activePane(page).getByRole("button", { name: "Delete" }).click();
  await expect(page).toHaveURL(/\/assets$/);
  await activePane(page).getByRole("button", { name: "Recently deleted" }).click();
  const trash = activePane(page).getByRole("region", { name: "Recently deleted files" });
  await trash.getByRole("button", { name: `Restore ${t}-spare.png` }).click();
  await expect(trash.getByRole("button", { name: `Restore ${t}-spare.png` })).toHaveCount(0);
  expect(existsSync(join(vaultDir(), "assets", lib.spare.id))).toBe(true);
});
