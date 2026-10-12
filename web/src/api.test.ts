import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError, api, setUnauthorizedHandler, usingNotes } from "./api";

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

  it("deletes an asset with unlink only when asked, and reads the notes a refusal lists", async () => {
    const notes = [{ id: "N1", type: "task", title: "Covered" }];
    const fetch = respond(409, { error: "asset_in_use", by: ["notes/task/N1.wcl"], notes, message: "in use" });
    vi.stubGlobal("fetch", fetch);
    const err = await api.deleteAsset("A1").catch((e) => e);
    expect(usingNotes(err)).toEqual(notes);
    await api.deleteAsset("A1", true).catch(() => {});
    const calls = fetch.mock.calls as unknown as [string, RequestInit][];
    expect(calls.map(([url, init]) => `${init.method} ${url}`)).toEqual([
      "DELETE /api/assets/A1",
      "DELETE /api/assets/A1?unlink=true",
    ]);
    expect(usingNotes(new ApiError(400, { notes }))).toEqual([]);
    expect(usingNotes(new Error("x"))).toEqual([]);
  });
});
