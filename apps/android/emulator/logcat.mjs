// The adb arguments of the logcat reads the checks make (TV-66). No adb here: checks.mjs runs them.

/**
 * The arguments of `adb logcat` for the lines since `since`, a device time such as
 * "11-07 15:00:00.000" (see `deviceTime` in checks.mjs). The time is one argument, without quote
 * characters of its own: adb quotes each argument it passes on, so a quote in the argument reaches
 * logcat as a literal character and logcat then reads no time.
 */
export function logcatSinceArgs(since) {
  if (!/^\d\d-\d\d \d\d:\d\d:\d\d\.\d{3}$/.test(since)) throw new Error(`logcat time "${since}" is not "MM-DD hh:mm:ss.mmm"`);
  return ["logcat", "-d", "-v", "time", "-t", since];
}
