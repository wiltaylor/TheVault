// Autosave for the always-editable note page: a save runs `delay` ms after the last change, or at
// once on a flush (blur, tab switch, leaving the page). Saves never overlap: a change made while a
// save is in flight is saved by another save once that one ends.

export type SaveState = "idle" | "dirty" | "saving" | "saved" | "error";

export interface Autosave {
  /** Record a change and restart the delay. */
  schedule(): void;
  /** Save now when there are unsaved changes. Resolves once nothing is left to save, or a save fails. */
  flush(): Promise<void>;
  /**
   * Drop the pending timer and any unsaved change, as when a conflict takes over. A save in
   * flight that cancels does not end as "saved": nothing it was given reached the server.
   */
  cancel(): void;
  dirty(): boolean;
  state(): SaveState;
}

export interface AutosaveOptions {
  delay: number;
  /** Save the current state. Throwing marks the save failed; the change stays unsaved. */
  save: () => Promise<void>;
  onState?: (state: SaveState) => void;
  setTimer?: (fn: () => void, ms: number) => unknown;
  clearTimer?: (handle: unknown) => void;
}

export function createAutosave(options: AutosaveOptions): Autosave {
  const setTimer = options.setTimer ?? ((fn, ms) => setTimeout(fn, ms));
  const clearTimer = options.clearTimer ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>));
  let timer: unknown = null;
  let dirty = false;
  let running: Promise<void> | null = null;
  let state: SaveState = "idle";
  // Bumped by `cancel`, so a save loop that was cancelled while it ran does not say "saved".
  let epoch = 0;

  function set(next: SaveState) {
    state = next;
    options.onState?.(next);
  }

  function stopTimer() {
    if (timer !== null) clearTimer(timer);
    timer = null;
  }

  async function run(): Promise<void> {
    let started = epoch;
    let sent = false;
    while (dirty) {
      dirty = false;
      set("saving");
      let failed = false;
      try {
        await options.save();
        sent = true;
      } catch {
        failed = true;
      }
      if (epoch !== started) {
        // Cancelled while it ran: what it was given is not "saved". A change made since is
        // saved from here on, as a fresh run.
        started = epoch;
        sent = false;
        continue;
      }
      if (failed) {
        dirty = true;
        set("error");
        return;
      }
    }
    if (sent) set("saved");
  }

  function flush(): Promise<void> {
    stopTimer();
    if (running) {
      // The loop in flight picks up any change made since it started.
      return running;
    }
    if (!dirty) return Promise.resolve();
    running = run().finally(() => {
      running = null;
    });
    return running;
  }

  return {
    schedule() {
      dirty = true;
      if (!running) set("dirty");
      stopTimer();
      timer = setTimer(() => {
        timer = null;
        void flush();
      }, options.delay);
    },
    flush,
    cancel() {
      epoch += 1;
      stopTimer();
      dirty = false;
      set("idle");
    },
    dirty: () => dirty,
    state: () => state,
  };
}
