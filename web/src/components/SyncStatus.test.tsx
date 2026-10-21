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

  it("shows a progress line and bar while a first sync pulls a lot", () => {
    const syncing: AppSync = {
      state: "syncing",
      phase: "syncing",
      queued: 0,
      last_ok_ms: null,
      last_attempt_ms: null,
      last_error: null,
      server: "https://vault.example.net",
      progress: {
        stage: "fetching",
        running: true,
        notes_done: 1200,
        notes_total: 1200,
        files_done: 3,
        files_total: 10,
        bytes_done: 12 * 1024 * 1024,
        bytes_total: 40 * 1024 * 1024,
        eta_secs: 130,
        elapsed_ms: 60_000,
        large: true,
      },
    };
    const { row, container } = details(syncing);
    expect(row("Progress")).toBe("Notes 1200 / 1200 · Files 3 / 10 (12.0 / 40.0 MB) · about 2 min left");
    const bar = container.querySelector<HTMLProgressElement>(".sync-progress progress");
    expect(bar?.value).toBeCloseTo((1 + 12 / 40) / 2);
    expect(bar?.getAttribute("aria-label")).toBe("Sync progress");
  });

  it("estimates the time left once it can, and counts the notes while it pulls", () => {
    const { row } = details({
      state: "syncing",
      phase: "syncing",
      queued: 0,
      last_ok_ms: null,
      last_error: null,
      server: null,
      progress: {
        stage: "pulling",
        running: true,
        notes_done: 500,
        notes_total: 1450,
        files_done: 0,
        files_total: 0,
        bytes_done: 0,
        bytes_total: 0,
        eta_secs: null,
        elapsed_ms: 400,
        large: true,
      },
    });
    expect(row("Progress")).toBe("Notes 500 / 1450 · estimating time left");
  });

  it("shows no progress for a small round, nor once a round ended", () => {
    const base: AppSync = {
      state: "syncing",
      phase: "syncing",
      queued: 0,
      last_ok_ms: NOW - 60_000,
      last_error: null,
      server: null,
    };
    const progress = {
      stage: "pulling" as const,
      running: true,
      notes_done: 2,
      notes_total: 3,
      files_done: 0,
      files_total: 1,
      bytes_done: 0,
      bytes_total: 1024,
      eta_secs: 1,
      elapsed_ms: 100,
      large: false,
    };
    expect(details({ ...base, progress }).row("Progress")).toBeNull();
    const ended = { ...progress, stage: "done" as const, running: false, large: true };
    expect(details({ ...base, state: "synced", phase: "synced", progress: ended }).row("Progress")).toBeNull();
    expect(details({ ...base, progress: null }).row("Progress")).toBeNull();
  });
});
