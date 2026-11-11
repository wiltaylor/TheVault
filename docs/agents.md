# Connecting an AI agent

A coding agent with a shell, such as Claude Code or Codex, drives the vault through the `vault` CLI. One command, run on the agent's machine, installs the CLI, gives it a token, and puts the vault skill where the agent reads it. An agent without a shell cannot use this.

A CLI agent using an `agent-<name>` token drafts Outbox items and the user sends them. It can create and edit only its own drafts, cannot mark them ready or write send results, and cannot use an in-app agent's Send without review permission. Calendar invitations, RSVP changes, cancellations and Google event deletes must go through the Outbox for review. This also applies to existing agent tokens.

Token kind is stored as user, device or agent. On upgrade, existing `agent-*` tokens become agent tokens and `device-*` tokens become device tokens, without regard to letter case. This includes user-created tokens with those names. New user tokens cannot use either prefix. Agent tokens are at most write scope, including existing admin-scoped agent tokens, which are downgraded at upgrade and capped whenever they authenticate. They cannot create credentials, change agent definitions or impersonate in-app agents when applying proposals. Their drafts are attributed to the full token name, such as `agent-cli`, separate from the in-app agent `cli`. Agents may delete or restore only their own Outbox drafts.

Agent tokens cannot run in-app agents or use caller-built `/api/ai/apply` requests, even for their own drafts. They cannot change any integration settings, accounts, keys or connections. New token names cannot use the reserved prefixes in any letter case.

To ask about your notes without an agent, use the AI panel in the web app or the desktop and Android apps, which calls your own AI provider with read-only note tools and shows each edit for you to apply. See [Chat panel](ai.md#chat-panel).

## Before you start

### Authentication scopes

| Credentials | Agent conversations | Agent and skill definitions |
|---|---|---|
| `read` token | Read only, no new turns or threads | Read only |
| `write` token, including a paired device | Start turns and threads, direct changes capped at write | No create, edit, delete, import or Apply |
| Non-agent `admin` API token | Direct changes capped by the agent's permissions | Manage definitions and Apply, subject to proposal guards |
| `agent-<name>` token, even admin | No turns, thread creation or fork, answers, truncate, discard, dispatch, Apply or memory writes | No agent/skill definition writes, credential creation, Assistant settings changes or git-sync mutations |
| Signed-in session | Direct changes capped by the agent's permissions | Manage definitions and Apply |

Chat mutations, including stopping a turn and discarding proposals, require write. Query POSTs and authorized event reads still require only read. Autonomous triggers use the agent's scope. Explicit dispatch runs retain their caller's scope ceiling.

- The server must hand out the `vault` CLI for the agent machine's architecture, `x86_64` or `aarch64`. Run `just cli-release` and `just publish-clients` to publish it. See [The vault CLI](clients.md#the-vault-cli).
- The agent's machine runs Linux and has `curl` and `sha256sum` or `shasum`.
- You are logged in to the web app. Install codes need the owner session or an `admin` token.

## Connect the agent

1. Open Settings > API tokens.
2. Under Connect an AI agent, type a name for the agent, such as `claude`. The token it gets is `agent-<name>`.
3. Choose a scope. `write` is selected, and lets the agent read, create, change and delete notes and assets. A deleted note goes to the trash, and the agent can list the trash and restore from it, but no agent can empty the trash or delete a note forever, whatever its scope: only you can, signed in to the web app. Choose `read` for an agent that only looks things up. Choose `admin` only on purpose: an admin agent can change types, scripts and tasks, and read secrets through scripts.
4. Select Create install command. The panel shows a command like this one:

   ```sh
   curl -fsSL https://vault.example/agent/install.sh | sh -s -- K7QM-3XRT
   ```

5. Select Copy, and run the command in the agent's shell within 10 minutes. The code works once.

The command prints what it did. It never prints the token. On a machine without Claude Code, it ends with one line for you to add to the agent's instructions file, such as `AGENTS.md`:

```text
Before using the vault, read ~/.config/thevault/SKILL.md; the vault CLI is configured.
```

The script edits no agent's files itself.

## What the script writes

| Path | What it is |
|---|---|
| `${VAULT_BIN_DIR:-~/.local/bin}/vault` | The CLI, downloaded from `/api/clients/cli/linux-<arch>` and checked against the SHA-256 that `/api/clients` lists |
| `~/.config/thevault/cli.env` | `VAULT_URL` and `VAULT_TOKEN`, mode 600 |
| `~/.config/thevault/SKILL.md` | The skill, the output of `vault skill --raw` |
| `~/.claude/skills/thevault/SKILL.md` | The same skill, written only when `~/.claude` exists |

When `XDG_CONFIG_HOME` is set, `$XDG_CONFIG_HOME/thevault` replaces `~/.config/thevault`. Set `VAULT_BIN_DIR` in front of `sh` to install the binary somewhere else:

```sh
curl -fsSL https://vault.example/agent/install.sh | VAULT_BIN_DIR="$HOME/bin" sh -s -- K7QM-3XRT
```

After it writes the files, the script runs `vault health` and `vault schema` with `VAULT_URL` and `VAULT_TOKEN` unset, so it fails when the server or the token does not work. It also says so when the binary's folder is not on `PATH`.

`vault` reads `cli.env` only for a setting that neither its flag nor its environment variable gives. `--url` and `VAULT_URL` win over the file's `VAULT_URL`, and `--token` and `VAULT_TOKEN` win over its `VAULT_TOKEN`.

The script reaches the server at the address the panel's command names, which is the address your browser used. When the agent reaches the server at a different address, edit the URL in the command before running it. The script puts the address it was fetched from into `cli.env`.

## Update the CLI and the skill

Run the script again without a code. It replaces the binary and both skill files and keeps the token in `cli.env`:

```sh
curl -fsSL https://vault.example/agent/install.sh | sh
```

To replace the token as well, create a new install command with the same agent name and run it. Redeeming the new code revokes the old `agent-<name>` token and writes the new one to `cli.env`.

A code that was already used, or is older than 10 minutes, fails with HTTP 410 and changes nothing. Create a new one.

## Revoke the agent's access

Open Settings > API tokens and select Revoke on the `agent-<name>` token. The token stops working at once. The CLI and the skill stay on the agent's machine until you delete them.

## Uninstall

On the agent's machine, delete the binary, the config and the skill files:

```sh
rm -f ~/.local/bin/vault ~/.config/thevault/cli.env ~/.config/thevault/SKILL.md
rm -rf ~/.claude/skills/thevault
```

Then revoke the token as above, and remove the line you added to the agent's instructions file.

## The API

| Endpoint | Credentials | What it does |
|---|---|---|
| `POST /api/agent-codes` `{ name, scope, server_url? }` | `admin` | Issues a code that works once for 10 minutes. Answers `{ code, expires, name, scope, server_url, command }`. `server_url` defaults to the address the request came in on. |
| `POST /api/agent-codes/redeem` `{ code }` | None | Creates the `agent-<name>` token, replacing one of that name. Answers 201 `{ server_url, token, name, scope }`, or 410 for a wrong, used or expired code. |
| `GET /agent/install.sh` | None | The install script, with the server's URL as the request reached it. Behind a proxy, the server reads `X-Forwarded-Host` and `X-Forwarded-Proto`. |

## External CLI agents

Agent tokens cannot run even an in-app agent with a matching name. In-app agents
use the agent tool for sub-agents, not the chat API.

# External agents and held notes

An external agent can use the filtered `/api/query` (GET or POST), `/api/search`,
and `/api/notes` list. These responses exclude held notes and include the fixed
notice `some notes may be held`. Current-note reads, note graphs and attachment
downloads check clearance too. Graphs omit held neighbours. Script tools run
with the external agent's full token name as their agent execution author.

Agent tokens cannot read history or historical versions, original email HTML,
canvases, sync changes, event streams, trash, pins, notifications, activity,
flagged-note lists, asset lists, asset trash or asset usage. Daily/planner aliases
and aggregate agent-thread, memory, run and dispatch lists are also refused.
Use filtered queries and current-note reads instead. These historical and
aggregate representations do not have a content-bound clearance of their own.
Normal user sessions retain access. Only a user session may release a held note.

