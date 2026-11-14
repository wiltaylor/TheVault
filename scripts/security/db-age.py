"""Fail closed on missing, future or stale Trivy vulnerability DB metadata."""
import datetime
import json
import math
import sys

metadata = json.load(open(sys.argv[1], encoding="utf-8"))
updated = datetime.datetime.fromisoformat(metadata["UpdatedAt"].replace("Z", "+00:00"))
age = (datetime.datetime.now(datetime.timezone.utc) - updated).total_seconds()
maximum = float(sys.argv[2]) * 86400
print(json.dumps({"UpdatedAt": metadata["UpdatedAt"], "age_seconds": age,
                  "maximum_age_seconds": maximum}))
if not math.isfinite(maximum) or maximum < 0 or not 0 <= age <= maximum:
    sys.exit("ADVISORY_DB_STALE")
