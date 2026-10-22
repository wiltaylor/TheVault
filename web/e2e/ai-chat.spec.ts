import { type ChildProcess, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { activePane, expect, logIn, test } from "./fixtures";

// The AI panel (ai D14, D16) against the fake Anthropic API, which the `ai_settings` secret's
// endpoints point at: ask beside a note, watch the answer arrive, apply the edit the model
// proposed, and see the note change.

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
