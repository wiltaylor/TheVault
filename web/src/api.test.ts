import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError, api, setUnauthorizedHandler } from "./api";

function respond(status: number, body: unknown) {
  return vi.fn(async () => new Response(JSON.stringify(body), { status }));
}

afterEach(() => {
  vi.unstubAllGlobals();
  setUnauthorizedHandler(() => {});
});

describe("api", () => {
  it("sends the base version with a save and reports a 409 as a conflict", async () => {
    const fetch = respond(409, { error: "conflict", current_version: "v2", message: "changed" });
    vi.stubGlobal("fetch", fetch);
    const err = await api.updateNote("N1", { fields: { title: "x" }, blocks: [] }, "v1").catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.isConflict).toBe(true);
    const [url, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("/api/notes/N1");
    expect(init.method).toBe("PUT");
    expect(JSON.parse(init.body as string)).toEqual({
      note: { fields: { title: "x" }, blocks: [] },
      base_version: "v1",
    });
  });

  it("calls the unauthorized handler on a 401, except for a failed login", async () => {
    const handler = vi.fn();
    setUnauthorizedHandler(handler);
    vi.stubGlobal("fetch", respond(401, { error: "unauthorized" }));
    await expect(api.login("wrong")).rejects.toMatchObject({ status: 401 });
    expect(handler).not.toHaveBeenCalled();
    await expect(api.types()).rejects.toMatchObject({ status: 401 });
    expect(handler).toHaveBeenCalledOnce();
  });
});
