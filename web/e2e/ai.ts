import { readFileSync, unlinkSync, writeFileSync } from "node:fs";
import type { APIRequestContext } from "@playwright/test";
import { expect, vaultDir } from "./fixtures";

// The one way the browser tests set up AI (accounts A12): an account, through the Settings > AI
// routes as the page uses them. Every spec shares one server, so an account is found by its name
// and brought up to date, never added twice. The default account is global-setup's, which no test
// scripts: a test that chats picks its own account at the chat's foot or names it in an agent.

export interface AccountSpec {
  name: string;
  kind: "anthropic" | "openai" | "openai_compatible" | "chatgpt";
  base_url?: string | null;
  /** The API key; left as it is when absent. */
  key?: string;
  model?: string | null;
  headers?: [string, string][];
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
  const named = async () => (await aiAccounts(admin)).accounts.find((a) => a.name.toLowerCase() === spec.name.toLowerCase());
  let there = await named();
  if (there && there.kind !== spec.kind) {
    await ok(await admin.delete(`/api/integrations/ai/accounts/${there.id}`));
    there = undefined;
  }
  let id: string;
  if (there) {
    id = there.id;
    await ok(await admin.put(`/api/integrations/ai/accounts/${id}`, { data: record }));
  } else {
    const added = await admin.post("/api/integrations/ai/accounts", { data: { kind: spec.kind, ...record } });
    const taken = added.status() === 400 && (await added.text()).includes("already an account named");
    if (!taken) id = (await ok<{ id: string }>(added)).id;
    else {
      // Another worker added it between the look and the add: bring theirs up to date.
      id = (await named())!.id;
      await ok(await admin.put(`/api/integrations/ai/accounts/${id}`, { data: record }));
    }
  }
  if (spec.key !== undefined) await ok(await admin.put(`/api/integrations/ai/accounts/${id}/key`, { data: { key: spec.key } }));
  return id;
}

/** Delete the account called `name`, if there is one. */
export async function dropAccount(admin: APIRequestContext, name: string): Promise<void> {
  const there = (await aiAccounts(admin)).accounts.find((a) => a.name.toLowerCase() === name.toLowerCase());
  if (there) await ok(await admin.delete(`/api/integrations/ai/accounts/${there.id}`));
}

/**
 * Run `fn`, which may change the default account, and put back the default there was. Only one
 * worker holds the default at a time (see `alone`).
 */
export async function withDefault<T>(admin: APIRequestContext, fn: () => Promise<T>): Promise<T> {
  return alone("default-account", async () => {
    const was = (await aiAccounts(admin)).default;
    try {
      return await fn();
    } finally {
      await ok(await admin.put("/api/integrations/ai/default", { data: { id: was } }));
    }
  });
}

/**
 * Run `fn` while no other worker runs one for `what`, a setting every spec's server shares: a
 * copy of the same test in another worker waits rather than changing it underneath. A worker that
 * died holding it, as a timed-out test's does, lets it go.
 */
export async function alone<T>(what: string, fn: () => Promise<T>): Promise<T> {
  const lock = `${vaultDir()}.${what}.lock`;
  for (;;) {
    try {
      writeFileSync(lock, String(process.pid), { flag: "wx" });
      break;
    } catch {
      if (!alive(lock)) unlinkLock(lock);
      else await new Promise((r) => setTimeout(r, 50));
    }
  }
  try {
    return await fn();
  } finally {
    unlinkLock(lock);
  }
}

/** Whether the worker that wrote `lock` still runs. */
function alive(lock: string): boolean {
  try {
    process.kill(Number(readFileSync(lock, "utf8")), 0);
    return true;
  } catch (e) {
    // EPERM means it runs as someone else; ENOENT, that the lock went meanwhile.
    return (e as NodeJS.ErrnoException).code === "EPERM";
  }
}

function unlinkLock(lock: string) {
  try {
    unlinkSync(lock);
  } catch {
    // Gone already.
  }
}
