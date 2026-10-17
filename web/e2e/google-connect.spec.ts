import { type ChildProcess, spawn } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { request } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// Settings > Integrations > Google (google-sync D1, D18): the loopback paste flow against the fake
// Google server, which the settings note's endpoints point at. The consent page is the fake's too:
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

test.beforeAll(async () => {
  google = spawn(binary("TESTKIT_FAKE_GOOGLE_BIN", "testkit-fake-google"), [], { stdio: ["pipe", "pipe", "inherit"] });
  fake = await new Promise<Fake>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("testkit-fake-google did not report its address")), 30_000);
    const lines = createInterface({ input: google.stdout! });
    lines.once("line", (line) => {
      clearTimeout(timer);
      resolve(JSON.parse(line) as Fake);
      lines.on("line", () => {});
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
  // The sync's tasks, running a script that does nothing, paused as shipped.
  expect((await admin.put("/api/scripts/google_noop", { data: { source: "fn run() { }\n" } })).ok()).toBe(true);
  for (const task of ["google_gmail", "google_calendar"]) {
    const res = await admin.put(`/api/tasks/${task}`, {
      data: { wcl: `schedule = "*/5 * * * *"\nscript = "google_noop"\npaused = true\n` },
    });
    expect(res.ok(), await res.text()).toBe(true);
  }
  // One settings note, pointing at the fake.
  const existing = (await (await admin.get("/api/notes?type=google_sync")).json()) as { notes: { id: string; version: string }[] };
  for (const n of existing.notes) await admin.delete(`/api/notes/${n.id}?base_version=${n.version}`);
  const made = await admin.post("/api/notes?type=google_sync", {
    data: { note: { fields: { title: "Google" }, blocks: [{ kind: "dict", labels: ["endpoints"], fields: fake.endpoints }] } },
  });
  expect(made.status(), await made.text()).toBe(201);

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

  await card.getByLabel("Address from the browser").fill(pasted);
  await card.getByRole("button", { name: "Finish connecting" }).click();
  await expect(card.getByText("Connected as me@example.com")).toBeVisible();
  await expect(card.getByRole("row", { name: "Gmail" })).toContainText("Syncing");
  await expect(card.getByRole("row", { name: "Calendar" })).toContainText("Syncing");
  await expect(card.getByRole("button", { name: "Run Gmail now" })).toBeVisible();
  await expect(card.getByRole("button", { name: "Disconnect" })).toBeVisible();

  const secrets = (await (await admin.get("/api/secrets")).json()) as { secrets: { name: string }[] };
  expect(secrets.secrets.map((s) => s.name)).toEqual(
    expect.arrayContaining(["google_client_id", "google_client_secret", "google_refresh_token"]),
  );
  const task = (await (await admin.get("/api/tasks/google_gmail")).json()) as { paused: boolean };
  expect(task.paused).toBe(false);

  const shots = join(repo, ".scratch", "shots");
  mkdirSync(shots, { recursive: true });
  await card.screenshot({ path: join(shots, "ticket-google-connect.png") });

  page.once("dialog", (d) => void d.accept());
  await card.getByRole("button", { name: "Disconnect" }).click();
  await expect(card.getByRole("button", { name: "Connect Google" })).toBeVisible();
  await expect(card.getByRole("row", { name: "Gmail" })).toContainText("Paused");
  const after = (await (await admin.get("/api/secrets")).json()) as { secrets: { name: string }[] };
  expect(after.secrets.map((s) => s.name).filter((n) => n.startsWith("google_"))).toEqual([]);
});
