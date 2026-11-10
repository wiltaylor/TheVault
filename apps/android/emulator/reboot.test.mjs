import assert from "node:assert/strict";
import { test } from "node:test";
import { ownedEmulatorGone, processState, recoverReboot } from "./reboot.mjs";

const expected = { serial: "emulator-5580", avd: "tv130-api35", home: "/home/agent/tv-130/avd" };
const record = { ...expected, pid: 123, start: "456" };

test("cold-boot recovery reopens the Android device and waits for its WebView", async () => {
  const calls = [];
  let waits = 0;
  await recoverReboot({
    wait: async () => { if (++waits === 1) throw new Error("device gone"); },
    gone: () => true,
    restart: async () => calls.push("cold boot"),
    reconnect: async () => { calls.push("close device", "open device", "wait WebView socket", "open WebView"); },
    log: () => {},
  });
  assert.deepEqual(calls, ["cold boot", "close device", "open device", "wait WebView socket", "open WebView"]);
});

test("ownership requires the exact serial, AVD, home and recorded process", () => {
  for (const bad of [null, { ...record, pid: -1 }, { ...record, serial: "emulator-5554" },
    { ...record, avd: "thevault" }, { ...record, home: "/other" }, { ...record, start: "" }]) {
    assert.throws(() => ownedEmulatorGone(bad, expected), /ownership record/);
  }
});

test("only an absent, zombie or reused PID permits recovery", () => {
  assert.equal(ownedEmulatorGone(record, expected, () => ({ start: "456", zombie: false })), false);
  for (const state of [null, { start: "456", zombie: true }, { start: "789", zombie: false }]) {
    assert.equal(ownedEmulatorGone(record, expected, () => state), true);
  }
});

test("process identity parses stat even when the name contains spaces or parentheses", () => {
  const fields = ["S", ...Array(18).fill("0"), "456"];
  assert.deepEqual(processState(123, () => `123 (qemu (test)) ${fields.join(" ")}`), { start: "456", zombie: false });
  assert.equal(processState(123, () => { throw Object.assign(new Error(), { code: "ENOENT" }); }), null);
  assert.throws(() => processState(123, () => { throw Object.assign(new Error("denied"), { code: "EACCES" }); }), /denied/);
});

test("successful reconnect never inspects ownership or restarts", async () => {
  await recoverReboot({ wait: async () => {}, gone: () => assert.fail(), restart: () => assert.fail() });
});

test("a live emulator is not restarted after a transport timeout", async () => {
  await assert.rejects(recoverReboot({ wait: async () => { throw new Error("offline"); },
    gone: () => false, restart: () => assert.fail() }), /offline/);
});

test("a dead owned emulator is restarted once, then waited for", async () => {
  const calls = [];
  await recoverReboot({ wait: async () => { calls.push("wait"); if (calls.length === 1) throw new Error("offline"); },
    gone: () => true, restart: async () => { calls.push("restart"); }, log: () => {} });
  assert.deepEqual(calls, ["wait", "restart", "wait"]);
});

test("a failed recovery is reported without an endless restart loop", async () => {
  let restarts = 0;
  await assert.rejects(recoverReboot({ wait: async () => { throw new Error("offline"); }, gone: () => true,
    restart: async () => { restarts++; }, log: () => {} }), /offline/);
  assert.equal(restarts, 1);
});
