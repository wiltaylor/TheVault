import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

interface Stored {
  id: string;
  version: string;
  wcl: string;
  note: { fields: Record<string, unknown>; blocks: { kind: string; labels?: string[]; blocks?: unknown[] }[] };
}

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
}

/** Create a task through the API with `body` as its content blocks. */
async function createTask(admin: APIRequestContext, title: string, body: unknown[] = []): Promise<Stored> {
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

function body(note: Stored): unknown[] | undefined {
  return note.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === "body")?.blocks;
}

/** A WCL string literal, as the canonical writer escapes it. */
function wclString(s: string): string {
  return `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

/** Open a note and put the caret in its body, which is always editable. */
async function editBody(page: Page, id: string) {
  await logIn(page, `/notes/${id}`);
  const doc = page.getByRole("textbox", { name: "Body" });
  await doc.click();
  return doc;
}

/** Select the first whole occurrence of `word` in the editor, as a drag across it would. */
async function selectWord(doc: Locator, word: string) {
  await doc.evaluate((root, w) => {
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const at = (n.textContent ?? "").search(new RegExp(`\\b${w}\\b`));
      if (at < 0) continue;
      const range = document.createRange();
      range.setStart(n, at);
      range.setEnd(n, at + w.length);
      const sel = window.getSelection()!;
      sel.removeAllRanges();
      sel.addRange(range);
      // The browser reports the change a moment later; report it now, so the editor has read
      // the new selection before the next action, as it has by the time a person moves on.
      document.dispatchEvent(new Event("selectionchange"));
      return;
    }
    throw new Error(`no ${w}`);
  }, word);
  await expect(doc.page().getByRole("toolbar", { name: "Body formatting" })).toBeVisible();
}

/** Leave the editor, which saves at once, and wait until the note is saved. */
async function saveAndWait(page: Page) {
  await page.getByLabel("Title", { exact: true }).focus();
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
}

test("typing each mark through shortcuts and the selection bubble saves the expected WCL", async ({ page, admin }) => {
  const word = `target${token()}`;
  const target = await createTask(admin, `Mention ${word}`);
  const note = await createTask(admin, `Marks ${token()}`);
  const doc = await editBody(page, note.id);
  const kb = page.keyboard;

  const shortcut = async (keys: string, text: string) => {
    await kb.press(keys);
    await kb.type(text);
    await kb.press(keys);
    await kb.type(" ");
  };
  await kb.type("plain ");
  await shortcut("ControlOrMeta+b", "bold");
  await shortcut("ControlOrMeta+i", "italic");
  await shortcut("ControlOrMeta+u", "under");
  await shortcut("ControlOrMeta+Shift+s", "strike");
  await shortcut("ControlOrMeta+e", "code");
  await shortcut("ControlOrMeta+Shift+h", "mark");

  // The rest is typed plain, then formatted through the bubble that a selection shows.
  const bubble = page.getByRole("toolbar", { name: "Body formatting" });
  await expect(bubble).toHaveCount(0);
  await kb.type("red tool docs");

  // "docs" becomes a link.
  for (let i = 0; i < 4; i++) await kb.press("Shift+ArrowLeft");
  await expect(bubble).toBeVisible();
  await bubble.getByRole("button", { name: "Link" }).click();
  await page.getByLabel("Link address").fill("https://example.com/docs");
  await page.getByLabel("Link address").press("Enter");
  await expect(doc.getByRole("link", { name: "docs" })).toBeVisible();
  await expect(doc).toBeFocused();
  await expect(bubble).toHaveCount(0);

  // "tool" becomes bold.
  await selectWord(doc, "tool");
  await bubble.getByRole("button", { name: "Bold" }).click();
  await expect(bubble.getByRole("button", { name: "Bold" })).toHaveAttribute("aria-pressed", "true");

  // "red" goes red. The bubble's select takes focus; the editor takes it back once a colour is chosen.
  await selectWord(doc, "red");
  await bubble.getByLabel("Text colour").selectOption("red");
  await expect(doc).toBeFocused();

  await kb.press("ControlOrMeta+End");
  await kb.type(" ");

  await kb.type("[[");
  const picker = page.getByRole("combobox", { name: "Find a note to mention" });
  await expect(picker).toBeFocused();
  await kb.type(word);
  await expect(page.getByRole("option", { name: new RegExp(word) })).toBeVisible();
  await kb.press("Enter");
  await expect(doc.getByTestId("mention")).toHaveText(`Mention ${word}`);
  await kb.type(" 2*3");

  await saveAndWait(page);

  const expected =
    "plain **bold** *italic* ++under++ ~~strike~~ `code` ==mark== {red}red{/} **tool** " +
    `[docs](https://example.com/docs) [[${target.id}]] 2\\*3`;
  const after = await stored(admin, note.id);
  expect(body(after)).toEqual([{ kind: "paragraph", fields: { text: expected } }]);
  expect(after.wcl).toContain(`text = ${wclString(expected)}`);

  // The saved markup renders as marks, not as the characters that encode them.
  await page.reload();
  const field = page.getByTestId("field-body");
  await expect(field.locator("strong").first()).toHaveText("bold");
  await expect(field.locator("em")).toHaveText("italic");
  await expect(field.locator("u")).toHaveText("under");
  await expect(field.locator("s")).toHaveText("strike");
  await expect(field.locator("code")).toHaveText("code");
  await expect(field.locator("mark")).toHaveText("mark");
  await expect(field.locator(".colour-red")).toHaveText("red");
  await expect(field.getByRole("link", { name: "docs" })).toHaveAttribute("href", "https://example.com/docs");
  await expect(field.getByTestId("mention")).toHaveText(`Mention ${word}`);
  await expect(field).toContainText("2*3");
});

test("text typed straight after choosing a mention and saved at once is all in the file", async ({ page, admin }) => {
  const word = `quick${token()}`;
  const target = await createTask(admin, `Mention ${word}`);
  const note = await createTask(admin, `Quick ${token()}`);
  await editBody(page, note.id);
  const kb = page.keyboard;

  await kb.type("see [[");
  await kb.type(word);
  await expect(page.getByRole("option", { name: new RegExp(word) })).toBeVisible();
  // A busy machine runs animation frames late. Anything that hands focus back to the editor on
  // the next frame drops the keys typed in between, so make that gap wide enough to type into.
  await page.evaluate(() => {
    window.requestAnimationFrame = (cb) => window.setTimeout(() => cb(performance.now()), 300);
  });
  // No waiting between choosing the note, typing and saving: every key must still land.
  await kb.press("Enter");
  await kb.type(" and 2*3 more");
  await saveAndWait(page);

  const expected = `see [[${target.id}]] and 2\\*3 more`;
  const after = await stored(admin, note.id);
  expect(body(after)).toEqual([{ kind: "paragraph", fields: { text: expected } }]);
  expect(after.wcl).toContain(`text = ${wclString(expected)}`);
});

test("headings, lists and quotes made with the / menu save as content blocks", async ({ page, admin }) => {
  const note = await createTask(admin, `Blocks ${token()}`);
  const doc = await editBody(page, note.id);
  const kb = page.keyboard;
  const menu = page.getByRole("listbox", { name: "Insert block" });
  const choose = async (query: string, option: string) => {
    await kb.type(`/${query}`);
    await expect(menu.getByRole("option").first()).toHaveText(new RegExp(option));
    await kb.press("Enter");
    await expect(menu).toHaveCount(0);
    await expect(doc).toBeFocused();
  };

  await choose("h2", "Heading 2");
  await kb.type("Plan");
  await kb.press("Enter");
  await choose("bullet", "Bullet list");
  await kb.type("one");
  await kb.press("Enter");
  await kb.type("two");
  await kb.press("Enter");
  await kb.press("Enter");
  await choose("quote", "Quote");
  await kb.type("said");
  await saveAndWait(page);

  expect(body(await stored(admin, note.id))).toEqual([
    { kind: "heading", fields: { level: 2, text: "Plan" } },
    {
      kind: "list",
      fields: { style: "bullet" },
      blocks: [
        { kind: "item", fields: { text: "one" } },
        { kind: "item", fields: { text: "two" } },
      ],
    },
    { kind: "quote", fields: { text: "said" } },
  ]);
});

test("renaming a mentioned note changes the rendered mention, not the mentioning note's file", async ({
  page,
  admin,
}) => {
  const target = await createTask(admin, `Before ${token()}`);
  const mentioning = await createTask(admin, `Mentions ${token()}`, [
    { kind: "paragraph", fields: { text: `See [[${target.id}]] and [[${target.id}|my label]].` } },
  ]);
  const before = await stored(admin, mentioning.id);

  await logIn(page, `/notes/${mentioning.id}`);
  const mentions = page.getByTestId("field-body").getByTestId("mention");
  await expect(mentions).toHaveText([target.note.fields.title as string, "my label"]);

  const renamed = `After ${token()}`;
  const put = await admin.put(`/api/notes/${target.id}`, {
    data: { note: { fields: { ...target.note.fields, title: renamed }, blocks: [] }, base_version: target.version },
  });
  expect(put.status(), await put.text()).toBe(200);

  await page.reload();
  await expect(mentions).toHaveText([renamed, "my label"]);
  // The body is the editor itself; opening the note saves nothing.
  await expect(page.getByRole("textbox", { name: "Body" }).getByTestId("mention")).toHaveText([renamed, "my label"]);
  await page.getByRole("textbox", { name: "Body" }).click();
  await page.getByLabel("Title", { exact: true }).focus();
  await page.waitForTimeout(2000);
  await expect(page.getByTestId("save-status")).toHaveText("");

  const after = await stored(admin, mentioning.id);
  expect(after.wcl).toBe(before.wcl);
  expect(after.version).toBe(before.version);
});

test("a Solid component inside a node view keeps focus and input", async ({ page, admin }) => {
  const note = await createTask(admin, `Focus ${token()}`);
  const doc = await editBody(page, note.id);
  const kb = page.keyboard;
  await kb.type("abc");
  await kb.type("[[");

  const picker = page.getByRole("combobox", { name: "Find a note to mention" });
  await expect(picker).toBeFocused();
  // Keys that ProseMirror would act on go to the input instead.
  await kb.type("hello world");
  await kb.press("Backspace");
  await kb.press("ArrowLeft");
  await kb.press("Home");
  await kb.type(">");
  await kb.press("ControlOrMeta+b");
  await expect(picker).toHaveValue(">hello worl");
  await expect(picker).toBeFocused();

  // A click inside the component lands in it, too.
  await doc.locator("p").first().click({ position: { x: 1, y: 1 } });
  await expect(picker).toHaveCount(0);
  await doc.press("End");
  await kb.type("[[");
  await page.getByRole("combobox", { name: "Find a note to mention" }).click();
  await kb.type("xyz");
  await expect(page.getByRole("combobox", { name: "Find a note to mention" })).toHaveValue("xyz");

  // Escape hands focus back to the editor where the picker stood.
  await kb.press("Escape");
  await expect(page.getByRole("combobox", { name: "Find a note to mention" })).toHaveCount(0);
  await expect(doc).toBeFocused();
  await kb.type("d");
  await saveAndWait(page);

  expect(body(await stored(admin, note.id))).toEqual([{ kind: "paragraph", fields: { text: "abcd" } }]);
});

test("the bubble shows only on a selection, and bolds the selected text", async ({ page, admin }) => {
  const note = await createTask(admin, `Bubble ${token()}`, [{ kind: "paragraph", fields: { text: "make this strong" } }]);
  const doc = await editBody(page, note.id);
  const bubble = page.getByRole("toolbar", { name: "Body formatting" });
  const kb = page.keyboard;

  // A caret is not a selection: typing never has the bubble over it.
  await kb.press("End");
  await expect(bubble).toHaveCount(0);
  await kb.type("!");
  await expect(bubble).toHaveCount(0);

  await kb.press("ArrowLeft");
  for (let i = 0; i < 6; i++) await kb.press("Shift+ArrowLeft");
  await expect(bubble).toBeVisible();
  // It sits just above the selection, clear of the line.
  const line = (await doc.locator("p").first().boundingBox())!;
  const box = (await bubble.boundingBox())!;
  expect(box.y + box.height).toBeLessThanOrEqual(line.y + 2);
  await expect(bubble.getByRole("button")).toHaveText(["B", "I", "U", "S", "</>", "H", "Link", "@"]);

  await bubble.getByRole("button", { name: "Bold" }).click();
  await expect(doc.locator("strong")).toHaveText("strong");
  await kb.press("ArrowRight");
  await expect(bubble).toHaveCount(0);
  await saveAndWait(page);
  expect(body(await stored(admin, note.id))).toEqual([{ kind: "paragraph", fields: { text: "make this **strong**!" } }]);
});

test("+ beside a block opens the block menu and inserts after that block", async ({ page, admin }) => {
  const note = await createTask(admin, `Plus ${token()}`, [
    { kind: "paragraph", fields: { text: "first" } },
    { kind: "paragraph", fields: { text: "second" } },
  ]);
  await logIn(page, `/notes/${note.id}`);
  const doc = page.getByRole("textbox", { name: "Body" });
  await doc.locator("p", { hasText: "first" }).hover();
  const add = page.getByTestId("field-body").getByRole("button", { name: "Add block" });
  await expect(add).toBeVisible();
  await add.click();
  const menu = page.getByRole("listbox", { name: "Insert block" });
  await expect(menu).toBeVisible();
  await page.keyboard.type("code");
  await expect(menu.getByRole("option")).toHaveText([/Code/]);
  await page.keyboard.press("Enter");
  await expect(menu).toHaveCount(0);
  await saveAndWait(page);
  expect(body(await stored(admin, note.id))).toEqual([
    { kind: "paragraph", fields: { text: "first" } },
    { kind: "code", fields: { source: "" } },
    { kind: "paragraph", fields: { text: "second" } },
  ]);

  // Escape after "+" leaves no stray "/" behind.
  await doc.locator("p", { hasText: "second" }).hover();
  await add.click();
  await expect(menu).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(doc).not.toContainText("/");
});

test("the / menu stays fully inside the window near the bottom of the page", async ({ page, admin }) => {
  const lines = Array.from({ length: 40 }, (_, i) => ({ kind: "paragraph", fields: { text: `line ${i + 1}` } }));
  const note = await createTask(admin, `Low ${token()}`, lines);
  await page.setViewportSize({ width: 1200, height: 700 });
  await logIn(page, `/notes/${note.id}`);
  const doc = page.getByRole("textbox", { name: "Body" });
  const last = doc.locator("p", { hasText: "line 40" });
  await last.scrollIntoViewIfNeeded();
  await last.click();
  await page.keyboard.press("End");
  await page.keyboard.type(" /");
  const menu = page.getByRole("listbox", { name: "Insert block" });
  await expect(menu).toBeVisible();
  const box = (await menu.boundingBox())!;
  const vp = page.viewportSize()!;
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.y + box.height).toBeLessThanOrEqual(vp.height);
  expect(box.x + box.width).toBeLessThanOrEqual(vp.width);
  // Nothing else of the editor is drawn over it.
  const centre = { x: box.x + box.width / 2, y: box.y + 20 };
  expect(await page.evaluate(({ x, y }) => !!document.elementFromPoint(x, y)?.closest(".suggest-menu"), centre)).toBe(true);
});

test("on a phone the bubble wraps so every button is in view", async ({ page, admin }) => {
  const note = await createTask(admin, `Phone ${token()}`, [{ kind: "paragraph", fields: { text: "make this strong" } }]);
  await page.setViewportSize({ width: 390, height: 844 });
  const doc = await editBody(page, note.id);
  await page.keyboard.press("End");
  for (let i = 0; i < 6; i++) await page.keyboard.press("Shift+ArrowLeft");
  const bubble = page.getByRole("toolbar", { name: "Body formatting" });
  await expect(bubble).toBeVisible();
  const box = (await bubble.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(390);
  for (const control of await bubble.locator("button, select").all()) {
    const b = (await control.boundingBox())!;
    expect(b.x).toBeGreaterThanOrEqual(box.x);
    expect(b.x + b.width).toBeLessThanOrEqual(box.x + box.width + 0.5);
    expect(b.y + b.height).toBeLessThanOrEqual(box.y + box.height + 0.5);
  }
  expect(await bubble.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  await expect(doc).toBeFocused();
});
