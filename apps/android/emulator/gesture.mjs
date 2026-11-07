// Gestures on rows that move, such as a notification in the shade just after
// `cmd statusbar expand-notifications` (TV-92). The shade opens with an animation, and a row that
// was just posted slides in: a row's first sighting in `uiautomator dump` can be mid-flight, and a
// swipe or tap sent then misses it. On the Android 15 emulator, checks 20 and 21 failed that way
// on alpha.26 and alpha.27 alike, and passed once the gesture came a few seconds later.
//
// No adb here: the callers pass the dump, the gesture, the wait and the clock, so tests can run
// without a device.

/** Whether `a` and `b`, two sightings of a node as `lib.mjs`'s `dump` gives them, are at the same place. */
export function samePlace(a, b) {
  return Boolean(a && b) && a.bounds.length === b.bounds.length && a.bounds.every((v, i) => v === b.bounds[i]);
}

/**
 * Looks for a node with `find` until it is at the same place `steady` times in a row, `every` ms
 * apart, and answers that last sighting. Throws after `timeout` ms with what it saw last.
 */
export async function waitSteady(find, { what = "the node", timeout = 10_000, every = 300, steady = 2, sleep, now = Date.now }) {
  const end = now() + timeout;
  let last = null;
  let seen = 0;
  for (;;) {
    let n = null;
    try {
      n = await find();
    } catch {
      n = null;
    }
    seen = n && samePlace(n, last) ? seen + 1 : n ? 1 : 0;
    last = n;
    if (seen >= steady) return n;
    if (now() > end) {
      const where = last ? `, last at [${last.bounds.join(",")}]` : ", not on screen";
      throw new Error(`timed out after ${timeout} ms waiting for ${what} to hold still${where}`);
    }
    await sleep(every);
  }
}

/**
 * Sends a gesture with `act` until `done` says it worked, at most `tries` times, asking `done`
 * `settle` ms after each gesture. Answers how many gestures it took, or throws `fail` when none
 * worked. `act` finds its target again each time, so a retry aims at where the row is now.
 */
export async function retryGesture(act, done, { tries = 3, settle = 1_000, fail = "the gesture had no effect", sleep }) {
  for (let i = 1; i <= tries; i++) {
    await act(i);
    await sleep(settle);
    if (await done()) return i;
  }
  throw new Error(`${fail} (${tries} tries)`);
}
