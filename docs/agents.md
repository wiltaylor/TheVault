# Connecting an AI agent

A coding agent with a shell, such as Claude Code or Codex, drives the vault through the `vault` CLI. One command, run on the agent's machine, installs the CLI, gives it a token, and puts the vault skill where the agent reads it. An agent without a shell cannot use this.

To ask about your notes without an agent, use the AI panel in the web app, which calls your own AI provider with read-only note tools and shows each edit for you to apply. See [Chat panel](ai.md#chat-panel).

## Before you start

- The server must hand out the `vault` CLI for the agent machine's architecture, `x86_64` or `aarch64`. Run `just cli-release` and `just publish-clients` to publish it. See [The vault CLI](clients.md#the-vault-cli).
- The agent's machine runs Linux and has `curl` and `sha256sum` or `shasum`.
- You are logged in to the web app. Install codes need the owner session or an `admin` token.

## Connect the agent

1. Open Settings > API tokens.
2. Under Connect an AI agent, type a name for the agent, such as `claude`. The token it gets is `agent-<name>`.
3. Choose a scope. `write` is selected, and lets the agent read, create, change and delete notes and assets. Choose `read` for an agent that only looks things up. Choose `admin` only on purpose: an admin agent can change types, scripts and tasks, and read secrets through scripts.
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
