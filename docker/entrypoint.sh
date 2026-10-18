#!/bin/sh
# The image's entrypoint, run under tini as uid 1000:
#
# 1. /data holding no vault: create one with `vault init`, the password from VAULT_PASSWORD_FILE
#    or VAULT_PASSWORD, the secret key at VAULT_SECRET_KEY_FILE (OQ-S2, OQ-S3). Neither set is a
#    one-line refusal. A vault made outside the container and mounted in is never re-created.
# 2. The vault last ran with an older version than this image's, or with one not recorded: write
#    `vault backup` to /data/backups/<old>-to-<new>-<time>.sqlar first, keeping the newest five.
#    The backup leaves the secret key out, as `vault backup` does by default.
# 3. Record this image's version in /data/.vault/server-version and exec vault-server on
#    0.0.0.0:8080, serving the web client and the clients folder VAULT_CLIENTS_DIR names.
#
# Extra arguments are passed to vault-server. Every line goes to stdout or stderr.
set -eu

data=/data
backups="$data/backups"
keep=5
version=$(cat /opt/thevault/VERSION)
record="$data/.vault/server-version"

# `vault` prints its result, and its errors, as JSON on stdout. Run it with that kept for the log
# only when it fails.
quiet() {
  if ! result=$("$@"); then
    printf '%s\n' "$result" >&2
    return 1
  fi
}

log() {
  printf 'thevault: %s\n' "$*" >&2
}

if [ ! -w "$data" ]; then
  log "/data is not writable by uid $(id -u); give the volume to $(id -u):$(id -g)"
  exit 1
fi

if [ ! -f "$data/.vault/state.db" ]; then
  if [ -n "${VAULT_PASSWORD_FILE:-}" ]; then
    if [ ! -r "$VAULT_PASSWORD_FILE" ]; then
      log "VAULT_PASSWORD_FILE names $VAULT_PASSWORD_FILE, which cannot be read"
      exit 1
    fi
    quiet vault init "$data" --password-stdin < "$VAULT_PASSWORD_FILE"
  elif [ -n "${VAULT_PASSWORD:-}" ]; then
    # printf is a shell builtin, so the password is on no command line.
    printf '%s' "$VAULT_PASSWORD" | quiet vault init "$data" --password-stdin
  else
    log "/data holds no vault: set VAULT_PASSWORD in .env to the login password the new vault should have"
    exit 1
  fi
  log "created a vault in /data with its secret key at $VAULT_SECRET_KEY_FILE"
  printf '%s\n' "$version" > "$record"
fi
# The password is only for the first run; the server never needs it.
unset VAULT_PASSWORD VAULT_PASSWORD_FILE

# A vault mounted in from outside may bring no key file. The server then looks for
# $XDG_CONFIG_HOME/thevault/<vault-id>.key, and without either, secrets cannot be read or written.
if [ -n "${VAULT_SECRET_KEY_FILE:-}" ] && [ ! -e "$VAULT_SECRET_KEY_FILE" ]; then
  log "no secret key at $VAULT_SECRET_KEY_FILE; looking for $XDG_CONFIG_HOME/thevault/<vault-id>.key instead"
  unset VAULT_SECRET_KEY_FILE
fi

last=$(cat "$record" 2> /dev/null || true)
if [ "$last" != "$version" ]; then
  newest=$(printf '%s\n%s\n' "$last" "$version" | sort -V | tail -n 1)
  if [ -n "$last" ] && [ "$newest" != "$version" ]; then
    log "the vault last ran with $last, newer than this image's $version; starting without a backup"
  else
    mkdir -p "$backups"
    out="$backups/${last:-unknown}-to-$version-$(date -u +%Y%m%dT%H%M%SZ).sqlar"
    if ! result=$(vault --vault "$data" backup "$out"); then
      log "the backup before upgrading from ${last:-an unrecorded version} to $version failed; not starting"
      printf '%s\n' "$result" >&2
      exit 1
    fi
    log "backed up the vault before upgrading from ${last:-an unrecorded version} to $version: $out"
    # The newest $keep upgrade backups stay; names hold no spaces.
    # shellcheck disable=SC2012
    ls -1t "$backups"/*-to-*.sqlar | tail -n +$((keep + 1)) | while read -r old; do
      rm -f "$old"
      log "removed the old backup $old"
    done
    printf '%s\n' "$version" > "$record"
  fi
fi

exec vault-server --vault "$data" --web /opt/thevault/web --listen 0.0.0.0:8080 "$@"
