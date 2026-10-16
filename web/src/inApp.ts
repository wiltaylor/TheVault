// Whether the page runs inside the desktop or Android app, whose embedded server answers
// `GET /api/app` (D18). In the apps some settings are the home server's and need the web app:
// the app's device token has `write` scope, never `admin` (D2).

import { type AppInfo, ApiError, api } from "./api";

let asked: Promise<AppInfo | null> | undefined;

/** The app the page runs in, or null in a browser. Asked once per page load. */
export function appInfo(): Promise<AppInfo | null> {
  asked ??= api.app().catch(() => null);
  return asked;
}

/** Test-only: forget the answer, so the next call asks again. */
export function forgetAppInfo() {
  asked = undefined;
}

/** Whether `e` says the app cannot reach its home server, or is not paired with one. */
export function needsServer(e: unknown): boolean {
  return e instanceof ApiError && e.needsServer;
}
