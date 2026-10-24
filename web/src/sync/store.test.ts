import { afterEach, describe, expect, it, vi } from "vitest";

// In an app the page has no live stream, so the sync status's count of definition changes is how
// it learns a round pulled a type, agent or skill, and reads its lists again.

const appSync = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, appSync } };
});
const definitionsChanged = vi.fn(() => Promise.resolve());
vi.mock("../definitions", () => ({ definitionsChanged }));

const { refreshSync, resetSync } = await import("./store");

afterEach(() => {
  resetSync();
  vi.clearAllMocks();
});

const answer = (definitions?: number) => ({
  state: "synced",
  phase: "synced",
  queued: 0,
  last_ok_ms: 1,
  last_error: null,
  ...(definitions === undefined ? {} : { definitions }),
});

describe("the app's sync status", () => {
  it("reads the definitions again when a round pulled one, and not before", async () => {
    appSync.mockResolvedValueOnce(answer(4));
    await refreshSync();
    expect(definitionsChanged).not.toHaveBeenCalled();

    appSync.mockResolvedValueOnce(answer(4));
    await refreshSync();
    expect(definitionsChanged).not.toHaveBeenCalled();

    appSync.mockResolvedValueOnce(answer(6));
    await refreshSync();
    expect(definitionsChanged).toHaveBeenCalledTimes(1);
    expect(definitionsChanged).toHaveBeenCalledWith(null);
  });

  it("ignores an embedded server too old to count them", async () => {
    appSync.mockResolvedValue(answer());
    await refreshSync();
    await refreshSync();
    expect(definitionsChanged).not.toHaveBeenCalled();
  });
});
