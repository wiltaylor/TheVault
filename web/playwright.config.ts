import { defineConfig, devices } from "@playwright/test";

// The browsers come from the dev image, which sets PLAYWRIGHT_BROWSERS_PATH=/opt/playwright.
// e2e/global-setup.ts starts vault-server on a seeded temporary vault and a free port, serving
// web/dist, and exports its URL and credentials as E2E_BASE_URL, E2E_PASSWORD and E2E_ADMIN_TOKEN.
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
