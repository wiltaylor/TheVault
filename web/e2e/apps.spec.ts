// Get the apps: the builds the server hands out from its clients folder, on Settings > Devices and
// on the public /apps page the login page links to. The APK has a QR code of its download address;
// with nothing published, both places say how to publish.

import { createHash } from "node:crypto";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, logIn, test, vaultDir } from "./fixtures";

// Every test here changes the one clients folder the server reads.
test.describe.configure({ mode: "serial" });

const clients = () => join(vaultDir(), ".vault", "clients");

const APK = Buffer.from("PK\x03\x04 a pretend android package");
const APPIMAGE = Buffer.from("\x7fELF a pretend appimage");

/** Publish a debug APK and a release AppImage, as `vault clients publish` lays them out. */
function publish() {
  const dir = clients();
  mkdirSync(dir, { recursive: true });
  const entry = (kind: string, platform: string, arch: string, build: string, filename: string, bytes: Buffer) => {
    writeFileSync(join(dir, filename), bytes);
    return {
      platform,
      kind,
      arch,
      version: "0.4.2",
      build,
      size: bytes.length,
      sha256: createHash("sha256").update(bytes).digest("hex"),
      filename,
      published: Date.UTC(2026, 9, 16, 9, 30),
    };
  };
  const apk = entry("apk", "android", "aarch64", "debug", "TheVault_0.4.2_aarch64-debug.apk", APK);
  const image = entry("appimage", "linux", "x86_64", "release", "TheVault_0.4.2_x86_64.AppImage", APPIMAGE);
  writeFileSync(join(dir, "manifest.json"), JSON.stringify({ clients: [apk, image] }, null, 2));
  return [apk, image] as const;
}

function unpublish() {
  rmSync(clients(), { recursive: true, force: true });
}

/** Screenshots of the page in the light and dark themes, into `$E2E_SHOTS` when it is set. */
async function shots(page: Page, name: string) {
  const dir = process.env.E2E_SHOTS;
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  for (const theme of ["light", "dark"]) {
    await page.evaluate((t) => (document.documentElement.dataset.theme = t), theme);
    await page.waitForTimeout(150);
    await page.screenshot({ path: join(dir, `apps-${name}-${theme}.png`), fullPage: true });
  }
  await page.evaluate(() => (document.documentElement.dataset.theme = "light"));
}

test.afterAll(unpublish);

test("with nothing published, both places explain how to publish", async ({ page }) => {
  unpublish();
  await page.goto("/apps");
  await expect(page.getByRole("heading", { name: "Get the apps" })).toBeVisible();
  await expect(page.getByText("This server has no app builds to hand out yet.")).toBeVisible();
  await expect(page.getByText("just publish-clients <dir>")).toBeVisible();
  await shots(page, "empty-public");

  await logIn(page, "/settings/devices");
  const section = page.getByRole("region", { name: "Get the apps" });
  await expect(section.getByText("This server has no app builds to hand out yet.")).toBeVisible();
});

test("Settings > Devices offers each build with its download, checksum and the APK's QR code", async ({ page }) => {
  const [apk, image] = publish();
  await logIn(page, "/settings/devices");
  const section = page.getByRole("region", { name: "Get the apps" });

  const android = section.getByRole("article", { name: "Android" });
  await expect(android).toContainText("0.4.2");
  await expect(android.locator(".client-badge")).toHaveText("debug");
  await expect(android.getByRole("img", { name: "QR code of the 0.4.2 APK download" })).toBeVisible();
  const origin = new URL(page.url()).origin;
  await expect(android.locator("figcaption")).toHaveText(`${origin}/api/clients/${apk.filename}`);
  await expect(android.getByLabel(`SHA-256 of ${apk.filename}`)).toHaveText(apk.sha256);
  const download = android.getByRole("link", { name: "Download APK" });
  await expect(download).toHaveAttribute("href", `/api/clients/${apk.filename}`);

  const linux = section.getByRole("article", { name: "Linux desktop" });
  await expect(linux.locator(".client-badge")).toHaveText("release");
  await expect(linux.getByRole("img")).toHaveCount(0);
  await expect(linux.getByLabel(`SHA-256 of ${image.filename}`)).toHaveText(image.sha256);

  // The link downloads the file, under its own name.
  const [file] = await Promise.all([page.waitForEvent("download"), linux.getByRole("link", { name: "Download AppImage" }).click()]);
  expect(file.suggestedFilename()).toBe(image.filename);

  // The pairing section is still below.
  await expect(page.getByRole("button", { name: "Pair a device" })).toBeVisible();
  await section.scrollIntoViewIfNeeded();
  await shots(page, "settings-devices");
});

test("the login page links to the public /apps page, which works without signing in", async ({ page, request }) => {
  const [apk] = publish();
  await page.goto("/login");
  await page.getByRole("link", { name: "Get the apps" }).click();
  await expect(page).toHaveURL(/\/apps$/);
  await expect(page.getByRole("heading", { name: "Get the apps", level: 1 })).toBeVisible();
  const android = page.getByRole("article", { name: "Android" });
  await expect(android.getByRole("img", { name: "QR code of the 0.4.2 APK download" })).toBeVisible();
  await expect(page.getByRole("article", { name: "Linux desktop" })).toContainText("AppImage");
  await expect(page.getByRole("link", { name: "Log in" })).toBeVisible();
  // Nothing sent the page back to the login form.
  await page.waitForTimeout(500);
  await expect(page).toHaveURL(/\/apps$/);
  await shots(page, "public");

  // The download needs no session either.
  const res = await request.get(`/api/clients/${apk.filename}`);
  expect(res.status()).toBe(200);
  expect(res.headers()["content-type"]).toBe("application/vnd.android.package-archive");
  expect(Buffer.from(await res.body()).equals(APK)).toBe(true);
});
