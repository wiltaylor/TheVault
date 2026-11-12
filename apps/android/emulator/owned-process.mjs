import { spawn } from "node:child_process";
import { openSync, closeSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { processState } from "./reboot.mjs";

// Stay the child's parent after boot.sh returns, so Node can reap its exact exit status.
// Reports are per launch: a recovery boot must not overwrite the preceding exit evidence.
export function supervise(command, args, { out, identity, ready }) {
  const launched = new Date().toISOString();
  const report = join(out, `exit-${Date.now()}-${process.pid}.json`);
  const fd = openSync(join(out, "emulator.log"), "w");
  const child = spawn(command, args, { stdio: ["ignore", fd, fd] });
  closeSync(fd);
  let owner;
  child.once("spawn", () => {
    const state = processState(child.pid);
    owner = { ...identity, pid: child.pid, start: state?.start, report };
    writeFileSync(join(out, "owner.json"), JSON.stringify(owner));
    writeFileSync(ready, String(child.pid));
  });
  child.once("error", (error) => {
    writeFileSync(report, JSON.stringify({ ...identity, launched, ended: new Date().toISOString(),
      code: null, signal: null, error: error.message }));
    process.exitCode = 1;
  });
  child.once("exit", (code, signal) => {
    writeFileSync(report, JSON.stringify({ ...owner, launched, ended: new Date().toISOString(), code, signal }));
  });
  return child;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [out, ready, command, ...args] = process.argv.slice(2);
  supervise(command, args, { out, ready, identity: {
    serial: process.env.EMULATOR_SERIAL, avd: process.env.EMULATOR_AVD,
    home: process.env.EMULATOR_AVD_HOME,
  } });
}
