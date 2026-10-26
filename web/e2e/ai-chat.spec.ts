import { type Fake, type Running, aiAccounts, alone, dropAccount, startFake, useAccount, withDefault } from "./ai";
import { activePane, expect, logIn, test } from "./fixtures";

// The AI panel (ai D14, D23) against the fake Anthropic API, which an account of the worker's own
// points at, chosen at the chat's foot (accounts A12): ask beside a note, watch the answer arrive,
// apply the edit the model proposed, and see the note change; apply a proposed template change
// (ai D18) and see it on a note of that type; apply a proposed delete and create (ai D20) and see
// the note in the trash and the new note open; and, with notes confirmation off (ai D22), see an
// edit land in the open note without pressing Apply; and, in an app (ai D23), run a turn through
// the turns form and apply on the home server, asking for a sync round; and the panel's thread
// picker lists the thread a message started and opens it in the agent screen as the same
// conversation (agents G18); and in the agent screen's chat, Enter sends and Shift+Enter starts a
// line, the model dropdown at the box's foot chooses the model a turn uses, starting from an
// agent's own model when it names one, and the session picker's search, Archive and Delete act on
// the chosen session.

let anthropic: Running;
let fake: Fake;
/** This worker's account on its fake, by name; the default is global-setup's, which no test scripts. */
let fakeAccount: string;

/** Send a command to the fake Anthropic API and wait until it has taken effect. */
function tell(line: string): Promise<Record<string, unknown>> {
  return anthropic.tell(line);
}

test.beforeAll(async () => {
  anthropic = await startFake("TESTKIT_FAKE_ANTHROPIC_BIN", "testkit-fake-anthropic");
  fake = anthropic.fake;
  fakeAccount = `Fake Anthropic ${new URL(fake.base_url).port}`;
});

test.afterAll(() => {
  anthropic?.stop();
});

function shopping(title: string, items: string[]) {
  return {
    fields: { title },
    blocks: [{ kind: "content", labels: ["body"], blocks: items.map((text) => ({ kind: "paragraph", fields: { text } })) }],
  };
}

test("the AI panel answers beside a note and an applied proposal changes the note", async ({ page, admin }) => {
  await useFake(admin);
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
  await pickFake(panel);
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
  await useFake(admin);
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
  await pickFake(panel);
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
  await useFake(admin);
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
  await pickFake(panel);
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
  await useFake(admin);
  // The confirmation switches are the agent's (agents G6). Assistant's are every test's on this
  // server, so an agent of the test's own turns notes confirmation off, on this worker's fake.
  const stamp = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  const agent = `errands_${stamp}`;
  const wcl = `label = "Errands ${stamp}"\naccount = "${fakeAccount}"\npermissions {\n  confirm {\n    notes = false\n  }\n}\n`;
  const put = await admin.put(`/api/agents/${agent}`, { data: { wcl } });
  expect(put.ok(), await put.text()).toBe(true);
  const title = `Errands ${stamp}`;
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
  await panel.getByLabel("Agent").selectOption(agent);
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
  expect((await admin.delete(`/api/agents/${agent}`)).ok()).toBe(true);
});

// In the desktop and Android apps (ai D23) the panel runs a turn in the turns form and polls the
// embedded server for its events; Apply goes to the home server's POST /api/ai/apply and the app
// asks for a sync round. The routes here stand in for the embedded server: `GET /api/app` says the
// page runs in an app, `GET /api/app/ai/turns/{turn}` reads the turn's stream from the server,
// and `/api/app/sync` records the round asked for.
test("in an app, the AI panel runs a turn through the turns form and Apply asks for a sync", async ({ page, admin, baseURL }) => {
  await useFake(admin);
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
  await expect(page.getByRole("tab", { name: "AI", exact: true })).toHaveAttribute("aria-selected", "true");
  const panel = page.getByRole("complementary", { name: "AI" });
  await pickFake(panel);
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
  await useFake(admin);
  const stamp = Date.now().toString(36);
  await tell(`push ${JSON.stringify({ text: `Hello from thread ${stamp}.` })}`);

  await logIn(page, "/daily");
  await page.keyboard.press("Control+Shift+L");
  const panel = page.getByRole("complementary", { name: "AI" });
  await pickFake(panel);
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
  await useFake(admin);
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
  await pickFake(chat);
  const model = chat.getByLabel("Model");
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

test("an agent's chat starts on the agent's own model, and a model picked at its foot runs the next turn", async ({ page, admin }) => {
  await useFake(admin);
  const stamp = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
  const name = `owner_${stamp}`;
  const put = await admin.put(`/api/agents/${name}`, { data: { wcl: `label = "Owner ${stamp}"\naccount = "${fakeAccount}"\nmodel = "claude-sonnet-5"\n` } });
  expect(put.ok(), await put.text()).toBe(true);
  await tell(`push ${JSON.stringify({ text: `On its own ${stamp}.` })}`);
  await tell(`push ${JSON.stringify({ text: `On the pick ${stamp}.` })}`);

  await logIn(page, `/agents/${name}`);
  const chat = activePane(page).getByRole("region", { name: "Chat" });
  const account = chat.getByLabel("Account");
  const model = chat.getByLabel("Model");
  await expect(account.locator("option:checked")).toHaveText(`Agent's account (${fakeAccount})`);
  await expect(model.locator("option:checked")).toHaveText("Agent's model (claude-sonnet-5)");
  await expect(model.locator("option")).toContainText(["Agent's model (claude-sonnet-5)", "Claude Opus 5", "Claude Haiku 4.5"]);

  const box = chat.getByLabel("Ask AI");
  const conversation = chat.getByRole("list", { name: "Conversation" });
  await box.fill("First");
  await box.press("Enter");
  await expect(conversation).toContainText(`On its own ${stamp}.`);
  await model.selectOption("claude-haiku-4-5");
  await box.fill("Second");
  await box.press("Enter");
  await expect(conversation).toContainText(`On the pick ${stamp}.`);

  // The first turn ran on the agent's model and the second on the one picked.
  await expect(page).toHaveURL(new RegExp(`/agents/${name}\\?thread=[0-9A-Z]+$`));
  const thread = new URL(page.url()).searchParams.get("thread")!;
  await expect
    .poll(async () => {
      const res = await admin.get(`/api/activity?category=ai&kind=ai.turn.end&limit=50`);
      const items = ((await res.json()) as { items: Record<string, unknown>[] }).items;
      return items
        .filter((i) => JSON.stringify(i).includes(thread))
        .map((i) => JSON.stringify(i).match(/"model":"([^"]+)"/)?.[1])
        .sort();
    })
    .toEqual(["claude-haiku-4-5", "claude-sonnet-5"]);

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

test("the chat shows how full the context is, and /compact summarizes the thread for the model", async ({ page, admin }) => {
  await useFake(admin);
  const title = `Context ${Date.now().toString(36)}`;
  await logIn(page, "/agents/assistant");
  await pickFake(page.locator(".agent-chat"));
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

/** Point an account at this worker's fake Anthropic (accounts A12); its id. */
async function useFake(admin: import("@playwright/test").APIRequestContext): Promise<string> {
  return useAccount(admin, { name: fakeAccount, kind: "anthropic", base_url: fake.base_url, key: fake.api_key, model: "claude-opus-5" });
}

/** Choose this worker's fake at the chat's foot, which starts on its model. */
async function pickFake(chat: import("@playwright/test").Locator) {
  await chat.getByLabel("Account").selectOption({ label: fakeAccount });
  await expect(chat.getByLabel("Model")).toHaveValue("claude-opus-5");
}

/** Where the conversation's messages and its box sit: the box stays in view under the messages, which scroll on their own. */
async function layoutOf(root: import("@playwright/test").Locator) {
  return root.evaluate((el) => {
    const scroll = el.querySelector<HTMLElement>("[data-testid=ai-scroll]")!;
    const form = el.querySelector<HTMLElement>(".ai-form")!;
    const turns = scroll.querySelectorAll(".ai-turn");
    const last = turns[turns.length - 1]!.getBoundingClientRect();
    const box = form.getBoundingClientRect();
    const area = scroll.getBoundingClientRect();
    return {
      scrolls: scroll.scrollHeight > scroll.clientHeight + 1,
      atEnd: scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight <= 2,
      areaEnd: area.bottom,
      boxTop: box.top,
      boxBottom: box.bottom,
      lastBottom: last.bottom,
      viewport: window.innerHeight,
    };
  });
}

for (const where of ["agent screen", "side panel"] as const) {
  test(`in the ${where}, a long conversation scrolls above the message box and never under it`, async ({ page, admin }) => {
    await useFake(admin);
    const tag = Date.now().toString(36);
    const long = Array.from({ length: 60 }, (_, i) => `Line ${i + 1} of a long answer ${tag}.`).join("\n\n");
    await tell(`push ${JSON.stringify({ text: long })}`);
    let root;
    if (where === "agent screen") {
      await logIn(page, "/agents/assistant");
      root = page.locator(".agent-chat");
    } else {
      const res = await admin.post("/api/notes?type=note", { data: { note: shopping(`Layout ${tag}`, ["Milk"]) } });
      expect(res.status(), await res.text()).toBe(201);
      await logIn(page, `/notes/${((await res.json()) as { id: string }).id}`);
      await page.keyboard.press("Control+Shift+L");
      root = page.getByRole("complementary", { name: "AI" });
    }
    await pickFake(root);
    const box = root.getByRole("textbox", { name: "Ask AI" });
    await box.fill(`Write a lot ${tag}`);
    await box.press("Enter");
    await expect(root.getByText(`Line 60 of a long answer ${tag}.`)).toBeAttached();
    await expect.poll(async () => (await layoutOf(root)).atEnd).toBe(true);
    const l = await layoutOf(root);
    expect(l.scrolls, "the messages scroll on their own").toBe(true);
    expect(l.boxBottom, "the box is in view").toBeLessThanOrEqual(l.viewport + 1);
    expect(l.areaEnd, "the messages end where the box starts").toBeLessThanOrEqual(l.boxTop + 1);
    expect(l.lastBottom, "the last message is above the box").toBeLessThanOrEqual(l.boxTop + 1);
  });
}

test("pasted and picked files are uploaded as assets and named to the model by their ids", async ({ page, admin }) => {
  await useFake(admin);
  const tag = Date.now().toString(36);
  await logIn(page, "/agents/assistant");
  await pickFake(page.locator(".agent-chat"));
  const box = page.getByRole("textbox", { name: "Ask AI" });
  // A pasted screenshot arrives as image.png.
  await box.evaluate((el) => {
    const data = new DataTransfer();
    data.items.add(new File([new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10])], "image.png", { type: "image/png" }));
    el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
  });
  await page.locator(".ai-form input[type=file]").setInputFiles({ name: `notes-${tag}.txt`, mimeType: "text/plain", buffer: Buffer.from("hello") });
  const chips = page.getByRole("list", { name: "Attached files" }).first();
  await expect(chips.getByRole("listitem")).toHaveCount(2);
  await expect(chips).toContainText(/pasted-.*\.png/);
  await expect(chips).toContainText(`notes-${tag}.txt`);
  await expect(page.getByRole("button", { name: "Send" })).toBeEnabled();

  await tell(`push ${JSON.stringify({ text: `Got your files ${tag}.` })}`);
  const before = (await tell("requests")).count as number;
  await box.fill(`Here are files ${tag}`);
  await box.press("Enter");
  await expect(page.getByText(`Got your files ${tag}.`)).toBeVisible();
  expect((await tell("requests")).count).toBe(before + 1);
  // The sent message shows its files, linked to the assets the vault now holds.
  const sent = page.locator(".ai-sent-attachments").last();
  await expect(sent.getByRole("link", { name: `notes-${tag}.txt` })).toHaveAttribute("href", /^\/assets\/[0-9A-Z]+$/);
  const assets = (await (await admin.get("/api/assets")).json()) as { assets: { id: string; filename: string }[] };
  const txt = assets.assets.find((a) => a.filename === `notes-${tag}.txt`);
  expect(txt).toBeTruthy();
  expect(assets.assets.some((a) => /^pasted-.*\.png$/.test(a.filename))).toBe(true);
  // The message the model reads names each file by its asset id and says how to use it.
  const threads = (await (await admin.get("/api/agents/assistant/threads")).json()) as { threads: { id: string; title: string }[] };
  // Titled by the question, not the list of files.
  const thread = threads.threads.find((t) => t.title === `Here are files ${tag}`)!;
  const read = (await (await admin.get(`/api/agents/assistant/threads/${thread.id}`)).json()) as { messages: { content: { text?: string }[] }[] };
  const said = read.messages[0]!.content.map((c) => c.text ?? "").join("\n");
  expect(said).toContain("read_asset");
  expect(said).toContain(`- notes-${tag}.txt (asset ${txt!.id}, 5 bytes)`);
  // Reopened, the thread still shows them.
  await page.reload();
  await expect(page.locator(".ai-sent-attachments").last().getByRole("link", { name: `notes-${tag}.txt` })).toBeVisible();
});

test("the conversation follows new text at the end, stops when scrolled up, and follows again at the end", async ({ page, admin }) => {
  await useFake(admin);
  const tag = Date.now().toString(36);
  const long = Array.from({ length: 60 }, (_, i) => `Row ${i + 1} ${tag}.`).join("\n\n");
  await tell(`push ${JSON.stringify({ text: long })}`);
  await logIn(page, "/agents/assistant");
  await pickFake(page.locator(".agent-chat"));
  const box = page.getByRole("textbox", { name: "Ask AI" });
  await box.fill(`Follow ${tag}`);
  await box.press("Enter");
  const scroll = page.locator(".agent-chat [data-testid=ai-scroll]");
  await expect(page.getByText(`Row 60 ${tag}.`)).toBeAttached();
  const atEnd = () => scroll.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight <= 4);
  await expect.poll(atEnd).toBe(true);
  const grow = () =>
    scroll.evaluate((el) => {
      const li = document.createElement("li");
      li.style.height = "300px";
      el.querySelector("ol")!.appendChild(li);
    });
  // Scroll the view by `by` pixels or to the end, and wait for the page to see the scroll before
  // anything grows: under load both can fall in one frame, and the scroll then arrives after the
  // growth it came before.
  const scrollView = (by: number | "end") =>
    scroll.evaluate(
      (el, by) =>
        new Promise<void>((seen) => {
          el.addEventListener("scroll", () => seen(), { once: true });
          el.scrollTop = by === "end" ? el.scrollHeight : el.scrollTop + by;
        }),
      by,
    );

  // Scrolled up a little, growth leaves the view where it is.
  await scrollView(-40);
  const top = await scroll.evaluate((el) => el.scrollTop);
  await grow();
  await page.waitForTimeout(200);
  expect(await scroll.evaluate((el) => el.scrollTop)).toBe(top);
  expect(await atEnd()).toBe(false);

  // Back at the end, it follows again.
  await scrollView("end");
  await grow();
  await expect.poll(atEnd).toBe(true);
});

test("messages sent while an answer runs are queued, Edit takes them back, and the queue goes once the answer ends", async ({ page, admin }) => {
  await useFake(admin);
  const tag = Date.now().toString(36);
  await tell(`push ${JSON.stringify({ text: `Slow answer ${tag}.`, delay_ms: 400 })}`);
  await tell(`push ${JSON.stringify({ text: `Answer to the queue ${tag}.` })}`);
  await logIn(page, "/agents/assistant");
  await pickFake(page.locator(".agent-chat"));
  const box = page.getByRole("textbox", { name: "Ask AI" });
  await box.fill(`First ${tag}`);
  await box.press("Enter");
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  await box.fill(`Second ${tag}`);
  await box.press("Enter");
  await box.fill(`Third ${tag}`);
  await box.press("Enter");
  const queue = page.getByRole("region", { name: "Queued messages" });
  await expect(queue.getByRole("listitem")).toHaveText([`Second ${tag}`, `Third ${tag}`]);
  // Edit brings them all back into the box and empties the queue.
  await queue.getByRole("button", { name: "Edit" }).click();
  await expect(queue).toHaveCount(0);
  await expect(box).toHaveValue(`Second ${tag}\n\nThird ${tag}`);
  await box.press("Enter");
  await expect(page.getByRole("region", { name: "Queued messages" }).getByRole("listitem")).toHaveCount(1);
  // The answer ends, and the queue goes as one message.
  await expect(page.getByText(`Answer to the queue ${tag}.`)).toBeVisible({ timeout: 15_000 });
  await expect(page.getByRole("region", { name: "Queued messages" })).toHaveCount(0);
  await expect(page.locator(".ai-question").last()).toHaveText(`Second ${tag}\n\nThird ${tag}`);
});

test("Edit cuts the chat back to a question for the box, and Fork opens a new chat with what came before", async ({ page, admin }) => {
  await useFake(admin);
  const tag = Date.now().toString(36);
  await tell(`push ${JSON.stringify({ text: `A1 ${tag}` })}`);
  await tell(`push ${JSON.stringify({ text: `A2 ${tag}` })}`);
  await logIn(page, "/agents/assistant");
  await pickFake(page.locator(".agent-chat"));
  const box = page.getByRole("textbox", { name: "Ask AI" });
  await box.fill(`Q1 ${tag}`);
  await box.press("Enter");
  await expect(page.getByText(`A1 ${tag}`)).toBeVisible();
  await box.fill(`Q2 ${tag}`);
  await box.press("Enter");
  await expect(page.getByText(`A2 ${tag}`)).toBeVisible();
  const original = new URL(page.url()).searchParams.get("thread");
  expect(original).toBeTruthy();

  // Fork at Q2: a new chat holding Q1 and A1, with Q2 in the box; the original keeps all four.
  const q2 = page.locator(".ai-turn-user").filter({ hasText: `Q2 ${tag}` });
  await q2.hover();
  await q2.getByRole("button", { name: "Fork from this message" }).click();
  await expect.poll(() => new URL(page.url()).searchParams.get("thread")).not.toBe(original);
  await expect(box).toHaveValue(`Q2 ${tag}`);
  await expect(page.getByText(`A1 ${tag}`)).toBeVisible();
  await expect(page.getByText(`A2 ${tag}`)).toHaveCount(0);
  const kept = (await (await admin.get(`/api/agents/assistant/threads/${original}`)).json()) as { messages: unknown[] };
  expect(kept.messages).toHaveLength(4);

  // Back in the original, Edit Q2: it and A2 go, and Q2 is in the box.
  await box.fill("");
  await page.goto(`/agents/assistant?thread=${original}`);
  await expect(page.getByText(`A2 ${tag}`)).toBeVisible();
  page.once("dialog", (d) => void d.accept());
  const again = page.locator(".ai-turn-user").filter({ hasText: `Q2 ${tag}` });
  await again.hover();
  await again.getByRole("button", { name: "Edit this message" }).click();
  await expect(page.getByText(`A2 ${tag}`)).toHaveCount(0);
  await expect(page.getByText(`A1 ${tag}`)).toBeVisible();
  await expect(box).toHaveValue(`Q2 ${tag}`);
  const cut = (await (await admin.get(`/api/agents/assistant/threads/${original}`)).json()) as { messages: unknown[] };
  expect(cut.messages).toHaveLength(2);
});

// Named AI accounts (accounts A11, A12, A13): accounts added in Settings > AI, chatted on from the
// foot picker, renamed with the agents that name them following, and a ChatGPT account's usage.
test.describe("AI accounts", () => {
  let compatible: Running;
  let plan: Running;

  test.beforeAll(async () => {
    compatible = await startFake("TESTKIT_FAKE_OPENAI_BIN", "testkit-fake-openai", ["chat_completions"]);
    plan = await startFake("TESTKIT_FAKE_OPENAI_BIN", "testkit-fake-openai", ["plan"]);
  });

  test.afterAll(() => {
    compatible?.stop();
    plan?.stop();
  });

  test("Settings > AI adds an Anthropic and an OpenAI-compatible account, makes one the default, and the chat runs on each", async ({ page, admin }) => {
    const tag = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
    const claude = `E2E Claude ${tag}`;
    const router = `E2E Router ${tag}`;
    await logIn(page, "/settings/ai");
    const accounts = page.getByRole("region", { name: "Accounts" });

    // An Anthropic account: the kind first, then its fields.
    await accounts.getByRole("button", { name: "Add account" }).click();
    let form = page.getByRole("region", { name: "Add an account" });
    await form.getByLabel("Kind").selectOption({ label: "Anthropic API" });
    await form.getByLabel("Name", { exact: true }).fill(claude);
    await form.getByLabel("Base URL").fill(fake.base_url);
    await form.getByLabel("API key").fill(fake.api_key);
    await form.getByLabel("Model").fill("claude-opus-5");
    await form.getByRole("button", { name: "Add account" }).click();
    await expect(page.getByRole("region", { name: `Edit ${claude}` })).toBeVisible();
    await page.getByRole("region", { name: `Edit ${claude}` }).getByRole("button", { name: "Close" }).click();
    await expect(page.getByRole("row", { name: claude })).toContainText("Key set");

    // An OpenAI-compatible account on an OpenRouter-style URL, with extra headers.
    await accounts.getByRole("button", { name: "Add account" }).click();
    form = page.getByRole("region", { name: "Add an account" });
    await form.getByLabel("Kind").selectOption({ label: "OpenAI-compatible" });
    await form.getByLabel("Name", { exact: true }).fill(router);
    await form.getByLabel("Base URL").fill(`${compatible.fake.base_url.replace(/\/+$/, "")}/api/v1`);
    await form.getByLabel("API key").fill(compatible.fake.api_key);
    await form.getByRole("button", { name: "Add header" }).click();
    await form.getByLabel("Header 1 name").fill("X-Title");
    await form.getByLabel("Header 1 value").fill(`TheVault ${tag}`);
    await form.getByRole("button", { name: "Add account" }).click();
    const edit = page.getByRole("region", { name: `Edit ${router}` });
    await expect(edit).toBeVisible();
    await expect(edit.getByLabel("Header 1 value")).toHaveValue(`TheVault ${tag}`);
    await edit.getByRole("button", { name: "Close" }).click();

    // Make it the default: the table marks it, and the chat starts on it. The default goes back
    // before the router's reply is queued, so no other turn takes it; the chat keeps its account.
    const chat = page.locator(".agent-chat");
    const account = chat.getByLabel("Account");
    await withDefault(admin, async () => {
      await page.getByRole("row", { name: router }).getByRole("button", { name: `Make ${router} the default` }).click();
      await expect(page.getByRole("row", { name: router }).getByText("Default", { exact: true })).toBeVisible();
      const status = await aiAccounts(admin);
      expect(status.accounts.find((a) => a.id === status.default)?.name).toBe(router);
      await page.goto("/agents/assistant");
      await expect(account.locator("option:checked")).toHaveText(router);
      await expect(chat.getByLabel("Model")).not.toHaveValue("");
    });

    // Its extra headers go with the call.
    await compatible.tell(`push ${JSON.stringify({ text: `From the router ${tag}.` })}`);
    const box = chat.getByRole("textbox", { name: "Ask AI" });
    await box.fill(`Router question ${tag}`);
    await box.press("Enter");
    await expect(chat.getByText(`From the router ${tag}.`)).toBeVisible();
    const headers = (await compatible.tell("last_headers")).headers as Record<string, string>;
    expect(headers["x-title"]).toBe(`TheVault ${tag}`);
    expect(headers.authorization).toBe(`Bearer ${compatible.fake.api_key}`);

    // Another account picked at the foot runs the next turn.
    await tell(`push ${JSON.stringify({ text: `From Claude ${tag}.` })}`);
    const before = (await tell("requests")).count as number;
    await account.selectOption({ label: claude });
    await expect(chat.getByLabel("Model")).toHaveValue("claude-opus-5");
    await box.fill(`Claude question ${tag}`);
    await box.press("Enter");
    await expect(chat.getByText(`From Claude ${tag}.`)).toBeVisible();
    expect((await tell("requests")).count).toBe(before + 1);

    for (const name of [claude, router]) await dropAccount(admin, name);
  });

  test("renaming an account rewrites the agents that name it, and the agent page follows", async ({ page, admin }) => {
    const tag = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
    const old = `Rename me ${tag}`;
    const renamed = `Renamed ${tag}`;
    await useAccount(admin, { name: old, kind: "anthropic", base_url: fake.base_url, key: fake.api_key });
    const agent = `named_${tag}`;
    const put = await admin.put(`/api/agents/${agent}`, { data: { wcl: `label = "Named ${tag}"\naccount = "${old}"\n` } });
    expect(put.ok(), await put.text()).toBe(true);

    await logIn(page, `/agents/${agent}/settings`);
    const settings = activePane(page).getByRole("region", { name: "General" });
    await expect(settings.getByLabel("Account").locator("option:checked")).toHaveText(old);

    await page.goto("/settings/ai");
    await page.getByRole("row", { name: old }).getByRole("button", { name: `Edit ${old}` }).click();
    const form = page.getByRole("region", { name: `Edit ${old}` });
    await form.getByLabel("Name", { exact: true }).fill(renamed);
    await form.getByRole("button", { name: "Save" }).click();
    await expect(page.getByRole("region", { name: `Edit ${renamed}` })).toBeVisible();
    await expect(page.getByRole("row", { name: renamed })).toBeVisible();
    await expect
      .poll(async () => ((await (await admin.get(`/api/agents/${agent}`)).json()) as { account?: string }).account)
      .toBe(renamed);

    await page.goto(`/agents/${agent}/settings`);
    const again = activePane(page).getByRole("region", { name: "General" });
    await expect(again.getByLabel("Account").locator("option:checked")).toHaveText(renamed);
    await expect(again.getByText(/does not exist/)).toHaveCount(0);

    // Deleted, the agent names an account that is gone: it says so, and runs on the default.
    await dropAccount(admin, renamed);
    await page.reload();
    await expect(activePane(page).getByRole("region", { name: "General" }).getByRole("alert")).toHaveText(
      `Account ${renamed} does not exist; this agent uses the default account.`,
    );
    expect((await admin.delete(`/api/agents/${agent}`)).ok()).toBe(true);
  });

  test("a ChatGPT account shows its plan's usage as bars, and the chat foot its used percent", async ({ page, admin }) => {
    const tag = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
    const name = `E2E plan ${tag}`;
    const id = await useAccount(admin, { name, kind: "chatgpt" });
    const chat = page.locator(".agent-chat");
    // The seam that points plans at a fake is the server's one setting, so this worker's holds it
    // until its turn is answered.
    await alone("chatgpt-endpoint", async () => {
      await signInPlan(admin, id, plan.fake.base_url);
      // A plan response carries the usage headers, which the server keeps as the account's reading.
      await plan.tell(`push ${JSON.stringify({ text: `From the plan ${tag}.` })}`);

      await logIn(page, "/agents/assistant");
      await chat.getByLabel("Account").selectOption({ label: name });
      await expect(chat.getByLabel("Model")).not.toHaveValue("");
      const box = chat.getByRole("textbox", { name: "Ask AI" });
      await box.fill(`Plan question ${tag}`);
      await box.press("Enter");
      await expect(chat.getByText(`From the plan ${tag}.`)).toBeVisible();
    });

    await page.goto("/settings/ai");
    const bars = page.getByRole("group", { name: `${name} usage` });
    await expect(bars.getByRole("meter")).not.toHaveCount(0);
    await expect(bars).toContainText(/\d+% used, resets in /);
    await expect(bars).toContainText(/as of /i);

    // The chat foot shows how much of the plan is used next to the account.
    await page.goto("/agents/assistant");
    const option = chat.getByLabel("Account").locator("option", { hasText: name });
    await expect(option).toHaveText(new RegExp(`^${name} \\(\\d+% used\\)$`));
    await chat.getByLabel("Account").selectOption({ label: (await option.textContent())! });
    await expect(chat.getByTestId("account-used")).toHaveText(/^\d+% used$/);

    await dropAccount(admin, name);
  });
});

/**
 * Sign ChatGPT account `id` in against the fake plan server: its stored sign-in, a token good for
 * a day, and the hidden test seam that points the plan's calls at the fake (accounts A1).
 */
async function signInPlan(admin: import("@playwright/test").APIRequestContext, id: string, base: string) {
  const day = Date.now() + 86_400_000;
  for (const [name, value] of [
    ["ai_settings", JSON.stringify({ endpoints: { chatgpt: base } })],
    [
      `ai_account_chatgpt_${id.toLowerCase()}`,
      JSON.stringify({
        client_id: "oaiapp_test",
        host_id: "urn:uuid:00000000-0000-4000-8000-000000000000",
        // The fake plan takes any token but its API key.
        access_token: "plan-access-token",
        refresh_token: "plan-refresh-token",
        expires_ms: day,
        account: "e2e@example.com",
      }),
    ],
  ]) {
    const res = await admin.put(`/api/secrets/${name}`, { data: { value } });
    expect(res.ok(), await res.text()).toBe(true);
  }
}
