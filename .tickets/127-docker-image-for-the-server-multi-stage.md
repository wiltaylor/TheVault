---
id: '127'
title: 'Docker image for the server: multi-stage build, non-root, /data and /config
  volumes, first-run init requiring VAULT_PASSWORD, backup before an upgrade, the
  version''s APK and AppImage baked in'
blocked_by: []
status: todo
verified: unknown
---

# Docker image for the server: multi-stage build, non-root, /data and /config volumes, first-run init requiring VAULT_PASSWORD, backup before an upgrade, the version's APK and AppImage baked in

The user decided (2026-10-18) to distribute TheVault as a Docker image on GitHub's container registry with a compose file. Answers: amd64 only for now; each image carries the signed APK and AppImage of its own version; first-run password required in .env (refuse to start without it); registry and repo private for now (users run docker login ghcr.io), public soon.

Build the image (this ticket; compose and CI are tickets after it):
1. `docker/Dockerfile` (keep docker/dev.Dockerfile as is): multi-stage. Build stage compiles `vault-server` and the `vault` CLI in release mode and builds the web client (`just web-build` equivalent). Runtime stage: debian:bookworm-slim (or the slimmest base the glibc binary runs on), the two binaries in /usr/local/bin, the web build in /opt/thevault/web, ca-certificates (Gmail and Calendar use rustls with bundled roots, but scripts' http and the OAuth exchange should still work; confirm), tini or equivalent as PID 1, a non-root user (uid 1000, gid 1000), EXPOSE 8080, HEALTHCHECK on /api/health. linux/amd64.
2. Client builds: a build context directory (for example `clients/` passed with `--build-context clients=<dir>` or a build arg naming it) holding the APK, the AppImage and the manifest.json that `just publish-clients` writes, copied to /opt/thevault/clients; the server runs with VAULT_CLIENTS_DIR=/opt/thevault/clients. An image built without that context still works and serves no clients (for local and test builds). Add a justfile recipe, for example `just image` (and `just image-with-clients <dir>`), using the justfile skill's conventions.
3. Volumes: /data holds the vault, /config holds the secret key file (VAULT_SECRET_KEY_FILE=/config/vault.key). Both owned by the runtime user.
4. Entrypoint (a small shell script or a Rust subcommand, whichever is simpler to test):
   - /data empty or holding no vault: require VAULT_PASSWORD (or VAULT_PASSWORD_FILE pointing at a file); if neither is set, print one clear line saying to set VAULT_PASSWORD in .env and exit non-zero. Otherwise run `vault init /data` with the password on stdin (it creates the key file at VAULT_SECRET_KEY_FILE; check `vault init --help` for how it places the key). Never print the password.
   - Before every start: if the vault's recorded server version (find where the server records the version a vault last ran, or record one in /data/.vault) is older than this image's version, write `vault backup` to /data/backups/<old>-to-<new>-<timestamp>.tar (or the backup command's format), keeping the newest 5, and log the path. The backup excludes the key, as `vault backup` does by default.
   - Then exec vault-server --vault /data --web /opt/thevault/web --listen 0.0.0.0:8080 (clients dir from the env).
   - A vault created by `vault init` outside the container and mounted in must also work (the entrypoint only inits an empty /data).
5. Logs go to stdout and stderr.

Acceptance criteria (prove by running Docker on this host; the host has docker):
- `docker build` produces a linux/amd64 image; its size is reported in the worklog with and without clients.
- Started with an empty /data and no VAULT_PASSWORD it exits non-zero with the one-line message.
- Started with VAULT_PASSWORD on a free port: /api/health answers, login with that password works, /api/version reports the workspace version, the healthcheck turns healthy, the process runs as uid 1000.
- With a clients context built from the current published clients (copy from /home/wil/dev/TheVault/.scratch/phone/vault/.vault/clients, read only), /api/clients lists the APK and AppImage and downloads one with the right sha256.
- Upgrade backup: starting the same /data with an image whose version is newer (a build arg can override the reported version for the test, or edit the recorded version) writes one backup file and logs it; a second start writes none.
- A test script (for example docker/test-image.sh) runs these checks end to end, binds only free ports, uses temp dirs, removes its containers and images, and never touches the live server on 100.81.94.109:8787 or its vault.
- ./dev just check passes.

## Worklog
