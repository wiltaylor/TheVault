import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { APIRequestContext } from "@playwright/test";
import { expect, logIn, test, vaultDir } from "./fixtures";

// Creating a note from the UI (S5): the New note action on a type's list opens the type's default
// template as an empty draft, and Create writes the note and opens it, where it then saves
// itself (D14, D40).

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

async function stored(admin: APIRequestContext, id: string) {
  const res = await admin.get(`/api/notes/${id}`);
  expect(res.status(), await res.text()).toBe(200);
  return (await res.json()) as {
    type: string;
    wcl: string;
    note: { fields: Record<string, unknown>; blocks: { kind: string; labels?: string[]; blocks?: unknown[] }[] };
  };
}

function file(type: string, id: string): string {
  return readFileSync(join(vaultDir(), "notes", type, `${id}.wcl`), "utf8");
}

const NOTE_URL = /\/notes\/([0-9A-HJKMNP-TV-Z]{26})$/;

test("creating a note of the starter note type from its list writes a valid file and opens it", async ({
  page,
  admin,
}) => {
  const title = `Created ${token()}`;
  await logIn(page, "/types/note");
  await page.getByRole("link", { name: "New note" }).click();
  await expect(page).toHaveURL(/\/notes\/new\?type=note$/);
  await expect(page.getByText("New Note", { exact: true })).toBeVisible();

  await page.getByLabel("Title").fill(title);
  const body = page.getByRole("textbox", { name: "Body" });
  await body.click();
  await page.keyboard.type("Written in the new note form.");
  await page.getByRole("button", { name: "Create" }).click();

  await expect(page).toHaveURL(NOTE_URL);
  const id = NOTE_URL.exec(page.url())![1]!;
  await expect(page.getByRole("heading", { name: title })).toBeVisible();
  await expect(page.getByRole("button", { name: "Create" })).toHaveCount(0);

  const note = await stored(admin, id);
  expect(note.type).toBe("note");
  expect(note.note.fields.title).toBe(title);
  const content = note.note.blocks.find((b) => b.kind === "content" && b.labels?.[0] === "body");
  expect(JSON.stringify(content?.blocks)).toContain("Written in the new note form.");
  const wcl = file("note", id);
  expect(wcl).toBe(note.wcl);
  expect(wcl).toMatch(new RegExp(`^title\\s+= "${title}"$`, "m"));

  // The created note saves itself from now on.
  await body.click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type(" Then edited.");
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
  expect(JSON.stringify((await stored(admin, id)).note.blocks)).toContain("Written in the new note form. Then edited.");

  // The new note is on its type's list.
  await page.goto("/types/note");
  await expect(page.getByRole("link", { name: title })).toBeVisible();
});

test("a new note's form shows field defaults, and the created file holds them", async ({ page, admin }) => {
  const type = `chore_${token()}`;
  const res = await admin.put(`/api/types/${type}`, {
    data: {
      wcl: `label  = "Chore"
plural = "Chores"

field priority {
  kind     = "select"
  label    = "Priority"
  required = true
  default  = "medium"
  options  = ["low", "medium", "high"]
}

field effort {
  kind    = "int"
  label   = "Effort"
  default = 3
}

template default {
  tab "Main" {
    slot "title"
    slot "priority"
    slot "effort"
  }
}
`,
    },
  });
  expect(res.status(), await res.text()).toBe(200);

  await logIn(page, `/types/${type}`);
  await page.getByRole("link", { name: "New chore" }).click();
  await expect(page.getByLabel("Priority")).toHaveValue("medium");
  await expect(page.getByLabel("Effort")).toHaveValue("3");
  await page.getByLabel("Effort").fill("5");
  await page.getByLabel("Title").fill("Sweep");
  await page.getByRole("button", { name: "Create" }).click();

  await expect(page).toHaveURL(NOTE_URL);
  const id = NOTE_URL.exec(page.url())![1]!;
  const note = await stored(admin, id);
  expect(note.note.fields).toMatchObject({ title: "Sweep", priority: "medium", effort: 5 });
  expect(file(type, id)).toBe(note.wcl);
});

test("a new note without its required title is not created, and Cancel goes back to the list", async ({
  page,
}) => {
  const created: string[] = [];
  page.on("request", (r) => {
    if (r.method() === "POST" && new URL(r.url()).pathname === "/api/notes") created.push(r.url());
  });
  await logIn(page, "/notes/new?type=note");
  await expect(page.getByText("New Note", { exact: true })).toBeVisible();
  // Create waits for every required field: here, the title.
  const create = page.getByRole("button", { name: "Create" });
  await expect(create).toBeDisabled();
  await expect(page.getByTestId("save-status")).toHaveText("Required: Title");
  await create.click({ force: true });
  expect(await page.getByLabel("Title").evaluate((el) => (el as HTMLInputElement).validity.valueMissing)).toBe(true);
  await expect(page).toHaveURL(/\/notes\/new\?type=note$/);
  expect(created).toEqual([]);
  await page.getByLabel("Title").fill("Now named");
  await expect(create).toBeEnabled();
  await page.getByLabel("Title").fill("");
  await expect(create).toBeDisabled();

  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page).toHaveURL(/\/types\/note$/);
});

test("a new note of a type that does not exist says so", async ({ page }) => {
  await logIn(page, "/notes/new?type=nosuchtype");
  await expect(page.getByText("There is no type nosuchtype.")).toBeVisible();
});

test("a long title wraps onto more lines in a narrow pane and stays editable in place", async ({ page, admin }) => {
  const title = `Tuesday 13 October 2026 weekly planning ${token()}`;
  const res = await admin.post("/api/notes?type=task", { data: { note: { fields: { title } } } });
  expect(res.status(), await res.text()).toBe(201);
  const { id } = (await res.json()) as { id: string };
  await page.setViewportSize({ width: 390, height: 844 });
  await logIn(page, `/notes/${id}`);
  const input = page.getByLabel("Title", { exact: true });
  await expect(input).toHaveValue(title);
  await expect(page.getByRole("heading", { level: 1, name: title })).toBeVisible();

  // Nothing is cut off: the box is as wide as the pane allows, and as tall as its lines.
  const size = () =>
    input.evaluate((el) => ({
      scrollWidth: el.scrollWidth,
      clientWidth: el.clientWidth,
      scrollHeight: el.scrollHeight,
      clientHeight: el.clientHeight,
      lineHeight: Number.parseFloat(getComputedStyle(el).lineHeight),
      right: el.getBoundingClientRect().right,
    }));
  const s = await size();
  expect(s.scrollWidth).toBeLessThanOrEqual(s.clientWidth + 1);
  expect(s.scrollHeight).toBeLessThanOrEqual(s.clientHeight + 1);
  expect(s.clientHeight).toBeGreaterThan(s.lineHeight * 1.5);
  expect(s.right).toBeLessThanOrEqual(390);

  // Still the title, edited in place: Enter saves rather than adding a line.
  await input.click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type(" again");
  await page.keyboard.press("Enter");
  await expect(input).toHaveValue(`${title} again`);
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
  expect((await stored(admin, id)).note.fields.title).toBe(`${title} again`);
});
