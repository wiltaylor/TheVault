#!/usr/bin/env bash
# Checkout updates belong to the schedule, never this runner.
set -euo pipefail
if [[ $# != 3 ]]; then
  printf 'usage: advisory-run.sh CHECKOUT SERVER@sha256:DIGEST BROWSER@sha256:DIGEST\n' >&2
  exit 2
fi
root=$(realpath "$1")
server=$2
browser=$3
for ref in "$server" "$browser"; do
  [[ $ref =~ ^[a-zA-Z0-9./:_-]+@sha256:[a-f0-9]{64}$ ]] || exit 2
done
[[ $server != "$browser" ]] || exit 2
base=${ADVISORY_OUT:-/src/advisory-out}
mkdir -p "$base"
base=$(realpath "$base")
[[ $base != "$root" && $base != "$root/"* ]] || exit 2
exec 9>"$base/run.lock"
flock -n 9 || exit 75
out="$base/$(date -u +%Y-%m-%dT%H%M%S.%N)"
mkdir "$out"
export PATH="${TRIVY_BIN_DIR:-/src/tools/trivy-0.58.2}:$PATH"
export TRIVY_CACHE_DIR="$base/cache"
help=$(trivy --help)
flags=()
for flag in --skip-version-check --disable-telemetry; do
  if grep -q -- "$flag" <<<"$help"; then flags+=("$flag"); fi
done
[[ $(trivy --version | head -n1) == 'Version: 0.58.2' ]] || exit 2
# Network preparation is separate from the offline reporter.
trivy image "${flags[@]}" --download-db-only
trivy image "${flags[@]}" --download-java-db-only
python3 "$(dirname "$0")/db-age.py" "$TRIVY_CACHE_DIR/db/metadata.json" "${ADVISORY_MAX_DB_AGE_DAYS:-8}" >"$out/database.json"
docker pull "$server"
docker pull "$browser"
docker save "$server" -o "$out/server.tar"
docker save "$browser" -o "$out/browser.tar"
printf '%s\n%s\n' "$server" "$browser" >"$out/images.txt"
export TRIVY_SKIP_VERSION_CHECK=true TRIVY_DISABLE_TELEMETRY=true
cd "$root"
node scripts/security/advisories.mjs "$root" "$out/server.tar" "$out/browser.tar" >"$out/report.json"
python3 - "$out" <<'PY'
import collections, json, pathlib, sys
p = pathlib.Path(sys.argv[1])
r = json.loads((p / 'report.json').read_text())
r['database'] = json.loads((p / 'database.json').read_text())
r['images'] = (p / 'images.txt').read_text().splitlines()
r['counts'] = dict(collections.Counter(f['severity'] for f in r['findings']))
(p / 'report.json').write_text(json.dumps(r) + '\n')
print(json.dumps({k: r[k] for k in ('database', 'images', 'counts')}))
PY
