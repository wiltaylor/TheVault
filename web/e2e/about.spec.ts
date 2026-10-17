import { expect, logIn, test } from "./fixtures";

// Settings > General's About section in a browser: the web app, the server's version and the web
// build baked in at build time. The in-app variants are in app-server-routes.spec.ts.

test("in a browser, Settings > General's About shows the web app, the server's version and the web build", async ({
  page,
  admin,
}) => {
  const res = await admin.get("/api/version");
  expect(res.status()).toBe(200);
  const server = ((await res.json()) as { server_version: string }).server_version;
  expect(server).toMatch(/^\d+\.\d+\.\d+/);

  await logIn(page, "/settings");
  const about = page.getByRole("group", { name: "About" });
  // It comes first, above the map tiles.
  await expect(page.locator(".settings > .settings-form").first()).toHaveAttribute("aria-label", "About");
  await expect(about.locator('[data-about="app"]')).toHaveText("Web app");
  await expect(about.locator('[data-about="server"]')).toHaveText(server);
  // The web build and the server share the workspace version.
  await expect(about.locator('[data-about="web"]')).toHaveText(new RegExp(`^${server.replace(/\./g, "\\.")}( \\([0-9a-f]+\\))?$`));
  // Only the apps have an update check, platform or device.
  await expect(about.getByRole("group", { name: "Updates" })).toHaveCount(0);
  await expect(about.locator('[data-about="platform"]')).toHaveCount(0);
  // Not in the sidebar.
  await expect(page.getByRole("navigation", { name: "Main" })).not.toContainText(server);
});
