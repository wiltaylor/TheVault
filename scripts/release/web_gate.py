#!/usr/bin/env python3
"""Full release web gate, with one rerun of only failed Playwright tests.

Both original JUnit reports are kept. Recovered failures are explicitly flakes.
Infrastructure failures, missing reports and empty selections never become green.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET


def cases(path):
    root = ET.parse(path).getroot()
    rows = root.findall(".//testcase")
    if not rows:
        raise ValueError("web gate has no test cases")
    return rows


def failed(rows):
    return {(row.get("classname", ""), row.get("name", "")) for row in rows
            if row.find("failure") is not None or row.find("error") is not None}


def gate(root, runner=subprocess.run):
    out = root / ".scratch/release-gate"
    out.mkdir(parents=True, exist_ok=True)
    first = root / ".scratch/nightly/e2e.xml"
    last = root / "web/test-results/.last-run.json"
    first.unlink(missing_ok=True)
    last.unlink(missing_ok=True)
    status = runner(["bash", "scripts/nightly/build.sh", "e2e"], cwd=root).returncode
    if not first.exists():
        raise ValueError("web gate failed before producing JUnit; no test-only rerun")
    shutil.copyfile(first, out / "e2e-first.xml")
    rows = cases(first)
    if all(row.find("skipped") is not None for row in rows):
        raise ValueError("web gate skipped every test")
    failures = failed(rows)
    if status == 0:
        if failures:
            raise ValueError("web gate exit code contradicts failing JUnit")
        (out / "summary.json").write_text(json.dumps({"status": "pass", "flakes": []}) + "\n")
        return 0
    if not failures or not last.exists():
        raise ValueError("web gate failed without identifiable failed tests; no rerun")
    selection = json.loads(last.read_text())
    if selection.get("status") != "failed" or not selection.get("failedTests"):
        raise ValueError("web gate has no failed-test selection")
    shutil.copyfile(last, out / "e2e-first-last-run.json")
    second = out / "e2e-rerun.xml"
    second.unlink(missing_ok=True)
    result = runner(["pnpm", "-C", "web", "exec", "playwright", "test", "--last-failed", "--retries=0", "--reporter=list,junit"],
                    cwd=root, env=dict(os.environ, CARGO_TARGET_DIR=str(root / "target"), PLAYWRIGHT_JUNIT_OUTPUT_FILE=str(second)))
    rerun = cases(second)
    selected = {(row.get("classname", ""), row.get("name", "")) for row in rerun}
    if selected != failures or any(row.find("skipped") is not None for row in rerun):
        raise ValueError("web rerun selection differs from the failed tests or skipped them")
    persistent = failed(rerun)
    summary = {"status": "pass" if result.returncode == 0 and not persistent else "fail",
               "flakes": sorted(failures - persistent), "persistent": sorted(persistent),
               "first_report": "e2e-first.xml", "rerun_report": "e2e-rerun.xml"}
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary))
    return 0 if summary["status"] == "pass" else 1


if __name__ == "__main__":
    try:
        sys.exit(gate(Path.cwd()))
    except (ValueError, OSError, ET.ParseError) as error:
        print("release web gate failed:", error, file=sys.stderr)
        sys.exit(1)
