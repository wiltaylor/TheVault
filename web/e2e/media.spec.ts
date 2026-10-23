import { mkdirSync, readFileSync } from "node:fs";
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

function body(note: Stored): Block[] | undefined {
  return note.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === "body")?.blocks;
}

/** The note's body: always an editor. */
async function startEditing(page: Page): Promise<Locator> {
  return page.getByRole("textbox", { name: "Body" });
}

/** Leave the editor, which saves at once, and wait until the note is saved. */
async function saveAndWait(page: Page) {
  await page.getByLabel("Title", { exact: true }).focus();
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
}

/** Leave the editor and give an autosave time to run, when nothing may need saving. */
async function settle(page: Page) {
  await page.getByLabel("Title", { exact: true }).focus();
  await page.waitForTimeout(2000);
  await expect(page.getByTestId("save-status")).not.toHaveText(/Saving|Unsaved/);
}

/** The text of a CodeMirror view: its lines joined with newlines. */
async function codeMirrorText(view: Locator): Promise<string> {
  return view.locator(".cm-content").evaluate((el) =>
    Array.from(el.querySelectorAll(".cm-line"), (line) => line.textContent ?? "").join("\n"),
  );
}

async function imageLoaded(img: Locator): Promise<boolean> {
  return img.evaluate((el) => (el as HTMLImageElement).complete && (el as HTMLImageElement).naturalWidth > 0);
}

// A 2 by 2 red PNG.
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAFklEQVR4nGP8z8DAwMDAxMDAwMDAAAANHQEDasKb6QAAAABJRU5ErkJggg==",
  "base64",
);

test("uploading an image shows it and stores it under assets/", async ({ page, admin }) => {
  const note = await createTask(admin, `Image ${token()}`);
  await logIn(page, `/notes/${note.id}`);
  const doc = await startEditing(page);
  await doc.click();

  const filename = `pixel-${token()}.png`;
  const chooser = page.waitForEvent("filechooser");
  await page.keyboard.type("/image");
  await page.keyboard.press("Enter");
  await (await chooser).setFiles({ name: filename, mimeType: "image/png", buffer: PNG });

  const img = doc.locator("figure.block-image img");
  await expect(img).toBeVisible();
  await expect.poll(() => imageLoaded(img)).toBe(true);
  // At rest the image shows alone; selected, it offers Edit and Remove.
  await expect(doc.getByLabel("Alt text")).toHaveCount(0);
  await img.click();
  await expect(doc.locator("figure.block-image.selected")).toHaveCount(1);
  await expect(doc.getByRole("button", { name: "Remove image" })).toBeVisible();
  await doc.getByRole("button", { name: "Edit image" }).click();
  const dialog = page.getByRole("dialog", { name: "Image" });
  await dialog.getByLabel("Alt text").fill("A red square");
  await dialog.getByLabel("Caption").fill("Two pixels wide");
  await dialog.getByRole("button", { name: "Save" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(doc.locator("figure.block-image figcaption")).toHaveText("Two pixels wide");
  await saveAndWait(page);

  const blocks = body(await stored(admin, note.id)) ?? [];
  expect(blocks).toHaveLength(1);
  const image = blocks[0] as Block;
  expect(image.kind).toBe("image");
  const asset = image.fields?.asset as string;
  expect(asset).toMatch(/^[0-9A-HJKMNP-TV-Z]{26}$/);
  expect(image.fields).toEqual({ asset, alt: "A red square", caption: "Two pixels wide" });

  // The upload is the file, byte for byte, at assets/<ULID>/<filename>.
  expect(readFileSync(join(vaultDir(), "assets", asset, filename))).toEqual(PNG);

  await page.reload();
  const shown = page.getByTestId("field-body").getByRole("img", { name: "A red square" });
  await expect(shown).toBeVisible();
  await expect.poll(() => imageLoaded(shown)).toBe(true);
  await expect(page.getByTestId("field-body").locator("figure.block-image figcaption")).toHaveText("Two pixels wide");
});

test("a file upload becomes an asset card that downloads the file", async ({ page, admin }) => {
  const note = await createTask(admin, `Asset ${token()}`);
  await logIn(page, `/notes/${note.id}`);
  const doc = await startEditing(page);
  await doc.click();

  const filename = `notes-${token()}.txt`;
  const chooser = page.waitForEvent("filechooser");
  // The "+" beside a block opens the same menu, for screens without a keyboard.
  await doc.locator("p").first().hover();
  await page.getByRole("button", { name: "Add block" }).click();
  await page.getByRole("listbox", { name: "Insert block" }).getByRole("option", { name: "File" }).click();
  await (await chooser).setFiles({ name: filename, mimeType: "text/plain", buffer: Buffer.from("hello\n") });
  await expect(doc.getByRole("link", { name: filename })).toBeVisible();
  await saveAndWait(page);

  const blocks = body(await stored(admin, note.id)) ?? [];
  expect(blocks.map((b) => b.kind)).toEqual(["asset"]);
  const asset = blocks[0]?.fields?.asset as string;
  expect(readFileSync(join(vaultDir(), "assets", asset, filename), "utf8")).toBe("hello\n");
  await expect(page.getByTestId("field-body").getByRole("link", { name: filename })).toHaveAttribute(
    "href",
    `/api/assets/${asset}`,
  );
});

test("a code block keeps tabs and trailing spaces through save and reload", async ({ page, admin }) => {
  // Tabs, trailing spaces and tabs, a whitespace-only line and no final newline.
  const source = 'fn main() {\n\tlet x = 1;   \n\t\tprintln!("{x}");\t\n}\n  \n\ttrailing   ';
  const note = await createTask(admin, `Code ${token()}`, [
    { kind: "code", fields: { language: "rust", caption: "main.rs", source } },
  ]);
  await logIn(page, `/notes/${note.id}`);

  const field = page.getByTestId("field-body");
  const view = field.locator(".block-code .cm-editor");
  await expect(view).toBeVisible();
  expect(await codeMirrorText(view)).toBe(source);
  // Rust highlighting: `fn` and `let` are keywords.
  await expect(view.locator(".tok-keyword").first()).toBeVisible();

  const doc = await startEditing(page);
  const code = doc.getByRole("textbox", { name: "Code" });
  await code.click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type(" end");
  await page.keyboard.press("Tab");
  await page.keyboard.type("  ");
  await doc.getByLabel("Language").selectOption({ label: "Python" });
  await saveAndWait(page);

  const edited = `${source} end\t  `;
  expect(body(await stored(admin, note.id))).toEqual([
    { kind: "code", fields: { language: "python", caption: "main.rs", source: edited } },
  ]);

  await page.reload();
  await expect(view).toBeVisible();
  expect(await codeMirrorText(view)).toBe(edited);

  // Visiting the code again without touching it changes nothing in it.
  await (await startEditing(page)).getByRole("textbox", { name: "Code" }).click();
  await settle(page);
  expect(body(await stored(admin, note.id))).toEqual([
    { kind: "code", fields: { language: "python", caption: "main.rs", source: edited } },
  ]);
});

test("a code block reads as numbered code with a copy button until the cursor is in it", async ({ page, admin }) => {
  const note = await createTask(admin, `Plain code ${token()}`, [
    { kind: "code", fields: { language: "rust", caption: "main.rs", source: "fn main() {}\n" } },
    { kind: "paragraph", fields: { text: "After the code." } },
  ]);
  await logIn(page, `/notes/${note.id}`);
  const block = page.getByTestId("field-body").locator(".block-code");

  // At rest: a header with the file name, language and a copy button over numbered code, and no
  // fields.
  const header = block.locator(".code-header");
  await expect(header.locator(".code-filename")).toHaveText("main.rs");
  await expect(header.locator(".code-language")).toHaveText("Rust");
  await expect(block.getByLabel("Language")).toHaveCount(0);
  await expect(block.getByLabel("Caption")).toHaveCount(0);
  await expect(block.locator(".cm-lineNumbers")).toBeVisible();
  await expect(block.locator(".cm-foldGutter")).toBeHidden();

  // Copy puts the source on the clipboard.
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await block.getByRole("button", { name: "Copy code" }).click();
  await expect(block.getByRole("button", { name: "Copy code" })).toHaveText("Copied");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("fn main() {}\n");

  // In the code: the language and file name fields show.
  await block.getByRole("textbox", { name: "Code" }).click();
  await expect(block.getByLabel("Language")).toBeVisible();
  await expect(block.getByLabel("Caption")).toHaveValue("main.rs");
  await expect(block.locator(".cm-foldGutter")).toBeVisible();
  await expect(header).toHaveCount(0);

  // Moving between its own fields keeps them open.
  await block.getByLabel("Caption").fill("lib.rs");
  await expect(block.getByLabel("Language")).toBeVisible();

  // Leaving the block puts it back to plain code, with the new name.
  await page.getByText("After the code.").click();
  await expect(block.getByLabel("Language")).toHaveCount(0);
  await expect(block.locator(".code-filename")).toHaveText("lib.rs");
  await expect(block.locator(".cm-lineNumbers")).toBeVisible();
});

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
    await what.screenshot({ path: join(dir, `${name}-${theme}.png`) });
  }
  await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
}

test("a shell command and its output are a bash and a stdout code block, inserted from the / menu", async ({
  page,
  admin,
}) => {
  const note = await createTask(admin, `Shell ${token()}`, [{ kind: "paragraph", fields: { text: "Try it first:" } }]);
  await logIn(page, `/notes/${note.id}`);
  const doc = await startEditing(page);
  await doc.locator("p", { hasText: "Try it first:" }).click();
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");

  const menu = page.getByRole("listbox", { name: "Insert block" });
  await page.keyboard.type("/shell");
  await expect(menu.getByRole("option")).toHaveText([/Shell command/]);
  await page.keyboard.press("Enter");
  await page.keyboard.type("/output");
  await expect(menu.getByRole("option")).toHaveText([/Output/]);
  await page.keyboard.press("Enter");
  await expect(menu).toHaveCount(0);

  const command = "vault import --dry-run export/";
  const output = "42 notes would be created\n3 skipped";
  await doc.getByRole("textbox", { name: "Code" }).click();
  await page.keyboard.type(command);
  await doc.getByRole("textbox", { name: "Output" }).click();
  await page.keyboard.type(output);
  await saveAndWait(page);

  const expected: Block[] = [
    { kind: "paragraph", fields: { text: "Try it first:" } },
    { kind: "code", fields: { language: "bash", source: command } },
    { kind: "code", fields: { language: "stdout", source: output } },
  ];
  const first = await stored(admin, note.id);
  expect(body(first)).toEqual(expected);
  expect(first.wcl).toContain('language = "stdout"');

  await page.reload();
  const blocks = page.getByTestId("field-body").locator(".block-code");
  await expect(blocks).toHaveCount(2);
  const shell = blocks.nth(0);
  const out = blocks.nth(1);

  // The command: highlighted as shell, labelled Bash, with line numbers.
  await expect(shell.locator(".code-language")).toHaveText("Bash");
  expect(await codeMirrorText(shell.locator(".cm-editor"))).toBe(command);
  await expect(shell.locator(".cm-content [class*='tok-']").first()).toBeVisible();
  await expect(shell.locator(".cm-lineNumbers")).toBeVisible();

  // The output: labelled Output, plain text, no gutter, on its own muted background, copyable.
  await expect(out).toHaveClass(/block-output/);
  await expect(out.locator(".code-language")).toHaveText("Output");
  expect(await codeMirrorText(out.locator(".cm-editor"))).toBe(output);
  await expect(out.locator(".cm-gutters")).toHaveCount(0);
  await expect(out.locator(".cm-content [class*='tok-']")).toHaveCount(0);
  await expect(out.getByRole("button", { name: "Copy code" })).toBeVisible();
  const background = (l: Locator) => l.evaluate((el) => getComputedStyle(el).backgroundColor);
  expect(await background(out.locator(".cm-editor"))).not.toBe(await background(shell.locator(".cm-editor")));

  // In the output block, the picker shows Output; the shell block's shows Bash.
  await out.getByRole("textbox", { name: "Output" }).click();
  await expect(out.getByLabel("Language")).toHaveValue("stdout");
  await shell.getByRole("textbox", { name: "Code" }).click();
  await expect(shell.getByLabel("Language").locator("option:checked")).toHaveText("Bash");

  // Visiting them again without a change saves nothing new.
  await settle(page);
  const second = await stored(admin, note.id);
  expect(body(second)).toEqual(expected);
  expect(second.wcl).toBe(first.wcl);

  await page.reload();
  await expect(blocks).toHaveCount(2);
  await shots(page, "stdout", page.getByTestId("field-body"));
});

test("a valid Mermaid diagram renders an SVG; an invalid one shows the error and keeps the source", async ({
  page,
  admin,
}) => {
  const valid = "graph LR\n  export --> importer --> vault\n";
  const note = await createTask(admin, `Mermaid ${token()}`, [{ kind: "mermaid", fields: { source: valid } }]);
  await logIn(page, `/notes/${note.id}`);

  const block = page.getByTestId("field-body").locator(".block-mermaid");
  await expect(block.locator("svg")).toBeVisible();
  await expect(block.locator("svg")).toContainText("importer");

  const doc = await startEditing(page);
  const source = doc.getByLabel("Mermaid source");
  await expect(source).toHaveValue(valid);
  const editing = doc.locator(".block-mermaid");
  await expect(editing.locator(".mermaid-render svg")).toBeVisible();

  // A live render follows the source as it is typed.
  await source.fill("graph LR\n  export --> checker\n");
  await expect(editing.locator(".mermaid-render svg")).toContainText("checker");

  const invalid = "graph LR\n  export --> [unclosed\n";
  await source.fill(invalid);
  await expect(editing.getByRole("alert")).toBeVisible();
  await expect(editing.locator(".mermaid-render svg")).toHaveCount(0);
  await expect(source).toHaveValue(invalid);
  await saveAndWait(page);

  expect(body(await stored(admin, note.id))).toEqual([{ kind: "mermaid", fields: { source: invalid } }]);

  await page.reload();
  await expect(block.getByRole("alert")).toBeVisible();
  await expect(block.getByLabel("Mermaid source")).toHaveValue(invalid);
});
