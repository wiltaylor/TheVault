"""Run one box test while sampling guest memory. Takes no staging or box-test lock."""
import json
from pathlib import Path
import subprocess
import sys
import time


def used_kib():
    fields = {line.split(":")[0]: int(line.split()[1])
              for line in Path("/proc/meminfo").read_text().splitlines()
              if line.startswith(("MemTotal:", "MemAvailable:"))}
    return fields["MemTotal"] - fields["MemAvailable"]


def main():
    peak = used_kib()
    samples = 0
    with Path("box-test.log").open("w") as log:
        child = subprocess.Popen(["thevault-box-test", sys.argv[1]], stdout=log,
                                 stderr=subprocess.STDOUT)
        while child.poll() is None:
            peak = max(peak, used_kib())
            samples += 1
            time.sleep(1)
    result = {"box_test_exit": child.returncode, "samples": samples,
              "guest_peak_used_kib": peak, "guest_peak_used_gib": round(peak / 1024**2, 3)}
    Path("memory.json").write_text(json.dumps(result) + "\n")
    print(json.dumps(result), flush=True)
    raise SystemExit(child.returncode)


if __name__ == "__main__":
    main()
