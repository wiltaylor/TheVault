# Off-host backup and isolated restore

This is a runbook, not evidence that production has off-host backups.
Same-volume pre-upgrade archives protect against some upgrade failures, not
loss of the host or volume. See [installation backup commands](install.md#back-up).
Agent, skill and classifier configuration coverage remains owned by TV-195.

## Ownership and schedule

Before enabling real copies, the lead and meta must record the named backup
owner, alternate owner, destination host/account and path, key custodian,
retention policy and next drill date on a separate operations card. That card
must obtain approval for exact production access, copies and retention deletion.
This runbook authorizes none of those operations.

- D1: Use a destination on a different host and failure domain, with encrypted
  storage and access limited to the backup owner and alternate. A directory on
  the vault host, a mounted data volume or a public artifact is not off-host.
  Archives contain private notes and encrypted credentials, not just notes.
- D2: Keep the secret key in separately controlled encrypted custody, not beside
  the archives or in their transfer account. Use the default key-excluding backup,
  never `--include-key` for this plan. Record the vault ID and custody reference,
  not the key or any credential value. Do not export production credentials as
  part of a drill.
- D3: Proposed cadence is daily copies and a copy before each upgrade, retaining
  14 daily and 3 monthly recovery points. The owner reviews transfer failures
  each day. Retention is finite but deletion needs separate approved execution.
  Until automated monitoring exists, explicitly record the daily human check.
- D4: Run the synthetic drill monthly and after backup/restore format changes.
  Schedule an approved isolated real-data recovery separately each quarter.
  Synthetic success proves the mechanism, not production coverage or key custody.

## Copy and integrity procedure (operations approval required)

1. Use the installed version's `vault backup <new-archive.sqlar> --vault <vault>`
   without `--include-key`. For rollback, take it with the old version before
   upgrading. Record build version, vault ID, timestamp, archive size and SHA-256.
2. Transfer to the approved off-host destination over an authenticated encrypted
   channel. Compute SHA-256 at the destination and compare to the source digest.
   Keep the digest and inventory in a separately access-controlled record.
   A digest stored beside a writable archive detects accidents, not a malicious
   replacement of both. Stop and report any mismatch or incomplete transfer.
3. Verify recoverability in isolation before counting a new backup as healthy.
   `restore-backup` validates chunk manifests, sizes and hashes. A structurally
   valid archive is not proof of application-level completeness. Record expected
   and recovered notes, history and state, and secret-decryption success without
   printing secret values. Never start restored sync, handlers or provider jobs.
4. Keep the last verified recovery point if copying or verification fails. Do not
   run pruning, key rotation, volume replacement or deletion from this runbook.

## Synthetic drill

Run on a development target with no production mounts, provider credentials or
network dependencies in the fixture. Each test uses fresh temporary vault and
config directories and only fabricated note content and a fabricated state secret.
The clock is not consulted for acceptance or scheduling in this drill.

From a clean checkout of the recorded commit, using the project's heavy wrapper:

```sh
set -eu
cd /src/tv-270
mkdir -p /src/tv-270-out
RUSTC_BOOTSTRAP=1 cargo test -p cli --test main \
  cli::isolated_off_host_restore_drill -- \
  -Z unstable-options --format json --report-time \
  > /src/tv-270-out/drill.json
```

The test invokes the existing `vault backup` and `vault restore-backup` commands.
It checks recovered note bytes and vault identity, refusal to decrypt the state
secret without a key, successful decryption after separately supplying the test
key, and rejection of a corrupt SQLite archive without creating the destination
or changing the successful recovery. Missing-key restore itself is allowed:
notes remain recoverable, but encrypted secrets must not become readable.
No server or provider is started. No production key is read or copied.

Also run the existing `a_restored_backup_serves_the_same_notes_and_history`
fixture for application-level list, history, trash and asset equivalence, and
`backup::tests::` for chunk corruption and staging cleanup. These are synthetic
fixtures, not a production restore. Upload original reports and record commit,
commands, exit codes, counts, failures and next drill date on the operations card.
Stop on the first unexpected failure, retain the evidence, and notify the owner.

Real recovery must use a new empty directory on an isolated host, with the
matching supported binary and enough space for staging. Supply the key only
through the approved separate custody channel. Never restore over a live vault.
Reconnection, credential use and replacement of a production volume require a
separate lead/meta decision with explicit scope and approval.
