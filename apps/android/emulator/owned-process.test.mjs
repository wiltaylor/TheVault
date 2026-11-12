import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

const supervisor = new URL("./owned-process.mjs", import.meta.url);
async function launch(out, script, command = process.execPath) {
  const child = spawn(process.execPath, [supervisor.pathname, out, join(out, "ready"), command,
    "-e", script], { env: { ...process.env, EMULATOR_SERIAL: "emulator-5584",
    EMULATOR_AVD: "unit-only", EMULATOR_AVD_HOME: out }, stdio: "ignore" });
  return (await once(child, "exit"))[0];
}
function fixture(t) {
  const out = mkdtempSync(join(tmpdir(), "owned-exit-"));
  t.after(() => rmSync(out, { recursive: true, force: true }));
  return out;
}
function reports(out) {
  return readdirSync(out).filter(n => n.startsWith("exit-")).map(n => JSON.parse(readFileSync(join(out, n))));
}

test("records exact exit code, owned identity and both output streams", async t => {
  const out = fixture(t);
  assert.equal(await launch(out, 'console.log("stdout"); console.error("stderr"); process.exit(23)'), 0);
  const owner = JSON.parse(readFileSync(join(out, "owner.json")));
  const [report] = reports(out);
  assert.equal(report.code, 23);
  assert.equal(report.signal, null);
  assert.equal(report.pid, owner.pid);
  assert.ok(owner.start);
  assert.equal(report.start, owner.start);
  assert.equal(report.serial, "emulator-5584");
  assert.equal(report.avd, "unit-only");
  assert.equal(report.home, out);
  assert.match(readFileSync(join(out, "emulator.log"), "utf8"), /stdout\nstderr/);
});

test("captures signal and preserves preceding launch report across restart", async t => {
  const out = fixture(t);
  await launch(out, 'process.kill(process.pid, "SIGTERM")');
  const [first] = reports(out);
  assert.equal(first.code, null);
  assert.equal(first.signal, "SIGTERM");
  await launch(out, "process.exit(0)");
  const all = reports(out);
  assert.equal(all.length, 2);
  assert.deepEqual(all.find(r => r.pid === first.pid), first);
  assert.ok(all.some(r => r.code === 0 && r.signal === null));
});

test("failed spawn is explicit, not misreported as an emulator signal", async t => {
  const out = fixture(t);
  assert.equal(await launch(out, "", join(out, "missing-command")), 1);
  const [report] = reports(out);
  assert.match(report.error, /ENOENT/);
  assert.equal(report.code, null);
  assert.equal(report.signal, null);
});
