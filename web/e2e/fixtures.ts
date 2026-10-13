import { type APIRequestContext, type Page, test as base, expect, request } from "@playwright/test";

function env(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`${name} is not set; global setup did not run`);
  }
  return value;
}

export const password = () => env("E2E_PASSWORD");

/** The vault folder the server runs on. */
export const vaultDir = () => env("E2E_VAULT");

export const test = base.extend<{ admin: APIRequestContext }>({
  baseURL: async ({}, use) => {
    await use(env("E2E_BASE_URL"));
  },
  // No test reaches the internet: a request to any other host fails unless the test answers it
  // with a route of its own, as the map tests do with their tile stand-in.
  context: async ({ context, baseURL }, use) => {
    const origin = new URL(baseURL!).origin;
    await context.route((url) => url.origin !== origin, (route) => route.abort());
    await use(context);
  },
  // The API as an agent with the admin token sees it, for writes the UI does not make.
  admin: async ({}, use) => {
    const ctx = await request.newContext({
      baseURL: env("E2E_BASE_URL"),
      extraHTTPHeaders: { Authorization: `Bearer ${env("E2E_ADMIN_TOKEN")}` },
    });
    await use(ctx);
    await ctx.dispose();
  },
});

/** Log in through the login page and wait for the app. */
export async function logIn(page: Page, path = "/") {
  await page.goto(`/login?next=${encodeURIComponent(path)}`);
  await page.getByLabel("Password").fill(password());
  await page.getByRole("button", { name: "Log in" }).click();
  await expect(page.getByRole("navigation", { name: "Main" })).toBeVisible();
}

export { expect } from "@playwright/test";
