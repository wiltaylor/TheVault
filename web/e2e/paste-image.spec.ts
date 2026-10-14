import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { expect, logIn, test, vaultDir } from "./fixtures";

interface Block {
  kind: string;
  labels?: string[];
  fields?: Record<string, unknown>;
  blocks?: Block[];
}

interface Stored {
  id: string;
  version: string;
  wcl: string;
  note: { fields: Record<string, unknown>; blocks: Block[] };
}

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
}

const p = (text: string): Block => ({ kind: "paragraph", fields: { text } });

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

// A 2 by 2 red PNG, and a 1 by 1 one.
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAFklEQVR4nGP8z8DAwMDAxMDAwMDAAAANHQEDasKb6QAAAABJRU5ErkJggg==",
  "base64",
);
const PNG1 = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==", "base64");

interface Clip {
  files?: { name: string; type: string; b64: string }[];
  text?: string;
  html?: string;
}

/** Fire `event` ("paste" or "drop") at the editor with a DataTransfer holding `clip`. */
async function fire(doc: Locator, event: "paste" | "drop", clip: Clip, at?: { x: number; y: number }) {
  await doc.evaluate(
    (el, { event, clip, at }) => {
      const dt = new DataTransfer();
      for (const f of clip.files ?? []) {
        const bytes = Uint8Array.from(atob(f.b64), (c) => c.charCodeAt(0));
        dt.items.add(new File([bytes], f.name, { type: f.type }));
      }
      if (clip.text !== undefined) dt.setData("text/plain", clip.text);
      if (clip.html !== undefined) dt.setData("text/html", clip.html);
      if (event === "paste") {
        el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
        return;
      }
      const init = { dataTransfer: dt, bubbles: true, cancelable: true, clientX: at!.x, clientY: at!.y };
      el.dispatchEvent(new DragEvent("dragenter", init));
      el.dispatchEvent(new DragEvent("dragover", init));
      el.dispatchEvent(new DragEvent("drop", init));
    },
    { event, clip, at },
  );
}

const png = (name: string, buf = PNG) => ({ name, type: "image/png", b64: buf.toString("base64") });

/** Put the caret `n` characters into the paragraph holding `text`. */
async function caretIn(doc: Locator, text: string, n: number) {
  await doc.evaluate(
    (el, { text, n }) => {
      const para = Array.from(el.querySelectorAll("p")).find((x) => x.textContent === text)!;
      const node = para.firstChild!;
      const range = document.createRange();
      range.setStart(node, n);
      range.collapse(true);
      const sel = window.getSelection()!;
      sel.removeAllRanges();
      sel.addRange(range);
    },
    { text, n },
  );
  // ProseMirror reads a selection change a moment after it happens.
  await doc.page().waitForTimeout(100);
}

async function open(page: Page, admin: APIRequestContext, blocks: Block[]): Promise<{ note: Stored; doc: Locator }> {
  const note = await createTask(admin, `Paste ${token()}`, blocks);
  await logIn(page, `/notes/${note.id}`);
  const doc = page.getByRole("textbox", { name: "Body" });
  await expect(doc).toBeVisible();
  await doc.click();
  return { note, doc };
}

const ULID = /^[0-9A-HJKMNP-TV-Z]{26}$/;

test("pasting an image in the middle of a paragraph uploads it and puts it at the caret", async ({ page, admin }) => {
  const { note, doc } = await open(page, admin, [p("beforeafter"), p("last")]);
  await caretIn(doc, "beforeafter", 6);
  const name = `shot-${token()}.png`;
  await fire(doc, "paste", { files: [png(name)] });

  const img = doc.locator("figure.block-image img");
  await expect(img).toBeVisible();
  await expect(doc.getByTestId("image-upload")).toHaveCount(0);
  // Typing carries on after the image.
  await page.keyboard.type("X");
  await saveAndWait(page);

  const blocks = body(await stored(admin, note.id));
  expect(blocks.map((b) => b.kind)).toEqual(["paragraph", "image", "paragraph", "paragraph"]);
  expect(blocks[0]!.fields?.text).toBe("before");
  expect(blocks[2]!.fields?.text).toBe("Xafter");
  const asset = blocks[1]!.fields?.asset as string;
  expect(asset).toMatch(ULID);
  expect(readFileSync(join(vaultDir(), "assets", asset, name))).toEqual(PNG);
  expect((await stored(admin, note.id)).wcl).toContain(asset);
});

test("pasting two images puts both at the caret, in order", async ({ page, admin }) => {
  const { note, doc } = await open(page, admin, [p("onetwo")]);
  await caretIn(doc, "onetwo", 3);
  const a = `a-${token()}.png`;
  const b = `b-${token()}.png`;
  await fire(doc, "paste", { files: [png(a), png(b, PNG1)] });

  await expect(doc.locator("figure.block-image img")).toHaveCount(2);
  await saveAndWait(page);
  const blocks = body(await stored(admin, note.id));
  expect(blocks.map((x) => x.kind)).toEqual(["paragraph", "image", "image", "paragraph"]);
  const names = blocks.slice(1, 3).map((x) => readdirSync(join(vaultDir(), "assets", x.fields?.asset as string))[0]);
  expect(names).toEqual([a, b]);
});

test("an image the clipboard holds as a file wins over its HTML, and a data URL in HTML is uploaded", async ({ page, admin }) => {
  const { note, doc } = await open(page, admin, [p("start")]);
  await caretIn(doc, "start", 5);
  const name = `copied-${token()}.png`;
  await fire(doc, "paste", { files: [png(name)], html: `<img src="https://example.com/x.png">`, text: "" });
  await expect(doc.locator("figure.block-image img")).toHaveCount(1);

  await page.keyboard.press("End");
  await fire(doc, "paste", { html: `<p>said <b>hi</b></p><img src="data:image/png;base64,${PNG1.toString("base64")}"><p>bye</p>` });
  await expect(doc.locator("figure.block-image img")).toHaveCount(2);
  await saveAndWait(page);

  const blocks = body(await stored(admin, note.id));
  const images = blocks.filter((x) => x.kind === "image");
  expect(images).toHaveLength(2);
  expect(readdirSync(join(vaultDir(), "assets", images[0]!.fields?.asset as string))).toEqual([name]);
  const second = images[1]!.fields?.asset as string;
  expect(second).toMatch(ULID);
  const [file] = readdirSync(join(vaultDir(), "assets", second));
  expect(readFileSync(join(vaultDir(), "assets", second, file!))).toEqual(PNG1);
  // The text around the data URL image is pasted as usual, and nothing embeds a data URL.
  expect(blocks.some((x) => x.kind === "paragraph" && x.fields?.text === "said **hi**")).toBe(true);
  expect(blocks.some((x) => x.kind === "paragraph" && x.fields?.text === "bye")).toBe(true);
  expect((await stored(admin, note.id)).wcl).not.toContain("data:image");
});

test("dropping an image file puts it where it is dropped", async ({ page, admin }) => {
  const { note, doc } = await open(page, admin, [p("first"), p("second")]);
  const box = await doc.locator("p", { hasText: "second" }).boundingBox();
  const name = `drop-${token()}.png`;
  // Just left of the start of "second".
  await fire(doc, "drop", { files: [png(name)] }, { x: box!.x + 1, y: box!.y + box!.height / 2 });

  await expect(doc.locator("figure.block-image img")).toBeVisible();
  await saveAndWait(page);
  const blocks = body(await stored(admin, note.id));
  expect(blocks.map((x) => x.kind)).toEqual(["paragraph", "image", "paragraph"]);
  expect(blocks[2]!.fields?.text).toBe("second");
  expect(readdirSync(join(vaultDir(), "assets", blocks[1]!.fields?.asset as string))).toEqual([name]);
});

test("pasting plain text still pastes text", async ({ page, admin }) => {
  const { note, doc } = await open(page, admin, [p("abc")]);
  await caretIn(doc, "abc", 1);
  await fire(doc, "paste", { text: "XYZ" });
  await expect(doc.locator("p", { hasText: "aXYZbc" })).toBeVisible();
  await saveAndWait(page);
  expect(body(await stored(admin, note.id))).toEqual([p("aXYZbc")]);
});

test("an upload that fails takes the placeholder away and says why", async ({ page, admin }) => {
  const { note, doc } = await open(page, admin, [p("keepme")]);
  let release!: () => void;
  const held = new Promise<void>((r) => (release = r));
  await page.route("**/api/assets?**", async (route) => {
    await held;
    await route.fulfill({ status: 500, contentType: "application/json", body: JSON.stringify({ error: "io", message: "disk full" }) });
  });
  await caretIn(doc, "keepme", 4);
  await fire(doc, "paste", { files: [png("fail.png")] });

  const placeholder = doc.getByTestId("image-upload");
  await expect(placeholder).toHaveText(/Uploading image/);
  // Typing while it uploads goes where the caret is, and the placeholder stays in place.
  await page.keyboard.type("Z");
  release();

  await expect(placeholder).toHaveCount(0);
  await expect(page.getByTestId("field-body").getByRole("alert")).toContainText("Could not upload fail.png");
  await expect(doc.locator("figure.block-image")).toHaveCount(0);
  await saveAndWait(page);
  expect(body(await stored(admin, note.id))).toEqual([p("keepZme")]);
});
