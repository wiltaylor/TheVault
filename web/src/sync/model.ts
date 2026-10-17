// What the sidebar's sync icon says about an app's sync (D19, D20, D27): green when the last round
// succeeded and nothing waits, yellow while a round runs or changes wait for one, red when sync
// failed or cannot run. The words go in the icon's accessible name and tooltip, so the state is
// never shown by colour alone.

import type { AppSync } from "../api";
import { relativeTime } from "../notifications/model";

export type SyncTone = AppSync["state"];

function changes(n: number): string {
  return `${n} ${n === 1 ? "change" : "changes"}`;
}

/** The icon's label: its accessible name and tooltip, such as "Synced 2 min ago". */
export function syncLabel(s: AppSync, now: number): string {
  const waiting = s.queued > 0 ? `${changes(s.queued)} waiting` : "";
  switch (s.phase) {
    case "starting":
      return waiting ? `Starting sync — ${waiting}` : "Starting sync…";
    case "syncing":
      return waiting ? `Syncing — ${waiting}` : "Syncing…";
    case "synced":
      if (waiting) return `Syncing — ${waiting}`;
      return s.last_ok_ms === null ? "Synced" : `Synced ${relativeTime(s.last_ok_ms, now)}`;
    case "offline":
      return `Sync failed: cannot reach the server — retrying${waiting ? ` · ${waiting}` : ""}`;
    case "update_required":
      return "Sync stopped: update the app to keep syncing";
    case "revoked":
      return "Sync stopped: this device was revoked — pair it again";
    case "unpaired":
      return "Not syncing: this device is not paired";
    default:
      return `Sync failed: ${firstLine(s.last_error) ?? "unknown error"}`;
  }
}

/** The first line of `text`, or null for none. */
function firstLine(text: string | null): string | null {
  const line = text?.split("\n")[0]?.trim();
  return line ? line : null;
}

/** Whether the phase needs the user to act, not just to wait: the one-time notice says so. */
export function needsAction(s: AppSync): string | null {
  if (s.phase === "update_required") return "TheVault needs an update to keep syncing with your server.";
  if (s.phase === "revoked") return "Your server revoked this device. Pair it again to keep syncing.";
  return null;
}

/** "Last synced" in the popover: relative, or "Never" before the first good round. */
export function lastSynced(s: AppSync, now: number): string {
  return s.last_ok_ms === null ? "Never" : relativeTime(s.last_ok_ms, now);
}
