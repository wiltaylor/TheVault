import { defineConfig, devices } from "@playwright/test";

// The browsers come from the dev image, which sets PLAYWRIGHT_BROWSERS_PATH=/opt/playwright.
// e2e/global-setup.ts serves web/dist on a free port and exports its URL as E2E_BASE_URL.
export default defineConfig({
  testDir: "./e2e",
  globalSetup: "./e2e/global-setup.ts",
  forbidOnly: !!process.env.CI,
  reporter: "list",
  use: {
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
