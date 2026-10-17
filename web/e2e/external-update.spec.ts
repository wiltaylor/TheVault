import type { APIRequestContext, Page } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// Typing after the open note was written somewhere else (a sync round pushing the device's
// queued edit and taking the server's result back, another device, a script) still reaches the
// server: a write that changed nothing this page edited is taken under the edits, and only a
// real clash shows the conflict view. Nothing typed is ever dropped while the page says Saved.

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

const TYPE = `label = "Synced"
plural = "Synced"

field status {
  kind    = "select"
  label   = "Status"
  options = ["todo", "doing", "done"]
}

field body {
  kind  = "content"
  label = "Body"
}

template default {
  tab "Main" {
    slot "status"
    slot "body"
  }
}
`;

interface Stored {
  id: string;
  version: string;
  note: { fields: Record<string, unknown>; blocks: { kind: string; labels?: string[]; blocks?: { kind: string; fields?: Record<string, unknown> }[] }[] };
}

async function setup(admin: APIRequestContext) {
  const name = `ext_${token()}`;
  const put = await admin.put(`/api/types/${name}`, { data: { wcl: TYPE } });
  expect(put.status(), await put.text()).toBe(200);
  const res = await admin.post(`/api/notes?type=${name}`, {
    data: {
      note: {
        fields: { title: `Synced ${token()}`, status: "todo" },
        blocks: [{ kind: "content", labels: ["body"], blocks: [{ kind: "paragraph", fields: { text: "First line" } }] }],
      },
    },
  });
  expect(res.status(), await res.text()).toBe(201);
  const note = (await res.json()) as Stored;
  return { note, title: note.note.fields.title as string };
}

async function stored(admin: APIRequestContext, id: string): Promise<Stored> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return (await res.json()) as Stored;
}

/** Write the note as stored with `fields` changed, as another writer would: a new version. */
async function writeElsewhere(admin: APIRequestContext, id: string, fields: Record<string, unknown> = {}) {
  const now = await stored(admin, id);
  const res = await admin.put(`/api/notes/${id}`, {
    data: { note: { ...now.note, fields: { ...now.note.fields, ...fields } }, base_version: now.version },
  });
  expect(res.status(), await res.text()).toBe(200);
  const next = (await res.json()) as Stored;
  expect(next.version).not.toBe(now.version);
  return next;
}

/** Type `text` at the end of the title. */
async function typeInTitle(page: Page, text: string) {
  const title = page.getByLabel("Title");
  await title.click();
  await page.keyboard.press("End");
  await page.keyboard.type(text);
}

const status = (page: Page) => page.getByTestId("save-status");

for (const live of [false, true]) {
  const how = live ? "with the live stream" : "with the live stream down";

  test(`typing after the same note was written elsewhere unchanged is saved (${how})`, async ({ page, admin }) => {
    const { note, title } = await setup(admin);
    if (!live) await page.route("**/api/events", (route) => route.abort());
    await logIn(page, `/notes/${note.id}`);
    await expect(page.getByLabel("Title")).toHaveText(title);

    await typeInTitle(page, " one");
    await expect(status(page)).toHaveText("Saved");
    await expect.poll(async () => (await stored(admin, note.id)).note.fields.title).toBe(`${title} one`);

    // The same note comes back with a new version, as a sync round's push does.
    await writeElsewhere(admin, note.id);
    await typeInTitle(page, " two");
    await expect.poll(async () => (await stored(admin, note.id)).note.fields.title).toBe(`${title} one two`);
    await expect(status(page)).toHaveText("Saved");
    await expect(page.getByTestId("conflict-view")).toHaveCount(0);

    // And the body, edited after the update, saves too.
    await page.getByRole("textbox", { name: "Body" }).getByText("First line").click();
    await page.keyboard.press("End");
    await page.keyboard.type(" and more");
    await expect
      .poll(async () => (await stored(admin, note.id)).note.blocks.find((b) => b.labels?.[0] === "body")?.blocks?.[0]?.fields?.text)
      .toBe("First line and more");
    await expect(status(page)).toHaveText("Saved");
  });

  test(`typing after another field was changed elsewhere keeps both changes (${how})`, async ({ page, admin }) => {
    const { note, title } = await setup(admin);
    if (!live) await page.route("**/api/events", (route) => route.abort());
    await logIn(page, `/notes/${note.id}`);
    await expect(page.getByLabel("Title")).toHaveText(title);

    await typeInTitle(page, " one");
    await expect(status(page)).toHaveText("Saved");

    await writeElsewhere(admin, note.id, { status: "done" });
    await typeInTitle(page, " two");
    await expect.poll(async () => (await stored(admin, note.id)).note.fields.title).toBe(`${title} one two`);
    expect((await stored(admin, note.id)).note.fields.status).toBe("done");
    await expect(page.getByTestId("field-status").getByLabel("Status")).toHaveValue("done");
    await expect(status(page)).toHaveText("Saved");
    await expect(page.getByTestId("conflict-view")).toHaveCount(0);
  });
}

test("a real clash shows the conflict view, and the title cannot take typing the page would not save", async ({ page, admin }) => {
  const { note, title } = await setup(admin);
  await page.route("**/api/events", (route) => route.abort());
  await logIn(page, `/notes/${note.id}`);
  await expect(page.getByLabel("Title")).toHaveText(title);

  await writeElsewhere(admin, note.id, { title: `${title} elsewhere` });
  await typeInTitle(page, " mine");
  const conflict = page.getByTestId("conflict-view");
  await expect(conflict).toBeVisible();
  await expect(status(page)).not.toHaveText("Saved");
  await expect(page.getByLabel("Title")).toHaveAttribute("contenteditable", "false");

  // Saving mine sends what was typed.
  await conflict.getByRole("button", { name: "Save my version" }).click();
  await expect(conflict).toHaveCount(0);
  await expect.poll(async () => (await stored(admin, note.id)).note.fields.title).toBe(`${title} mine`);
});
