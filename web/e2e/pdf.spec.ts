import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { activePane, expect, logIn, test, vaultDir } from "./fixtures";

// PDFs: an asset block of a PDF shows it in the pdf.js viewer, a pasted PDF becomes one, an
// asset field and the Assets page show a first-page thumbnail that opens the viewer, and
// full-text search finds a note by a word only inside its PDF. The fixture has three pages; the
// word "zephyrine" is on page 2 and nowhere else.

interface Block {
  kind: string;
  labels?: string[];
  fields?: Record<string, unknown>;
  blocks?: Block[];
}

interface Stored {
  id: string;
  version: string;
  note: { fields: Record<string, unknown>; blocks: Block[] };
}

const PDF = readFileSync(join(import.meta.dirname, "fixtures", "three-pages.pdf"));

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
}

async function createTask(admin: APIRequestContext, title: string, body: Block[] = []): Promise<Stored> {
  const blocks = body.length ? [{ kind: "content", labels: ["body"], blocks: body }] : [];
  const res = await admin.post("/api/notes?type=task", { data: { note: { fields: { title }, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as Stored;
}

async function stored(admin: APIRequestContext, id: string): Promise<Stored> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return (await res.json()) as Stored;
}

function body(note: Stored): Block[] {
  return note.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === "body")?.blocks ?? [];
}

async function saveAndWait(page: Page) {
  await page.getByLabel("Title", { exact: true }).focus();
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
}

async function uploadPdf(admin: APIRequestContext, name = `report-${token()}.pdf`): Promise<{ id: string; name: string }> {
  const res = await admin.post(`/api/assets?filename=${encodeURIComponent(name)}`, { data: PDF });
  expect(res.status(), await res.text()).toBe(201);
  return { id: ((await res.json()) as { id: string }).id, name };
}

/** Whether page `n` of `viewer` has been drawn: its canvas is in place with pixels. */
async function pageDrawn(viewer: Locator, n: number): Promise<boolean> {
  return viewer.locator(`.pdf-page[data-page="${n}"] canvas`).evaluate((c) => (c as HTMLCanvasElement).width > 0).catch(() => false);
}

/** The viewer's controls and pages behave: page 1 drawn, next page, zoom in. */
async function checkViewer(viewer: Locator) {
  await expect(viewer.getByTestId("pdf-page")).toHaveText("1 / 3");
  await expect.poll(() => pageDrawn(viewer, 1)).toBe(true);
  // The text layer holds the page's text, so it selects and copies.
  await expect(viewer.locator('.pdf-page[data-page="1"] .textLayer')).toContainText("three page sample");

  await viewer.getByRole("button", { name: "Next page" }).click();
  await expect(viewer.getByTestId("pdf-page")).toHaveText("2 / 3");
  await expect.poll(() => pageDrawn(viewer, 2)).toBe(true);
  await expect(viewer.locator('.pdf-page[data-page="2"] .textLayer')).toContainText("zephyrine");
  await viewer.getByRole("button", { name: "Previous page" }).click();
  await expect(viewer.getByTestId("pdf-page")).toHaveText("1 / 3");

  const zoom = async () => Number.parseInt((await viewer.getByTestId("pdf-zoom").textContent()) ?? "0", 10);
  const before = await zoom();
  const width = async () => (await viewer.locator('.pdf-page[data-page="1"]').boundingBox())?.width ?? 0;
  const wide = await width();
  await viewer.getByRole("button", { name: "Zoom in" }).click();
  await expect.poll(zoom).toBeGreaterThan(before);
  await expect.poll(width).toBeGreaterThan(wide * 1.2);
  await expect(viewer.getByRole("button", { name: "Fit width" })).toHaveAttribute("aria-pressed", "false");
  await viewer.getByRole("button", { name: "Fit width" }).click();
  await expect.poll(zoom).toBe(before);
}

test("a PDF uploaded into a file block shows inline, pages and zooms, and folds to its card", async ({ page, admin }) => {
  const note = await createTask(admin, `PDF block ${token()}`);
  await logIn(page, `/notes/${note.id}`);
  const doc = page.getByRole("textbox", { name: "Body" });
  await doc.click();

  const name = `three-${token()}.pdf`;
  const chooser = page.waitForEvent("filechooser");
  await doc.locator("p").first().hover();
  await page.getByRole("button", { name: "Add block" }).click();
  await page.getByRole("listbox", { name: "Insert block" }).getByRole("option", { name: "File" }).click();
  await (await chooser).setFiles({ name, mimeType: "application/pdf", buffer: PDF });

  const viewer = doc.getByTestId("pdf-viewer");
  await expect(viewer).toBeVisible();
  await checkViewer(viewer);
  await saveAndWait(page);

  const blocks = body(await stored(admin, note.id));
  expect(blocks.map((b) => b.kind)).toEqual(["asset"]);
  const asset = blocks[0]!.fields?.asset as string;
  expect(readFileSync(join(vaultDir(), "assets", asset, name))).toEqual(PDF);

  // Fold it to the file card: the block keeps that, and the card can open the viewer again.
  await viewer.getByRole("button", { name: "Show as file card" }).click();
  await expect(doc.getByRole("link", { name })).toBeVisible();
  await expect(doc.getByTestId("pdf-viewer")).toHaveCount(0);
  await saveAndWait(page);
  expect(body(await stored(admin, note.id))[0]!.fields).toEqual({ asset, card: true });
  await page.reload();
  const field = page.getByTestId("field-body");
  await expect(field.getByRole("link", { name })).toBeVisible();
  await field.getByRole("button", { name: "Show the PDF inline" }).click();
  await expect(field.getByTestId("pdf-viewer")).toBeVisible();
  await expect(field.getByTestId("pdf-viewer").getByTestId("pdf-page")).toHaveText("1 / 3");
});

test("pasting a PDF into the editor uploads it as a file block that shows the PDF", async ({ page, admin }) => {
  const note = await createTask(admin, `PDF paste ${token()}`, [{ kind: "paragraph", fields: { text: "Attached:" } }]);
  await logIn(page, `/notes/${note.id}`);
  const doc = page.getByRole("textbox", { name: "Body" });
  await doc.click();
  await page.keyboard.press("ControlOrMeta+End");
  const name = `pasted-${token()}.pdf`;
  await doc.evaluate(
    (el, { b64, name }) => {
      const dt = new DataTransfer();
      dt.items.add(new File([Uint8Array.from(atob(b64), (c) => c.charCodeAt(0))], name, { type: "application/pdf" }));
      el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
    },
    { b64: PDF.toString("base64"), name },
  );
  const viewer = doc.getByTestId("pdf-viewer");
  await expect(viewer).toBeVisible();
  await expect(viewer.getByTestId("pdf-page")).toHaveText("1 / 3");
  await expect.poll(() => pageDrawn(viewer, 1)).toBe(true);
  await saveAndWait(page);
  const blocks = body(await stored(admin, note.id));
  expect(blocks[0]!.kind).toBe("paragraph");
  expect(blocks.filter((b) => b.kind === "asset")).toHaveLength(1);
  const asset = blocks.find((b) => b.kind === "asset")!.fields?.asset as string;
  expect(readFileSync(join(vaultDir(), "assets", asset, name))).toEqual(PDF);
});

test("an asset field shows a PDF's first page, which opens the viewer over the page", async ({ page, admin }) => {
  const type = `papers_${token()}`;
  const wcl =
    `label = "Paper"\n\nfield file {\n  kind = "asset"\n  label = "File"\n}\n\n` +
    `template default {\n  tab "Main" {\n    slot "title"\n    slot "file"\n  }\n}\n`;
  const put = await admin.put(`/api/types/${type}`, { data: { wcl } });
  expect(put.status(), await put.text()).toBe(200);
  const pdf = await uploadPdf(admin);
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields: { title: "Paper", file: pdf.id }, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  const { id } = (await res.json()) as { id: string };
  await logIn(page, `/notes/${id}`);

  const thumb = page.getByTestId("field-file").getByRole("button", { name: `View ${pdf.name}` });
  await expect(thumb).toBeVisible();
  await expect.poll(() => thumb.locator("img").evaluate((i) => (i as HTMLImageElement).naturalWidth).catch(() => 0)).toBeGreaterThan(0);
  await thumb.click();
  const overlay = page.getByTestId("pdf-overlay");
  await expect(overlay).toBeVisible();
  await expect(overlay).toHaveAttribute("aria-label", pdf.name);
  await checkViewer(overlay.getByTestId("pdf-viewer"));
  await page.keyboard.press("Escape");
  await expect(overlay).toHaveCount(0);

  // The close button closes it too.
  await thumb.click();
  await page.getByTestId("pdf-overlay").getByRole("button", { name: "Close" }).click();
  await expect(page.getByTestId("pdf-overlay")).toHaveCount(0);
});

test("the Assets page shows a PDF's first page and opens it", async ({ page, admin }) => {
  const pdf = await uploadPdf(admin);
  await logIn(page, "/assets");
  const row = activePane(page).locator(`tr[data-asset="${pdf.id}"]`);
  const thumb = row.getByRole("button", { name: `View ${pdf.name}` });
  await expect(thumb).toBeVisible();
  // The thumbnail is drawn once it scrolls into view; other tests' assets come first.
  await thumb.scrollIntoViewIfNeeded();
  await expect.poll(() => thumb.locator("img").evaluate((i) => (i as HTMLImageElement).naturalWidth).catch(() => 0)).toBeGreaterThan(0);
  await thumb.click();
  const viewer = page.getByTestId("pdf-overlay").getByTestId("pdf-viewer");
  await expect(viewer.getByTestId("pdf-page")).toHaveText("1 / 3");
  await expect.poll(() => pageDrawn(viewer, 1)).toBe(true);
});

test("full-text search finds a note by a word only inside its PDF", async ({ page, admin }) => {
  const pdf = await uploadPdf(admin);
  const title = `Holds the PDF ${token()}`;
  await createTask(admin, title, [{ kind: "asset", fields: { asset: pdf.id } }]);
  await logIn(page, "/search?q=zephyrine");
  await expect(activePane(page).getByTestId("note-row").filter({ hasText: title })).toBeVisible();
});

test("find in the PDF counts matches and goes to the page holding them", async ({ page, admin }) => {
  const pdf = await uploadPdf(admin);
  const note = await createTask(admin, `PDF find ${token()}`, [{ kind: "asset", fields: { asset: pdf.id } }]);
  await logIn(page, `/notes/${note.id}`);
  const viewer = page.getByTestId("field-body").getByTestId("pdf-viewer");
  await expect.poll(() => pageDrawn(viewer, 1)).toBe(true);
  await viewer.getByRole("button", { name: "Find in PDF" }).click();
  await viewer.getByLabel("Find text").fill("zephyrine");
  await expect(viewer.getByTestId("pdf-find-count")).toHaveText("1 of 1");
  await expect(viewer.getByTestId("pdf-page")).toHaveText("2 / 3");
  await expect(viewer.locator('.pdf-page[data-page="2"] .pdf-hit')).toHaveCount(1);
  await viewer.getByLabel("Find text").fill("nowhere-at-all");
  await expect(viewer.getByTestId("pdf-find-count")).toHaveText("No matches");
});
