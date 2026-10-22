import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const sendDeviceNotes = vi.fn();
const appSync = vi.fn();
vi.mock("../api", async (orig) => {
  const real = await orig<typeof import("../api")>();
  return { ...real, api: { ...real.api, sendDeviceNotes, appSync } };
});

const { ApiError } = await import("../api");
const { SendDeviceNotes, sentText } = await import("./SendDeviceNotes");

const BUTTON = "Send this device's notes to the server";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("sentText", () => {
  it("counts the notes and files queued and says when they go", () => {
    expect(sentText({ queued: 2, notes: ["a", "b"], assets: 0, syncing: true })).toBe(
      "Queued 2 notes the server did not have. Sync is sending them now.",
    );
    expect(sentText({ queued: 1, notes: ["a"], assets: 1, syncing: false })).toBe(
      "Queued 1 note and 1 file the server did not have. The next sync sends them.",
    );
    expect(sentText({ queued: 0, notes: [], assets: 0, syncing: true })).toContain("already has every note");
  });
});

describe("SendDeviceNotes", () => {
  it("queues the device's notes the server lacks and says how many", async () => {
    sendDeviceNotes.mockResolvedValue({ queued: 435, notes: [], assets: 0, syncing: true });
    appSync.mockResolvedValue({ state: "syncing" });
    const m = render(() => <SendDeviceNotes />);
    fireEvent.click(m.getByRole("button", { name: BUTTON }));
    await waitFor(() => expect(m.getByRole("status")).toHaveTextContent("Queued 435 notes"));
    expect(m.getByRole("status")).toHaveTextContent("Sync is sending them now.");
    expect(sendDeviceNotes).toHaveBeenCalledTimes(1);
    // The sync icon is asked where the round stands.
    await waitFor(() => expect(appSync).toHaveBeenCalled());
    expect(m.getByRole("button", { name: BUTTON })).not.toBeDisabled();
  });

  it("says why when the server cannot be reached", async () => {
    sendDeviceNotes.mockRejectedValue(
      new ApiError(503, { error: "offline", message: "Needs a connection to your server" }),
    );
    const m = render(() => <SendDeviceNotes />);
    fireEvent.click(m.getByRole("button", { name: BUTTON }));
    await waitFor(() => expect(m.getByRole("alert")).toBeInTheDocument());
    expect(m.queryByRole("status")).toBeNull();
  });
});
