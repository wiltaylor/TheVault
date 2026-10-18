---
id: '128'
title: Compose file with default, https (Caddy) and cloudflare (tunnel) profiles,
  .env.example, and docs/install.md
blocked_by:
- '127'
status: done
verified: verified
---

# Compose file with default, https (Caddy) and cloudflare (tunnel) profiles, .env.example, and docs/install.md

Builds on the image ticket (docker/Dockerfile, /data and /config volumes, VAULT_PASSWORD required on first run, port 8080, clients baked in).

Build:
1. `deploy/compose.yaml` and `deploy/.env.example`:
   - Service `thevault`: image ghcr.io/wiltaylor/thevault:${THEVAULT_VERSION:-latest}, restart unless-stopped, volumes for /data and /config (named volumes by default; the .env can switch to host paths), env from .env (VAULT_PASSWORD, TZ), ports `${THEVAULT_PORT:-8080}:8080` bound to all interfaces by default with a comment on binding to 127.0.0.1 when a proxy or tunnel is used, healthcheck.
   - Profile `https`: service `caddy` (official image) serving ${THEVAULT_DOMAIN} with automatic TLS and reverse_proxy to thevault:8080, with a Caddyfile in deploy/ (or inline via a config), volumes for Caddy data and config, ports 80 and 443. The `thevault` port need not be published in this profile (document).
   - Profile `cloudflare`: service `cloudflared` (official image) running `tunnel --no-autoupdate run` with TUNNEL_TOKEN=${CLOUDFLARE_TUNNEL_TOKEN} from .env, depending on thevault being healthy. The public hostname is mapped to http://thevault:8080 in the Cloudflare dashboard (document the exact steps). No host ports needed.
   - .env.example documents every variable with a one-line comment, VAULT_PASSWORD first with a note that it is only used to create the vault on first start and how to change it later (`docker compose exec thevault vault passwd`).
2. `docs/install.md` (human-facing; use the technical-writing standard: plain, task-ordered): prerequisites (Docker, docker login ghcr.io while the image is private, with a fine-grained token with read:packages), install (download compose.yaml and .env.example from the GitHub Release, set the password, `docker compose up -d`), first login, pairing the phone, choosing access: LAN or Tailscale (default profile), your own domain (`--profile https`), Cloudflare Tunnel (`--profile cloudflare`), Google sign-in notes per access method (https or tunnel origin can use the direct redirect), backups (`docker compose exec thevault vault backup`, where the automatic pre-upgrade backups land, and that the key in /config must be backed up separately), upgrades (`docker compose pull && docker compose up -d`; apps update themselves), restore, and uninstall. Link it from README.

Acceptance criteria:
- `docker compose config` succeeds for the default, `--profile https` and `--profile cloudflare` combinations with a filled-in .env.
- The default profile, run locally against an image built from this checkout (override the image tag in a test .env), comes up healthy on a free port and the login works; the test removes its containers and volumes.
- The https profile, run locally with Caddy's internal TLS for a test domain (an override file for the test only), serves the app over https; document that a real domain uses automatic certificates.
- The cloudflare profile cannot be proven without a Cloudflare account: verify the config renders, the container starts and logs that it needs a valid token when given a dummy one, and record the real-tunnel check as unmet in the worklog for the user to try.
- docs/install.md covers every item above; README links it.
- ./dev just check passes.

## Worklog

### 2026-10-18T09:03:39Z — /work

Claimed.

### 2026-10-18T09:36:17Z — wil

Verified set to verified.

### 2026-10-18T09:36:17Z — wil

Proved with 'just compose-test' (docker/test-compose.sh, on the host, image built from this checkout and tagged ghcr.io/wiltaylor/thevault:<run> so compose.yaml runs unchanged with THEVAULT_VERSION from a test .env): 74 PASS, 0 FAIL, nothing left behind; './dev just check' exit 0. Covers: .env.example documents every variable compose reads (VAULT_PASSWORD first, passwd command named); config renders default, https, cloudflare and COMPOSE_PROFILES=https; default profile healthy on 127.0.0.1:<free>, login 200 and wrong 401, vault passwd, vault backup, docker compose cp, and restore-backup into a new data volume with login after; https profile through Caddy tls internal (docker/compose-test/https.yaml with ports !override) verified against Caddy's root CA for vault.test; cloudflared started after thevault healthy and logged 'Provided Tunnel token is not valid.' for a dummy token. UNMET by this ticket, for the user to try: a real Cloudflare Tunnel (needs an account) and a real Let's Encrypt certificate (needs a public domain). Notes for ticket 129: GHCR does not accept fine-grained tokens, so docs/install.md says classic token with read:packages, not what the ticket said; install.md downloads releases/latest/download/{compose.yaml,Caddyfile,.env.example}, so 129 must attach those names (check GitHub keeps the leading dot of .env.example). cloudflared has no major-version tag, so it is pinned to 2026.10.1 via CLOUDFLARED_VERSION. vault backup does not create its folder (ticket 133); install.md runs mkdir -p first. docker/Dockerfile not touched.

### 2026-10-18T09:36:31Z — wil

Status set to done.
