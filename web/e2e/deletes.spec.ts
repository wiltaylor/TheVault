import type { APIRequestContext } from "@playwright/test";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { expect, logIn, test, vaultDir } from "./fixtures";

// Deleting types, taxonomies and assets from the UI (S5, D40): each refusal names what is in the
// way, and each delete that goes through removes the file.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

async function putType(admin: APIRequestContext, name: string, fields: string) {
  const wcl = `label = "${name}"\n\n${fields}\ntemplate default {\n  tab "Main" {\n    slot "title"\n  }\n}\n`;
  const res = await admin.put(`/api/types/${name}`, { data: { wcl } });
  expect(res.status(), await res.text()).toBe(200);
}

async function createNote(admin: APIRequestContext, type: string, fields: Record<string, unknown>) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string; version: string };
}

async function deleteNote(admin: APIRequestContext, note: { id: string; version: string }) {
  const res = await admin.delete(`/api/notes/${note.id}?base_version=${note.version}`);
  expect(res.status(), await res.text()).toBe(200);
}

async function upload(admin: APIRequestContext, filename: string) {
  const res = await admin.post(`/api/assets?filename=${filename}`, { data: Buffer.from("bytes") });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string };
}

test("a type with notes is refused with their count, and deleted once they are gone", async ({ page, admin }) => {
  const name = `doomed_${token()}`;
  await putType(admin, name, "");
  const one = await createNote(admin, name, { title: "First" });
  const two = await createNote(admin, name, { title: "Second" });

  await logIn(page, `/settings/types/${name}/edit`);
  page.once("dialog", (d) => void d.accept());
  await page.getByRole("button", { name: "Delete type" }).click();
  await expect(page.getByText(`${name} still has 2 notes.`)).toBeVisible();
  await expect(page.getByRole("link", { name: "Show its notes" })).toHaveAttribute("href", `/types/${name}`);
  expect(existsSync(join(vaultDir(), "types", `${name}.wcl`))).toBe(true);

  await deleteNote(admin, one);
  await deleteNote(admin, two);
  page.once("dialog", (d) => void d.accept());
  await page.getByRole("button", { name: "Delete type" }).click();
  await expect(page).toHaveURL(/\/settings\/types$/);
  await page.goto("/settings/types");
  await expect(page.getByRole("list", { name: "Types" }).getByRole("link", { name })).toHaveCount(0);
  expect(existsSync(join(vaultDir(), "types", `${name}.wcl`))).toBe(false);
});

test("a taxonomy or term in use is refused listing the notes, and an unused one is deleted", async ({
  page,
  admin,
}) => {
  const t = token();
  const taxonomy = `phase_${t}`;
  const res = await admin.put(`/api/taxonomies/${taxonomy}`, {
    data: { wcl: `label = "Phase ${t}"\n\nterm "Draft"\n\nterm "Final"\n` },
  });
  expect(res.status(), await res.text()).toBe(200);
  const type = `doc_${t}`;
  await putType(admin, type, `field phase {\n  kind     = "taxonomy"\n  taxonomy = "${taxonomy}"\n}\n`);
  const note = await createNote(admin, type, { title: `Spec ${t}`, phase: "Draft" });

  await logIn(page, `/taxonomies/${taxonomy}`);
  page.once("dialog", (d) => void d.accept());
  await page.getByRole("button", { name: "Delete taxonomy" }).click();
  await expect(page.getByText(`Fields that use it: ${type}.phase.`)).toBeVisible();
  const using = page.getByRole("list", { name: "Notes using it" });
  await expect(using.getByRole("link", { name: `Spec ${t}` })).toHaveAttribute("href", `/notes/${note.id}`);
  expect(existsSync(join(vaultDir(), "taxonomies", `${taxonomy}.wcl`))).toBe(true);

  // Removing the term the note holds is refused the same way.
  await page.getByRole("button", { name: "Remove Draft" }).click();
  await page.getByRole("button", { name: "Save taxonomy" }).click();
  await expect(page.getByText("Notes still hold terms this change removes.")).toBeVisible();
  await expect(using.getByRole("link", { name: `Spec ${t}` })).toBeVisible();
  const kept = await (await admin.get(`/api/taxonomies/${taxonomy}`)).text();
  expect(kept).toContain("Draft");

  // A taxonomy no field uses goes.
  const spare = `spare_${t}`;
  expect((await admin.put(`/api/taxonomies/${spare}`, { data: { wcl: `label = "Spare"\n\nterm "One"\n` } })).status()).toBe(200);
  await page.goto(`/taxonomies/${spare}`);
  page.once("dialog", (d) => void d.accept());
  await page.getByRole("button", { name: "Delete taxonomy" }).click();
  await expect(page).toHaveURL(/\/taxonomies$/);
  expect(existsSync(join(vaultDir(), "taxonomies", `${spare}.wcl`))).toBe(false);
});

test("an asset notes use is deleted after confirming, and leaves those notes", async ({ page, admin }) => {
  const t = token();
  const type = `album_${t}`;
  await putType(admin, type, `field cover {\n  kind = "asset"\n}\n`);
  const used = await upload(admin, `cover-${t}.png`);
  const lone = await upload(admin, `lone-${t}.png`);
  const note = await createNote(admin, type, { title: `Album ${t}`, cover: used.id });

  await logIn(page, "/assets");

  // An asset nothing names goes after the browser's confirm.
  page.once("dialog", (d) => void d.accept());
  await page.getByRole("button", { name: `Delete lone-${t}.png` }).click();
  await expect(page.getByRole("status")).toHaveText(`Deleted lone-${t}.png.`);
  await expect(page.locator(`tr[data-asset="${lone.id}"]`)).toHaveCount(0);
  expect(existsSync(join(vaultDir(), "assets", lone.id))).toBe(false);

  // One a note names asks again, listing the note; cancelling keeps everything.
  page.once("dialog", (d) => void d.accept());
  await page.getByRole("button", { name: `Delete cover-${t}.png` }).click();
  const panel = page.getByRole("alertdialog", { name: `Delete cover-${t}.png` });
  await expect(panel.getByRole("link", { name: `Album ${t}` })).toHaveAttribute("href", `/notes/${note.id}`);
  await panel.getByRole("button", { name: "Cancel" }).click();
  await expect(panel).toHaveCount(0);
  expect(existsSync(join(vaultDir(), "assets", used.id))).toBe(true);

  page.once("dialog", (d) => void d.accept());
  await page.getByRole("button", { name: `Delete cover-${t}.png` }).click();
  await panel.getByRole("button", { name: "Remove from notes and delete" }).click();
  await expect(page.getByRole("status")).toHaveText(`Deleted cover-${t}.png and took it out of 1 note.`);
  await expect(page.locator(`tr[data-asset="${used.id}"]`)).toHaveCount(0);
  expect(existsSync(join(vaultDir(), "assets", used.id))).toBe(false);
  const after = (await (await admin.get(`/api/notes/${note.id}`)).json()) as { note: { fields: Record<string, unknown> } };
  expect(after.note.fields.cover).toBeUndefined();
  expect(after.note.fields.title).toBe(`Album ${t}`);
});
