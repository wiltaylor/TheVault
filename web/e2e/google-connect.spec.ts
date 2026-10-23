import { type ChildProcess, spawn } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { type APIRequestContext, request } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// Settings > Integrations > Google (google-sync D1, D18): the loopback paste flow against the fake
// Google server, which the `google_settings` secret's endpoints point at. The consent page is the fake's too:
// the test plays the browser's part by sending the consent URL's query to it and pasting the
// address it redirects to.
//
// The Outbox (outbox X7, X8) runs against the same fake in this file, so the two tests never
// change the Google secrets at once: compose an email from the side menu's Outbox, send it, and
// find it in Sent with the copy Gmail kept linked.

const repo = fileURLToPath(new URL("../..", import.meta.url));

function binary(env: string, name: string): string {
  const path = process.env[env] ?? join(repo, "target", "debug", name);
  if (!existsSync(path)) throw new Error(`${path} is missing. Run \`cargo build -p server -p testkit --bins\`.`);
  return path;
}

interface Fake {
  base_url: string;
  host: string;
  endpoints: Record<string, string>;
  auth: string;
}

let google: ChildProcess;
let fake: Fake;
/** Waiters for the fake's answers to the commands sent on its standard input, oldest first. */
const answers: ((line: string) => void)[] = [];

/** Send `line` to the fake (see `testkit-fake-google`) and wait until it has taken effect. */
async function tell(line: string): Promise<void> {
  const answer = new Promise<string>((resolve) => answers.push(resolve));
  google.stdin!.write(`${line}\n`);
  expect(JSON.parse(await answer)).toEqual({ ok: line });
}

test.beforeAll(async () => {
  google = spawn(binary("TESTKIT_FAKE_GOOGLE_BIN", "testkit-fake-google"), [], { stdio: ["pipe", "pipe", "inherit"] });
  fake = await new Promise<Fake>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("testkit-fake-google did not report its address")), 30_000);
    const lines = createInterface({ input: google.stdout! });
    lines.once("line", (line) => {
      clearTimeout(timer);
      resolve(JSON.parse(line) as Fake);
      lines.on("line", (answer) => answers.shift()?.(answer));
    });
    google.once("exit", (code) => reject(new Error(`testkit-fake-google exited with ${code}`)));
  });
});

test.afterAll(() => {
  google?.kill();
});

test("the Google card connects through a pasted loopback address, shows the account and disconnects", async ({
  page,
  admin,
}) => {
  // The sync's tasks, running the shipped scripts against the fake, paused as shipped.
  for (const [task, state] of [
    ["google_gmail", "google_gmail_state"],
    ["google_calendar", "google_calendar_state"],
  ]) {
    const secrets = ["google_client_id", "google_client_secret", "google_refresh_token", "google_settings", state];
    const res = await admin.put(`/api/tasks/${task}`, {
      data: {
        wcl:
          `schedule = "*/5 * * * *"\nscript = "${task}"\ntimeout = "PT5M"\nretries = 0\n` +
          `allow_domains = ["${fake.host}"]\nsecrets = ${JSON.stringify(secrets)}\npaused = true\n`,
      },
    });
    expect(res.ok(), await res.text()).toBe(true);
  }
  // The settings secret, pointing at the fake (google-sync D21).
  const settings = await admin.put("/api/secrets/google_settings", {
    data: { value: JSON.stringify({ endpoints: fake.endpoints }) },
  });
  expect(settings.ok(), await settings.text()).toBe(true);

  await logIn(page, "/settings");
  await page.getByRole("navigation", { name: "Settings sections" }).getByRole("link", { name: "Integrations" }).click();
  await expect(page).toHaveURL(/\/settings\/integrations$/);
  const card = page.getByRole("region", { name: "Google" });
  await expect(card.getByRole("link", { name: "How to set up a Google OAuth client" })).toHaveAttribute("href", /docs\/google\.md$/);
  await expect(card.getByRole("row", { name: "Gmail" })).toContainText("Paused");

  await card.getByLabel("Client ID").fill("fake-client-id.apps.googleusercontent.com");
  await card.getByLabel("Client secret").fill("fake-client-secret");
  await card.getByRole("button", { name: "Connect Google" }).click();

  // On http the page stays, linking to Google's consent page and asking for the address to paste.
  const open = card.getByRole("link", { name: "Open Google's sign-in page" });
  const authUrl = new URL((await open.getAttribute("href"))!);
  expect(authUrl.origin).toBe("https://accounts.google.com");
  expect(authUrl.searchParams.get("redirect_uri")).toBe("http://127.0.0.1:53682/");
  expect(authUrl.searchParams.get("access_type")).toBe("offline");
  expect(authUrl.searchParams.get("prompt")).toBe("consent");

  // The browser's part: approve on the (fake) consent page and copy the address it fails to load.
  const browser = await request.newContext();
  const consent = await browser.get(`${fake.auth}${authUrl.search}`, { maxRedirects: 0 });
  expect(consent.status()).toBe(302);
  const pasted = consent.headers()["location"]!;
  expect(pasted).toMatch(/^http:\/\/127\.0\.0\.1:53682\/\?state=.*&code=/);
  await browser.dispose();

  // The first runs wait on the fake until it is told to answer, so the card shows them syncing
  // however quickly it reads the status (ticket 123).
  await tell("gmail-delay 600000");
  await tell("calendar-delay 600000");
  await card.getByLabel("Address from the browser").fill(pasted);
  await card.getByRole("button", { name: "Finish connecting" }).click();
  await expect(card.getByText("Connected as me@example.com")).toBeVisible();
  const gmail = card.getByRole("row", { name: "Gmail" });
  const calendar = card.getByRole("row", { name: "Calendar" });
  await expect(gmail).toContainText("Syncing");
  await expect(calendar).toContainText("Syncing");
  await expect(card.getByRole("button", { name: "Run Gmail now" })).toBeDisabled();

  // Released, the runs finish and the card, polling, shows what each pulled.
  await tell("gmail-delay off");
  await tell("calendar-delay off");
  await expect(gmail).toContainText("Gmail: 0 new", { timeout: 30_000 });
  await expect(gmail).not.toContainText("Syncing");
  await expect(gmail).toContainText("No errors");
  await expect(calendar).toContainText("0 created, 0 updated, 0 trashed", { timeout: 30_000 });
  await expect(calendar).not.toContainText("Syncing");
  await expect(card.getByRole("button", { name: "Run Gmail now" })).toBeEnabled();
  await expect(card.getByRole("button", { name: "Disconnect" })).toBeVisible();

  const secrets = (await (await admin.get("/api/secrets")).json()) as { secrets: { name: string }[] };
  expect(secrets.secrets.map((s) => s.name)).toEqual(
    expect.arrayContaining(["google_client_id", "google_client_secret", "google_refresh_token"]),
  );
  const task = (await (await admin.get("/api/tasks/google_gmail")).json()) as { paused: boolean };
  expect(task.paused).toBe(false);

  // The settings form saves what the sync pulls.
  const form = card.getByRole("form", { name: "Google sync settings" });
  await form.getByLabel("Window (days)").fill("30");
  await form.getByRole("button", { name: "Save" }).click();
  await expect(card.getByText("Saved. The next runs use these settings.")).toBeVisible();
  const status = (await (await admin.get("/api/integrations/google")).json()) as { settings: { gmail_window_days: number } };
  expect(status.settings.gmail_window_days).toBe(30);

  const shots = join(repo, ".scratch", "shots");
  mkdirSync(shots, { recursive: true });
  await card.screenshot({ path: join(shots, "ticket-google-connect.png") });

  page.once("dialog", (d) => void d.accept());
  await card.getByRole("button", { name: "Disconnect" }).click();
  await expect(card.getByRole("button", { name: "Connect Google" })).toBeVisible();
  await expect(card.getByRole("row", { name: "Gmail" })).toContainText("Paused");
  const after = (await (await admin.get("/api/secrets")).json()) as { secrets: { name: string }[] };
  // The settings outlast the sign-in, and so do the runs' state secrets, their cursors cleared (docs/google.md).
  expect(after.secrets.map((s) => s.name).filter((n) => n.startsWith("google_"))).toEqual([
    "google_calendar_state",
    "google_gmail_state",
    "google_settings",
  ]);
});

/** Outbox item `id`'s fields, as stored. */
async function itemFields(admin: APIRequestContext, id: string): Promise<Record<string, unknown>> {
  return ((await (await admin.get(`/api/notes/${id}`)).json()) as { note: { fields: Record<string, unknown> } }).note.fields;
}

test("an email composed in the Outbox is sent through Gmail and shows in Sent with its copy linked", async ({ page, admin }) => {
  const subject = `Outbox lunch ${Date.now().toString(36)}`;
  for (const [name, value] of [
    ["google_client_id", "fake-client-id.apps.googleusercontent.com"],
    ["google_client_secret", "fake-client-secret"],
    ["google_refresh_token", "fake-refresh-token"],
    ["google_account", "me@example.com"],
    ["google_settings", JSON.stringify({ endpoints: fake.endpoints, scopes: ["https://www.googleapis.com/auth/gmail.send"] })],
  ]) {
    const res = await admin.put(`/api/secrets/${name}`, { data: { value } });
    expect(res.ok(), await res.text()).toBe(true);
  }
  // The shipped Outbox type, with the fake's host allowed as the server tests do.
  const shipped = (await (await admin.get("/api/types/outbox")).json()) as { wcl: string };
  const allowed = shipped.wcl.replaceAll("allow_domains = [", `allow_domains = ["${fake.host}", `);
  const put = await admin.put("/api/types/outbox", { data: { wcl: allowed } });
  expect(put.ok(), await put.text()).toBe(true);

  try {
    await logIn(page, "/daily");
    const outbox = page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: /^Outbox/ });
    await outbox.click();
    await expect(page).toHaveURL(/\/outbox$/);
    await page.getByRole("button", { name: "New email" }).click();
    await expect(page).toHaveURL(/\/notes\/[0-9A-Z]{26}$/);
    const id = /\/notes\/([0-9A-Z]{26})$/.exec(page.url())![1]!;

    // Compose: the title, To, Subject and the body.
    const title = page.getByRole("textbox", { name: "Title" });
    await title.click();
    await page.keyboard.press("ControlOrMeta+A");
    await page.keyboard.type(subject);
    await page.getByRole("button", { name: "Add to To" }).click();
    const to = page.getByRole("combobox", { name: "Add to To" });
    await to.fill("ada@example.com");
    await to.press("Enter");
    await to.press("Escape");
    await page.getByLabel("Subject", { exact: true }).fill(subject);
    await page.getByRole("tab", { name: "Message" }).click();
    await page.getByRole("textbox", { name: "Body" }).click();
    await page.keyboard.type("Shall we meet at noon?");
    await expect(page.getByTestId("save-status")).toHaveText("Saved");

    await page.getByRole("button", { name: "Send", exact: true }).click();
    await expect.poll(() => itemFields(admin, id), { timeout: 30_000 }).toMatchObject({ status: "sent", to: ["ada@example.com"], subject });
    await expect.poll(async () => (await itemFields(admin, id)).result, { timeout: 30_000 }).toBeTruthy();
    const fields = await itemFields(admin, id);
    expect(fields.sent_ref).toBeTruthy();
    const copy = fields.result as string;

    // The Outbox lists it under Sent, linking to the copy.
    await outbox.click();
    await page.getByRole("tab", { name: "Sent" }).click();
    const item = page
      .getByRole("list", { name: "Sent items" })
      .getByRole("listitem")
      .filter({ has: page.getByRole("link", { name: subject, exact: true }) });
    await expect(item).toBeVisible();
    await expect(item.getByLabel("Email preview")).toContainText("ada@example.com");
    await expect(item.getByLabel("Email preview")).toContainText("Shall we meet at noon?");
    const link = item.getByRole("link", { name: /^Sent copy/ });
    await expect(link).toHaveAttribute("href", `/notes/${copy}`);
    await link.click();
    await expect(page).toHaveURL(new RegExp(`/notes/${copy}$`));
    await expect(page.getByRole("textbox", { name: "Title" })).toHaveText(subject);
    // The copy is an email note, so it offers Reply.
    await expect(page.getByRole("button", { name: "Reply" })).toBeVisible();
  } finally {
    await admin.put("/api/types/outbox", { data: { wcl: shipped.wcl } });
    for (const name of ["google_client_id", "google_client_secret", "google_refresh_token", "google_account"]) {
      await admin.delete(`/api/secrets/${name}`);
    }
  }
});
