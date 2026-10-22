// The notifications bell: a notification made through the API shows on the badge, opens its note
// or page in the pane when clicked and is then read; mark all read, dismiss and clear read; the
// phone's sheet under the top bar; keyboard use; a notification made while the page is open
// reaching the badge at once over the live event stream, and on the next poll when the stream is
// blocked; and an open note taking a change made elsewhere.
//
// Notifications are global to a vault, and other specs make them (a failing scheduled script
// does), so these tests run against a server of their own, one test at a time.

import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { type APIRequestContext, type Page, test as base, expect, request } from "@playwright/test";
import { PROJECT_TYPE, TASK_TYPE } from "./vault";

const repo = fileURLToPath(new URL("../..", import.meta.url));

function binary(env: string, name: string): string {
  const path = process.env[env] ?? join(repo, "target", "debug", name);
  if (!existsSync(path)) throw new Error(`${path} is missing. Run \`cargo build -p server -p testkit --bins\`.`);
  return path;
}

interface Server {
  url: string;
  password: string;
  token: string;
}

/** A vault-server of this spec's own, on a fresh vault and port 0. */
async function startServer(): Promise<{ server: Server; stop: () => void }> {
  const vault = mkdtempSync(join(tmpdir(), "thevault-e2e-notify-"));
  mkdirSync(join(vault, "types"));
  writeFileSync(join(vault, "types/task.wcl"), TASK_TYPE);
  writeFileSync(join(vault, "types/project.wcl"), PROJECT_TYPE);
  const seeded = JSON.parse(execFileSync(binary("TESTKIT_SEED_BIN", "testkit-seed"), [vault], { encoding: "utf8" })) as {
    password: string;
    admin_token: string;
  };
  const web = join(repo, "web", "dist");
  const child: ChildProcess = spawn(binary("VAULT_SERVER_BIN", "vault-server"), ["--vault", vault, "--listen", "127.0.0.1:0", "--web", web], {
    stdio: ["ignore", "pipe", "inherit"],
  });
  const stop = () => {
    child.kill();
    rmSync(vault, { recursive: true, force: true });
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

const test = base.extend<{ admin: APIRequestContext }, { server: Server }>({
  server: [
    async ({}, use) => {
      const { server, stop } = await startServer();
      await use(server);
      stop();
    },
    { scope: "worker" },
  ],
  baseURL: async ({ server }, use) => use(server.url),
  context: async ({ context, baseURL }, use) => {
    const origin = new URL(baseURL!).origin;
    await context.route((url) => url.origin !== origin, (route) => route.abort());
    await use(context);
  },
  admin: async ({ server }, use) => {
    const ctx = await request.newContext({ baseURL: server.url, extraHTTPHeaders: { Authorization: `Bearer ${server.token}` } });
    // Every test starts with no notifications.
    expect((await ctx.post("/api/notifications/read-all")).status()).toBe(200);
    expect((await ctx.delete("/api/notifications")).status()).toBe(200);
    await use(ctx);
    await ctx.dispose();
  },
});

test.describe.configure({ mode: "serial" });

async function logIn(page: Page, server: Server, path = "/daily") {
  await page.goto(`/login?next=${encodeURIComponent(path)}`);
  await page.getByLabel("Password").fill(server.password);
  await page.getByRole("button", { name: "Log in" }).click();
  await expect(page.getByRole("navigation", { name: "Main" })).toBeVisible();
}

async function createTask(admin: APIRequestContext, title: string): Promise<{ id: string }> {
  const res = await admin.post("/api/notes?type=task", { data: { note: { fields: { title } } } });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string };
}

async function notify(admin: APIRequestContext, data: Record<string, unknown>): Promise<{ id: string }> {
  const res = await admin.post("/api/notifications", { data });
  expect(res.status(), await res.text()).toBe(201);
  return (await res.json()) as { id: string };
}

const activePane = (page: Page) => page.locator(".pane[data-active]");
const tabbarBell = (page: Page) => page.getByRole("tablist", { name: "Open tabs" }).getByRole("button", { name: /^Notifications/ });
const topbarBell = (page: Page) => page.locator("header.topbar").getByRole("button", { name: /^Notifications/ });
const panel = (page: Page) => page.getByRole("dialog", { name: "Notifications" });
const items = (page: Page) => panel(page).getByRole("listitem");
const badge = (bell: ReturnType<typeof tabbarBell>) => bell.locator(".notif-badge");

test("a notification opens its note in the active pane and is then read", async ({ page, admin, server }) => {
  const note = await createTask(admin, "Quarterly tax return");
  await notify(admin, { title: "Tax return is due Friday", body: "The accountant needs the receipts.", note: note.id });
  await logIn(page, server);
  const bell = tabbarBell(page);
  await expect(bell).toHaveAccessibleName("Notifications, 1 unread");
  await expect(badge(bell)).toHaveText("1");
  await expect(bell).toHaveAttribute("aria-expanded", "false");

  await bell.click();
  await expect(bell).toHaveAttribute("aria-expanded", "true");
  await expect(panel(page)).toBeVisible();
  await expect(items(page)).toHaveCount(1);
  const item = items(page).first();
  await expect(item).toContainText("Tax return is due Friday");
  await expect(item).toContainText("The accountant needs the receipts.");
  await expect(item).toContainText("just now");
  // Made with the admin token, so from "admin".
  await expect(item.locator(".notif-source")).toHaveText("admin");
  await expect(item.locator(".notif-icon")).toBeVisible();

  await item.getByRole("link", { name: /Tax return is due Friday/ }).click();
  await expect(panel(page)).toHaveCount(0);
  await expect(activePane(page).getByRole("heading", { name: "Quarterly tax return" })).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`/notes/${note.id}$`));
  await expect(badge(bell)).toHaveCount(0);
  await expect(bell).toHaveAccessibleName("Notifications");
  const list = await (await admin.get("/api/notifications")).json();
  expect(list.unread).toBe(0);

  // Reopened, it shows as read.
  await bell.click();
  await expect(items(page).first()).not.toHaveClass(/unread/);
  await page.keyboard.press("Escape");
  await expect(panel(page)).toHaveCount(0);
  await expect(bell).toBeFocused();
});

test("a notification with a url opens that page, and Ctrl+click opens a background tab", async ({ page, admin, server }) => {
  let res = await admin.put("/api/scripts/digest", { data: { source: "fn run() { }\n" } });
  expect(res.status(), await res.text()).toBe(200);
  res = await admin.put("/api/tasks/digest", { data: { wcl: 'label = "Morning digest"\nschedule = "0 7 * * *"\nscript = "digest"\n' } });
  expect(res.status(), await res.text()).toBe(200);
  await notify(admin, { title: "Digest settings changed", url: "/settings/scheduled/digest" });
  const note = await createTask(admin, "Background reading");
  await notify(admin, { title: "Read this later", note: note.id });
  await logIn(page, server);
  const bell = tabbarBell(page);
  await expect(badge(bell)).toHaveText("2");
  const tabs = page.getByRole("tablist", { name: "Open tabs" }).getByRole("tab");
  const before = await tabs.count();

  await bell.click();
  await items(page).filter({ hasText: "Read this later" }).getByRole("link").click({ modifiers: ["ControlOrMeta"] });
  await expect(tabs).toHaveCount(before + 1);
  // A background tab: the pane stays where it was, and the panel stays open.
  await expect(page).toHaveURL(/\/daily/);
  await expect(panel(page)).toBeVisible();
  await expect(badge(bell)).toHaveText("1");

  await items(page).filter({ hasText: "Digest settings changed" }).getByRole("link").click();
  await expect(page).toHaveURL(/\/settings\/scheduled\/digest$/);
  await expect(activePane(page).getByRole("heading", { level: 1 })).toContainText("Morning digest");
  await expect(badge(bell)).toHaveCount(0);
});

test("mark all read, dismiss, clear read and load more", async ({ page, admin, server }) => {
  for (let i = 1; i <= 23; i++) await notify(admin, { title: `Batch item ${String(i).padStart(2, "0")}` });
  await logIn(page, server);
  const bell = tabbarBell(page);
  await expect(badge(bell)).toHaveText("23");
  await bell.click();
  await expect(items(page)).toHaveCount(20);
  await expect(items(page).first()).toContainText("Batch item 23");
  await panel(page).getByRole("button", { name: "Load more" }).click();
  await expect(items(page)).toHaveCount(23);
  await expect(panel(page).getByRole("button", { name: "Load more" })).toHaveCount(0);

  // Dismiss shows on hover.
  const doomed = items(page).filter({ hasText: "Batch item 22" });
  const x = doomed.getByRole("button", { name: "Dismiss Batch item 22" });
  await expect(x).toHaveCSS("opacity", "0");
  await doomed.hover();
  await expect(x).toHaveCSS("opacity", "1");
  await x.click();
  await expect(items(page)).toHaveCount(22);
  await expect(badge(bell)).toHaveText("22");

  await panel(page).getByRole("button", { name: "Mark all read" }).click();
  await expect(badge(bell)).toHaveCount(0);
  await expect(panel(page).locator(".notif-item.unread")).toHaveCount(0);
  await expect(panel(page).getByRole("button", { name: "Mark all read" })).toBeDisabled();
  expect((await (await admin.get("/api/notifications?unread=true")).json()).items).toHaveLength(0);

  await panel(page).getByRole("button", { name: "Clear read" }).click();
  await expect(panel(page).getByText("No notifications")).toBeVisible();
  expect((await (await admin.get("/api/notifications")).json()).items).toHaveLength(0);

  // A click outside closes the panel.
  await page.mouse.click(400, 500);
  await expect(panel(page)).toHaveCount(0);
});

test("the keyboard opens the panel, moves through it and closes it", async ({ page, admin, server }) => {
  await notify(admin, { title: "Older one" });
  await notify(admin, { title: "Newer one" });
  await logIn(page, server);
  const bell = tabbarBell(page);
  await expect(badge(bell)).toHaveText("2");
  await bell.focus();
  await page.keyboard.press("Enter");
  await expect(panel(page)).toBeFocused();
  await expect(items(page)).toHaveCount(2);
  await page.keyboard.press("ArrowDown");
  await expect(panel(page).getByRole("button", { name: /Newer one/ }).first()).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(panel(page).getByRole("button", { name: /Older one/ }).first()).toBeFocused();
  // Without a note or page, Enter only marks it read.
  await page.keyboard.press("Enter");
  await expect(badge(bell)).toHaveText("1");
  await page.keyboard.press("Escape");
  await expect(panel(page)).toHaveCount(0);
  await expect(bell).toBeFocused();
});

test("on a phone the bell sits in the top bar and the panel is a full-width sheet", async ({ page, admin, server }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const note = await createTask(admin, "Pick up the parcel");
  await notify(admin, { title: "Your parcel arrived", note: note.id });
  await logIn(page, server);
  const bell = topbarBell(page);
  await expect(bell).toBeVisible();
  await expect(page.getByRole("tablist", { name: "Open tabs" }).getByRole("button", { name: /^Notifications/ })).toHaveCount(0);
  await expect(badge(bell)).toHaveText("1");
  // Just left of the right sidebar's button.
  const right = page.locator("header.topbar").getByRole("button", { name: "Right sidebar" });
  const [b, r] = [await bell.boundingBox(), await right.boundingBox()];
  expect(b!.x + b!.width).toBeLessThanOrEqual(r!.x + 1);

  await bell.click();
  const sheet = panel(page);
  await expect(sheet).toBeVisible();
  const box = await sheet.boundingBox();
  expect(Math.round(box!.width)).toBe(390);
  expect(box!.x).toBe(0);
  const top = await page.locator("header.topbar").boundingBox();
  expect(Math.round(box!.y)).toBe(Math.round(top!.y + top!.height));
  await sheet.getByRole("link", { name: /Your parcel arrived/ }).click();
  await expect(sheet).toHaveCount(0);
  await expect(activePane(page).getByRole("heading", { name: "Pick up the parcel" })).toBeVisible();
  await expect(badge(bell)).toHaveCount(0);
});

/** Make the poll every `ms` before the app loads. Test-only. */
async function pollEvery(page: Page, ms: number) {
  await page.addInitScript((ms) => {
    (window as unknown as { __THEVAULT_TEST_NOTIFICATION_POLL_MS__: number }).__THEVAULT_TEST_NOTIFICATION_POLL_MS__ = ms;
  }, ms);
}

/** Log in and wait until the live event stream is open, so nothing made afterwards is missed. */
async function logInLive(page: Page, server: Server, path = "/daily") {
  const stream = page.waitForResponse((r) => new URL(r.url()).pathname === "/api/events" && r.status() === 200);
  await logIn(page, server, path);
  await stream;
}

test("changes made elsewhere reach the bell over the live stream without waiting for the poll", async ({ page, admin, server }) => {
  // The poll would not come within the test: only the stream can bring the changes.
  await pollEvery(page, 60 * 60 * 1000);
  await logInLive(page, server);
  const bell = tabbarBell(page);
  await expect(badge(bell)).toHaveCount(0);
  const first = await notify(admin, { title: "Backup finished" });
  await expect(badge(bell)).toHaveText("1");
  await expect(page.locator(".visually-hidden[aria-live=polite]")).toHaveText("New notification: Backup finished");

  // With the panel open, a new one joins the list, and reading or dismissing one elsewhere shows.
  await bell.click();
  await expect(items(page)).toHaveCount(1);
  const second = await notify(admin, { title: "Second backup finished" });
  await expect(items(page)).toHaveCount(2);
  await expect(items(page).first()).toContainText("Second backup finished");
  await expect(badge(bell)).toHaveText("2");
  expect((await admin.post(`/api/notifications/${first.id}/read`)).status()).toBe(200);
  await expect(badge(bell)).toHaveText("1");
  expect((await admin.delete(`/api/notifications/${second.id}`)).status()).toBe(200);
  await expect(items(page)).toHaveCount(1);
  await expect(badge(bell)).toHaveCount(0);
});

test("with the live stream blocked, a notification reaches the badge on the next poll", async ({ page, admin, server }) => {
  let blocked = 0;
  await page.route("**/api/events", (route) => {
    blocked += 1;
    return route.abort();
  });
  // Test-only: poll every half second rather than every 30.
  await pollEvery(page, 500);
  await logIn(page, server);
  await expect.poll(() => blocked).toBeGreaterThan(0);
  const bell = tabbarBell(page);
  await expect(bell).toHaveAccessibleName("Notifications");
  await expect(badge(bell)).toHaveCount(0);
  await notify(admin, { title: "Backup finished" });
  await expect(badge(bell)).toHaveText("1", { timeout: 5_000 });
  await expect(page.locator(".visually-hidden[aria-live=polite]")).toHaveText("New notification: Backup finished");

  // With the panel open, a new one joins the list too.
  await bell.click();
  await expect(items(page)).toHaveCount(1);
  await notify(admin, { title: "Second backup finished" });
  await expect(items(page)).toHaveCount(2, { timeout: 5_000 });
  await expect(items(page).first()).toContainText("Second backup finished");
  await expect(badge(bell)).toHaveText("2");
});

test("an open note takes a change made elsewhere", async ({ page, admin, server }) => {
  await pollEvery(page, 60 * 60 * 1000);
  const note = await createTask(admin, "Draft agenda");
  await logInLive(page, server, `/notes/${note.id}`);
  const pane = activePane(page);
  await expect(pane.getByRole("heading", { name: "Draft agenda" })).toBeVisible();
  const stored = (await (await admin.get(`/api/notes/${note.id}`)).json()) as { version: string };
  const res = await admin.put(`/api/notes/${note.id}`, { data: { base_version: stored.version, note: { fields: { title: "Final agenda" } } } });
  expect(res.status(), await res.text()).toBe(200);
  await expect(pane.getByRole("heading", { name: "Final agenda" })).toBeVisible();
});

test("screenshots of the panel with task, API and system notifications", async ({ page, admin, server }) => {
  const dir = process.env.E2E_SHOTS;
  test.skip(!dir, "E2E_SHOTS is not set");
  mkdirSync(dir!, { recursive: true });
  const put = async (path: string, data: unknown) => {
    const res = await admin.put(path, { data });
    expect(res.status(), await res.text()).toBe(200);
  };
  const note = await createTask(admin, "Invoice 2026-031 from Acme");
  await put("/api/scripts/digest", {
    source: `use vault\n\nfn run() -> Result[(), string] {\n    vault::notify("Morning digest is ready", "3 tasks are due today and 1 is overdue. The weather looks dry until Thursday.", "${note.id}")?\n    Ok(())\n}\n`,
  });
  await put("/api/tasks/digest", { wcl: 'label = "Morning digest"\nschedule = "0 7 * * *"\nscript = "digest"\nretries = 0\n' });
  await put("/api/scripts/backup", { source: 'fn run() -> Result[(), string] { Err("could not reach the backup server: connection refused") }\n' });
  await put("/api/tasks/backup", { wcl: 'label = "Nightly backup"\nschedule = "0 2 * * *"\nscript = "backup"\nretries = 0\n' });
  await notify(admin, { title: "Imported 12 invoices", body: "Two have no category yet.", note: note.id });
  expect((await admin.post("/api/tasks/backup/run?wait=true")).status()).toBe(200);
  expect((await admin.post("/api/tasks/digest/run?wait=true")).status()).toBe(200);
  const read = await notify(admin, { title: "Weekly review moved to Monday" });
  expect((await admin.post(`/api/notifications/${read.id}/read`)).status()).toBe(200);

  for (const [name, size] of [
    ["desktop", { width: 1280, height: 800 }],
    ["phone", { width: 390, height: 844 }],
  ] as const) {
    await page.setViewportSize(size);
    if (name === "desktop") await logIn(page, server);
    else await page.goto("/daily");
    const bell = name === "phone" ? topbarBell(page) : tabbarBell(page);
    await expect(badge(bell)).toHaveText("3");
    await bell.click();
    await expect(items(page)).toHaveCount(4);
    for (const theme of ["light", "dark"]) {
      await page.evaluate((x) => (document.documentElement.dataset.theme = x), theme);
      await page.screenshot({ path: join(dir!, `notify-${name}-${theme}.png`) });
    }
    await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
    await page.keyboard.press("Escape");
  }
});
