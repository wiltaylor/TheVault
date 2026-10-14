// An image in a template that shows an asset field of each note (docs/vault-format.md, "Images
// from fields"): inserted from the template editor, filled in from the note page, and drawn as the
// cover of the type page's cards.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { deflateSync } from "node:zlib";
import type { APIRequestContext, APIResponse, Locator, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
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

function chunk(kind: string, data: Buffer): Buffer {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(kind, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

/** A `w` by `h` PNG shaded from colour `a` at the top left to `b` at the bottom right. */
function gradientPng(w: number, h: number, a: [number, number, number], b: [number, number, number]): Buffer {
  const rows: Buffer[] = [];
  for (let y = 0; y < h; y++) {
    const row = Buffer.alloc(1 + w * 3);
    for (let x = 0; x < w; x++) {
      const t = (x / w + y / h) / 2;
      for (let i = 0; i < 3; i++) row[1 + x * 3 + i] = Math.round(a[i]! + (b[i]! - a[i]!) * t);
    }
    rows.push(row);
  }
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

const ALBUM_TYPE = `label  = "Album"
plural = "Albums"

field cover {
  kind  = "asset"
  label = "Cover"
}

field artist {
  kind  = "string"
  label = "Artist"
}

template default {
  tab "Main" {
    slot "title"
    slot "artist"
  }
}

template card {
  tab "Card" {
    slot "artist"
  }
}
`;

async function ok(res: Promise<APIResponse>, want: number): Promise<APIResponse> {
  const r = await res;
  expect(r.status(), await r.text()).toBe(want);
  return r;
}

async function uploadPng(admin: APIRequestContext, name: string, png: Buffer): Promise<string> {
  const res = await ok(admin.post(`/api/assets?filename=${name}`, { data: png }), 201);
  return ((await res.json()) as { id: string }).id;
}

async function createAlbum(admin: APIRequestContext, type: string, fields: Record<string, unknown>): Promise<string> {
  const res = await ok(admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks: [] } } }), 201);
  return ((await res.json()) as { id: string }).id;
}

async function imageLoaded(img: Locator): Promise<boolean> {
  return img.evaluate((el) => (el as HTMLImageElement).complete && (el as HTMLImageElement).naturalWidth > 0);
}

/**
 * With E2E_SHOTS set to a folder, save light and dark screenshots of `what` there. Without it,
 * nothing is written.
 */
async function shots(page: Page, name: string, what: Locator) {
  const dir = process.env.E2E_SHOTS;
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  for (const theme of ["light", "dark"]) {
    await page.evaluate((t) => (document.documentElement.dataset.theme = t), theme);
    await page.waitForTimeout(100);
    await what.screenshot({ path: join(dir, `tplimage-${name}-${theme}.png`) });
  }
  await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
}

/** In the open template's editor, insert at the end an image from asset field `label`. */
async function insertFieldImage(page: Page, editor: Locator, label: string) {
  const menu = page.getByRole("listbox", { name: "Insert block" });
  await editor.locator("p").last().click();
  await page.keyboard.type("/image");
  await menu.getByRole("option", { name: "Image", exact: true }).click();
  // The follow-up choice: a fixed upload, or each asset field of the type.
  await expect(menu.getByRole("option", { name: /^Upload an image/ })).toBeVisible();
  await menu.getByRole("option", { name: new RegExp(`^From field: ${label}`) }).click();
  await expect(editor.getByTestId("field-image-placeholder")).toHaveText(`Image from ${label}`);
}

test("an image from an asset field: added in the template editor, filled in on a note, a cover on cards", async ({
  page,
  admin,
}) => {
  const t = token();
  const type = `album_${t}`;
  await ok(admin.put(`/api/types/${type}`, { data: { wcl: ALBUM_TYPE } }), 200);
  const cover = await uploadPng(admin, `train-${t}.png`, gradientPng(480, 320, [37, 99, 235], [236, 72, 153]));
  await createAlbum(admin, type, { title: "Blue Train", artist: "John Coltrane", cover });
  const empty = await createAlbum(admin, type, { title: "Kind of Blue", artist: "Miles Davis" });

  // The default template: an image from Cover after the artist.
  await logIn(page, `/settings/types/${type}/templates`);
  const main = page.getByRole("textbox", { name: "Main content" });
  await expect(main.getByTestId("template-item-artist")).toBeVisible();
  await insertFieldImage(page, main, "Cover");
  await main.getByLabel("Alt text").fill("Cover of {{title}}");
  await page.getByRole("button", { name: "Save template" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved the default template.");
  await shots(page, "editor", page.locator(".template-canvas"));

  // The card template: the image first, then the artist, so the image is the card's cover.
  await page.getByRole("group", { name: "Templates" }).getByRole("button", { name: "Card" }).click();
  const card = page.getByRole("textbox", { name: "Card content" });
  await card.getByRole("button", { name: "Remove artist" }).click();
  await insertFieldImage(page, card, "Cover");
  await page.keyboard.type("/artist");
  await page.getByRole("listbox", { name: "Insert block" }).getByRole("option", { name: /^Artist/ }).click();
  await expect(card.getByTestId("template-item-artist")).toBeVisible();
  await page.getByRole("button", { name: "Save template" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved the card template.");

  const wcl = async (name: string) => {
    const res = await ok(admin.get(`/api/types/${type}/templates/${name}`, { headers: { Accept: "application/json" } }), 200);
    return ((await res.json()) as { wcl: string }).wcl;
  };
  expect(await wcl("default")).toContain('    slot "artist"\n    image {\n      field = "cover"\n      alt   = "Cover of {{title}}"\n    }\n');
  expect(await wcl("card")).toContain('  tab "Card" {\n    image {\n      field = "cover"\n    }\n    slot "artist"\n');

  // A note with no cover offers to add one; the upload sets the field and saves.
  await page.goto(`/notes/${empty}`);
  const add = page.getByTestId("field-image-add");
  await expect(add).toHaveText("Add image");
  await shots(page, "note-empty", page.locator(".note-view"));
  const chooser = page.waitForEvent("filechooser");
  await add.click();
  await (await chooser).setFiles({
    name: `blue-${t}.png`,
    mimeType: "image/png",
    buffer: gradientPng(480, 320, [14, 116, 144], [253, 224, 71]),
  });
  const shown = page.getByRole("img", { name: "Cover of Kind of Blue" });
  await expect(shown).toBeVisible();
  await expect.poll(() => imageLoaded(shown)).toBe(true);
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
  await expect(page.getByTestId("field-image-add")).toHaveCount(0);
  const note = (await (await ok(admin.get(`/api/notes/${empty}`), 200)).json()) as { note: { fields: Record<string, unknown> } };
  expect(note.note.fields.cover).toMatch(/^[0-9A-HJKMNP-TV-Z]{26}$/);
  await page.reload();
  await expect.poll(() => imageLoaded(page.getByRole("img", { name: "Cover of Kind of Blue" }))).toBe(true);
  await shots(page, "note", page.locator(".note-view"));

  // The type page's cards show each note's cover above its title.
  await page.goto(`/types/${type}`);
  const cards = activePane(page).getByTestId("note-card");
  await expect(cards).toHaveCount(2);
  for (const c of await cards.all()) {
    const img = c.getByTestId("card-cover").locator("img");
    await expect(img).toBeVisible();
    await expect.poll(() => imageLoaded(img)).toBe(true);
    // The cover is drawn above the title, the card's full width.
    const [box, title, whole] = await Promise.all([img.boundingBox(), c.getByTestId("query-title").boundingBox(), c.boundingBox()]);
    expect(box!.y).toBeLessThan(title!.y);
    expect(box!.width).toBeGreaterThan(whole!.width * 0.8);
  }
  await shots(page, "cards", activePane(page).getByTestId("type-cards"));
});
