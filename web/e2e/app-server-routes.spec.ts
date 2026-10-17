import { type Page, request } from "@playwright/test";
import { activePane, expect, logIn, password, test } from "./fixtures";

// The web client inside the desktop or Android app, whose embedded server sends Settings >
// Devices, the notifications and the history to the home server (D2, D18). The tests stand in
// for the embedded server with routes: `GET /api/app` says which device the page runs in, and a
// 503 says the home server cannot be reached or the app is not paired.

function unique(): string {
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
}

async function pair(name: string): Promise<string> {
  const ctx = await request.newContext({ baseURL: process.env.E2E_BASE_URL });
  const res = await ctx.post("/api/devices/pair", { data: { password: password(), name, platform: "linux" } });
  expect(res.status(), await res.text()).toBe(201);
  const id = ((await res.json()) as { device_id: string }).device_id;
  await ctx.dispose();
  return id;
}

/** Make the page an app's, paired as device `id`, built as `build`. */
async function inApp(
  page: Page,
  id: string | null,
  build: { version: string; platform: string; build: string } = { version: "0.1.3", platform: "linux-appimage", build: "release" },
) {
  await page.route("**/api/app", (route) =>
    route.fulfill({
      json: {
        app: true,
        paired: id !== null,
        device_id: id,
        device_name: id && "This laptop",
        server_url: id && "https://home",
        ...build,
      },
    }),
  );
}

async function unavailable(page: Page, path: string, error: "offline" | "not_paired") {
  const message = error === "offline" ? "Needs a connection to your server" : "Not paired";
  await page.route(path, (route) => route.fulfill({ status: 503, json: { error, message } }));
}

test("in an app, Settings > Devices marks this device and leaves changes to the web app", async ({ page }) => {
  const name = `Laptop ${unique()}`;
  const id = await pair(name);
  await inApp(page, id);
  await logIn(page, "/settings/devices");

  const card = page.locator(`[data-device="${id}"]`);
  await expect(card.getByRole("heading", { name })).toContainText("This device");
  await expect(card.getByLabel("Name")).toBeDisabled();
  await expect(card.getByRole("button", { name: "Save" })).toHaveCount(0);
  await expect(card.getByRole("button", { name: "Revoke" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Pair a device" })).toHaveCount(0);
  await expect(page.getByRole("note")).toContainText("in the web app");

  // Credentials need admin, which an app's device token never has.
  const tabs = page.getByRole("navigation", { name: "Settings sections" });
  await expect(tabs.getByRole("link", { name: "Devices" })).toBeVisible();
  await expect(tabs.getByRole("link", { name: "Secrets" })).toHaveCount(0);
  await expect(tabs.getByRole("link", { name: "API tokens" })).toHaveCount(0);
  await page.goto("/settings/tokens");
  await expect(page.getByRole("status")).toContainText("Manage API tokens in the web app");
});

test("in an app offline, the pages backed by the home server say they need it", async ({ page, admin }) => {
  await inApp(page, "01JZZZZZZZZZZZZZZZZZZZZZZZ");
  await unavailable(page, "**/api/devices", "offline");
  await unavailable(page, "**/api/clients", "offline");
  await unavailable(page, "**/api/notifications?**", "offline");
  await logIn(page, "/settings/devices");
  await expect(page.getByRole("alert")).toHaveText("Needs a connection to your server");
  await expect(page.getByText("No devices are paired.")).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Get the apps" })).toHaveCount(0);

  // The bell says so too.
  await page.getByRole("tablist", { name: "Open tabs" }).getByRole("button", { name: /^Notifications/ }).click();
  await expect(page.getByRole("dialog", { name: "Notifications" })).toContainText("Needs a connection to your server");
  await page.keyboard.press("Escape");

  // The history is the app's own, and says so.
  const created = await admin.post("/api/notes?type=note", { data: { note: { fields: { title: `Offline ${unique()}` }, blocks: [] } } });
  expect(created.status(), await created.text()).toBe(201);
  const note = (await created.json()) as { id: string };
  await page.route(`**/api/notes/${note.id}/history`, async (route) => {
    const res = await route.fetch();
    const body = (await res.json()) as Record<string, unknown>;
    await route.fulfill({ response: res, json: { ...body, source: "device" } });
  });
  await page.goto(`/notes/${note.id}`);
  await activePane(page).getByRole("button", { name: "History", exact: true }).click();
  await expect(page.getByRole("complementary", { name: "History" }).getByRole("note")).toHaveText(
    "Showing only the versions this device saved. Connect to your server for the full history.",
  );
});

test("in an app that is not paired, Settings > Devices says so", async ({ page }) => {
  await inApp(page, null);
  await unavailable(page, "**/api/devices", "not_paired");
  await logIn(page, "/settings/devices");
  await expect(page.getByRole("alert")).toHaveText("Not paired");
});

test("in the desktop app, Settings > General's About shows the app, the home server and the update check", async ({
  page,
  admin,
}) => {
  const server = ((await (await admin.get("/api/version")).json()) as { server_version: string }).server_version;
  await inApp(page, "01JZZZZZZZZZZZZZZZZZZZZZZZ");
  let checked = false;
  await page.route("**/_desktop/status", (route) =>
    route.fulfill({
      json: {
        update: checked ? { version: "0.1.4", ready: true, url: "x", source: "server" } : null,
        updater: {
          install: "appimage",
          checks: checked ? 2 : 1,
          ready: checked ? "0.1.4" : null,
          error: null,
          checking: false,
          last_checked: checked ? 2_000 : 1_000,
          available: true,
        },
        settings: { update_check: true },
      },
    }),
  );
  await page.route("**/_desktop/update/check", (route) => {
    checked = true;
    return route.fulfill({ status: 202, json: {} });
  });
  await logIn(page, "/settings");
  const about = page.getByRole("group", { name: "About" });
  await expect(about.locator('[data-about="app"]')).toHaveText("TheVault 0.1.3");
  await expect(about.locator('[data-about="platform"]')).toHaveText("Linux AppImage");
  await expect(about.locator('[data-about="build"]')).toHaveText("release");
  await expect(about.locator('[data-about="device"]')).toHaveText("This laptop");
  await expect(about.locator('[data-about="server"]')).toHaveText(server);
  await expect(about.locator('[data-about="updates"]')).toHaveText("Up to date");
  await about.getByRole("button", { name: "Check for updates" }).click();
  await expect(about.locator('[data-about="updates"]')).toHaveText("0.1.4 ready — restart to update");
  await expect(about.getByRole("button", { name: "Restart to update" })).toBeVisible();
});

test("in the Android app offline, About says the home server is unreachable", async ({ page }) => {
  await inApp(page, "01JZZZZZZZZZZZZZZZZZZZZZZZ", { version: "0.1.3", platform: "android", build: "debug" });
  await unavailable(page, "**/api/version", "offline");
  await logIn(page, "/settings");
  const about = page.getByRole("group", { name: "About" });
  await expect(about.locator('[data-about="app"]')).toHaveText("TheVault 0.1.3");
  await expect(about.locator('[data-about="platform"]')).toHaveText("Android");
  await expect(about.locator('[data-about="build"]')).toHaveText("debug");
  await expect(about.locator('[data-about="server"]')).toHaveText("unreachable");
});
