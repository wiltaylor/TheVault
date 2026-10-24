// Ticket 79: the Android app draws edge to edge, so the page pads itself clear of the status bar
// and the gesture bar by the insets the app's VaultSystemBars bridge hands it. Here a stand-in
// bridge reports a 24px status bar and a 16px gesture bar on a phone viewport.
//
// E2E_SHOTS_DIR, when set, is where the light and dark screenshots of the padded shell go.

import path from "node:path";
import type { Page } from "@playwright/test";
import { expect, logIn, test } from "./fixtures";

const TOP = 24;
const BOTTOM = 16;
const HEIGHT = 844;

/** Installs a stand-in for the Android app's bridge before the page's scripts run. */
async function bridge(page: Page, theme: "light" | "dark") {
  await page.addInitScript(
    ({ top, bottom, theme }) => {
      localStorage.setItem("thevault.theme", theme);
      const w = window as unknown as { VaultSystemBars: unknown; darkCalls: boolean[] };
      w.darkCalls = [];
      w.VaultSystemBars = {
        insets: () => JSON.stringify({ top, right: 0, bottom, left: 0 }),
        setDark: (dark: boolean) => w.darkCalls.push(dark),
      };
    },
    { top: TOP, bottom: BOTTOM, theme },
  );
}

async function box(page: Page, selector: string) {
  const b = await page.locator(selector).first().boundingBox();
  expect(b, selector).not.toBeNull();
  return b!;
}

async function shot(page: Page, name: string) {
  const dir = process.env.E2E_SHOTS_DIR;
  if (dir) await page.screenshot({ path: path.join(dir, name) });
}

test.describe("on a phone drawn edge to edge", () => {
  test.use({ viewport: { width: 390, height: HEIGHT }, hasTouch: true, isMobile: true });

  for (const theme of ["light", "dark"] as const) {
    test(`the top bar and a toast sit inside the safe area (${theme})`, async ({ page }) => {
      await bridge(page, theme);
      await logIn(page, "/daily");

      // The status bar's icons follow the page's theme.
      expect(await page.evaluate(() => (window as unknown as { darkCalls: boolean[] }).darkCalls.at(-1))).toBe(
        theme === "dark",
      );

      const bar = await box(page, ".topbar");
      expect(bar.y).toBeGreaterThanOrEqual(TOP);
      await shot(page, `edge-${theme}.png`);

      // The left drawer opens under the top bar, not under the status bar.
      await page.locator(".topbar .menu-button").first().click();
      const drawer = await box(page, ".sidebar");
      expect(drawer.y).toBeGreaterThanOrEqual(TOP + bar.height - 1);
      await shot(page, `edge-${theme}-drawer.png`);
      await page.locator(".topbar .menu-button").first().click();
      await expect(page.locator(".menu-open")).toHaveCount(0);

      // A capture's toast stays above the gesture bar.
      await page.locator(".topbar .menu-button").first().click();
      await page.getByRole("navigation", { name: "Main" }).getByRole("button", { name: "Capture" }).click();
      const dialog = page.getByRole("dialog", { name: "Capture" });
      const sheet = await box(page, ".capture-dialog");
      expect(sheet.y).toBeGreaterThanOrEqual(TOP);
      await dialog.getByRole("textbox", { name: "Capture text" }).fill(`Edge to edge ${theme} ${Date.now()}`);
      await dialog.getByRole("button", { name: "Save" }).click();
      const toast = page.locator(".toast").filter({ hasText: "Saved to inbox" });
      await expect(toast).toBeVisible();
      const t = await toast.boundingBox();
      expect(t!.y + t!.height).toBeLessThanOrEqual(HEIGHT - BOTTOM);
      await shot(page, `edge-${theme}-toast.png`);
    });
  }

  test("a browser without the app's bridge keeps its insets at 0", async ({ page }) => {
    await logIn(page, "/daily");
    const bar = await box(page, ".topbar");
    expect(bar.y).toBe(0);
  });
});
