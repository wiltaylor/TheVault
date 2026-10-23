import type { APIRequestContext, Page } from "@playwright/test";
import { activePane, expect, logIn, test } from "./fixtures";

// Typing `@` at the start of a word opens the mention picker, as `[[` does. Besides the notes
// found, the picker offers "New <type>" for the text typed, which inserts a placeholder link
// `[[+type|title]]`. Clicking a placeholder creates the note, rewrites the placeholder as a
// mention of it in the note that holds it, and opens the new note. "Ask <Label>" opens a request
// box instead, and Enter commits the call of the agent as one link (agents G19).

interface Stored {
  id: string;
  version: string;
  type: string;
  wcl: string;
  note: { fields: Record<string, unknown>; blocks: { kind: string; labels?: string[]; blocks?: { kind: string; fields?: Record<string, unknown>; blocks?: unknown[] }[] }[] };
}

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
}

const para = (text: string) => ({ kind: "paragraph", fields: { text } });

async function createNote(admin: APIRequestContext, type: string, title: string, body: unknown[] = []): Promise<Stored> {
  const blocks = body.length ? [{ kind: "content", labels: ["body"], blocks: body }] : [];
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields: { title }, blocks } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as Stored;
}

async function stored(admin: APIRequestContext, id: string): Promise<Stored> {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status()).toBe(200);
  return (await res.json()) as Stored;
}

/** The text of the first paragraph of the note's body. */
async function bodyText(admin: APIRequestContext, id: string): Promise<string> {
  const n = await stored(admin, id);
  const body = n.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === "body");
  return String(body?.blocks?.[0]?.fields?.text ?? "");
}

/** The ids of the notes whose title is exactly `title`. */
async function notesTitled(admin: APIRequestContext, title: string): Promise<string[]> {
  const res = await admin.get(`/api/search?${new URLSearchParams({ q: title, page_size: "50" })}`);
  expect(res.status()).toBe(200);
  const page = (await res.json()) as { rows: { id: string; title: string }[] };
  return page.rows.filter((r) => r.title === title).map((r) => r.id);
}

/** Open a note and put the caret at the end of its body. */
async function editBody(page: Page, id: string) {
  await logIn(page, `/notes/${id}`);
  const doc = activePane(page).getByRole("textbox", { name: "Body" });
  await doc.click();
  await page.keyboard.press("ControlOrMeta+End");
  return doc;
}

async function saveAndWait(page: Page) {
  await activePane(page).getByLabel("Title", { exact: true }).focus();
  await expect(activePane(page).getByTestId("save-status")).toHaveText("Saved");
}

const picker = (page: Page) => page.getByRole("combobox", { name: "Find a note to mention" });
const ULID = "[0-9A-HJKMNP-TV-Z]{26}";

test("typing @ and a word finds a note to mention", async ({ page, admin }) => {
  const word = `atpick${token()}`;
  const target = await createNote(admin, "task", `Target ${word}`);
  const note = await createNote(admin, "task", `Source ${token()}`);
  const doc = await editBody(page, note.id);
  const kb = page.keyboard;

  await kb.type("ask @");
  await expect(picker(page)).toBeFocused();
  await expect(page.getByTestId("mention-picker")).toHaveAttribute("data-trigger", "@");
  await kb.type(word);
  await expect(page.getByTestId("mention-result").filter({ hasText: word })).toBeVisible();
  // Below the notes found: new notes of a type, titled with the text.
  await expect(page.getByTestId("mention-new").first()).toContainText(`New Task: “${word}”`);
  await kb.press("Enter");
  await expect(doc.getByTestId("mention")).toHaveText(`Target ${word}`);
  await kb.type(" now");
  await saveAndWait(page);
  expect(await bodyText(admin, note.id)).toBe(`ask [[${target.id}]] now`);
});

test("@ inside a word is text, and a dropped @ picker leaves what was typed", async ({ page, admin }) => {
  const note = await createNote(admin, "task", `Email ${token()}`);
  await editBody(page, note.id);
  const kb = page.keyboard;

  await kb.type("mail wil@example.com");
  await expect(page.getByTestId("mention-picker")).toHaveCount(0);

  // Escape leaves the @ and the text after it.
  await kb.type(" or @someone");
  await expect(picker(page)).toHaveValue("someone");
  await kb.press("Escape");
  await expect(page.getByTestId("mention-picker")).toHaveCount(0);

  // Backspace in the empty box takes the picker back to a bare @.
  await kb.type(" @");
  await expect(picker(page)).toBeFocused();
  await kb.press("Backspace");
  await expect(page.getByTestId("mention-picker")).toHaveCount(0);
  await kb.type("x");
  await saveAndWait(page);
  expect(await bodyText(admin, note.id)).toBe("mail wil@example.com or @someone @x");
});

test("New Task inserts a placeholder; a click creates the note, links it and opens it", async ({ page, admin }) => {
  const title = `Fix pump ${token()}`;
  const note = await createNote(admin, "task", `Plans ${token()}`);
  const doc = await editBody(page, note.id);
  const kb = page.keyboard;

  await kb.type("next @");
  await kb.type(title);
  const newTask = page.getByTestId("mention-new").filter({ hasText: "New Task:" });
  await expect(newTask).toBeVisible();
  await newTask.click();
  const chip = doc.getByTestId("placeholder-link");
  await expect(chip).toHaveText(title);
  await expect(chip).toHaveAttribute("title", "Click to create this Task");
  await saveAndWait(page);
  const saved = await stored(admin, note.id);
  expect(saved.wcl).toContain(`text = "next [[+task|${title}]]"`);
  expect(await notesTitled(admin, title)).toEqual([]);

  // A double click makes one note, not two.
  await chip.dblclick();
  await expect(activePane(page).getByLabel("Title", { exact: true })).toHaveText(title);
  await expect(page).not.toHaveURL(new RegExp(`/notes/${note.id}$`));
  const created = await notesTitled(admin, title);
  expect(created).toHaveLength(1);
  expect(page.url()).toContain(`/notes/${created[0]}`);
  await expect.poll(() => bodyText(admin, note.id)).toBe(`next [[${created[0]}]]`);
  expect((await stored(admin, created[0]!)).type).toBe("task");
});

test("a placeholder chosen by type: shows that type, and a Ctrl+click opens the note in a background tab", async ({ page, admin }) => {
  const title = `Kickoff ${token()}`;
  const note = await createNote(admin, "task", `Plans ${token()}`);
  const doc = await editBody(page, note.id);
  const kb = page.keyboard;

  await kb.type("@");
  await kb.type(title);
  await page.getByTestId("mention-choose-type").click();
  await expect(picker(page)).toHaveValue(`${title} type:`);
  await kb.type("proj");
  await expect(page.getByTestId("mention-new")).toHaveCount(1);
  await expect(page.getByTestId("mention-new")).toHaveText(`New Project: “${title}”`);
  await kb.press("ArrowDown");
  await kb.press("Enter");
  await expect(doc.getByTestId("placeholder-link")).toHaveAttribute("data-type", "project");
  await saveAndWait(page);

  const tabs = page.getByRole("tablist", { name: "Open tabs" }).getByRole("tab");
  await expect(tabs).toHaveCount(1);
  await doc.getByTestId("placeholder-link").click({ modifiers: ["ControlOrMeta"] });
  await expect(tabs).toHaveCount(2);
  await expect(tabs.nth(1)).toHaveAccessibleName(title);
  await expect(page).toHaveURL(new RegExp(`/notes/${note.id}$`));
  await expect(doc.getByTestId("mention")).toHaveText(title);
  await saveAndWait(page);
  const [id] = await notesTitled(admin, title);
  expect(await bodyText(admin, note.id)).toBe(`[[${id}]]`);
});

test("a type with a required field opens the new-note form, and creating the note links it", async ({ page, admin }) => {
  const t = token();
  const type = `errand_${t}`;
  const put = await admin.put(`/api/types/${type}`, {
    data: {
      wcl: `label = "Errand"\n\nfield owner {\n  kind     = "string"\n  label    = "Owner"\n  required = true\n}\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n    slot "owner"\n  }\n}\n`,
    },
  });
  expect(put.status(), await put.text()).toBe(200);
  const title = `Buy seals ${t}`;
  const note = await createNote(admin, "task", `Errands ${t}`, [para(`then [[+${type}|${title}]]`)]);
  await logIn(page, `/notes/${note.id}`);
  const chip = activePane(page).getByRole("textbox", { name: "Body" }).getByTestId("placeholder-link");
  await expect(chip).toHaveText(title);
  await chip.click();

  await expect(page).toHaveURL(/\/notes\/new\?/);
  await expect(activePane(page).getByLabel("Title", { exact: true })).toHaveText(title);
  expect(await notesTitled(admin, title)).toEqual([]);
  await activePane(page).getByLabel("Owner").fill("Wil");
  await activePane(page).getByRole("button", { name: "Create" }).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${ULID}$`));
  const [id] = await notesTitled(admin, title);
  expect(page.url()).toContain(`/notes/${id}`);
  await expect.poll(() => bodyText(admin, note.id)).toBe(`then [[${id}]]`);
});

test("a placeholder on a canvas card creates the note and is rewritten in the note", async ({ page, admin }) => {
  const t = token();
  const type = `boardy_${t}`;
  const put = await admin.put(`/api/types/${type}`, {
    data: {
      wcl: `label = "Boardy"\n\nfield board {\n  kind  = "canvas"\n  label = "Board"\n}\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n    slot "board"\n  }\n}\n`,
    },
  });
  expect(put.status(), await put.text()).toBe(200);
  const title = `Card task ${t}`;
  const res = await admin.post(`/api/notes?type=${type}`, {
    data: {
      note: {
        fields: { title: `Board ${t}` },
        blocks: [
          {
            kind: "canvas",
            labels: ["board"],
            fields: { height: 200 },
            blocks: [
              { kind: "viewport", fields: { x: 0, y: 0, zoom: 1 } },
              {
                kind: "node",
                labels: ["card"],
                fields: { kind: "card", x: 20, y: 20, width: 420, height: 120 },
                blocks: [para(`Do [[+task|${title}]]`)],
              },
            ],
          },
        ],
      },
    },
  });
  expect(res.status(), await res.text()).toBe(201);
  const board = (await res.json()) as Stored;
  await logIn(page, `/notes/${board.id}`);
  const chip = activePane(page).getByTestId("canvas-card").getByTestId("placeholder-link");
  await expect(chip).toHaveText(title);
  await chip.click();
  await expect(activePane(page).getByLabel("Title", { exact: true })).toHaveText(title);
  const [id] = await notesTitled(admin, title);
  await expect
    .poll(async () => (await stored(admin, board.id)).wcl)
    .toContain(`text = "Do [[${id}]]"`);
});

test("a placeholder in a version shown from history only shows", async ({ page, admin }) => {
  const title = `Old idea ${token()}`;
  const note = await createNote(admin, "task", `History ${token()}`, [para(`see [[+task|${title}]]`)]);
  const upd = await admin.put(`/api/notes/${note.id}`, {
    data: { note: { fields: { title: note.note.fields.title }, blocks: [] }, base_version: note.version },
  });
  expect(upd.status(), await upd.text()).toBe(200);
  await logIn(page, `/notes/${note.id}`);
  await activePane(page).getByRole("button", { name: "History", exact: true }).click();
  const versions = page.getByRole("complementary", { name: "History" }).getByRole("list", { name: "Versions" }).getByRole("listitem");
  await versions.nth(1).getByRole("button").first().click();
  await expect(page.getByRole("region", { name: "Version preview" })).toBeVisible();
  const chip = activePane(page).getByTestId("placeholder-link");
  await expect(chip).toHaveText(title);
  await expect(chip).not.toHaveClass(/clickable/);
  await chip.click();
  await expect(page).toHaveURL(new RegExp(`/notes/${note.id}$`));
  expect(await notesTitled(admin, title)).toEqual([]);
});

test("Ask <Label> commits a call in one go, shown as a chip that opens the agent", async ({ page, admin }) => {
  // The agent is disabled once the page has listed it, so the server records the call without a
  // run, and no run of it fails in this shared vault.
  const tok = token();
  const name = `caller_${tok}`;
  const label = `Caller${tok}`;
  const put = (wcl: string) => admin.put(`/api/agents/${name}`, { data: { wcl } });
  expect((await put(`label = "${label}"\n`)).ok()).toBe(true);
  const note = await createNote(admin, "task", `Call ${tok}`);
  const doc = await editBody(page, note.id);
  expect((await put(`label = "${label}"\nenabled = false\n`)).ok()).toBe(true);

  await page.keyboard.type(`@${name}`);
  await page.getByTestId("mention-agent").filter({ hasText: `Ask ${label}` }).click();
  const box = page.getByRole("textbox", { name: `Ask ${label}…` });
  await expect(box).toBeFocused();
  await expect(box).toHaveAttribute("placeholder", `What should ${label} do?`);
  await box.fill("tidy this");
  await box.press("Enter");
  await expect(page.getByTestId("mention-picker")).toHaveCount(0);
  await saveAndWait(page);
  const text = await bodyText(admin, note.id);
  expect(text).toMatch(new RegExp(`^\\[@${label} tidy this\\]\\(agent:${name}\\?call=(${ULID})\\)\\s*$`));
  const call = new RegExp(`call=(${ULID})`).exec(text)![1];
  await expect.poll(async () => (await admin.get(`/api/agents/calls/${call}`)).status()).toBe(200);

  const chip = doc.getByTestId("agent-call");
  await expect(chip).toHaveText(`@${label} tidy this`);
  await expect(doc.getByText("Not answered")).toBeVisible({ timeout: 10_000 });
  await chip.click();
  await expect(page).toHaveURL(new RegExp(`/agents/${name}/chat$`));
  expect((await admin.delete(`/api/agents/${name}`)).ok()).toBe(true);
});
