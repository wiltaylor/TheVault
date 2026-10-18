---
id: '131'
title: 'Connect an AI agent: one-time install codes, a served install.sh that installs
  the CLI, its config and the skill, and a Settings panel with the one-line command'
blocked_by:
- '130'
status: todo
verified: unknown
---

# Connect an AI agent: one-time install codes, a served install.sh that installs the CLI, its config and the skill, and a Settings panel with the one-line command

Builds on the CLI distribution ticket (static Linux CLI served at /api/clients/cli/linux-<arch>). The user decided (2026-10-18): a "Connect an AI agent" panel with a one-line install for coding agents that have a shell; default token scope write, admin a deliberate choice; no Claude Code plugin yet; no MCP yet.

Build (A2):
1. Install codes: an admin creates a one-time agent install code (POST /api/agent-codes { name, scope: read|write|admin }), valid 10 minutes, single use, like device pair codes. Exchanging it (POST /api/agent-codes/redeem { code }, unauthenticated) creates an API token named `agent-<name>` with that scope and returns { server_url, token, name, scope }. A used or expired code answers 410. The token shows in Settings > Access like any other token and can be revoked there.
2. `GET /agent/install.sh` (unauthenticated, served by the server, generated with the server's URL as the browser sees it, the same way pairing works out server_url): a POSIX sh script taking the code as its first argument. It detects the arch (x86_64 or aarch64, else a clear error), downloads the CLI from /api/clients/cli/linux-<arch> to ${VAULT_BIN_DIR:-$HOME/.local/bin}/vault and verifies its sha256 against /api/clients, redeems the code, writes $XDG_CONFIG_HOME/thevault/cli.env (default ~/.config/thevault/cli.env, mode 600) with VAULT_URL and VAULT_TOKEN, writes the skill (`vault skill` output) to ~/.config/thevault/SKILL.md and, when ~/.claude exists, to ~/.claude/skills/thevault/SKILL.md, runs `vault health`, and prints what it did plus, for agents other than Claude Code, one line to add to their instructions file ("Before using the vault, read ~/.config/thevault/SKILL.md; the vault CLI is configured.") without editing any agent's files itself. Running it again updates the CLI and skill and replaces the token only when given a new code. It never prints the token.
3. CLI: when VAULT_URL or VAULT_TOKEN is not set, `vault` reads them from $XDG_CONFIG_HOME/thevault/cli.env (simple KEY=VALUE lines); flags and environment still win. Document in the skill's "Before you start" section (crates/cli/skill/thevault/SKILL.md) and `vault --help`.
4. Web: Settings > Access gains "Connect an AI agent": a name, a scope choice (write selected; admin shows a one-line warning that it can change types, scripts and tasks and read secrets through scripts), a Create button that shows the command `curl -fsSL <server>/agent/install.sh | sh -s -- <code>` with a Copy button and the 10-minute expiry, and a note that the agent needs a shell. Agent tokens are listed with the others.
5. Docs: docs/clients.md (or a new docs/agents.md, linked from README) with the steps, what the script writes where, how to revoke, and how to uninstall (delete the binary, cli.env and the skill files).

Acceptance criteria:
- Server tests: create code (admin only), redeem once (token with the chosen scope works for that scope only), reuse answers 410, expiry answers 410, install.sh is served with the server URL in it.
- A script test (shell, in a temp HOME, against a test server with a clients folder holding the x86_64 CLI build): running the one-liner with a fresh code installs the binary, writes cli.env with mode 600, writes both skill files when ~/.claude exists and only the config one when not, `vault health` and `vault list` work with no env set, and the token never appears in the script's output.
- CLI test: cli.env is read when env is unset; env and flags override it.
- Web component tests for the panel (scope default write, admin warning, the command shown with copy).
- ./dev just check passes.

## Worklog
