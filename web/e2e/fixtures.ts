import { test as base } from "@playwright/test";

export const test = base.extend({
  baseURL: async ({}, use) => {
    const url = process.env.E2E_BASE_URL;
    if (!url) {
      throw new Error("E2E_BASE_URL is not set; global setup did not run");
    }
    await use(url);
  },
});

export { expect } from "@playwright/test";
