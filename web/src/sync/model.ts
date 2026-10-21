// What the sidebar's sync icon says about an app's sync (D19, D20, D27): green when the last round
// succeeded and nothing waits, yellow while a round runs or changes wait for one, red when sync
// failed or cannot run. The words go in the icon's accessible name and tooltip, so the state is
// never shown by colour alone.

import type { AppSync, StuckAsset } from "../api";
import { sizeText } from "../assetLibrary";
import { relativeTime } from "../notifications/model";

export type SyncTone = AppSync["state"];

function changes(n: number): string {
  return `${n} ${n === 1 ? "change" : "changes"}`;
}

function assets(n: number): string {
  return `${n} ${n === 1 ? "asset" : "assets"}`;
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
      if (s.assets_waiting) return `Sync incomplete: ${assets(s.assets_waiting)} not sent${waiting ? ` · ${waiting}` : ""}`;
      if (waiting) return `Syncing — ${waiting}`;
      return s.last_ok_ms === null ? "Synced" : `Synced ${relativeTime(s.last_ok_ms, now)}`;
    case "offline":
      return `Sync failed: cannot reach the server — retrying${waiting ? ` · ${waiting}` : ""}`;
    case "update_required":
      return "Sync stopped: update the app to keep syncing";
    case "updating":
      return "Sync waits: TheVault is updating itself to match the server";
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

/**
 * What This device says about the version gate (D27): the update sync waits for, or the warning
 * that the server is newer than the app and the app cannot update to it. Null when neither.
 */
export function gateNotice(s: AppSync): string | null {
  if (s.phase === "updating") return s.last_error ?? "Updating TheVault before syncing.";
  return s.warning ?? null;
}

/**
 * "Last synced" in the popover: relative, or "Never" before the first good round, with when the
 * last round was tried once one was.
 */
export function lastSynced(s: AppSync, now: number): string {
  if (s.last_ok_ms !== null) return relativeTime(s.last_ok_ms, now);
  return s.last_attempt_ms != null ? `Never (last tried ${relativeTime(s.last_attempt_ms, now)})` : "Never";
}

/** Whether the popover shows the last error: whenever a round failed or did not do everything, not only while red. */
export function showsError(s: AppSync): boolean {
  return !!s.last_error && s.phase !== "updating";
}

/** "Waiting" in the popover: the changes and assets that have not reached the server. */
export function waitingText(s: AppSync): string {
  const parts: string[] = [];
  if (s.queued > 0) parts.push(changes(s.queued));
  if (s.assets_waiting) parts.push(assets(s.assets_waiting));
  if (parts.length === 0) return "Nothing";
  const why: string[] = [];
  if (s.held) why.push(`${s.held} held for ${s.assets_waiting === 1 ? "the asset" : "the assets"}`);
  if (s.refused_notes?.length) why.push(`${s.refused_notes.length} refused by the server`);
  return parts.join(", ") + (why.length ? ` (${why.join(", ")})` : "");
}

/** A note the server refused, in the popover: which, and why (ticket 143). */
export function refusedText(n: { note_id: string; reason: string }): string {
  return `Note ${n.note_id}: ${n.reason}`;
}

/** A stuck asset in the popover: its name and size, why it failed, and when it is tried again. */
export function stuckText(a: StuckAsset, now: number): string {
  const wait = a.next_ms - now;
  const next = wait <= 60_000 ? "next round" : `in ${Math.ceil(wait / 60_000)} min`;
  const tries = a.attempts === 1 ? "1 try" : `${a.attempts} tries`;
  return `${a.filename} (${sizeText(a.size)}): ${a.error}. Failed ${tries}; retrying ${next}.`;
}
