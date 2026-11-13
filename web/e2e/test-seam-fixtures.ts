import { request } from "@playwright/test";
import { test as shared } from "./fixtures";
import { test as isolated } from "./isolated-fixtures";

// Fixture writes to reserved names require an owner session, never an admin bearer.
const owner = async ({}, use: (ctx: import("@playwright/test").APIRequestContext) => Promise<void>) => {
  const ctx = await request.newContext({ baseURL: process.env.E2E_BASE_URL });
  try {
    const login = await ctx.post("/api/login", { data: { password: process.env.E2E_PASSWORD } });
    if (!login.ok()) throw new Error(`owner fixture login failed: ${login.status()}`);
    await use(ctx);
  } finally {
    await ctx.dispose();
  }
};

export const test = shared.extend({ admin: owner });
export const isolatedTest = isolated.extend({ admin: owner });
export { expect, logIn } from "./fixtures";
