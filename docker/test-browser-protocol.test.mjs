// Runs the image gate's HTTP checks against the service binary when Docker is unavailable.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdtemp, mkdir, rm } from "node:fs/promises";
import { tmpdir, userInfo } from "node:os";
import { join, resolve } from "node:path";
import { createServer } from "node:net";
import { once } from "node:events";
import { randomBytes } from "node:crypto";

test("image gate HTTP checks use signed, separate and resettable profiles", { timeout: 240000 }, async (t) => {
  const root = await mkdtemp(join(tmpdir(), "vault-browser-protocol-"));
  const home = join(root, "home");
  const work = join(root, "work");
  const tmp = join(root, "tmp");
  await Promise.all([home, work, tmp].map((p) => mkdir(p)));
  // Reserve a random port, never a hard-coded port on a shared target.
  const reservation = createServer();
  reservation.listen(0, "127.0.0.1");
  await once(reservation, "listening");
  const port = reservation.address().port;
  await new Promise((done) => reservation.close(done));
  const base = `http://127.0.0.1:${port}`;
  const token = randomBytes(24).toString("hex");
  const binary = process.env.BROWSER_AGENT_BIN || join(process.env.CARGO_TARGET_DIR || "target", "debug/browser-agent");
  const child = spawn(resolve(binary), [], {
    env: { ...process.env, BROWSER_TOKEN: token, BROWSER_LISTEN: `127.0.0.1:${port}`, BROWSER_HOME_DIR: home, BROWSER_WORK_DIR: work, PLAYWRIGHT_CLI_VERSION: "0.1.22", DISPLAY: ":1" },
    stdio: "ignore",
  });
  const stopped = once(child, "exit");
  try {
    const deadline = Date.now() + 5000;
    while (true) {
      try { if ((await fetch(`${base}/health`)).ok) break; } catch { /* service is starting */ }
      assert.ok(Date.now() < deadline, "browser service did not start");
      await new Promise((done) => setTimeout(done, 20));
    }
    const { stdout } = await promisify(execFile)("bash", ["docker/test-browser-image.sh"], {
      env: { ...process.env, BOX_URL: base, BOX_TOKEN: token, BOX_HOME_DIR: home, BOX_SERVICE_PID: String(child.pid), BOX_EXPECTED_USER: userInfo().username, BOX_EXPECTED_UID: String(process.getuid()), TMPDIR: tmp },
      // The dev filesystem can build all4000 levels using external mkdir;
      // allow the script's180s setup budget plus its other HTTP checks.
      timeout: 210000,
    });
    assert.match(stdout, /all checks passed/);
    assert.match(stdout, /a second profile cannot read the screenshot/);
    assert.match(stdout, /a retired profile is refused/);
    assert.match(stdout, /container.*NOT run/);
    t.diagnostic(stdout);
  } finally {
    child.kill("SIGTERM");
    await stopped;
    await rm(root, { recursive: true, force: true });
  }
});
