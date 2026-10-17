import type { Page } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

// The sync icon beside Archive in the sidebar footer, in the desktop and Android apps (D18, D19,
// D20, D27). The tests stand in for the embedded server with routes: `GET /api/app` says the page
// runs in an app, `GET /api/app/sync` where its sync stands, `POST /api/app/sync` runs a round.

type Sync = {
  state: "synced" | "syncing" | "error";
  phase: string;
  queued: number;
  last_ok_ms: number | null;
  last_error: string | null;
  server: string | null;
};

const SYNCED: Sync = {
  state: "synced",
  phase: "synced",
  queued: 0,
  last_ok_ms: null,
  last_error: null,
  server: "https://home.example",
};

/** Make the page an app's, its sync answering what `current()` says; returns the POSTs seen. */
async function inApp(page: Page, current: () => Sync) {
  const asked = { now: 0 };
  await page.route("**/api/app", (route) =>
    route.fulfill({
      json: {
        app: true,
        paired: true,
        device_id: "01JZZZZZZZZZZZZZZZZZZZZZZZ",
        device_name: "This laptop",
        server_url: "https://home.example",
        version: "0.1.4",
        platform: "linux-appimage",
        build: "release",
      },
    }),
  );
  await page.route("**/api/app/sync", (route) => {
    if (route.request().method() === "POST") asked.now++;
    return route.fulfill({ status: route.request().method() === "POST" ? 202 : 200, json: current() });
  });
  return asked;
}

const footer = (page: Page) => page.locator(".sidebar-footer");
const icon = (page: Page) => footer(page).locator(".sync-button");

test("in an app, the sync icon sits beside Archive and turns green, yellow and red", async ({ page }) => {
  let sync: Sync = { ...SYNCED, last_ok_ms: Date.now() - 2 * 60_000 };
  await inApp(page, () => sync);
  await logIn(page, "/");

  // Green, beside Archive, its name and tooltip saying when it last synced.
  const button = footer(page).getByRole("button", { name: "Synced 2 min ago" });
  await expect(button).toBeVisible();
  await expect(button).toHaveAttribute("title", "Synced 2 min ago");
  await expect(button).toHaveAttribute("data-sync-state", "synced");
  const order = await footer(page)
    .locator(":scope > *")
    .evaluateAll((els) => els.map((e) => e.getAttribute("aria-label")));
  expect(order.indexOf("Archive")).toBe(order.indexOf("Synced 2 min ago") + 1);

  // Yellow while changes wait.
  sync = { ...sync, state: "syncing", queued: 3 };
  await expect(icon(page)).toHaveAttribute("data-sync-state", "syncing");
  await expect(icon(page)).toHaveAccessibleName("Syncing — 3 changes waiting");
  await expect(icon(page)).toHaveAttribute("title", "Syncing — 3 changes waiting");

  // Red when the server cannot be reached.
  sync = { ...sync, state: "error", phase: "offline", queued: 0, last_error: "cannot reach the server: connection refused" };
  await expect(icon(page)).toHaveAttribute("data-sync-state", "error");
  await expect(icon(page)).toHaveAccessibleName("Sync failed: cannot reach the server — retrying");

  // The colours differ: green, yellow and red are not the same tint.
  const colours = new Set<string>();
  for (const state of ["synced", "syncing", "error"] as const) {
    sync = { ...SYNCED, state, phase: state === "error" ? "error" : state, last_error: state === "error" ? "boom" : null };
    await expect(icon(page)).toHaveAttribute("data-sync-state", state);
    colours.add(await icon(page).evaluate((el) => getComputedStyle(el).color));
  }
  expect(colours.size).toBe(3);

  // No sync toast: the old floating badge is gone, and no toast says anything about sync.
  await expect(page.locator("#thevault-sync")).toHaveCount(0);
  await expect(page.locator(".toast")).toHaveCount(0);
});

test("the sync popover shows the details and Sync now runs a round", async ({ page }) => {
  const at = Date.now() - 5 * 60_000;
  let sync: Sync = {
    state: "error",
    phase: "offline",
    queued: 2,
    last_ok_ms: at,
    last_error: "cannot reach the server: connection refused",
    server: "https://home.example",
  };
  const asked = await inApp(page, () => sync);
  await logIn(page, "/");

  await icon(page).click();
  const panel = page.getByRole("dialog", { name: "Sync" });
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("Sync failed: cannot reach the server — retrying · 2 changes waiting");
  await expect(panel.locator("dt", { hasText: "Last synced" }).locator("+ dd")).toHaveText("5 min ago");
  await expect(panel.locator("dt", { hasText: "Waiting" }).locator("+ dd")).toHaveText("2 changes");
  await expect(panel.locator("dt", { hasText: "Server" }).locator("+ dd")).toHaveText("https://home.example");
  await expect(panel.locator("dt", { hasText: "Last error" }).locator("+ dd")).toHaveText(
    "cannot reach the server: connection refused",
  );

  sync = { ...SYNCED, last_ok_ms: Date.now() };
  await panel.getByRole("button", { name: "Sync now" }).click();
  await expect.poll(() => asked.now).toBe(1);
  await expect(panel).toContainText("Synced just now");
  await expect(panel.locator("dt", { hasText: "Last error" })).toHaveCount(0);
  await expect(icon(page)).toHaveAttribute("data-sync-state", "synced");

  // Escape closes it and gives the focus back.
  await page.keyboard.press("Escape");
  await expect(panel).toHaveCount(0);
  await expect(icon(page)).toBeFocused();
});

test("an app that needs updating says so once, as well as the red icon", async ({ page }) => {
  await inApp(page, () => ({
    ...SYNCED,
    state: "error",
    phase: "update_required",
    last_error: "Update the app to keep syncing.",
  }));
  await logIn(page, "/");
  await expect(icon(page)).toHaveAccessibleName("Sync stopped: update the app to keep syncing");
  const toast = page.locator(".toast");
  await expect(toast).toHaveCount(1);
  await expect(toast).toContainText("needs an update to keep syncing");
  await expect(toast.getByRole("link", { name: "Check for updates" })).toHaveAttribute("href", "/settings");
  // Polls go on; the notice is not shown again.
  await page.waitForTimeout(3500);
  await expect(page.locator(".toast")).toHaveCount(1);
});

test("a local write asks for the sync status at once", async ({ page }) => {
  const polls: number[] = [];
  await inApp(page, () => {
    polls.push(Date.now());
    return SYNCED;
  });
  await logIn(page, "/");
  await expect(icon(page)).toBeVisible();
  await page.getByRole("button", { name: "Quick capture" }).click();
  await page.getByLabel("Capture text").fill("A thought");
  const saved = page.waitForResponse((r) => r.request().method() === "POST" && r.url().includes("/api/notes"));
  await page.keyboard.press("Control+Enter");
  await saved;
  const at = Date.now();
  // Twice at once: one ask may have left before the write. The poll alone asks every 3 s.
  await expect.poll(() => polls.filter((t) => t >= at - 100).length, { timeout: 1000 }).toBeGreaterThanOrEqual(2);
});

test("in a browser, there is no sync icon", async ({ page }) => {
  let asked = 0;
  await page.route("**/api/app/sync", (route) => {
    asked++;
    return route.fallback();
  });
  await logIn(page, "/");
  await expect(footer(page).getByRole("link", { name: "Archive" })).toBeVisible();
  await expect(icon(page)).toHaveCount(0);
  await page.waitForTimeout(500);
  expect(asked).toBe(0);
});

// Screenshots for review, written only when SYNC_SHOTS names a folder.
const shots = process.env.SYNC_SHOTS;
test.describe("screenshots", () => {
  test.skip(!shots, "SYNC_SHOTS is not set");

  for (const theme of ["light", "dark"] as const) {
    test(`the footer and popover in each state, ${theme}`, async ({ page }) => {
      await page.addInitScript((t) => localStorage.setItem("thevault.theme", t), theme);
      await page.emulateMedia({ reducedMotion: "reduce" });
      let sync: Sync = { ...SYNCED, last_ok_ms: Date.now() - 2 * 60_000 };
      await inApp(page, () => sync);
      await page.setViewportSize({ width: 1280, height: 800 });
      await logIn(page, "/");
      const states: [string, Sync][] = [
        ["synced", sync],
        ["syncing", { ...sync, state: "syncing", queued: 3 }],
        [
          "error",
          { ...sync, state: "error", phase: "offline", queued: 1, last_error: "cannot reach the server: connection refused" },
        ],
      ];
      for (const [name, s] of states) {
        sync = s;
        await expect(icon(page)).toHaveAttribute("data-sync-state", s.state);
        await footer(page).screenshot({ path: `${shots}/syncicon-${name}-${theme}.png` });
      }
      await icon(page).click();
      const panel = page.getByRole("dialog", { name: "Sync" });
      await expect(panel).toBeVisible();
      const box = (await panel.boundingBox())!;
      const foot = (await footer(page).boundingBox())!;
      await page.screenshot({
        path: `${shots}/syncicon-popover-${theme}.png`,
        clip: { x: 0, y: box.y - 10, width: Math.max(box.x + box.width, foot.width) + 20, height: foot.y + foot.height - box.y + 20 },
      });
      await page.keyboard.press("Escape");

      // The phone drawer.
      await page.setViewportSize({ width: 390, height: 844 });
      sync = states[1]![1];
      await page.getByRole("button", { name: "Menu" }).click();
      await expect(icon(page)).toHaveAttribute("data-sync-state", "syncing");
      await page.screenshot({ path: `${shots}/syncicon-phone-${theme}.png` });
    });
  }
});
