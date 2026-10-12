import type { APIRequestContext, Page } from "@playwright/test";
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

async function editBody(page: Page, id: string) {
  await logIn(page, `/notes/${id}`);
  await page.getByRole("button", { name: "Edit" }).click();
  const doc = page.getByRole("textbox", { name: "Body" });
  await doc.click();
  return doc;
}

async function saveAndWait(page: Page) {
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("button", { name: "Edit" })).toBeVisible();
}

test("typing each mark through shortcuts and the toolbar saves the expected WCL", async ({ page, admin }) => {
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

  const toolbar = page.getByRole("toolbar", { name: "Body formatting" });
  // The toolbar's selects take focus; the editor takes it back once a colour is chosen.
  await toolbar.getByLabel("Text colour").selectOption("red");
  await expect(doc).toBeFocused();
  await kb.type("red");
  await toolbar.getByLabel("Text colour").selectOption("");
  await expect(doc).toBeFocused();
  await kb.type(" ");

  await toolbar.getByRole("button", { name: "Bold" }).click();
  await kb.type("tool");
  await toolbar.getByRole("button", { name: "Bold" }).click();
  await kb.type(" ");

  await kb.type("docs");
  for (let i = 0; i < 4; i++) await kb.press("Shift+ArrowLeft");
  await toolbar.getByRole("button", { name: "Link" }).click();
  await page.getByLabel("Link address").fill("https://example.com/docs");
  await page.getByLabel("Link address").press("Enter");
  await expect(doc.getByRole("link", { name: "docs" })).toBeVisible();
  await expect(doc).toBeFocused();
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

test("headings, lists and quotes made with the toolbar save as content blocks", async ({ page, admin }) => {
  const note = await createTask(admin, `Blocks ${token()}`);
  const doc = await editBody(page, note.id);
  const kb = page.keyboard;
  const toolbar = page.getByRole("toolbar", { name: "Body formatting" });

  await toolbar.getByLabel("Block type").selectOption("h2");
  await expect(doc).toBeFocused();
  await kb.type("Plan");
  await kb.press("Enter");
  await toolbar.getByRole("button", { name: "Bullet list" }).click();
  await kb.type("one");
  await kb.press("Enter");
  await kb.type("two");
  await kb.press("Enter");
  await kb.press("Enter");
  await toolbar.getByLabel("Block type").selectOption("quote");
  await expect(doc).toBeFocused();
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
  await page.getByRole("button", { name: "Edit" }).click();
  await expect(page.getByRole("textbox", { name: "Body" }).getByTestId("mention")).toHaveText([renamed, "my label"]);

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
