// Rich import (rich-import H1, H2, H3, H6): a newsletter-like email, seeded through the fake Gmail
// and pulled by the shipped `google_gmail` script, shows on its note page as headings, its inline
// image, a table and its quoted history in a closed toggle, and View original opens the HTML it
// came from in a sandboxed frame.
//
// The Gmail sync reads the Google secrets, which google-connect.spec.ts changes on the shared
// server, so this spec runs against a server of its own.

import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { type APIRequestContext, type Page, test as base, expect, request } from "@playwright/test";

const repo = fileURLToPath(new URL("../..", import.meta.url));

function binary(env: string, name: string): string {
  const path = process.env[env] ?? join(repo, "target", "debug", name);
  if (!existsSync(path)) throw new Error(`${path} is missing. Run \`cargo build -p server -p testkit --bins\`.`);
  return path;
}

/** The first JSON line `child` prints. */
function firstLine<T>(child: ChildProcess, what: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`${what} did not report its address`)), 60_000);
    const lines = createInterface({ input: child.stdout! });
    lines.once("line", (line) => {
      clearTimeout(timer);
      resolve(JSON.parse(line) as T);
      lines.on("line", (l) => answers.shift()?.(l));
    });
    child.once("exit", (code) => reject(new Error(`${what} exited with ${code}`)));
  });
}

interface Server {
  url: string;
  password: string;
  token: string;
}

interface Fake {
  host: string;
  endpoints: Record<string, string>;
}

/** Waiters for the fake Google's answers to the commands sent on its standard input. */
const answers: ((line: string) => void)[] = [];

/** A vault-server of this spec's own, on a fresh vault, port 0 and a secrets key. */
async function startServer(): Promise<{ server: Server; stop: () => void }> {
  const vault = mkdtempSync(join(tmpdir(), "thevault-e2e-rich-"));
  const config = mkdtempSync(join(tmpdir(), "thevault-e2e-rich-config-"));
  const key = join(config, "vault.key");
  writeFileSync(key, randomBytes(32), { mode: 0o600 });
  const seeded = JSON.parse(execFileSync(binary("TESTKIT_SEED_BIN", "testkit-seed"), [vault], { encoding: "utf8" })) as {
    password: string;
    admin_token: string;
  };
  const child = spawn(binary("VAULT_SERVER_BIN", "vault-server"), ["--vault", vault, "--listen", "127.0.0.1:0", "--web", join(repo, "web", "dist")], {
    stdio: ["ignore", "pipe", "inherit"],
    env: { ...process.env, VAULT_SECRET_KEY_FILE: key },
  });
  const stop = () => {
    child.kill();
    rmSync(vault, { recursive: true, force: true });
    rmSync(config, { recursive: true, force: true });
  };
  const address = await new Promise<string>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("vault-server did not report its address")), 60_000);
    const lines = createInterface({ input: child.stdout! });
    lines.once("line", (line) => {
      clearTimeout(timer);
      resolve(JSON.parse(line).listen);
      lines.on("line", () => {});
    });
    child.once("exit", (code) => reject(new Error(`vault-server exited with ${code}`)));
  }).catch((e) => {
    stop();
    throw e;
  });
  return { server: { url: `http://${address}`, password: seeded.password, token: seeded.admin_token }, stop };
}

const test = base.extend<{ admin: APIRequestContext }, { server: Server; google: { fake: Fake; tell: (line: string) => Promise<void> } }>({
  server: [
    async ({}, use) => {
      const { server, stop } = await startServer();
      await use(server);
      stop();
    },
    { scope: "worker" },
  ],
  google: [
    async ({}, use) => {
      const child = spawn(binary("TESTKIT_FAKE_GOOGLE_BIN", "testkit-fake-google"), [], { stdio: ["pipe", "pipe", "inherit"] });
      const fake = await firstLine<Fake>(child, "testkit-fake-google");
      const tell = async (line: string) => {
        const answer = new Promise<string>((resolve) => answers.push(resolve));
        child.stdin!.write(`${line}\n`);
        expect(JSON.parse(await answer)).toEqual({ ok: line });
      };
      await use({ fake, tell });
      child.kill();
    },
    { scope: "worker" },
  ],
  baseURL: async ({ server }, use) => use(server.url),
  // The browser reaches nothing but the server: the email's remote images are blocked by the
  // frame's own policy, and the test proves it never asks for them.
  context: async ({ context, baseURL }, use) => {
    const origin = new URL(baseURL!).origin;
    await context.route((url) => url.origin !== origin, (route) => route.abort());
    await use(context);
  },
  admin: async ({ server }, use) => {
    const ctx = await request.newContext({ baseURL: server.url, extraHTTPHeaders: { Authorization: `Bearer ${server.token}` } });
    await use(ctx);
    await ctx.dispose();
  },
});

async function logIn(page: Page, server: Server, path: string) {
  await page.goto(`/login?next=${encodeURIComponent(path)}`);
  await page.getByLabel("Password").fill(server.password);
  await page.getByRole("button", { name: "Log in" }).click();
  await expect(page.getByRole("navigation", { name: "Main" })).toBeVisible();
}

/** A 2 by 2 PNG. */
const PNG = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAFklEQVR4nGP8z8DAwMDAxMDAwMDAAAANHQEDasKb6QAAAABJRU5ErkJggg==";

/** The email: a preheader, a heading, an inline image, a data table, a button, a remote image
 * and a quoted reply, all anonymised. */
const HTML = [
  '<html><head><style>.x{color:red}</style></head><body>',
  '<span class="preheader" style="display:none">Hidden preview text</span>',
  '<h1>Trails of the week</h1>',
  "<p>Three <strong>new</strong> routes opened this weekend.</p>",
  '<p><img src="cid:map@mail.example" alt="Trail map" width="200" height="120"></p>',
  "<table><tr><th>Route</th><th>Length</th></tr><tr><td>Ridge loop</td><td>12 km</td></tr><tr><td>Lake path</td><td>5 km</td></tr></table>",
  '<p><a href="https://shop.example.com/maps" style="background-color:#16a34a;padding:8px">Get the maps</a></p>',
  // On the loopback address, so the server refuses it rather than reaching any network.
  '<p><img src="http://127.0.0.1:9/hero.jpg" alt="Hero photo" width="560"></p>',
  '<div class="gmail_quote"><div class="gmail_attr">On Mon, Alex Example wrote:</div><blockquote class="gmail_quote">Earlier message text</blockquote></div>',
  "</body></html>",
].join("");

test("an imported newsletter shows as rich blocks and View original opens it in a sandbox", async ({ page, admin, server, google }) => {
  const { fake, tell } = google;
  for (const [name, value] of [
    ["google_client_id", "fake-client-id.apps.googleusercontent.com"],
    ["google_client_secret", "fake-client-secret"],
    ["google_refresh_token", "fake-refresh-token"],
    ["google_settings", JSON.stringify({ endpoints: fake.endpoints, gmail_mode: "all" })],
  ]) {
    const res = await admin.put(`/api/secrets/${name}`, { data: { value } });
    expect(res.ok(), await res.text()).toBe(true);
  }
  const secrets = ["google_client_id", "google_client_secret", "google_refresh_token", "google_settings", "google_gmail_state"];
  const task = await admin.put("/api/tasks/google_gmail", {
    data: {
      wcl:
        `schedule = "*/10 * * * *"\nscript = "google_gmail"\ntimeout = "PT5M"\nretries = 0\n` +
        `allow_domains = ["${fake.host}"]\nsecrets = ${JSON.stringify(secrets)}\npaused = true\n`,
    },
  });
  expect(task.ok(), await task.text()).toBe(true);
  await tell(
    `add-message ${JSON.stringify({
      id: "news-1",
      labels: ["INBOX"],
      at: Date.now() - 3_600_000,
      from: "Example Outdoors <news@example.com>",
      to: ["me@example.com"],
      subject: "Trails of the week",
      text: "Three new routes opened this weekend.",
      html: HTML,
      attachments: [{ filename: "map.png", mime: "image/png", base64: PNG, content_id: "map@mail.example" }],
    })}`,
  );
  const run = await admin.post("/api/tasks/google_gmail/run?wait=true");
  expect(run.ok(), await run.text()).toBe(true);
  expect(((await run.json()) as { outcome: string }).outcome).toBe("succeeded");
  const query = await admin.post("/api/query", { data: { query: 'from email where gmail_ref == "news-1"' } });
  const rows = ((await query.json()) as { rows: { id: string }[] }).rows;
  expect(rows).toHaveLength(1);

  // A request the browser makes for a remote image of the original: the frame's policy blocks
  // it before it leaves, and anything that left would reach a route that aborts it.
  const remote: string[] = [];
  page.on("requestfailed", (r) => {
    if (r.url().includes("hero.jpg")) remote.push(r.failure()?.errorText ?? "");
  });
  page.on("requestfinished", (r) => {
    if (r.url().includes("hero.jpg")) remote.push("finished");
  });
  await logIn(page, server, `/notes/${rows[0]!.id}`);
  const pane = page.locator(".pane[data-active]");
  await expect(pane.getByRole("heading", { name: "Trails of the week" }).first()).toBeVisible();
  await expect(pane.getByText("Three new routes opened this weekend.")).toBeVisible();
  // The inline image, stored as an asset, where the HTML showed it.
  const image = pane.locator("img[alt='Trail map']");
  await expect(image).toBeVisible();
  await expect(image).toHaveAttribute("src", /\/api\/assets\//);
  await expect.poll(() => image.evaluate((img: HTMLImageElement) => img.naturalWidth)).toBe(2);
  // The data table, its header row first.
  await expect(pane.getByRole("cell", { name: "Ridge loop" })).toBeVisible();
  await expect(pane.locator("th", { hasText: "Route" })).toBeVisible();
  // The button is a bold link of its own.
  await expect(pane.getByRole("link", { name: "Get the maps" }).locator("strong")).toBeVisible();
  // The quoted reply sits in a closed toggle.
  await expect(pane.getByText("Quoted text", { exact: true })).toBeVisible();
  await expect(pane.getByText("Earlier message text")).toBeHidden();
  // The preheader never shows.
  await expect(pane.getByText("Hidden preview text")).toHaveCount(0);

  await pane.getByRole("button", { name: "View original" }).click();
  const dialog = page.getByRole("dialog", { name: "Original" });
  const frame = dialog.locator("iframe");
  await expect(frame).toHaveAttribute("sandbox", "allow-popups allow-popups-to-escape-sandbox");
  const original = dialog.frameLocator("iframe");
  await expect(original.getByRole("heading", { name: "Trails of the week" })).toBeVisible();
  // The inline image is in place in the original too, as a data: URI.
  await expect(original.locator("img[alt='Trail map']")).toHaveAttribute("src", /^data:image\/png;base64,/);
  // Remote images wait until asked for, and the page never asked for one.
  await expect(dialog.getByRole("button", { name: "Load remote images" })).toBeVisible();
  expect(remote.every((why) => why === "csp"), JSON.stringify(remote)).toBe(true);
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
});
