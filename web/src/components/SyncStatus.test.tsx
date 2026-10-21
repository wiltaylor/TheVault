import { render } from "@solidjs/testing-library";
import { describe, expect, it } from "vitest";
import type { AppSync } from "../api";
import { SyncDetails } from "./SyncStatus";

const NOW = Date.UTC(2026, 9, 21, 7, 0, 0);

/** The phone of ticket 141: after an import, one large MMS part will not upload. */
const stuck: AppSync = {
  state: "syncing",
  phase: "synced",
  queued: 41,
  held: 1,
  assets_waiting: 1,
  stuck_assets: [
    {
      id: "01J00000000000000000000000",
      filename: "VID_0042.mp4",
      size: 72 * 1024 * 1024,
      error: "sending asset 01J00000000000000000000000 stalled: no bytes moved for 60 s",
      attempts: 3,
      last_ms: NOW - 60_000,
      next_ms: NOW + 4 * 60_000,
    },
  ],
  last_ok_ms: null,
  last_attempt_ms: NOW - 2 * 60_000,
  last_error:
    "1 asset could not be sent to the server: VID_0042.mp4 (72.0 MB): sending asset 01J00000000000000000000000 stalled: no bytes moved for 60 s. 1 note waits for it; the others synced. It is tried again on later rounds.",
  server: "https://vault.example.net",
};

function details(s: AppSync) {
  const r = render(() => <SyncDetails status={s} now={NOW} />);
  const row = (term: string) => {
    const dt = [...r.container.querySelectorAll("dt")].find((d) => d.textContent === term);
    return dt?.nextElementSibling?.textContent ?? null;
  };
  return { ...r, row };
}

describe("the sync popover's details", () => {
  it("shows the last error and names the stuck asset while yellow", () => {
    const { row } = details(stuck);
    expect(row("Last synced")).toBe("Never (last tried 2 min ago)");
    expect(row("Waiting")).toBe("41 changes, 1 asset (1 held for the asset)");
    expect(row("Not sent")).toBe(
      "VID_0042.mp4 (72.0 MB): sending asset 01J00000000000000000000000 stalled: no bytes moved for 60 s. Failed 3 tries; retrying in 4 min.",
    );
    expect(row("Last error")).toMatch(/^1 asset could not be sent to the server: VID_0042\.mp4 \(72\.0 MB\)/);
  });

  it("shows the last error of a failed round while a new one runs", () => {
    const { row } = details({
      ...stuck,
      phase: "syncing",
      stuck_assets: [],
      assets_waiting: 0,
      held: 0,
      last_error: "cannot reach the server: connection refused",
    });
    expect(row("Last error")).toBe("cannot reach the server: connection refused");
    expect(row("Not sent")).toBeNull();
    expect(row("Waiting")).toBe("41 changes");
  });

  it("names a note the server refused, with why, while the others synced", () => {
    const { row } = details({
      ...stuck,
      queued: 1,
      held: 0,
      assets_waiting: 0,
      stuck_assets: [],
      refused_notes: [
        { note_id: "01J00000000000000000000001", reason: "writing note 01J00000000000000000000001: the server answered 400" },
      ],
      last_ok_ms: NOW - 60_000,
      last_error:
        "1 note could not be sent to the server: note 01J00000000000000000000001: writing note 01J00000000000000000000001: the server answered 400. The others synced; it is tried again on later rounds.",
    });
    expect(row("Waiting")).toBe("1 change (1 refused by the server)");
    expect(row("Not sent")).toBe(
      "Note 01J00000000000000000000001: writing note 01J00000000000000000000001: the server answered 400",
    );
    expect(row("Last error")).toMatch(/^1 note could not be sent to the server: note 01J00000000000000000000001/);
    expect(row("Last synced")).toBe("1 min ago");
  });

  it("shows no error once a round did everything", () => {
    const { row } = details({
      state: "synced",
      phase: "synced",
      queued: 0,
      last_ok_ms: NOW - 5_000,
      last_attempt_ms: NOW - 5_000,
      last_error: null,
      server: "https://vault.example.net",
    });
    expect(row("Last synced")).toBe("just now");
    expect(row("Waiting")).toBe("Nothing");
    expect(row("Last error")).toBeNull();
    expect(row("Not sent")).toBeNull();
  });
});
