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

  it("in the Android app, sends bodies over IPC and leaves bodiless requests on fetch", async () => {
    const fetch = respond(200, { types: [] });
    vi.stubGlobal("fetch", fetch);
    const uploads: string[] = [];
    const invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "api_upload_start") return 7;
      if (cmd === "api_upload_append") {
        uploads.push(String(args?.chunk));
        return null;
      }
      const req = args?.request as { path: string };
      if (req.path.startsWith("/api/assets")) {
        return { status: 201, headers: [["content-type", "application/json"]], body: '{"id":"A1"}' };
      }
      if (req.path.startsWith("/api/types/")) return { status: 204, headers: [], body: "" };
      return { status: 400, headers: [], body: '{"error":"bad_request","message":"nope"}' };
    });
    vi.stubGlobal("__TAURI_INTERNALS__", { invoke });
    vi.stubGlobal("__thevaultApiOverIpc", true);

    await api.types();
    expect(fetch).toHaveBeenCalledOnce();
    expect(invoke).not.toHaveBeenCalled();

    const err = await api.createNote("note", { fields: { title: "probe" }, blocks: [] }).catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(400);
    expect(err.message).toBe("nope");
    const [cmd, args] = invoke.mock.calls[0]!;
    expect(cmd).toBe("api_request");
    const sent = args?.request as { method: string; path: string; headers: [string, string][]; text: string };
    expect(sent.method).toBe("POST");
    expect(sent.path).toBe("/api/notes?type=note");
    expect(sent.headers).toContainEqual(["Content-Type", "application/json"]);
    expect(JSON.parse(sent.text)).toEqual({ note: { fields: { title: "probe" }, blocks: [] } });

    // Bytes go in base64 chunks of 1 MiB, then the request names the upload.
    const bytes = new Uint8Array(1024 * 1024 + 3).map((_, i) => i % 256);
    const asset = await api.uploadAsset(new File([bytes], "a b.bin"));
    expect(asset).toEqual({ id: "A1" });
    expect(uploads).toHaveLength(2);
    const [first, second] = uploads.map((c) => Uint8Array.from(atob(c), (ch) => ch.charCodeAt(0))) as [Uint8Array, Uint8Array];
    expect(first.length + second.length).toBe(bytes.length);
    expect([...first, ...second]).toEqual([...bytes]);
    const upload = invoke.mock.calls.at(-1)?.[1]?.request as { path: string; upload: number; text?: string };
    expect(upload.path).toBe("/api/assets?filename=a%20b.bin");
    expect(upload.upload).toBe(7);
    expect(upload.text).toBeUndefined();

    // A 204 answer has no body.
    await expect(api.putType("task", "label = \"Task\"")).resolves.toBeUndefined();
    expect(fetch).toHaveBeenCalledOnce();
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
