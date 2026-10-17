// The app update check as Settings > General shows it (D27). The desktop app's own routes under
// `/_desktop` hold its updater's state (apps/desktop/src-tauri/src/lib.rs); the Android app's
// device plugin answers from AppUpdateWorker's preferences. A browser has neither.

import type { AppInfo } from "./api";
import { thisPhone } from "./deviceImport";

/** Where the app update check stands. */
export interface UpdateState {
  /** One line: "Up to date", "0.1.4 ready — restart to update", and so on. */
  summary: string;
  /** When the last check finished, in milliseconds since the epoch. */
  checked: number | null;
  /** Whether a check runs now. */
  checking: boolean;
  /** A downloaded update waits for a restart (desktop AppImage). */
  restart: boolean;
  /** A newer release is offered as a download (desktop deb, rpm or other binary). */
  download: boolean;
  /** Whether "Check for updates" can run: the check is not turned off for this build. */
  canCheck: boolean;
  /** The automatic check is turned off; a manual check still runs. */
  automaticOff: boolean;
}

/** What the desktop app's `GET /_desktop/status` says about updates, as far as this reads it. */
interface DesktopStatus {
  update?: { version: string; ready?: boolean } | null;
  updater?: {
    checks?: number;
    error?: string | null;
    checking?: boolean;
    last_checked?: number | null;
    available?: boolean;
  };
  settings?: { update_check?: boolean };
}

function isAndroid(info: AppInfo): boolean {
  return info.platform === "android";
}

async function desktop(path: string, method: "GET" | "POST" = "GET"): Promise<Response> {
  const res = await fetch(path, { method });
  if (!res.ok && res.status !== 409) throw new Error(`${method} ${path} answered ${res.status}`);
  return res;
}

/** The desktop app's update state from its status. */
export function desktopState(status: DesktopStatus): UpdateState {
  const u = status.updater ?? {};
  const offer = status.update ?? null;
  const checks = u.checks ?? 0;
  let summary: string;
  if (offer?.ready) summary = `${offer.version} ready — restart to update`;
  else if (offer) summary = `${offer.version} available`;
  else if (u.error) summary = `The last check failed: ${u.error}`;
  else if (u.available === false) summary = "Update checks are turned off in this build";
  else if (checks > 0) summary = "Up to date";
  else summary = "Not checked yet";
  return {
    summary,
    checked: u.last_checked ?? null,
    checking: u.checking ?? false,
    restart: offer?.ready === true,
    download: offer !== null && offer.ready !== true,
    canCheck: u.available !== false,
    automaticOff: status.settings?.update_check === false,
  };
}

/** The Android app's update state from AppUpdateWorker's last status. */
export function androidState(last: { status: string | null; message: string | null; checked: number | null }): UpdateState {
  let summary: string;
  if (last.status === null) summary = "Not checked yet";
  else if (last.status === "up_to_date") summary = "Up to date";
  else summary = last.message || last.status;
  return {
    summary,
    checked: last.checked,
    checking: false,
    restart: false,
    download: false,
    canCheck: true,
    automaticOff: false,
  };
}

/** Where the update check of the app `info` names stands. */
export async function updateState(info: AppInfo): Promise<UpdateState> {
  if (isAndroid(info)) return androidState(await thisPhone.updateStatus());
  return desktopState((await (await desktop("/_desktop/status")).json()) as DesktopStatus);
}

/** Run the update check now. */
export async function checkForUpdates(info: AppInfo): Promise<void> {
  if (isAndroid(info)) return thisPhone.checkForUpdates();
  await desktop("/_desktop/update/check", "POST");
}

/** Desktop: install the downloaded update and start it. */
export async function restartToUpdate(): Promise<void> {
  await desktop("/_desktop/update/restart", "POST");
}

/** Desktop: open the newer release's download in the browser. */
export async function openDownload(): Promise<void> {
  await desktop("/_desktop/update/open", "POST");
}
