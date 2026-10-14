import { type APIRequestContext, type Page, request } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// Scripts and handlers, scheduled tasks, secrets, tokens, history and trash in the UI (S5; D9,
// D10, D11, D13).

function token(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

async function putType(admin: APIRequestContext, name: string, wcl: string) {
  const res = await admin.put(`/api/types/${name}`, { data: { wcl } });
  expect(res.status(), await res.text()).toBe(200);
}

async function putScript(admin: APIRequestContext, name: string, source: string) {
  const res = await admin.put(`/api/scripts/${name}`, { data: { source } });
  expect(res.status(), await res.text()).toBe(200);
}

async function createNote(admin: APIRequestContext, type: string, title: string) {
  const res = await admin.post(`/api/notes?type=${type}`, { data: { note: { fields: { title }, blocks: [] } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string; version: string };
}

async function readNote(admin: APIRequestContext, id: string) {
  const res = await admin.get(`/api/notes/${id}`);
  return { status: res.status(), body: (await res.json()) as { version: string; note: { fields: Record<string, unknown> } } };
}

/** Replace everything in the CodeMirror editor on the page with `source`, as one input. */
async function replaceCode(page: Page, source: string) {
  await page.locator(".script-editor .cm-content").click();
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.insertText(source);
}

const COUNT = `use vault

fn count_words(note: Value) -> Result[Value, string] {
    let fields = note.get("note").unwrap().get("fields").unwrap().as_map().unwrap()
    let title = fields.get("title").unwrap().as_string().unwrap()
    fields["words"] = Value::Int(title.split(" ").len())
    Ok(Value::Map(#{ "fields": Value::Map(fields) }))
}
`;

test("a before_update handler created in the UI runs when a note is saved", async ({ page, admin }) => {
  const t = token();
  const type = `article_${t}`;
  const script = `count_${t}`;
  await putType(
    admin,
    type,
    `label = "Article ${t}"

field words {
  kind     = "int"
  label    = "Words"
  computed = true
}

template default {
  tab "Main" {
    slot "title"
    slot "words"
  }
}
`,
  );
  const note = await createNote(admin, type, "Hello");

  // Add a before_update handler, and write its script right there in the handler. A broken draft
  // shows why it does not compile.
  await logIn(page, `/settings/types/${type}/edit`);
  await page.getByRole("button", { name: "Add handler" }).click();
  const card = page.getByTestId("handler-card-0");
  await card.getByLabel("Event").selectOption("before_update");
  await card.getByLabel("Script", { exact: true }).selectOption({ label: "New script…" });
  await card.getByLabel("New script name").fill(script);
  await card.getByRole("button", { name: "Create script" }).click();
  await expect(card.getByLabel("Script", { exact: true })).toHaveValue(script);
  await replaceCode(page, "fn count_words( {\n");
  await card.getByRole("button", { name: "Save script" }).click();
  await expect(card.getByRole("alert", { name: "Script problems" })).toContainText("does not compile");
  await replaceCode(page, COUNT);
  await card.getByRole("button", { name: "Save script" }).click();
  await expect(card.getByRole("status")).toHaveText("Saved.");
  await expect(card.getByRole("alert", { name: "Script problems" })).toHaveCount(0);
  expect(((await (await admin.get(`/api/scripts/${script}`)).json()) as { source: string }).source).toBe(COUNT);
  await card.getByLabel("Function").fill("count_words");
  await page.getByRole("button", { name: "Preview changes" }).click();
  await page.getByRole("button", { name: "Apply changes" }).click();
  await expect(page).toHaveURL(new RegExp(`/types/${type}$`));
  const schema = (await (await admin.get(`/api/types/${type}`, { headers: { Accept: "text/plain" } })).text());
  expect(schema).toContain(`handler "before_update"`);

  // Saving the note runs it.
  await page.goto(`/notes/${note.id}`);
  await page.getByLabel("Title").fill("Hello brave new world");
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
  await expect(page.getByTestId("field-words")).toContainText("4");
  expect((await readNote(admin, note.id)).body.note.fields.words).toBe(4);
});

test("a failing task shows as failed with its log, on its page and in the task list", async ({ page, admin }) => {
  const t = token();
  const script = `flaky_${t}`;
  const task = `flaky_${t}`;
  await putScript(
    admin,
    script,
    `use vault\nfn run() -> Result[int, string] {\n    vault::log("checking upstream ${t}")\n    Err("the upstream is down")\n}\n`,
  );

  await logIn(page, "/tasks/new");
  await page.getByLabel("Name").fill(task);
  await page.getByLabel("Schedule (cron)").fill("0 3 * * *");
  await page.getByLabel("Script", { exact: true }).selectOption(script);
  await page.getByLabel("Retries").fill("0");
  await page.getByRole("button", { name: "Create task" }).click();
  await expect(page).toHaveURL(new RegExp(`/tasks/${task}$`));
  const file = await (await admin.get(`/api/tasks/${task}`, { headers: { Accept: "text/plain" } })).text();
  expect(file).toContain(`script   = "${script}"`);

  await page.getByRole("button", { name: "Run now" }).click();
  await expect(page.getByRole("status")).toHaveText(/The run failed: .*the upstream is down/);
  const runs = page.getByRole("list", { name: "Runs" });
  await expect(runs.locator("[data-outcome=failed]")).toHaveCount(1);
  await expect(runs.getByText(`checking upstream ${t}`)).toBeVisible();
  await expect(page.getByRole("heading", { level: 1 })).toContainText("failing");

  await page.goto("/tasks");
  const row = page.locator(`tr[data-task="${task}"]`);
  await expect(row).toContainText("failing");
  await expect(row.locator("[data-outcome=failed]")).toBeVisible();

  // The log is still there after a reload.
  await page.goto(`/tasks/${task}`);
  await page.getByRole("list", { name: "Runs" }).locator("summary").first().click();
  await expect(page.getByRole("list", { name: "Runs" }).getByText(`checking upstream ${t}`)).toBeVisible();
});

test("a note is restored from its history and from the trash in the UI", async ({ page, admin }) => {
  const t = token();
  const note = await createNote(admin, "task", `First ${t}`);
  const res = await admin.put(`/api/notes/${note.id}`, {
    data: { note: { fields: { title: `Second ${t}` }, blocks: [] }, base_version: note.version },
  });
  expect(res.status(), await res.text()).toBe(200);

  await logIn(page, `/notes/${note.id}`);
  await page.getByRole("button", { name: "History", exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${note.id}$`));
  const versions = page.getByRole("complementary", { name: "History" }).getByRole("list", { name: "Versions" }).getByRole("listitem");
  await expect(versions).toHaveCount(2);
  await expect(versions.first()).toContainText("current");
  await versions.last().getByRole("button").first().click();
  const preview = page.getByRole("region", { name: "Version preview" });
  await expect(preview).toBeVisible();
  await expect(page.getByLabel("Title", { exact: true })).toHaveText(`First ${t}`);
  page.once("dialog", (d) => void d.accept());
  await preview.getByRole("button", { name: "Restore this version" }).click();
  await expect(preview).toHaveCount(0);
  await expect(page).toHaveURL(new RegExp(`/notes/${note.id}$`));
  await expect(page.getByRole("heading", { level: 1, name: `First ${t}`, exact: true })).toBeVisible();
  expect((await readNote(admin, note.id)).body.note.fields.title).toBe(`First ${t}`);

  // Delete it from the note page, then bring it back from the trash.
  page.once("dialog", (d) => void d.accept());
  await page.getByRole("button", { name: "Delete" }).click();
  await expect(page).toHaveURL(/\/types\/task$/);
  expect((await readNote(admin, note.id)).status).toBe(404);

  await page.goto("/trash");
  const row = page.locator(`tr[data-note="${note.id}"]`);
  await expect(row).toContainText(`First ${t}`);
  await row.getByRole("button", { name: "Restore" }).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${note.id}$`));
  await expect(page.getByRole("heading", { level: 1, name: `First ${t}`, exact: true })).toBeVisible();
  expect((await readNote(admin, note.id)).status).toBe(200);
  await page.goto("/trash");
  await expect(page.locator(`tr[data-note="${note.id}"]`)).toHaveCount(0);
});

test("a new token's secret is shown once and is gone after a reload", async ({ page, baseURL }) => {
  const name = `agent-${token()}`;
  await logIn(page, "/tokens");
  await page.getByLabel("Name").fill(name);
  await page.getByLabel("Scope").selectOption("write");
  await page.getByRole("button", { name: "Create token" }).click();
  const shown = page.getByRole("region", { name: "New token secret" });
  await expect(shown).toBeVisible();
  const secret = await shown.getByLabel("Token secret").inputValue();
  expect(secret.length).toBeGreaterThan(16);
  await expect(page.locator(`tr[data-token="${name}"]`)).toContainText("write");

  // The secret works as a bearer token.
  const agent = await request.newContext({ baseURL, extraHTTPHeaders: { Authorization: `Bearer ${secret}` } });
  expect((await agent.get("/api/types")).status()).toBe(200);

  await page.reload();
  await expect(page.locator(`tr[data-token="${name}"]`)).toBeVisible();
  await expect(page.getByRole("region", { name: "New token secret" })).toHaveCount(0);
  expect(await page.content()).not.toContain(secret);

  // Revoking it stops the secret working.
  page.once("dialog", (d) => void d.accept());
  await page.locator(`tr[data-token="${name}"]`).getByRole("button", { name: "Revoke" }).click();
  await expect(page.locator(`tr[data-token="${name}"]`)).toHaveCount(0);
  expect((await agent.get("/api/types")).status()).toBe(401);
  await agent.dispose();
});

test("a secret's value is written but never shown", async ({ page, admin }) => {
  const name = `hook_${token()}`;
  const value = `s3cret-${token()}`;
  await logIn(page, "/secrets");
  await page.getByLabel("Name").fill(name);
  await page.getByLabel("Value").fill(value);
  await page.getByRole("button", { name: "Save secret" }).click();
  await expect(page.getByRole("status")).toHaveText(`Stored ${name}.`);
  await expect(page.locator(`tr[data-secret="${name}"]`)).toBeVisible();
  await expect(page.getByLabel("Value")).toHaveValue("");
  expect(await page.content()).not.toContain(value);
  const listed = await (await admin.get("/api/secrets")).text();
  expect(listed).toContain(name);
  expect(listed).not.toContain(value);

  page.once("dialog", (d) => void d.accept());
  await page.locator(`tr[data-secret="${name}"]`).getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(`tr[data-secret="${name}"]`)).toHaveCount(0);
});
