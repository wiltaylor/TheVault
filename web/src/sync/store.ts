// The app's sync status, kept fresh while the page runs in the desktop or Android app: asked of the
// embedded server's `GET /api/app/sync` every few seconds, every second while a round runs or
// changes wait, at once after each note write this client makes, and when the window comes back.
// In a browser nothing is asked: there is no app sync to show.

import { createSignal } from "solid-js";
import { type AppSync, api } from "../api";
import { onNoteWritten } from "../changes";

/** How often the status is asked while it is green or red. */
export const IDLE_MS = 3000;
/** How often while a round runs or changes wait, so yellow turns green promptly. */
export const BUSY_MS = 1000;

const [status, setStatus] = createSignal<AppSync | null>(null);

/** Reactive: the latest sync status, or null before the first answer or in a browser. */
export { status as syncStatus };

let timer: ReturnType<typeof setTimeout> | undefined;
let users = 0;
let asking: Promise<void> | null = null;

function schedule() {
  clearTimeout(timer);
  if (users === 0) return;
  const s = status();
  timer = setTimeout(() => void refreshSync(), s && s.state === "syncing" ? BUSY_MS : IDLE_MS);
}

/** Ask for the status now. A call while one is in flight shares it. */
export function refreshSync(): Promise<void> {
  if (asking) return asking;
  const ask = api
    .appSync()
    .then(
      (s) => {
        setStatus(s);
      },
      () => {},
    )
    .finally(() => {
      asking = null;
      schedule();
    });
  asking = ask;
  return ask;
}

/** Run a round now, then show where it got to. */
export async function syncNow(): Promise<void> {
  try {
    setStatus(await api.appSyncNow());
  } catch {
    // The status poll shows the reason.
  }
  await refreshSync();
}

/** Start keeping the status fresh. Returns the function that stops it. */
export function followSync(): () => void {
  users++;
  // A request already in flight may have left before the write: ask once more after it.
  const offWrite = onNoteWritten(() => void refreshSync().then(refreshSync));
  const onVisible = () => {
    if (document.visibilityState === "visible") void refreshSync();
  };
  document.addEventListener("visibilitychange", onVisible);
  void refreshSync();
  return () => {
    users--;
    offWrite();
    document.removeEventListener("visibilitychange", onVisible);
    if (users === 0) clearTimeout(timer);
  };
}

/** Test-only: forget the status and stop asking. */
export function resetSync() {
  users = 0;
  clearTimeout(timer);
  asking = null;
  setStatus(null);
}
