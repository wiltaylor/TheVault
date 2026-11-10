import { readFileSync } from "node:fs";

// Linux PID plus start time avoids confusing a reused PID with the emulator we started.
export function processState(pid, read = readFileSync) {
  try {
    const stat = read(`/proc/${pid}/stat`, "utf8");
    const fields = stat.slice(stat.lastIndexOf(")") + 2).split(" ");
    return { start: fields[19], zombie: fields[0] === "Z" };
  } catch (e) {
    if (e.code === "ENOENT") return null;
    throw e;
  }
}

export function ownedEmulatorGone(record, expected, state = processState) {
  if (!record || !Number.isSafeInteger(record.pid) || record.pid <= 0 || !record.start ||
      record.serial !== expected.serial || record.avd !== expected.avd || record.home !== expected.home) {
    throw new Error("No matching emulator ownership record; refusing cold-boot recovery");
  }
  const current = state(record.pid);
  return !current || current.zombie || current.start !== record.start;
}

/** A failed adb wait cannot distinguish a dead emulator from a disconnected transport. */
export async function recoverReboot({ wait, gone, restart, reconnect = async () => {}, log = console.log }) {
  try {
    await wait();
    return;
  } catch (e) {
    if (!gone()) throw e;
    log(`Emulator process exited during reboot: ${e.message}. Cold-booting the same AVD without wiping data.`);
  }
  await restart();
  await wait(); // One recovery only. A second failure must remain visible.
  await reconnect();
}
