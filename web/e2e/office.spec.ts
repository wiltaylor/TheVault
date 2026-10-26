import type { APIRequestContext } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// Word documents and spreadsheets: the text inside an uploaded docx and xlsx is searchable, so a
// word only inside one finds the note holding it in full-text search, and the file itself in the
// images and files library, under its Documents or Spreadsheets filter. The files are built
// here as small zips, each with words made for the run so parallel runs never share them.

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

/** A zip of `parts`, stored without compression. */
function zip(parts: Record<string, string>): Buffer {
  const locals: Buffer[] = [];
  const centrals: Buffer[] = [];
  let offset = 0;
  for (const [name, text] of Object.entries(parts)) {
    const file = Buffer.from(name);
    const data = Buffer.from(text);
    const crc = crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x21, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(file.length, 26);
    locals.push(local, file, data);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(0x21, 14);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(data.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(file.length, 28);
    central.writeUInt32LE(offset, 42);
    centrals.push(central, file);
    offset += local.length + file.length + data.length;
  }
  const directory = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(Object.keys(parts).length, 8);
  end.writeUInt16LE(Object.keys(parts).length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, directory, end]);
}

const W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/** A Word document with a paragraph and a table, `word` in a table cell. */
function docx(word: string): Buffer {
  const p = (t: string) => `<w:p><w:r><w:t>${t}</w:t></w:r></w:p>`;
  const body = `${p("Garden plan")}<w:tbl><w:tr><w:tc>${p("Bed")}</w:tc><w:tc>${p(word)}</w:tc></w:tr></w:tbl>`;
  return zip({
    "[Content_Types].xml": `<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>`,
    "word/document.xml": `<?xml version="1.0"?><w:document xmlns:w="${W}"><w:body>${body}</w:body></w:document>`,
  });
}

/** An Excel workbook of two sheets, `word` in a cell of the second. */
function xlsx(word: string): Buffer {
  const ns = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
  const rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
  const pkg = "http://schemas.openxmlformats.org/package/2006/relationships";
  const sheet = (rows: string) => `<?xml version="1.0"?><worksheet xmlns="${ns}"><sheetData>${rows}</sheetData></worksheet>`;
  const cell = (r: string, t: string) => `<c r="${r}" t="inlineStr"><is><t>${t}</t></is></c>`;
  return zip({
    "[Content_Types].xml": `<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>`,
    "_rels/.rels": `<?xml version="1.0"?><Relationships xmlns="${pkg}"><Relationship Id="rId1" Type="${rel}/officeDocument" Target="xl/workbook.xml"/></Relationships>`,
    "xl/workbook.xml": `<?xml version="1.0"?><workbook xmlns="${ns}" xmlns:r="${rel}"><sheets><sheet name="Costs" sheetId="1" r:id="rId1"/><sheet name="Seeds" sheetId="2" r:id="rId2"/></sheets></workbook>`,
    "xl/_rels/workbook.xml.rels": `<?xml version="1.0"?><Relationships xmlns="${pkg}"><Relationship Id="rId1" Type="${rel}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="${rel}/worksheet" Target="worksheets/sheet2.xml"/></Relationships>`,
    "xl/worksheets/sheet1.xml": sheet(`<row r="1">${cell("A1", "Item")}<c r="B1"><v>12</v></c></row>`),
    "xl/worksheets/sheet2.xml": sheet(`<row r="1">${cell("A1", "Sow")}${cell("B1", word)}</row>`),
  });
}

async function upload(admin: APIRequestContext, filename: string, data: Buffer) {
  const res = await admin.post(`/api/assets?filename=${encodeURIComponent(filename)}`, { data });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string; filename: string };
}

async function createTask(admin: APIRequestContext, title: string, asset: string) {
  const blocks = [{ kind: "content", labels: ["body"], blocks: [{ kind: "asset", fields: { asset } }] }];
  const res = await admin.post("/api/notes?type=task", { data: { note: { fields: { title }, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
}

test("a word only inside a docx or an xlsx finds its note and the file itself", async ({ page, admin }) => {
  const t = token();
  const inDoc = `quill${t}`;
  const inSheet = `marsh${t}`;
  const doc = await upload(admin, `${t}-plan.docx`, docx(inDoc));
  const book = await upload(admin, `${t}-budget.xlsx`, xlsx(inSheet));
  await createTask(admin, `Plan ${t}`, doc.id);
  await createTask(admin, `Budget ${t}`, book.id);

  // Full-text search finds each note by the word inside its file, and only that note.
  await logIn(page, `/search?q=${inDoc}`);
  const rows = activePane(page).getByTestId("note-row");
  await expect(rows.filter({ hasText: `Plan ${t}` })).toBeVisible();
  await expect(rows).toHaveCount(1);
  await page.goto(`/search?q=${inSheet}`);
  await expect(rows.filter({ hasText: `Budget ${t}` })).toBeVisible();
  await expect(rows).toHaveCount(1);

  // The library search finds the file by the word inside it, under its kind's filter.
  await page.goto("/assets");
  const pane = activePane(page);
  const box = pane.getByRole("searchbox", { name: "Search images and files" });
  const filters = pane.getByRole("group", { name: "Show" });
  const cards = pane.getByTestId("asset-card");
  await box.fill(inDoc);
  await expect(cards).toHaveCount(1);
  await expect(cards.first()).toContainText(doc.filename);
  await expect(cards.first()).toContainText("DOCX document");
  await expect(filters.getByRole("button", { name: /^Documents/ })).toHaveText("Documents1");
  await filters.getByRole("button", { name: /^Spreadsheets/ }).click();
  await expect(cards).toHaveCount(0);
  await box.fill(inSheet);
  await expect(cards).toHaveCount(1);
  await expect(cards.first()).toContainText(book.filename);
  await expect(cards.first()).toContainText("XLSX spreadsheet");
  await filters.getByRole("button", { name: /^Documents/ }).click();
  await expect(cards).toHaveCount(0);
});
