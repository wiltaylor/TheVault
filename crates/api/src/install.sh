#!/bin/sh
# Connects an AI agent on this machine to TheVault at the address below.
#
#   curl -fsSL <server>/agent/install.sh | sh -s -- <install code>
#
# Installs the vault CLI to ${VAULT_BIN_DIR:-$HOME/.local/bin}/vault, redeems the one-time install
# code for an API token, writes the CLI's config to ${XDG_CONFIG_HOME:-$HOME/.config}/thevault/cli.env
# and the agent skill beside it, and to ~/.claude/skills/thevault/ when ~/.claude exists. Run it
# again to update the CLI and the skill; give it a new code to replace the token too. It never
# prints the token, and it edits no agent's own files.
set -eu

VAULT_URL='@VAULT_URL@'

fail() {
	printf 'install.sh: %s\n' "$*" >&2
	exit 1
}

code="${1:-}"
case "$code" in
*[!A-Za-z0-9-]*) fail "the install code \"$code\" has characters an install code never has" ;;
esac

[ -n "${HOME:-}" ] || fail "HOME is not set"
[ "$(uname -s)" = Linux ] || fail "the vault CLI is built for Linux only; this is $(uname -s)"
machine="$(uname -m)"
case "$machine" in
x86_64 | amd64) arch=x86_64 ;;
aarch64 | arm64) arch=aarch64 ;;
*) fail "there is no vault CLI build for $machine; there are builds for x86_64 and aarch64" ;;
esac
command -v curl >/dev/null 2>&1 || fail "curl is needed"
if command -v sha256sum >/dev/null 2>&1; then
	sha256() { sha256sum "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
	sha256() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
	fail "sha256sum or shasum is needed to check the download"
fi

bin_dir="${VAULT_BIN_DIR:-$HOME/.local/bin}"
config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/thevault"
bin="$bin_dir/vault"
env_file="$config_dir/cli.env"
if [ -z "$code" ] && [ ! -f "$env_file" ]; then
	fail "give the install code from Settings > API tokens: curl -fsSL $VAULT_URL/agent/install.sh | sh -s -- <code>"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
trap 'exit 1' HUP INT TERM

# The CLI, checked against the SHA-256 the server lists for it.
curl -fsSL -D "$tmp/headers" -o "$tmp/vault" "$VAULT_URL/api/clients/cli/linux-$arch" ||
	fail "could not download the vault CLI for linux-$arch from $VAULT_URL; is it published (just publish-clients)?"
file="$(tr -d '\r' <"$tmp/headers" | sed -n 's/^[Cc]ontent-[Dd]isposition:.*filename="\([^"]*\)".*/\1/p' | tail -n 1)"
[ -n "$file" ] || fail "the server did not name the CLI build it sent"
curl -fsSL -o "$tmp/clients.json" "$VAULT_URL/api/clients" || fail "could not read $VAULT_URL/api/clients"
want="$(tr '{}' '\n\n' <"$tmp/clients.json" | grep -F "\"filename\":\"$file\"" | sed -n 's/.*"sha256":"\([0-9a-f]*\)".*/\1/p' | head -n 1)"
[ -n "$want" ] || fail "$VAULT_URL/api/clients does not list $file"
got="$(sha256 "$tmp/vault")"
[ "$got" = "$want" ] || fail "the downloaded $file has SHA-256 $got, but the server lists $want"
mkdir -p "$bin_dir"
chmod 755 "$tmp/vault"
cp "$tmp/vault" "$bin.new"
mv -f "$bin.new" "$bin"
printf 'Installed %s to %s (SHA-256 checked).\n' "$file" "$bin"

# The token, from the install code.
mkdir -p "$config_dir"
chmod 700 "$config_dir"
if [ -n "$code" ]; then
	status="$(curl -sS -o "$tmp/redeem.json" -w '%{http_code}' -X POST \
		-H 'Content-Type: application/json' -d "{\"code\":\"$code\"}" \
		"$VAULT_URL/api/agent-codes/redeem")" || fail "could not reach $VAULT_URL to redeem the install code"
	case "$status" in
	201) ;;
	410) fail "the install code is wrong, used or expired; create a new one in Settings > API tokens" ;;
	*) fail "redeeming the install code answered HTTP $status" ;;
	esac
	token="$(sed -n 's/.*"token":"\([^"]*\)".*/\1/p' "$tmp/redeem.json")"
	name="$(sed -n 's/.*"name":"\([^"]*\)".*/\1/p' "$tmp/redeem.json")"
	scope="$(sed -n 's/.*"scope":"\([^"]*\)".*/\1/p' "$tmp/redeem.json")"
	[ -n "$token" ] || fail "the server's answer held no token"
	(
		umask 077
		printf 'VAULT_URL=%s\nVAULT_TOKEN=%s\n' "$VAULT_URL" "$token" >"$env_file.new"
	)
	chmod 600 "$env_file.new"
	mv -f "$env_file.new" "$env_file"
	unset token
	printf 'Wrote %s with the server URL and the %s token %s (mode 600).\n' "$env_file" "$scope" "$name"
else
	chmod 600 "$env_file"
	printf 'Kept the token in %s; give a new install code to replace it.\n' "$env_file"
fi

# The skill, where the agent reads it.
"$bin" skill --raw >"$config_dir/SKILL.md"
printf 'Wrote the agent skill to %s.\n' "$config_dir/SKILL.md"
claude_skill=""
if [ -d "$HOME/.claude" ]; then
	claude_skill="$HOME/.claude/skills/thevault/SKILL.md"
	mkdir -p "$HOME/.claude/skills/thevault"
	cp "$config_dir/SKILL.md" "$claude_skill"
	printf 'Wrote the Claude Code skill to %s.\n' "$claude_skill"
fi

# The CLI reads cli.env when VAULT_URL and VAULT_TOKEN are not set.
(
	unset VAULT_URL VAULT_TOKEN
	"$bin" health >/dev/null 2>&1 || fail "vault health could not reach the server"
	"$bin" schema >/dev/null 2>&1 || fail "the server refused the token in $env_file"
)
printf 'vault health: the server answers and accepts the token.\n'

case ":${PATH:-}:" in
*":$bin_dir:"*) ;;
*) printf 'Add %s to PATH so the agent finds vault.\n' "$bin_dir" ;;
esac
if [ -n "$claude_skill" ]; then
	printf 'Claude Code loads the thevault skill from %s.\n' "$claude_skill"
fi
shown="$config_dir/SKILL.md"
case "$shown" in
"$HOME"/*) shown="~${shown#"$HOME"}" ;;
esac
printf 'For any other agent, add this line to its instructions file (AGENTS.md or similar):\n'
printf '  Before using the vault, read %s; the vault CLI is configured.\n' "$shown"
