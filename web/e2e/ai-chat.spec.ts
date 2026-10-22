import { type ChildProcess, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { activePane, expect, logIn, test } from "./fixtures";

// The AI panel (ai D14, D16) against the fake Anthropic API, which the `ai_settings` secret's
// endpoints point at: ask beside a note, watch the answer arrive, apply the edit the model
// proposed, and see the note change; apply a proposed template change (ai D18) and see it on a
// note of that type; apply a proposed delete and create (ai D20) and see the note in the trash and
// the new note open.

const repo = fileURLToPath(new URL("../..", import.meta.url));

function binary(env: string, name: string): string {
  const path = process.env[env] ?? join(repo, "target", "debug", name);
  if (!existsSync(path)) throw new Error(`${path} is missing. Run \`cargo build -p server -p testkit --bins\`.`);
  return path;
}

interface Fake {
  base_url: string;
  api_key: string;
}

let anthropic: ChildProcess;
let fake: Fake;
const answers: ((line: string) => void)[] = [];

/** Send a command to the fake (see `testkit-fake-anthropic`) and wait until it has taken effect. */
async function tell(line: string): Promise<Record<string, unknown>> {
  const answer = new Promise<string>((resolve) => answers.push(resolve));
  anthropic.stdin!.write(`${line}\n`);
  const got = JSON.parse(await answer) as Record<string, unknown>;
  expect(got.error).toBeUndefined();
  return got;
}

test.beforeAll(async () => {
  anthropic = spawn(binary("TESTKIT_FAKE_ANTHROPIC_BIN", "testkit-fake-anthropic"), [], { stdio: ["pipe", "pipe", "inherit"] });
  fake = await new Promise<Fake>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("testkit-fake-anthropic did not report its address")), 30_000);
    const lines = createInterface({ input: anthropic.stdout! });
    lines.once("line", (line) => {
      clearTimeout(timer);
      resolve(JSON.parse(line) as Fake);
      lines.on("line", (answer) => answers.shift()?.(answer));
    });
    anthropic.once("exit", (code) => reject(new Error(`testkit-fake-anthropic exited with ${code}`)));
  });
});

test.afterAll(() => {
  anthropic?.kill();
});

function shopping(title: string, items: string[]) {
  return {
    fields: { title },
    blocks: [{ kind: "content", labels: ["body"], blocks: items.map((text) => ({ kind: "paragraph", fields: { text } })) }],
  };
}

test("the AI panel answers beside a note and an applied proposal changes the note", async ({ page, admin }) => {
  for (const [name, value] of [
    ["ai_anthropic_api_key", fake.api_key],
    ["ai_settings", JSON.stringify({ default_provider: "anthropic", models: { anthropic: "claude-opus-5" }, endpoints: { anthropic: fake.base_url } })],
  ]) {
    const res = await admin.put(`/api/secrets/${name}`, { data: { value } });
    expect(res.ok(), await res.text()).toBe(true);
  }
  const title = `Shopping ${Date.now().toString(36)}`;
  const created = await admin.post("/api/notes?type=note", { data: { note: shopping(title, ["Milk"]) } });
  expect(created.status(), await created.text()).toBe(201);
  const note = (await created.json()) as { id: string; version: string };

  // The model reads the note, proposes adding eggs, then answers.
  await tell(`push ${JSON.stringify({ tools: [{ id: "toolu_read", name: "get_note", input: { id: note.id } }] })}`);
  const edit = { id: note.id, base_version: note.version, note: JSON.stringify(shopping(title, ["Milk", "Eggs"])) };
  await tell(`push ${JSON.stringify({ tools: [{ id: "toolu_edit", name: "propose_edit", input: edit }] })}`);
  await tell(`push ${JSON.stringify({ text: `I proposed adding eggs to [${title}](/notes/${note.id}). Apply it to save the change.` })}`);

  await logIn(page, `/notes/${note.id}`);
  await expect(activePane(page)).toContainText("Milk");
  await page.keyboard.press("Control+Shift+L");
  const panel = page.getByRole("complementary", { name: "AI" });
  await expect(panel).toBeVisible();
  await expect(panel.getByLabel("Model")).toHaveValue("claude-opus-5");
  await panel.getByLabel("Ask AI").fill("Add eggs to this list.");
  await panel.getByLabel("Ask AI").press("Enter");

  const conversation = panel.getByRole("list", { name: "Conversation" });
  await expect(conversation).toContainText("I proposed adding eggs");
  await expect(conversation.getByRole("link", { name: title }).first()).toHaveAttribute("href", `/notes/${note.id}`);
  await expect(conversation).toContainText("Read a note");
  const card = panel.getByRole("region", { name: `Proposed edit to ${title}` });
  await expect(card).toContainText("+ Eggs");
  expect((await tell("requests")).count).toBe(3);

  // Proposing wrote nothing.
  const before = await admin.get(`/api/notes/${note.id}`);
  expect(((await before.json()) as { version: string }).version).toBe(note.version);

  await card.getByRole("button", { name: "Apply" }).click();
  await expect(card.getByText("Applied.")).toBeVisible();
  await expect(activePane(page)).toContainText("Eggs");
  const after = (await (await admin.get(`/api/notes/${note.id}`)).json()) as { version: string; note: unknown };
  expect(after.version).not.toBe(note.version);
  expect(JSON.stringify(after.note)).toContain("Eggs");
});

test("a create and a delete proposed in the AI panel apply: the new note opens and the old one is in the trash", async ({ page, admin }) => {
  for (const [name, value] of [
    ["ai_anthropic_api_key", fake.api_key],
    ["ai_settings", JSON.stringify({ default_provider: "anthropic", models: { anthropic: "claude-opus-5" }, endpoints: { anthropic: fake.base_url } })],
  ]) {
    const res = await admin.put(`/api/secrets/${name}`, { data: { value } });
    expect(res.ok(), await res.text()).toBe(true);
  }
  const stamp = Date.now().toString(36);
  const oldTitle = `Old list ${stamp}`;
  const newTitle = `New list ${stamp}`;
  const created = await admin.post("/api/notes?type=note", { data: { note: shopping(oldTitle, ["Bread"]) } });
  expect(created.status(), await created.text()).toBe(201);
  const old = (await created.json()) as { id: string; version: string };

  // The model proposes a new list and deleting the old one, then answers.
  const create = { type: "note", note: JSON.stringify(shopping(newTitle, ["Bread", "Butter"])) };
  await tell(`push ${JSON.stringify({ tools: [{ id: "toolu_create", name: "propose_create", input: create }] })}`);
  await tell(`push ${JSON.stringify({ tools: [{ id: "toolu_delete", name: "propose_delete", input: { id: old.id } }] })}`);
  await tell(`push ${JSON.stringify({ text: "I proposed a new list and deleting the old one." })}`);

  await logIn(page, `/notes/${old.id}`);
  await expect(activePane(page)).toContainText("Bread");
  await page.keyboard.press("Control+Shift+L");
  const panel = page.getByRole("complementary", { name: "AI" });
  await expect(panel.getByLabel("Model")).toHaveValue("claude-opus-5");
  await panel.getByLabel("Ask AI").fill("Replace this list with a new one.");
  await panel.getByLabel("Ask AI").press("Enter");

  const conversation = panel.getByRole("list", { name: "Conversation" });
  await expect(conversation).toContainText("I proposed a new list");
  const createCard = panel.getByRole("region", { name: `Proposed new note ${newTitle}` });
  const deleteCard = panel.getByRole("region", { name: `Proposed delete of ${oldTitle}` });
  await expect(createCard).toContainText("Butter");
  await expect(deleteCard).toContainText("moves the note to the trash");
  await expect(panel.getByRole("button", { name: "Apply all (2)" })).toBeVisible();

  // Proposing wrote nothing.
  expect(((await (await admin.get(`/api/notes/${old.id}`)).json()) as { version: string }).version).toBe(old.version);

  await deleteCard.getByRole("button", { name: "Apply", exact: true }).click();
  await expect(deleteCard.getByText("Moved to the trash.")).toBeVisible();
  expect((await admin.get(`/api/notes/${old.id}`)).status()).toBe(404);
  const trash = (await (await admin.get("/api/trash")).json()) as { notes: { id: string }[] };
  expect(trash.notes.map((n) => n.id)).toContain(old.id);

  await createCard.getByRole("button", { name: "Apply", exact: true }).click();
  await expect(createCard.getByText("Created.")).toBeVisible();
  await createCard.getByRole("link", { name: "Open the new note" }).click();
  await expect(page).toHaveURL(/\/notes\/[0-9A-Z]{26}$/);
  await expect(activePane(page)).toContainText("Butter");
  const id = new URL(page.url()).pathname.split("/").pop()!;
  const made = (await (await admin.get(`/api/notes/${id}`)).json()) as { note: { fields: { title: string } } };
  expect(made.note.fields.title).toBe(newTitle);
});

test("a type change proposed in the AI panel and applied shows on a note of that type", async ({ page, admin }) => {
  for (const [name, value] of [
    ["ai_anthropic_api_key", fake.api_key],
    ["ai_settings", JSON.stringify({ default_provider: "anthropic", models: { anthropic: "claude-opus-5" }, endpoints: { anthropic: fake.base_url } })],
  ]) {
    const res = await admin.put(`/api/secrets/${name}`, { data: { value } });
    expect(res.ok(), await res.text()).toBe(true);
  }
  const type = `memo_${Date.now().toString(36)}`;
  const wcl = `label = "Memo"\n\nfield body {\n  kind = "content"\n}\n\ntemplate default {\n  tab "Main" {\n    slot "title"\n    slot "body"\n  }\n}\n`;
  const put = await admin.put(`/api/types/${type}`, { data: { wcl } });
  expect(put.ok(), await put.text()).toBe(true);
  const stored = ((await put.json()) as { wcl: string }).wcl;
  const created = await admin.post(`/api/notes?type=${type}`, { data: { note: shopping("Weekly memo", ["Plain text"]) } });
  expect(created.status(), await created.text()).toBe(201);
  const note = (await created.json()) as { id: string };

  // The model reads the type, proposes a fixed line at the top of its template, then answers.
  const changed = stored.replace('slot "title"', 'paragraph {\n      text = "Reviewed weekly"\n    }\n    slot "title"');
  await tell(`push ${JSON.stringify({ tools: [{ id: "toolu_type", name: "get_type", input: { name: type } }] })}`);
  await tell(`push ${JSON.stringify({ tools: [{ id: "toolu_change", name: "propose_type_change", input: { name: type, wcl: changed } }] })}`);
  await tell(`push ${JSON.stringify({ text: "I proposed a line at the top of the memo template. Apply it to save the change." })}`);

  await logIn(page, `/notes/${note.id}`);
  await expect(activePane(page)).toContainText("Plain text");
  await expect(activePane(page)).not.toContainText("Reviewed weekly");
  await page.keyboard.press("Control+Shift+L");
  const panel = page.getByRole("complementary", { name: "AI" });
  await expect(panel.getByLabel("Model")).toHaveValue("claude-opus-5");
  await panel.getByLabel("Ask AI").fill("Add a Reviewed weekly line to this type's template.");
  await panel.getByLabel("Ask AI").press("Enter");

  const conversation = panel.getByRole("list", { name: "Conversation" });
  await expect(conversation).toContainText("I proposed a line");
  await expect(conversation).toContainText(`Read type ${type}`);
  const card = panel.getByRole("region", { name: `Proposed change to type ${type}` });
  await expect(card).toContainText("Reviewed weekly");
  await expect(card.locator("ins")).toHaveCount(3);

  // Proposing wrote nothing.
  expect(((await (await admin.get(`/api/types/${type}`)).json()) as { wcl: string }).wcl).toBe(stored);

  await card.getByRole("button", { name: "Apply" }).click();
  await expect(card.getByText("Applied.")).toBeVisible();
  expect(((await (await admin.get(`/api/types/${type}`)).json()) as { wcl: string }).wcl).toContain("Reviewed weekly");
  await expect(activePane(page)).toContainText("Reviewed weekly");
  await expect(activePane(page)).toContainText("Plain text");
});
