// Tests for gesture.mjs (TV-92): run with `node --test apps/android/emulator/gesture.test.mjs`. No device needed.

import assert from "node:assert/strict";
import { test } from "node:test";
import { noteIdOf, retryGesture, samePlace, waitSteady } from "./gesture.mjs";

/** A fake clock: `sleep` moves it on at once. */
function clock() {
  let t = 0;
  return { now: () => t, sleep: async (ms) => void (t += ms) };
}

const row = (y) => ({ text: "Check 20 swipe", bounds: [179, y - 28, 996, y + 29], x: 588, y });

test("samePlace compares the bounds of two sightings", () => {
  assert.equal(samePlace(row(730), row(730)), true);
  assert.equal(samePlace(row(730), row(731)), false);
  assert.equal(samePlace(null, row(730)), false);
  assert.equal(samePlace(row(730), undefined), false);
});

test("waitSteady skips a row that is still sliding in and answers where it stopped", async () => {
  // The shade opening: the row is first seen mid-flight, then at its place.
  const seen = [null, row(1650), row(1100), row(730), row(730)];
  let i = 0;
  const c = clock();
  const n = await waitSteady(() => seen[Math.min(i++, seen.length - 1)], { ...c, every: 300 });
  assert.equal(n.y, 730);
  assert.equal(i, 5);
});

test("waitSteady does not take the first sighting of a row", async () => {
  const c = clock();
  let calls = 0;
  const n = await waitSteady(() => (calls++, row(730)), { ...c });
  assert.equal(n.y, 730);
  assert.equal(calls, 2);
});

test("waitSteady starts again when the row goes and comes back", async () => {
  const seen = [row(730), null, row(730), row(730)];
  let i = 0;
  const c = clock();
  await waitSteady(() => seen[i++], { ...c });
  assert.equal(i, 4);
});

test("waitSteady counts a failed dump as not seen", async () => {
  const seen = [row(730), "throw", row(730), row(730)];
  let i = 0;
  const c = clock();
  await waitSteady(
    () => {
      const s = seen[i++];
      if (s === "throw") throw new Error("uiautomator dump failed");
      return s;
    },
    { ...c },
  );
  assert.equal(i, 4);
});

test("waitSteady times out on a row that never holds still, naming where it was", async () => {
  let y = 700;
  const c = clock();
  await assert.rejects(waitSteady(() => row((y += 10)), { ...c, what: '"Check 20 swipe"', timeout: 3_000 }), /timed out after 3000 ms waiting for "Check 20 swipe" to hold still, last at \[179,/);
});

test("waitSteady times out on a row that never shows", async () => {
  const c = clock();
  await assert.rejects(waitSteady(() => undefined, { ...c, timeout: 1_000 }), /not on screen/);
});

test("retryGesture sends the gesture again until it works, and answers how many it took", async () => {
  const c = clock();
  let sent = 0;
  const tries = await retryGesture(
    async () => void sent++,
    () => sent >= 2,
    { ...c },
  );
  assert.equal(tries, 2);
  assert.equal(sent, 2);
});

test("retryGesture asks done only after the gesture had time to settle", async () => {
  const c = clock();
  let sentAt = -1;
  await retryGesture(
    async () => void (sentAt = c.now()),
    () => c.now() - sentAt >= 1_000,
    { ...c, settle: 1_000 },
  );
});

test("retryGesture stops after tries and throws fail", async () => {
  const c = clock();
  let sent = 0;
  await assert.rejects(
    retryGesture(
      async () => void sent++,
      () => false,
      { ...c, tries: 3, fail: "the swipe did not remove the notification" },
    ),
    /the swipe did not remove the notification \(3 tries\)/,
  );
  assert.equal(sent, 3);
});

test("noteIdOf answers the note a page shows, and null for other pages", () => {
  assert.equal(noteIdOf("http://vault.localhost/notes/01M6V61GVNQQNMG854ZT6H19DA"), "01M6V61GVNQQNMG854ZT6H19DA");
  assert.equal(noteIdOf("http://vault.localhost/notes/01M6V61GVNQQNMG854ZT6H19DA/"), "01M6V61GVNQQNMG854ZT6H19DA");
  assert.equal(noteIdOf("http://vault.localhost/notes/a%20b?x=1#y"), "a b");
  assert.equal(noteIdOf("http://vault.localhost/notes/01M6V61GVNQQNMG854ZT6H19DA/history"), null);
  assert.equal(noteIdOf("http://vault.localhost/planner"), null);
  assert.equal(noteIdOf("http://vault.localhost/notes/"), null);
  assert.equal(noteIdOf("not a url"), null);
});

test("noteIdOf tells two notes apart, so a warm tap cannot pass on the cold tap's page", () => {
  const cold = "01M6V61GVNQQNMG854ZT6H19DA";
  const warm = "01M6V61GVNQQNMG854ZT6H19DB";
  const left = `http://vault.localhost/notes/${cold}`;
  assert.notEqual(noteIdOf(left), warm);
  assert.equal(left.includes(cold), true);
});
