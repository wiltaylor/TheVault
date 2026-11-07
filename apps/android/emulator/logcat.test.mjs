// Tests for logcat.mjs (TV-66): run with `node --test apps/android/emulator/logcat.test.mjs`. No device needed.

import assert from "node:assert/strict";
import { test } from "node:test";
import { logcatSinceArgs } from "./logcat.mjs";

test("logcatSinceArgs passes the device time as one argument, without quote characters", () => {
  const args = logcatSinceArgs("11-07 15:00:00.000");
  assert.deepEqual(args, ["logcat", "-d", "-v", "time", "-t", "11-07 15:00:00.000"]);
  assert.equal(args.some((a) => a.includes("'") || a.includes('"')), false);
});

test("logcatSinceArgs refuses a time that is not MM-DD hh:mm:ss.mmm", () => {
  assert.throws(() => logcatSinceArgs("'11-07 15:00:00.000'"), /not "MM-DD hh:mm:ss.mmm"/);
  assert.throws(() => logcatSinceArgs("11-07 15:00:00"), /not "MM-DD hh:mm:ss.mmm"/);
  assert.throws(() => logcatSinceArgs(""), /not "MM-DD hh:mm:ss.mmm"/);
});
