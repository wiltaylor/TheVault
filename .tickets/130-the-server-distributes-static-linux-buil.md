---
id: '130'
title: The server distributes static Linux builds of the vault CLI (amd64, arm64)
  like the apps, and the CLI says when the server is newer
blocked_by: []
status: claimed
verified: unknown
---

# The server distributes static Linux builds of the vault CLI (amd64, arm64) like the apps, and the CLI says when the server is newer

The user decided (2026-10-18) to package the `vault` CLI and its agent skill for AI agents: the server distributes the matching CLI like it distributes the apps (A1), and Settings gets a "Connect an AI agent" panel with a one-line install (A2, next ticket). Platforms: Linux amd64 and arm64 only.

Build (A1):
1. Static `vault` CLI builds for x86_64-unknown-linux-musl and aarch64-unknown-linux-musl (check the CLI's dependencies build for musl: reqwest with rustls, no openssl; add the targets and any cross linker to docker/dev.Dockerfile, which rebuilds by hash). A justfile recipe builds both (use the justfile skill's conventions), for example `just cli-release`, writing target/cli-release/vault-<version>-linux-<arch> and SHA256SUMS.
2. `just publish-clients` also publishes both CLI builds into the clients folder and manifest as platform `cli`, kind `binary`, arch `x86_64`/`aarch64`, version and sha256, like the apps. `/api/clients` lists them; `GET /api/clients/<file>` serves them (Range already supported). Add `GET /api/clients/cli/linux-<arch>` (unauthenticated, like the other client downloads) answering the newest CLI build for that arch, so a script needs no manifest parsing. The Docker image bakes the clients folder in (ticket 127), so the CLI comes with it.
3. Version check: every `vault` command that reaches the server compares the server's version (from GET /api/version, cached for the process) with its own; when the server is newer, it prints one line to stderr ("vault <x> is older than the server <y>; update with: curl -fsSL <server>/api/clients/cli/linux-<arch> -o $(command -v vault)") and continues. stdout JSON is unchanged.
4. docs/clients.md documents the CLI downloads.

Acceptance criteria:
- Both musl binaries build through ./dev and run `vault --version` (the arm64 one under qemu if the dev image has binfmt, else record it as built-but-unrun in the worklog) and are statically linked (`file` says statically linked).
- A server test: a clients folder with CLI entries answers /api/clients with them, and /api/clients/cli/linux-x86_64 serves the right bytes; an arch with no build answers 404.
- A CLI test: against a test server reporting a newer version, a command prints the update line on stderr and its normal JSON on stdout; against the same version it prints nothing extra.
- ./dev just check passes.

## Worklog

### 2026-10-18T08:43:37Z — /work

Claimed.
