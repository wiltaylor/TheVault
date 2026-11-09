// Format checklist evidence without starting the emulator harness.
export function resultsTap(titles, results, { only = null, completed = false } = {}) {
  const cases = Object.entries(titles).filter(([n]) => !only || only.has(Number(n)));
  const tap = ["TAP version 13", `1..${cases.length}`];
  for (const [i, [n, title]] of cases.entries()) {
    const r = results[n];
    const unimplemented = !r && completed;
    const skip = unimplemented ? " # SKIP not implemented"
      : r && ["na", "partial"].includes(r.status) ? ` # SKIP ${r.status}` : "";
    const ok = unimplemented || (r && r.status !== "fail");
    tap.push(`${ok ? "ok" : "not ok"} ${i + 1} - Check ${n}: ${title.doing.replace(/[\r\n#]/g, " ")}${skip}`);
    tap.push(`# ${(r?.note ?? (completed ? "Not implemented." : "The run stopped before this check.")).replace(/[\r\n]/g, " ")}`);
  }
  return `${tap.join("\n")}\n`;
}
