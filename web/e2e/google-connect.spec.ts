import { type ChildProcess, spawn } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { request } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// Settings > Integrations > Google (google-sync D1, D18): the loopback paste flow against the fake
// Google server, which the `google_settings` secret's endpoints point at. The consent page is the fake's too:
// the test plays the browser's part by sending the consent URL's query to it and pasting the
// address it redirects to.

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
