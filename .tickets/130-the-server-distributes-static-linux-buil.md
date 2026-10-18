---
id: '130'
title: The server distributes static Linux builds of the vault CLI (amd64, arm64)
  like the apps, and the CLI says when the server is newer
blocked_by: []
status: done
verified: verified
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

### 2026-10-18T09:46:20Z — wil

Verified set to verified.

### 2026-10-18T09:46:20Z — wil

Status set to done.

### 2026-10-18T09:46:20Z — wil

Built through ./dev: just cli-release (cargo zigbuild, profile cli = release + strip + thin LTO; host build-script artifacts land in target/cli, outputs in target/cli-release). vault-0.1.11-linux-x86_64 16595840 bytes, vault-0.1.11-linux-aarch64 14790160 bytes, both 'statically linked, stripped' per file(1); x86_64 runs natively, aarch64 run under qemu-aarch64 (qemu-user, no binfmt needed) printing 'vault 0.1.11', and it reached https://github.com over rustls. Zig comes from the ziglang PyPI wheel (sha256-pinned) because ziglang.org downloads stalled from this network. The dev image rebuilds (new layer at the end of docker/dev.Dockerfile). Web Apps page filters by platform android/linux, so cli builds do not show there; the Settings panel is ticket 131. docker/Dockerfile (ticket 127) untouched.
