import { type ChildProcess, spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { activePane, expect, logIn, test } from "./fixtures";

// The AI panel (ai D14, D23) against the fake Anthropic API, which the `ai_settings` secret's
// endpoints point at: ask beside a note, watch the answer arrive, apply the edit the model
// proposed, and see the note change; apply a proposed template change (ai D18) and see it on a
// note of that type; apply a proposed delete and create (ai D20) and see the note in the trash and
// the new note open; and, with notes confirmation off (ai D22), see an edit land in the open note
// without pressing Apply; and, in an app (ai D23), run a turn through the turns form and apply
// on the home server, asking for a sync round; and the panel's thread picker lists the thread a
// message started and opens it in the agent screen as the same conversation (agents G18); and in
// the agent screen's chat, Enter sends and Shift+Enter starts a line, the model dropdown at the
// box's foot chooses the model a turn uses, and the session picker's search, Archive and Delete
// act on the chosen session.

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

test("with notes confirmation off, an edit asked for in the AI panel shows in the open note without Apply", async ({ page, admin }) => {
  for (const [name, value] of [
    ["ai_anthropic_api_key", fake.api_key],
    [
      "ai_settings",
      JSON.stringify({
        default_provider: "anthropic",
        models: { anthropic: "claude-opus-5" },
        endpoints: { anthropic: fake.base_url },
      }),
    ],
  ]) {
    const res = await admin.put(`/api/secrets/${name}`, { data: { value } });
    expect(res.ok(), await res.text()).toBe(true);
  }
  // The chat's confirmation switches are Assistant's (agents G6); the settings route still writes them there.
  const put = await admin.put("/api/integrations/ai/settings", { data: { confirm: { notes: false } } });
  expect(put.ok(), await put.text()).toBe(true);
  const providers = (await (await admin.get("/api/ai/providers")).json()) as { confirm: Record<string, boolean> };
  expect(providers.confirm).toMatchObject({ notes: false, archive: true, delete: true, types: true });
  const title = `Errands ${Date.now().toString(36)}`;
  const created = await admin.post("/api/notes?type=note", { data: { note: shopping(title, ["Post office"]) } });
  expect(created.status(), await created.text()).toBe(201);
  const note = (await created.json()) as { id: string; version: string };

  const edit = { id: note.id, base_version: note.version, note: JSON.stringify(shopping(title, ["Post office", "Pharmacy"])) };
  await tell(`push ${JSON.stringify({ tools: [{ id: "toolu_auto", name: "propose_edit", input: edit }] })}`);
  await tell(`push ${JSON.stringify({ text: "I added the pharmacy to your errands." })}`);

  await logIn(page, `/notes/${note.id}`);
  await expect(activePane(page)).toContainText("Post office");
  await expect(activePane(page)).not.toContainText("Pharmacy");
  await page.keyboard.press("Control+Shift+L");
  const panel = page.getByRole("complementary", { name: "AI" });
  await expect(panel.getByLabel("Model")).toHaveValue("claude-opus-5");
  await panel.getByLabel("Ask AI").fill("Add the pharmacy to this list.");
  await panel.getByLabel("Ask AI").press("Enter");

  const conversation = panel.getByRole("list", { name: "Conversation" });
  await expect(conversation).toContainText("I added the pharmacy");
  const card = panel.getByRole("region", { name: `Edited ${title}` });
  await expect(card.getByRole("link", { name: title })).toHaveAttribute("href", `/notes/${note.id}`);
  await expect(panel.getByRole("button", { name: "Apply" })).toHaveCount(0);
  await expect(activePane(page)).toContainText("Pharmacy");
  const after = (await (await admin.get(`/api/notes/${note.id}`)).json()) as { version: string; note: unknown };
  expect(after.version).not.toBe(note.version);
  expect(JSON.stringify(after.note)).toContain("Pharmacy");
  // Assistant is shared by every test on this server: ask before edits again.
  const back = await admin.put("/api/integrations/ai/settings", { data: { confirm: { notes: true } } });
  expect(back.ok(), await back.text()).toBe(true);
});

// In the desktop and Android apps (ai D23) the panel runs a turn in the turns form and polls the
// embedded server for its events; Apply goes to the home server's POST /api/ai/apply and the app
// asks for a sync round. The routes here stand in for the embedded server: `GET /api/app` says the
// page runs in an app, `GET /api/app/ai/turns/{turn}` reads the turn's stream from the server,
// and `/api/app/sync` records the round asked for.
test("in an app, the AI panel runs a turn through the turns form and Apply asks for a sync", async ({ page, admin, baseURL }) => {
  for (const [name, value] of [
    ["ai_anthropic_api_key", fake.api_key],
    ["ai_settings", JSON.stringify({ default_provider: "anthropic", models: { anthropic: "claude-opus-5" }, endpoints: { anthropic: fake.base_url } })],
  ]) {
    const res = await admin.put(`/api/secrets/${name}`, { data: { value } });
    expect(res.ok(), await res.text()).toBe(true);
  }
  const title = `Chores ${Date.now().toString(36)}`;
  const created = await admin.post("/api/notes?type=note", { data: { note: shopping(title, ["Dishes"]) } });
  expect(created.status(), await created.text()).toBe(201);
  const note = (await created.json()) as { id: string; version: string };
  const edit = { id: note.id, base_version: note.version, note: JSON.stringify(shopping(title, ["Dishes", "Laundry"])) };
  await tell(`push ${JSON.stringify({ tools: [{ id: "toolu_app", name: "propose_edit", input: edit }] })}`);
  await tell(`push ${JSON.stringify({ text: "I proposed adding the laundry." })}`);

  await page.route("**/api/app", (route) =>
    route.fulfill({ json: { app: true, paired: true, device_id: "d1", device_name: "This laptop", server_url: baseURL } }),
  );
  const polled: string[] = [];
  await page.route("**/api/app/ai/turns/**", async (route) => {
    const url = new URL(route.request().url());
    const turn = url.pathname.split("/").pop()!;
    polled.push(turn);
    const after = Number(url.searchParams.get("after") ?? "0");
    const res = await route.fetch({ url: `${url.origin}/api/ai/chat/turns/${turn}/events`, method: "GET" });
    const events = (await res.text())
      .split("\n\n")
      .map((frame) => {
        const lines = frame.split("\n");
        const event = lines.find((l) => l.startsWith("event:"))?.slice(6).trim();
        const data = lines.find((l) => l.startsWith("data:"))?.slice(5).trim();
        return event && data ? { event, data: JSON.parse(data) as unknown } : null;
      })
      .filter((e) => e !== null);
    await route.fulfill({ json: { events: events.slice(after), next: events.length, done: true } });
  });
  let syncs = 0;
  await page.route("**/api/app/sync", async (route) => {
    if (route.request().method() === "POST") syncs++;
    await route.fulfill({ json: { state: "synced", phase: "synced", queued: 0, last_attempt_ms: Date.now() + 60_000 } });
  });
  const chats: string[] = [];
  page.on("request", (r) => {
    if (r.method() === "POST" && new URL(r.url()).pathname.startsWith("/api/ai/")) chats.push(new URL(r.url()).pathname);
  });

  await logIn(page, `/notes/${note.id}`);
  await expect(activePane(page)).toContainText("Dishes");
  await page.keyboard.press("Control+Shift+L");
  await expect(page.getByRole("tab", { name: "AI" })).toHaveAttribute("aria-selected", "true");
  const panel = page.getByRole("complementary", { name: "AI" });
  await expect(panel.getByLabel("Model")).toHaveValue("claude-opus-5");
  await panel.getByLabel("Ask AI").fill("Add the laundry.");
  await panel.getByLabel("Ask AI").press("Enter");

  const conversation = panel.getByRole("list", { name: "Conversation" });
  await expect(conversation).toContainText("I proposed adding the laundry.");
  expect(polled.length).toBeGreaterThan(0);
  expect(chats).toContain("/api/ai/chat/turns");
  expect(chats).not.toContain("/api/ai/chat");

  const card = panel.getByRole("region", { name: `Proposed edit to ${title}` });
  await card.getByRole("button", { name: "Apply" }).click();
  await expect(card.getByText("Applied.")).toBeVisible();
  expect(chats).toContain("/api/ai/apply");
  await expect.poll(() => syncs).toBeGreaterThan(0);
  const after = (await (await admin.get(`/api/notes/${note.id}`)).json()) as { version: string; note: unknown };
  expect(after.version).not.toBe(note.version);
  expect(JSON.stringify(after.note)).toContain("Laundry");
});

test("the AI panel's thread opens in the agent screen as the same conversation, and the picker lists it", async ({ page, admin }) => {
  for (const [name, value] of [
    ["ai_anthropic_api_key", fake.api_key],
    ["ai_settings", JSON.stringify({ default_provider: "anthropic", models: { anthropic: "claude-opus-5" }, endpoints: { anthropic: fake.base_url } })],
  ]) {
    const res = await admin.put(`/api/secrets/${name}`, { data: { value } });
    expect(res.ok(), await res.text()).toBe(true);
  }
  const stamp = Date.now().toString(36);
  await tell(`push ${JSON.stringify({ text: `Hello from thread ${stamp}.` })}`);

  await logIn(page, "/daily");
  await page.keyboard.press("Control+Shift+L");
  const panel = page.getByRole("complementary", { name: "AI" });
  await expect(panel.getByLabel("Model")).toHaveValue("claude-opus-5");
  await expect(panel.getByRole("button", { name: "Open in agent screen" })).toBeDisabled();
  await panel.getByLabel("Ask AI").fill(`Say hello ${stamp}.`);
  await panel.getByLabel("Ask AI").press("Enter");
  await expect(panel.getByRole("list", { name: "Conversation" })).toContainText(`Hello from thread ${stamp}.`);
  await expect(panel.getByRole("button", { name: "Send" })).toBeVisible();

  // The picker lists the new thread, checked, under New thread.
  await panel.getByRole("button", { name: /^Thread: / }).click();
  const menu = panel.getByRole("menu", { name: "Threads" });
  await expect(menu.getByRole("menuitem", { name: "New thread" })).toBeVisible();
  await expect(menu.locator('[role="menuitemradio"][aria-checked="true"]')).toHaveCount(1);
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);

  await panel.getByRole("button", { name: "Open in agent screen" }).click();
  await expect(page).toHaveURL(/\/agents\/assistant\?thread=[0-9A-Z]+$/);
  const chat = activePane(page).getByRole("region", { name: "Chat" });
  const shown = chat.getByRole("list", { name: "Conversation" });
  await expect(shown).toContainText(`Say hello ${stamp}.`);
  await expect(shown).toContainText(`Hello from thread ${stamp}.`);
  await expect(chat.getByRole("button", { name: /^Chat session: / })).not.toHaveAccessibleName("Chat session: New chat");

  // Starting a new thread in the panel leaves the agent screen's alone.
  await panel.getByRole("button", { name: /^Thread: / }).click();
  await menu.getByRole("menuitem", { name: "New thread" }).click();
  await expect(panel.getByRole("button", { name: "Thread: New thread" })).toBeVisible();
  await expect(panel.getByRole("list", { name: "Conversation" })).not.toContainText(`Hello from thread ${stamp}.`);
  await expect(shown).toContainText(`Hello from thread ${stamp}.`);
});

/** A fresh agent of no model of its own, so the chat's model dropdown decides; its name. */
async function freshAgent(admin: import("@playwright/test").APIRequestContext, stamp: string): Promise<string> {
  for (const [name, value] of [
    ["ai_anthropic_api_key", fake.api_key],
    ["ai_settings", JSON.stringify({ default_provider: "anthropic", models: { anthropic: "claude-opus-5" }, endpoints: { anthropic: fake.base_url } })],
  ]) {
    const res = await admin.put(`/api/secrets/${name}`, { data: { value } });
    expect(res.ok(), await res.text()).toBe(true);
  }
  const name = `chatter_${stamp}`;
  const res = await admin.put(`/api/agents/${name}`, { data: { wcl: `label = "Chatter ${stamp}"\n` } });
  expect(res.ok(), await res.text()).toBe(true);
  return name;
}

test("the agent screen's chat sends on Enter, keeps Shift+Enter as a new line, and runs the model chosen at its foot", async ({ page, admin }) => {
  const stamp = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  const name = await freshAgent(admin, stamp);
  await tell(`push ${JSON.stringify({ text: `Two lines read ${stamp}.` })}`);

  await logIn(page, `/agents/${name}`);
  const chat = activePane(page).getByRole("region", { name: "Chat" });
  const model = chat.getByLabel("Model");
  await expect(model).toHaveValue("claude-opus-5");
  // The dropdown sits at the bottom right, beside Send.
  const send = (await chat.getByRole("button", { name: "Send" }).boundingBox())!;
  const picked = (await model.boundingBox())!;
  expect(picked.y + picked.height).toBeGreaterThan(send.y);
  expect(picked.x).toBeLessThan(send.x);
  await model.selectOption("claude-sonnet-5");

  const box = chat.getByLabel("Ask AI");
  await box.click();
  await page.keyboard.type("Line one");
  await page.keyboard.press("Shift+Enter");
  await page.keyboard.type("Line two");
  await expect(box).toHaveValue("Line one\nLine two");
  const conversation = chat.getByRole("list", { name: "Conversation" });
  await expect(conversation.getByRole("listitem")).toHaveCount(0);
  await page.keyboard.press("Enter");
  await expect(conversation).toContainText(`Two lines read ${stamp}.`);
  await expect(box).toHaveValue("");
  await expect(conversation.locator(".ai-question")).toHaveText("Line one\nLine two");

  // The address names the new session, and the turn ran on the model chosen.
  await expect(page).toHaveURL(new RegExp(`/agents/${name}\\?thread=[0-9A-Z]+$`));
  const thread = new URL(page.url()).searchParams.get("thread")!;
  await expect(chat.getByRole("button", { name: /^Chat session: / })).not.toHaveAccessibleName("Chat session: New chat");
  await expect
    .poll(async () => {
      const res = await admin.get(`/api/activity?category=ai&kind=ai.turn.end&limit=50`);
      const items = ((await res.json()) as { items: Record<string, unknown>[] }).items;
      const mine = items.find((i) => JSON.stringify(i).includes(thread));
      return mine ? JSON.stringify(mine) : "";
    })
    .toContain('"claude-sonnet-5"');

  expect((await admin.delete(`/api/agents/${name}`)).ok()).toBe(true);
});

test("the agent screen's session picker searches the chats, and Archive and Delete act on the one chosen", async ({ page, admin }) => {
  const stamp = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  const name = await freshAgent(admin, stamp);
  const start = async (title: string) => {
    const res = await admin.post(`/api/agents/${name}/threads`, { data: { title } });
    expect(res.ok(), await res.text()).toBe(true);
    return ((await res.json()) as { id: string }).id;
  };
  await start(`Alpha ${stamp}`);
  const beta = await start(`Beta ${stamp}`);

  await logIn(page, `/agents/${name}`);
  const chat = activePane(page).getByRole("region", { name: "Chat" });
  const picker = chat.getByRole("button", { name: /^Chat session: / });
  await expect(picker).toHaveAccessibleName("Chat session: New chat");
  await expect(chat.getByRole("button", { name: "Archive chat" })).toBeDisabled();

  // Search, and open the match with Enter.
  await picker.click();
  const search = chat.getByRole("combobox", { name: "Search chats" });
  await expect(search).toBeFocused();
  const options = chat.getByRole("listbox", { name: "Chats" }).getByRole("option");
  await expect(options).toHaveCount(3);
  await search.fill("beta");
  await expect(options).toHaveCount(2);
  await expect(options.nth(1)).toContainText(`Beta ${stamp}`);
  await search.press("Enter");
  await expect(chat.getByRole("listbox")).toHaveCount(0);
  await expect(picker).toHaveAccessibleName(`Chat session: Beta ${stamp}`);
  await expect(picker).toBeFocused();
  await expect(page).toHaveURL(new RegExp(`/agents/${name}\\?thread=${beta}$`));

  // Archive: the chat stays open, marked, and leaves the list until Show archived.
  await chat.getByRole("button", { name: "Archive chat" }).click();
  await expect(chat.getByRole("button", { name: "Unarchive chat" })).toBeVisible();
  await expect.poll(async () => ((await (await admin.get(`/api/agents/${name}/threads/${beta}`)).json()) as { archived?: boolean }).archived).toBe(true);
  await picker.click();
  await expect(options).toHaveCount(2);
  await expect(options.nth(1)).toContainText(`Alpha ${stamp}`);
  await chat.getByLabel("Show archived").check();
  await expect(options).toHaveCount(3);
  await page.keyboard.press("Escape");
  await expect(chat.getByRole("listbox")).toHaveCount(0);

  // Delete, after confirming: a new chat takes its place.
  page.once("dialog", (d) => void d.accept());
  await chat.getByRole("button", { name: "Delete chat" }).click();
  await expect(picker).toHaveAccessibleName("Chat session: New chat");
  await expect(page).toHaveURL(new RegExp(`/agents/${name}$`));
  await expect.poll(async () => (await admin.get(`/api/agents/${name}/threads/${beta}`)).status()).toBe(404);

  expect((await admin.delete(`/api/agents/${name}`)).ok()).toBe(true);
});

test("the chat shows how full the context is, and /compact summarizes the thread for the model", async ({ page }) => {
  const title = `Context ${Date.now().toString(36)}`;
  await logIn(page, "/agents/assistant");
  const box = page.getByRole("textbox", { name: "Ask AI" });
  await tell(`push ${JSON.stringify({ text: `${title}: a long answer.`, usage: [170_000, 2_000] })}`);
  await box.fill(`${title} question`);
  await box.press("Enter");
  await expect(page.getByText(`${title}: a long answer.`)).toBeVisible();
  const meter = page.getByTestId("context-meter");
  await expect(meter).toContainText("172k / 200k");
  // 86% is past the default 80% compaction point.
  await expect(meter).toHaveClass(/ai-context-high/);

  const before = (await tell("requests")).count as number;
  await tell(`push ${JSON.stringify({ text: `Summary of ${title}.`, usage: [172_000, 300] })}`);
  await box.fill("/compact");
  await box.press("Enter");
  await expect(page.getByRole("status").filter({ hasText: "Conversation compacted" })).toBeVisible();
  const divider = page.getByTestId("compaction");
  await expect(divider).toBeVisible();
  await divider.locator("summary").click();
  await expect(divider).toContainText(`Summary of ${title}.`);
  await expect(meter).toContainText("300 / 200k");
  await expect(meter).toHaveClass(/ai-context-ok/);
  expect((await tell("requests")).count).toBe(before + 1);

  // The thread keeps its messages, and the divider stays where it was after a reload.
  await page.reload();
  await expect(page.getByText(`${title}: a long answer.`)).toBeVisible();
  await expect(page.getByTestId("compaction")).toBeVisible();
  await expect(page.getByTestId("context-meter")).toContainText("300 / 200k");
});
