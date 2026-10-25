import type { APIRequestContext } from "@playwright/test";
import { expect } from "./fixtures";

// The one way the browser tests set up AI (accounts A12): an account, through the Settings > AI
// routes as the page uses them. Every spec shares one server, so an account is found by its name
// and brought up to date, never added twice.

export interface AccountSpec {
  name: string;
  kind: "anthropic" | "openai" | "openai_compatible" | "chatgpt";
  base_url?: string | null;
  /** The API key; left as it is when absent. */
  key?: string;
  model?: string | null;
  headers?: [string, string][];
  /** Make it the default account. */
  default?: boolean;
}

interface Status {
  accounts: { id: string; name: string; kind: string }[];
  default: string | null;
}

async function ok<T>(res: Awaited<ReturnType<APIRequestContext["get"]>>): Promise<T> {
  const text = await res.text();
  expect(res.ok(), `${res.url()}: ${text}`).toBe(true);
  return (text ? JSON.parse(text) : null) as T;
}

/** The accounts and the default, as Settings > AI reads them. */
export async function aiAccounts(admin: APIRequestContext): Promise<Status> {
  return ok<Status>(await admin.get("/api/integrations/ai"));
}

/** Add the account called `spec.name`, or change the one there is to match; its id. */
export async function useAccount(admin: APIRequestContext, spec: AccountSpec): Promise<string> {
  const record = {
    name: spec.name,
    base_url: spec.base_url ?? null,
    model: spec.model ?? null,
    headers: spec.headers ?? [],
    context: null,
  };
  const there = (await aiAccounts(admin)).accounts.find((a) => a.name.toLowerCase() === spec.name.toLowerCase());
  let id: string;
  if (there && there.kind === spec.kind) {
    id = there.id;
    await ok(await admin.put(`/api/integrations/ai/accounts/${id}`, { data: record }));
  } else {
    if (there) await ok(await admin.delete(`/api/integrations/ai/accounts/${there.id}`));
    id = (await ok<{ id: string }>(await admin.post("/api/integrations/ai/accounts", { data: { kind: spec.kind, ...record } }))).id;
  }
  if (spec.key !== undefined) await ok(await admin.put(`/api/integrations/ai/accounts/${id}/key`, { data: { key: spec.key } }));
  if (spec.default) await ok(await admin.put("/api/integrations/ai/default", { data: { id } }));
  return id;
}

/** Delete the account called `name`, if there is one. */
export async function dropAccount(admin: APIRequestContext, name: string): Promise<void> {
  const there = (await aiAccounts(admin)).accounts.find((a) => a.name.toLowerCase() === name.toLowerCase());
  if (there) await ok(await admin.delete(`/api/integrations/ai/accounts/${there.id}`));
}
