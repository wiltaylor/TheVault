// What the sidebar's sync icon says about an app's sync (D19, D20, D27): green when the last round
// succeeded and nothing waits, yellow while a round runs or changes wait for one, red when sync
// failed or cannot run. The words go in the icon's accessible name and tooltip, so the state is
// never shown by colour alone.

import type { AppSync, StuckAsset, SyncProgress } from "../api";
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
  const held = s.held ? ` (${s.held} held for ${s.assets_waiting === 1 ? "the asset" : "the assets"})` : "";
  return parts.join(", ") + held;
}

/** The progress the popover shows: that of a running round that pulls or downloads a lot (ticket 144). */
export function shownProgress(s: AppSync): SyncProgress | null {
  const p = s.progress;
  return p && p.running && p.large ? p : null;
}

function megabytes(n: number): string {
  return (n / (1024 * 1024)).toFixed(1);
}

/** About how long is left, such as "about 40 s left" or "about 3 min left". */
export function timeLeft(secs: number | null): string {
  if (secs === null) return "estimating time left";
  if (secs === 0) return "finishing";
  if (secs < 60) return `about ${secs} s left`;
  const min = Math.round(secs / 60);
  if (min < 60) return `about ${min} min left`;
  return `about ${Math.floor(min / 60)} h ${min % 60} min left`;
}

/** The popover's progress line, such as "Notes 120 / 1200 · Files 3 / 10 (12.0 / 40.0 MB) · about 2 min left". */
export function progressText(p: SyncProgress): string {
  const pulled = p.stage === "pulling" || p.stage === "fetching";
  if (!pulled) return p.stage === "pushing" ? "Sending changes first" : "Starting";
  const parts = [`Notes ${p.notes_done} / ${p.notes_total}`];
  if (p.files_total > 0) {
    parts.push(`Files ${p.files_done} / ${p.files_total} (${megabytes(p.bytes_done)} / ${megabytes(p.bytes_total)} MB)`);
  }
  parts.push(timeLeft(p.eta_secs));
  return parts.join(" · ");
}

/** How much of the round is done, from 0 to 1: the notes and the bytes, each half when both are to do. */
export function progressShare(p: SyncProgress): number {
  const parts: number[] = [];
  if (p.notes_total > 0) parts.push(p.notes_done / p.notes_total);
  if (p.bytes_total > 0) parts.push(p.bytes_done / p.bytes_total);
  if (parts.length === 0) return 0;
  return Math.min(1, parts.reduce((a, b) => a + b, 0) / parts.length);
}

/** A stuck asset in the popover: its name and size, why it failed, and when it is tried again. */
export function stuckText(a: StuckAsset, now: number): string {
  const wait = a.next_ms - now;
  const next = wait <= 60_000 ? "next round" : `in ${Math.ceil(wait / 60_000)} min`;
  const tries = a.attempts === 1 ? "1 try" : `${a.attempts} tries`;
  return `${a.filename} (${sizeText(a.size)}): ${a.error}. Failed ${tries}; retrying ${next}.`;
}
