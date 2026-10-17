import { describe, expect, it } from "vitest";
import type { AppSync } from "../api";
import { lastSynced, needsAction, syncLabel } from "./model";

const NOW = Date.UTC(2026, 9, 17, 12, 0, 0);

function status(change: Partial<AppSync>): AppSync {
  return {
    state: "synced",
    phase: "synced",
    queued: 0,
    last_ok_ms: NOW - 2 * 60_000,
    last_error: null,
    server: "https://home",
    ...change,
  };
}

describe("the sync icon's words", () => {
  it("says when it last synced while green", () => {
    expect(syncLabel(status({}), NOW)).toBe("Synced 2 min ago");
    expect(syncLabel(status({ last_ok_ms: NOW - 5_000 }), NOW)).toBe("Synced just now");
    expect(syncLabel(status({ last_ok_ms: null }), NOW)).toBe("Synced");
  });

  it("counts the changes waiting while yellow", () => {
    expect(syncLabel(status({ state: "syncing", queued: 3 }), NOW)).toBe("Syncing — 3 changes waiting");
    expect(syncLabel(status({ state: "syncing", queued: 1 }), NOW)).toBe("Syncing — 1 change waiting");
    expect(syncLabel(status({ state: "syncing", phase: "syncing" }), NOW)).toBe("Syncing…");
    expect(syncLabel(status({ state: "syncing", phase: "syncing", queued: 2 }), NOW)).toBe("Syncing — 2 changes waiting");
    expect(syncLabel(status({ state: "syncing", phase: "starting", last_ok_ms: null }), NOW)).toBe("Starting sync…");
  });

  it("says what failed while red", () => {
    const offline = status({ state: "error", phase: "offline", last_error: "cannot reach the server: refused" });
    expect(syncLabel(offline, NOW)).toBe("Sync failed: cannot reach the server — retrying");
    expect(syncLabel({ ...offline, queued: 2 }, NOW)).toBe(
      "Sync failed: cannot reach the server — retrying · 2 changes waiting",
    );
    expect(syncLabel(status({ state: "error", phase: "update_required" }), NOW)).toBe(
      "Sync stopped: update the app to keep syncing",
    );
    expect(syncLabel(status({ state: "error", phase: "revoked" }), NOW)).toBe(
      "Sync stopped: this device was revoked — pair it again",
    );
    expect(syncLabel(status({ state: "error", phase: "unpaired", server: null }), NOW)).toBe(
      "Not syncing: this device is not paired",
    );
    expect(syncLabel(status({ state: "error", phase: "error", last_error: "the sync tables: locked\nmore" }), NOW)).toBe(
      "Sync failed: the sync tables: locked",
    );
    expect(syncLabel(status({ state: "error", phase: "error", last_error: null }), NOW)).toBe("Sync failed: unknown error");
  });

  it("asks the user to act only when waiting will not help", () => {
    expect(needsAction(status({ state: "error", phase: "update_required" }))).toMatch(/needs an update/);
    expect(needsAction(status({ state: "error", phase: "revoked" }))).toMatch(/Pair it again/);
    expect(needsAction(status({ state: "error", phase: "offline" }))).toBeNull();
    expect(needsAction(status({}))).toBeNull();
  });

  it("says never before the first good round", () => {
    expect(lastSynced(status({ last_ok_ms: null }), NOW)).toBe("Never");
    expect(lastSynced(status({}), NOW)).toBe("2 min ago");
  });
});
