# Disposable fake-only staging

This is not a production clone or a prompt-quality test. It checks API and
permission flows with scripted replies. Real agent, skill and classifier files
remain only in the live vault (TV-195 tracks future version control).

## Isolation and lifecycle

One fixed Compose project, `tv-145-staging`, runs as uid 1000 with a read-only
root, dropped capabilities and a **2 GiB total memory limit**. All four providers
and the vault server share a network namespace with **only loopback**
(`network_mode: none`). No ports, proxy, phone pairing, browser, Docker socket,
host directories or production credentials are exposed. Access is exclusively
through `terrarium exec thevault-docker-vm` and `docker compose exec` in the guest.

The server has a random password created in the private config volume on each
recreation, never testkit's password/admin token. Fixed provider keys are dummy
values accepted only by the local fakes. The supervisor keeps the fakes' stdin
open. Fakes get fresh ports and queues on each start. The Google task stays
paused after its explicit seed run. Keep staging idle between uses.

`recreate.sh` deletes only this project's two disposable volumes and container,
rebuilds the image, initializes a fresh vault/key/password, seeds fabricated
Gmail and session-created SMS notes, and runs a read-only agent trigger. It
does not restore a backup. No check depends on state surviving a VM rebuild.
The script never acquires the box-test lock. Box tests always have priority.
Do not run multiple copies of this project on the same guest.

## Build and recreate from nothing

Build binaries from the reviewed card commit in the one dev checkout. Run the
following build via `terrarium run thevault-dev --name staging-binaries -- sh
/src/thevault-heavy.sh sh -c '<command>'`:

```sh
cd /src/tv-145
cargo build --locked -p server -p cli -p testkit --bins
mkdir -p .scratch/staging/bin
cp "$CARGO_TARGET_DIR"/debug/vault "$CARGO_TARGET_DIR"/debug/vault-server \
  "$CARGO_TARGET_DIR"/debug/testkit-fake-* .scratch/staging/bin/
tar -czf .scratch/staging/binaries.tar.gz -C .scratch/staging bin
```

Transfer the archive through `terrarium cp` to the sandbox, then to
`thevault-docker-vm:/home/build/runs/tv-145/binaries.tar.gz`. Copy this directory's
tracked files to `/home/build/runs/tv-145/staging/` on that guest. Never run git
on a target. Record the source commit and binary archive SHA-256 with the run.
The runtime base is digest-pinned. The local staging image is built from these
exact binaries, not a floating server release.

Run in the guest (use `terrarium run` for the potentially long recreation):

```sh
tar -xzf /home/build/runs/tv-145/binaries.tar.gz -C /home/build/runs/tv-145/staging
cd /home/build/runs/tv-145/staging
sh recreate.sh
docker compose -p tv-145-staging -f compose.yaml ps
```

Run it twice to prove tear-down and recreation. Never use prune, cleanup filters,
or delete shared/base images. For this project's removal only:

```sh
docker compose -p tv-145-staging -f compose.yaml down --volumes
docker image rm tv-145-staging:local
```

## Try one agent by hand

Do not export the live vault, a backup, state database, notes, keys or account
secrets. Have the owner supply **one sanitized agent WCL**. Inspect it for copied
personal data, literal secrets, real addresses and remote accounts. Replace
`account` with `Staging anthropic` or `Staging openai`, and classifier account
references with `Staging decisions`. Stage only deliberately synthetic types
and data. Keep confirmation switches on and permissions narrow. Never copy
production account settings to make an agent load.

Copy the reviewed single file through `terrarium cp` into your own guest
directory, then import it through the authenticated API. For example, with
`candidate.wcl` beside the Compose file, inside the guest:

```sh
docker compose -p tv-145-staging -f compose.yaml cp candidate.wcl staging:/tmp/candidate.wcl
docker compose -p tv-145-staging -f compose.yaml exec -T staging python3 -c '
import json, subprocess
from pathlib import Path
subprocess.run(["python3", "/app/staging.py", "api", "PUT", "/api/agents/candidate",
                json.dumps(Path("/tmp/candidate.wcl").read_text())], check=True)
'
```

Queue one reply for each expected provider request before creating the synthetic
note that matches its `on`, `type` and `where` clauses. For example:

```sh
docker compose -p tv-145-staging -f compose.yaml exec -T staging \
  python3 /app/staging.py tell anthropic 'push {"text":"Synthetic note read."}'
docker compose -p tv-145-staging -f compose.yaml exec -T staging \
  python3 /app/staging.py api POST '/api/notes?type=staging_sms' \
  '{"note":{"fields":{"title":"Fabricated SMS for candidate"}}}'
docker compose -p tv-145-staging -f compose.yaml exec -T staging \
  python3 /app/staging.py api GET /api/agents/candidate/runs
```

Allow the scheduler's debounce window. For existing notes, POST
`/api/agents/candidate/triggers/<name>/backfill` first with `{"limit":1}`, inspect
the selected count, then POST `{"preview":"<returned preview>"}` to confirm.
The helper uses a password session, so backfill's session-only gate still applies.
For decisions, `tell decisions '<JSON response>'` queues the exact documented
decisions response, while `tell decisions requests` reports the count. Other
fakes accept their existing testkit stdin commands. An empty queue must fail
visibly, never fall back to a real provider.

## Evidence and limits

`python3 /app/staging.py smoke` asserts that loopback is the only interface,
attempts numeric external connections and asserts failure, then queues a fake
Anthropic response, creates an SMS and verifies a successful trigger and an
increased fake request count. No real outbound request can leave this network
namespace, including from copied scripts or agents. Synthetic SMS notes test
session writes, **not** device-origin provenance or phone capture. Fake models
do not prove prompt quality, classification quality or real-model safety.

For guest peak memory, run `python3 measure_memory.py tv-145` in the guest via
`terrarium run` with staging healthy. It runs one normal `thevault-box-test`
without taking its lock itself and samples `MemTotal - MemAvailable` every
second. Record the peak and exit code. Above approximately 7 GiB, tell the lead.
The measurement includes guest cache pressure, not just staging's cgroup usage.

Targeted supervisor tests: `STAGING_DECISIONS_BIN=<built binary> node --test
--test-reporter=tap deploy/staging/test_staging.test.mjs > <report.tap>`. Also run `sh -n
deploy/staging/recreate.sh`, package-scoped formatting/clippy for the new Rust
binary and the relevant testkit tests. Upload original reports with
`terrarium test push --card TV-145`.
