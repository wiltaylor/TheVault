# Install TheVault with Docker Compose

This guide runs the TheVault server on one machine with Docker Compose. It covers the first start, how you reach the server, backups, upgrades, restoring, rolling back and removing it.

The server is one container, `thevault`. It keeps the vault in the `/data` volume and the vault's secret key in the `/config` volume. Two optional profiles add a front door: `https` adds Caddy for your own domain, and `cloudflare` adds a Cloudflare Tunnel. A third, `browser`, adds the [browser box](#browser-box), a desktop where agents drive a web browser.

## What you need

- A Linux machine with Docker Engine and the Docker Compose plugin, version 2.20 or later. Run `docker compose version` to check.
- The image is `linux/amd64`, so the machine needs an x86-64 processor.

The image, `ghcr.io/wiltaylor/thevault`, is public, so no registry login is needed.

## Install

1. Make a folder for TheVault and go into it:

   ```sh
   mkdir thevault && cd thevault
   ```

2. Download `compose.yaml`, `Caddyfile` and `env.example` from the latest GitHub Release, saving `env.example` as `.env`:

   ```sh
   base=https://github.com/wiltaylor/TheVault/releases/latest/download
   curl -fLO "$base/compose.yaml"
   curl -fLO "$base/Caddyfile"
   curl -fL -o .env "$base/env.example"
   ```

   The same files are in `deploy/` in the repository, where `env.example` is `deploy/.env.example`. The release names it without the dot because GitHub renames a release file whose name starts with one.

3. Open `.env` and set `VAULT_PASSWORD` to the password you want to log in with.
4. Start the server:

   ```sh
   docker compose up -d
   ```

5. Wait until `docker compose ps` shows `thevault` as `healthy`. The first start takes a few seconds.

On its first start the container sees that `/data` holds no vault. It creates one with `VAULT_PASSWORD` as the login password, and writes the secret key to `/config/vault.key`. If `VAULT_PASSWORD` is empty, the container stops with the line `/data holds no vault: set VAULT_PASSWORD in .env ...`. Set it and run `docker compose up -d` again.

Every later start ignores `VAULT_PASSWORD`. After the first login you can delete the value from `.env`.

### Keep the vault in host folders

By default the vault and the key live in the named volumes `thevault_data` and `thevault_config`. To keep them in folders on the host instead, create the folders, give them to uid 1000, and name them in `.env`:

```sh
mkdir data config
sudo chown 1000:1000 data config
```

```sh
THEVAULT_DATA=./data
THEVAULT_CONFIG=./config
```

The container runs as uid 1000. If it cannot write to `/data`, it stops and says so.

## Log in for the first time

1. Open `http://<server>:8080` in a browser. `THEVAULT_PORT` in `.env` sets the port.
2. Enter the password from `VAULT_PASSWORD` and log in.

The browser stays logged in for 30 days from your last visit, so a browser you use at least once a month never asks again. The server keeps its sessions in `state.db`, so restarting or upgrading the container does not log you out. Logging out ends that browser's session at once.

To change the password later, run this and type the new password twice:

```sh
docker compose exec thevault vault passwd
```

Changing the password logs every browser out. Log in again with the new password.

## Pair your phone

The server hands out the Android app itself, when the image was built with it.

1. On the phone, open the server's address in the browser and select **Get the apps** on the login page.
2. Download the APK and install it. Android asks you to allow installs from the browser the first time.
3. In the web app on a computer, open **Settings > Devices** and select **Pair a device**. The page shows a pairing code and its QR code.
4. Open TheVault on the phone and select **Scan pairing QR code**. Scan the code, then select **Pair**.

You can also type the server address and the vault password on the phone instead of scanning. The phone must reach the server at the address in the QR code. That is the address the web app was opened at, so open the web app at the address the phone uses.

The desktop app pairs the same way: enter the server address and either the password or the pairing code.

## Choose how you reach the server

Pick one of the three. Each one changes only `.env`, plus a few steps outside the server for the domain and tunnel options.

| You reach TheVault | Profile | Ports on the host |
|---|---|---|
| On your home network or over Tailscale | none | `THEVAULT_PORT`, 8080 by default |
| At your own domain, with HTTPS | `https` | 80 and 443 |
| At a domain on Cloudflare, through a tunnel | `cloudflare` | none |

To make every `docker compose` command use a profile, set `COMPOSE_PROFILES` in `.env`, for example `COMPOSE_PROFILES=https`. The rest of this guide assumes you did. Otherwise, pass `--profile https` or `--profile cloudflare` to every `docker compose` command, including `pull` and `down`.

### Home network or Tailscale

This is the default and needs no profile. The server listens on port 8080 on every interface of the host. Open `http://<lan-address>:8080`, or `http://<tailscale-address>:8080` from a device on your tailnet.

To reach the server from Tailscale only, set `THEVAULT_PORT` to the host's Tailscale address and port, for example `THEVAULT_PORT=100.64.0.10:8080`. Docker then cannot start the container while that address is down, so Tailscale must be up before Docker starts it.

### Your own domain

Caddy serves your domain over HTTPS and gets a certificate from Let's Encrypt on its own. It renews the certificate without any help from you.

1. Point a DNS `A` record for the domain, such as `vault.example.com`, at the server's public address.
2. Make ports 80 and 443 on the server reachable from the internet. Let's Encrypt connects to port 80 or 443 to check that you own the domain.
3. In `.env`, set these three values:

   ```sh
   COMPOSE_PROFILES=https
   THEVAULT_DOMAIN=vault.example.com
   THEVAULT_PORT=127.0.0.1:8080
   ```

   `THEVAULT_PORT=127.0.0.1:8080` keeps the plain HTTP port to the server itself, so only Caddy serves the app to the network. Caddy reaches the app over the compose network and does not need that port.

4. Run `docker compose up -d`.
5. Open `https://vault.example.com`. The first visit can take a few seconds while Caddy gets the certificate. `docker compose logs caddy` shows its progress.

Caddy keeps its certificates in the `thevault_caddy-data` volume. To change the proxy, edit `Caddyfile` beside `compose.yaml` and run `docker compose restart caddy`.

### Cloudflare Tunnel

A tunnel connects out from the server to Cloudflare, so you open no ports on your router. You need a Cloudflare account with a domain on it.

1. In the [Cloudflare dashboard](https://one.dash.cloudflare.com/), go to **Networks > Tunnels** and select **Create a tunnel**.
2. Choose **Cloudflared** as the connector type, name the tunnel, for example `thevault`, and save it.
3. On the install page, choose **Docker**. Copy the token, which is the long string after `--token` in the command shown. Do not run that command.
4. In `.env`, set these three values:

   ```sh
   COMPOSE_PROFILES=cloudflare
   CLOUDFLARE_TUNNEL_TOKEN=<the token>
   THEVAULT_PORT=127.0.0.1:8080
   ```

5. Run `docker compose up -d`. Back in the dashboard, the connector shows as connected after a few seconds.
6. Select **Next**. Under **Public hostname** (**Published application routes** in newer dashboards), add a route:
   - **Subdomain** and **Domain**: the address you want, for example `vault` and `example.com`.
   - **Service type**: `HTTP`.
   - **URL**: `thevault:8080`.
7. Save the tunnel and open `https://vault.example.com`.

The URL is `thevault:8080` because `cloudflared` shares the Compose `proxy` network with the server and finds it by its service name. Cloudflare provides the HTTPS certificate.

If `docker compose logs cloudflared` shows `Provided Tunnel token is not valid.`, copy the token again into `.env` and run `docker compose up -d`.

The `cloudflared` image is pinned to `2026.10.1` by `CLOUDFLARED_VERSION` in `.env`. Cloudflare publishes no major-version tag. Cloudflare supports a `cloudflared` release for one year, so raise the version at least once a year.

## Set up Google sign-in

Gmail and Google Calendar sync need an OAuth client in your own Google Cloud project. [Connecting Gmail and Google Calendar](google.md) has the full setup. The access method decides which client type you create:

- Home network or Tailscale on `http://`: create a **Desktop app** client. After you approve in Google, you paste the address from the browser back into TheVault.
- Your own domain or a Cloudflare Tunnel on `https://`: create a **Web application** client with the redirect URI `https://<your-domain>/integrations/google/callback`. Google sends the browser straight back to TheVault.

If Settings > Integrations shows `https://oauth2.googleapis.com/token: could not connect` for Gmail or Calendar, the container cannot look up names. Check the host's `/etc/resolv.conf`: when it names no `nameserver`, Docker has no DNS server to give the container. Uncomment the `dns:` line under the `thevault` service in `compose.yaml` and run `docker compose up -d`.

## Browser box

The browser box is an optional second container, `browser`, from the image `ghcr.io/wiltaylor/thevault-browser`. It runs an XFCE desktop with Chromium and [`playwright-cli`](https://playwright.dev/agent-cli/introduction). An agent that has the **Browser automation** permission runs shell commands in the box and drives the browser with `playwright-cli`, while you watch the desktop from the web app. The permission is off for every agent until you turn it on.

### Turn it on

1. Make a token that the server and the box share:

   ```sh
   openssl rand -hex 32
   ```

2. In `.env`, set `BROWSER_TOKEN` to the token, and add `browser` to `COMPOSE_PROFILES`, such as `COMPOSE_PROFILES=browser` or `COMPOSE_PROFILES=https,browser`.
3. Start it:

   ```sh
   docker compose up -d
   ```

   Without `COMPOSE_PROFILES`, name the profile on every command instead: `docker compose --profile browser up -d`.
4. Wait until `docker compose ps` shows `browser` as `healthy`.

The box refuses to start without `BROWSER_TOKEN`. The server gets the box's address, `http://browser:7700`, and the token only when `BROWSER_TOKEN` is set, so leave it empty while the box is off.

To watch the desktop or take it over, choose **Watch** on a browser step in an agent's conversation, which opens the desktop in a pane beside it, or **Open browser box** in the command palette, or **Settings > Integrations > Browser box > Open desktop**, all in the web app; **Open in a new tab** gives it a browser tab of its own. The card also says whether the box answers, and which box and `playwright-cli` versions it runs. The server shows the desktop only to a password login: it proxies `/browser/` to the box's noVNC, and refuses every API token, `admin` included, with 403 `human_only`, so no agent, script or app reaches the desktop. The desktop is not offered in the desktop and Android apps. The box publishes no ports, and it shares a network only with `thevault`.

The server finds noVNC on port 6080 of the host in `VAULT_BROWSER_URL`, such as `http://browser:6080`. To run noVNC somewhere else, set `VAULT_BROWSER_DESKTOP_URL` on the `thevault` service to its `http://` address. When `VAULT_BROWSER_URL` is empty, the server has no `/browser/` at all.

### What it keeps and uses

- `/home/agent` is the named volume `thevault_browser-home`. Each agent gets a separate managed directory at `/home/agent/profiles/<opaque-id>`, containing its home, cache, runtime files and working directory. The server chooses and signs the identity, never the model. Missing or forged identities are refused. Existing shared logins are not copied into new profiles. Open sites with `playwright-cli open <url> --headed --persistent` to retain logins across browser restarts. Use relative paths for screenshots, captures and installed skills, not the old shared `/work` directory.
- Profiles survive turns and display-label changes. Deleting an agent retires its profile and requests cleanup from the box. If the box is offline, cleanup is retried before later browser turns. Creating another agent with the same name gets an empty profile. Throwaway sub-agents get fresh profiles and request cleanup when their run ends, including cancellation. A box outage can delay cleanup.
- Upgrade the server and rebuild the browser-box image together for this protocol change. An old box cannot enforce the signed-profile protocol. Deployments using a locally built box must rebuild it, not just update the server's version pin.
- At most 64 profiles are admitted. Commands are refused when their profile exceeds 512 MiB, with a clear error and no eviction. This is an admission check, not a hard filesystem quota. File reads and profile removal remain available for cleanup.
- The owner can choose **Reset browser profile** on the agent's Permissions page. This signs out and clears only that agent's browser files, including an over-quota profile. It needs a signed-in owner session or an admin user token, never an agent or device token. If the box is offline the reset is recorded and cleanup retries later. A throwaway's identity is recorded before its first browser request; startup retires any left pending by a crash. Retired IDs stay in one durable JSON index so stale signed identities cannot recreate profiles. The index grows by one compact ID per retirement, not one marker file per run.
- Profiles stop accidental sharing; a misled agent with a shell in the box can still reach other profiles. All shells and the desktop use the same Linux user. The signing key is removed from command environments but cannot be protected from same-user `/proc` access. Profiles are not an OS security boundary. A password must be in the page to work, prompt injection can cause sign-in and actions, and a secret without origin restrictions can be entered on a look-alike page.
- The box gets 1 GB of shared memory for Chromium, and at most 2 GB of memory and 2 CPUs. Set `BROWSER_MEMORY` and `BROWSER_CPUS` in `.env` to change the limits.
- It follows `THEVAULT_VERSION`, so `docker compose pull` upgrades it with the server.

### What it can reach

The box holds no vault data, no secrets and no Docker socket. It needs the internet, and like any container on a Docker network it can also reach your local network. Two things follow:

- Whatever you log in to in the box's browser, every agent with the Browser automation permission can use. Log in only to accounts you would let those agents act in.
- A web page can carry text that tries to instruct the agent reading it. Keep the permission off for agents that do not need it.

To keep the box off your local network, add firewall rules for its network's private ranges on the host. Find the network's subnet first:

```sh
subnet=$(docker network inspect thevault_browser -f '{{(index .IPAM.Config 0).Subnet}}')
sudo iptables -I DOCKER-USER 1 -s "$subnet" -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN
sudo iptables -I DOCKER-USER 2 -s "$subnet" -d 10.0.0.0/8 -j REJECT
sudo iptables -I DOCKER-USER 3 -s "$subnet" -d 172.16.0.0/12 -j REJECT
sudo iptables -I DOCKER-USER 4 -s "$subnet" -d 192.168.0.0/16 -j REJECT
sudo iptables -I DOCKER-USER 5 -s "$subnet" -d 169.254.0.0/16 -j REJECT
sudo iptables -I DOCKER-USER 6 -s "$subnet" -d 100.64.0.0/10 -j REJECT
```

The first rule lets the box answer the server. If your DNS server is on the local network, allow it before the others, such as `sudo iptables -I DOCKER-USER 2 -s "$subnet" -d 192.168.1.1 -p udp --dport 53 -j RETURN`. The rules last until the host restarts; keep them with your distribution's iptables persistence. The subnet changes if the network is removed, such as by `docker compose down`, so check it again after that.

### Add an Android emulator

The Android box, `docker/android.Dockerfile`, is the browser box plus the Android SDK, `adb` and an x86_64 Android 15 emulator with Google Play. The emulator window opens on the same desktop, and agents drive it through `browser_run` with `adb`, such as `adb shell input tap 540 1200`, `adb exec-out uiautomator dump /dev/tty` and `adb exec-out screencap -p > shot.png`. The Google Play system image may not be redistributed, so no published image exists: you build it yourself, and the build accepts the Android SDK licences for you.

The host needs `/dev/kvm`. The image adds about 5.5 GB to the browser box.

1. Build it from a checkout of this repository. This builds the browser box first:

   ```sh
   just android-image-build
   ```

2. Next to `compose.yaml`, create `compose.override.yaml`. `pull_policy: never` keeps `docker compose pull` from looking for the local image in a registry:

   ```yaml
   services:
     browser:
       image: thevault-android:local
       pull_policy: never
       devices: [/dev/kvm]
       group_add: ["${KVM_GID}"]
   ```

3. In `.env`, set `KVM_GID` to the output of `stat -c %g /dev/kvm`, and set `BROWSER_MEMORY=8g`. The box uses over 5 GB once the emulator is up, so the default 2 GB limit kills it.
4. Run `docker compose up -d`.

The box refuses to start without `/dev/kvm`. The emulator cold-boots on every start, in under a minute on the default 2 CPUs. Its AVD lives in `/home/agent/.android` on the `thevault_browser-home` volume, so Google sign-ins and installed apps persist. Sign in to Google on the desktop yourself, and treat that account like a browser login: every agent with Browser automation can use it. `ANDROID_RAM_MB` sets the emulator's memory, default `4096`.

The Android box does not follow `THEVAULT_VERSION`, so `docker compose pull` leaves it as it is. To upgrade it, build it again and run `docker compose up -d`. To build it on another machine, put it on the server with `docker save thevault-android:local | ssh <server> docker load`.

For `just dev`, `just dev-android` starts the Android box in place of the one `just dev-browser` starts.

### Turn it off

Stop the box and remove its container:

```sh
docker compose --profile browser rm -sf browser
```

Then remove `browser` from `COMPOSE_PROFILES`, empty `BROWSER_TOKEN` in `.env`, and run `docker compose up -d`, so the server stops offering the box to agents.

To delete its saved logins and profiles too, run `docker volume rm thevault_browser-home`.

## Back up

Run a backup whenever you want a copy of the vault:

```sh
docker compose exec thevault sh -c 'vault backup /data/backups/manual-$(date -u +%Y%m%dT%H%M%SZ).sqlar'
```

The backup holds every vault file and `state.db` in one SQLite archive. Copy it off the volume to keep it somewhere else:

Files over 256 MiB are stored as ordered, uncompressed parts under `.vault-backup/`, with a versioned JSON manifest recording each original path, total size, part count and SHA-256. This avoids SQLite's single-blob size limit, including for large `state.db` snapshots. Use `vault restore-backup` to reassemble them and verify their size and hash before any destination files are written. Restore stages files on the destination filesystem, inside the destination if it exists or its closest existing parent otherwise, not in the system temporary directory. It removes parts as they are reassembled, renames validated files into place without copying, and removes the staging directory on success or failure. Allow space for the extracted archive plus one file being reassembled. Small files keep the original single-entry format and old backups still restore unchanged. `sqlite3 -A` can extract the individual entries, but does not reassemble chunked files. Manual recovery requires concatenating the parts in manifest order and checking the recorded size and SHA-256.

```sh
docker compose cp thevault:/data/backups/manual-20261018T090000Z.sqlar .
```

The container also writes a backup before every upgrade, to `/data/backups/<old>-to-<new>-<time>.sqlar`, and keeps the newest five. Those backups live in the same volume as the vault. Copy them off too if the disk is the thing you are protecting against.

No backup holds the secret key. Without `/config/vault.key` a restored vault cannot read its stored secrets, such as the Google sign-in. Copy the key once and keep it apart from the backups:

```sh
docker compose cp thevault:/config/vault.key ./vault.key
```

To put the key in a backup anyway, add `--include-key` to the `vault backup` command. A vault with no id yet has no key file to include, so that backup fails: start the server once, then back up again.

Each vault has an id, which names its key file and its git sync branch. `vault init` makes it and keeps it in `state.db`, and a backup copies it. A vault that has no id in `state.db`, such as one from an older build, gets a new id in each backup, and the vault keeps having none until the server starts or git sync runs. The id of a later backup or sync then differs from the id of the earlier backups. A backup taken before that does not restore as the same git sync instance: it syncs as an instance of its own. Start the server once before you take a backup that you want to keep as the same vault.

## Upgrade

When updating `compose.yaml`, follow [Proxy network upgrades](#proxy-network-upgrades)
before restarting. Pulling an image alone does not update a local Compose file.

1. If you may want to go back to this version, take a backup with it first, as [Roll back an upgrade](#roll-back-an-upgrade) says.
2. Pull the new image and restart:

   ```sh
   docker compose pull && docker compose up -d
   ```

3. Check that `docker compose ps` shows `thevault` as `healthy`.

The container writes a backup to `/data/backups` before it starts the new version. `docker compose logs thevault` shows its path.

The desktop and Android apps update themselves from the server once it hands out builds of the new version. You do not need to install them again.

To stay on one version, set `THEVAULT_VERSION` in `.env` to that version, such as `0.1.12`, instead of `latest`.

### How a release is published

A release is cut by a `release: true` or `pre-release: true` trailer on a commit pushed to `main`, and GitHub Actions then publishes it in this order ([docs/releasing.md](releasing.md)):

1. `release.yml` builds the desktop AppImage, deb, rpm and AUR package and the static `vault` CLI for Linux `x86_64` and `aarch64`. At the same time `android-release.yml` builds the signed APK and runs the release smoke test on it.
2. When both have passed, the train creates the tag and the GitHub Release with all of them.
3. `image.yml` downloads the APK, the AppImage and the two CLI builds from the release, checks them against its `SHA256SUMS` files, and builds the image with them baked in. It starts the image, checks that it serves all four, and pushes `ghcr.io/wiltaylor/thevault`. A release is tagged `<version>`, `<major>.<minor>`, `latest` and `edge`, and a pre-release only `<version>` and `edge`. Then it attaches `compose.yaml`, `Caddyfile` and `env.example` to the release.
4. At the same time, `browser-image.yml` builds the browser box image, checks it, and pushes `ghcr.io/wiltaylor/thevault-browser` with the same tags.

So `docker compose pull` gets a new version only after its apps are on the release, and `latest` never gets a pre-release. To follow the pre-releases, set `THEVAULT_VERSION` in `.env` to `edge`. The image carries the source, version, revision and licence as OCI labels: `docker image inspect ghcr.io/wiltaylor/thevault:latest` shows them. `just image-release <assets>` builds and checks the same image on your machine from a folder of the release's files, without pushing it.

## Restore a backup

A restore replaces the vault in the `thevault_data` volume with the backup. Copy the backup out of the volume first, because these steps delete the volume.

1. Copy the backup to the folder beside `compose.yaml`, if it is not there already:

   ```sh
   docker compose cp thevault:/data/backups/<backup>.sqlar ./restore.sqlar
   ```

2. Stop the server and delete the data volume:

   ```sh
   docker compose down
   docker volume rm thevault_data
   ```

3. Unpack the backup into a new data volume:

   ```sh
   docker compose run --rm --no-deps -v "$PWD/restore.sqlar:/restore.sqlar:ro" \
     --entrypoint vault thevault restore-backup /restore.sqlar /data
   ```

4. Start the server with `docker compose up -d` and log in with the password the vault had when you took the backup.

The restored vault uses the key that is already in `/config`. If you restore on a new machine, put the old `vault.key` in place before you start the server:

```sh
docker compose run --rm --no-deps -v "$PWD/vault.key:/restore.key:ro" \
  --entrypoint cp thevault /restore.key /config/vault.key
```

If the backup was made with `--include-key`, `restore-backup` writes the key itself when `/config/vault.key` does not exist yet.

The container runs as uid 1000. If your host user is not uid 1000, make the files readable first with `chmod a+r restore.sqlar vault.key`.

With host folders instead of named volumes, empty the `THEVAULT_DATA` folder instead of removing the volume in step 2.

## Roll back an upgrade

Some versions change the format of `state.db`. An older version refuses to start on a `state.db` that a newer version changed, so going back then needs a backup that holds `state.db` as the older version left it.

The backup that the container writes before an upgrade, `<old>-to-<new>-<time>.sqlar` in `/data/backups`, is such a backup when `<new>` is `0.1.14-alpha.25` or later. Copy it beside `compose.yaml` and go on at "To go back":

```sh
docker compose cp thevault:/data/backups/<old>-to-<new>-<time>.sqlar ./restore.sqlar
```

When `<new>` is `0.1.14-alpha.24` or earlier, that backup does not help. Those versions change `state.db` before they back it up. So for an upgrade to one of them, take your own backup first:

1. Before you upgrade, back up with the version that runs now, and copy the backup beside `compose.yaml`:

   ```sh
   docker compose exec thevault vault backup /data/backups/before-upgrade.sqlar
   docker compose cp thevault:/data/backups/before-upgrade.sqlar ./restore.sqlar
   docker compose exec thevault rm /data/backups/before-upgrade.sqlar
   ```

2. Upgrade as in [Upgrade](#upgrade).

To go back:

1. Set `THEVAULT_VERSION` in `.env` to the version you ran before, such as `0.1.14-alpha.20`, and start it:

   ```sh
   docker compose up -d
   ```

2. If `docker compose ps` shows `thevault` as `healthy`, you are done.
3. If the server does not start and `docker compose logs thevault` shows a line like `state.db is corrupt: schema version 18 is newer than this build supports (17)`, restore `restore.sqlar` as in [Restore a backup](#restore-a-backup), from step 2, with the old version still in `.env`.

Everything written to the vault since `restore.sqlar` was taken is lost.

## Uninstall

To stop TheVault and remove its containers, but keep the vault and the key:

```sh
docker compose down
```

To delete everything, including the vault, the key, and Caddy's certificates, back up first and then run:

```sh
docker compose down -v
```

`down -v` cannot be undone. Remove the folder with `compose.yaml` afterwards, and the images with `docker image rm ghcr.io/wiltaylor/thevault:latest ghcr.io/wiltaylor/thevault-browser:latest`.

## Credential attempt limits and reverse proxies

Login, device pairing (password or code), and agent-code redemption share failure
counters. Five failures from one client in 60 seconds cause a 60-second backoff.
Repeated exhausted windows double this delay, up to 15 minutes. Success resets
that client's count. After 24 hours without a failed check its backoff resets.
HTTP 429 includes `Retry-After` in seconds. Fifty failures globally in 60 seconds
also trigger backoff for clients with recent failures, but not for a clean client.
During a sustained global flood, an owner who has already failed in the window
must wait too; a clean attempt from another address key still passes.
IPv4 uses its full address, including when received as an IPv4-mapped IPv6 address.
Native IPv6 addresses share a budget within each /64, so rotating privacy addresses
does not bypass the limit. A fully uploaded check is charged as a provisional
failure before verification. Success refunds its global charge and resets the
address's completed failures, but not other pending checks. Cancellation or any
non-success result retains the charge and can notify the owner, so disconnecting
cannot bypass the hash budget. Concurrent checks return 429 when the budget is
full. Incomplete bodies consume no reservation and time
out after 10 seconds, with a 2 MiB body limit.
After five failures in a minute the owner gets a notification through the normal
notification feed and device push, at most once per 15 minutes. Counters are in
memory and reset on restart. History retains at most 65,536 address keys. A full
table still admits a clean client using a temporary bucket. If that check fails,
the oldest non-in-flight history is evicted through an ordered index (clean history first, then failing
history). A successful check evicts nothing. Accepted trade-off: a flood from
more than 65,536 distinct failing IPv6 /64s or IPv4 addresses can churn the oldest
failure history, rather than prevent the owner from signing in. Active checks
have temporary state until completion or cancellation. If every retained bucket
has a check in flight, a new failure contributes to the global counter but cannot
evict address history until a bucket is free. Missing peer metadata shares one bucket.

The server trusts no forwarded addresses by default. `VAULT_TRUSTED_PROXIES`
sets comma-separated literal peer IPs with a proxy type, for example
`caddy=10.141.0.2,cloudflare=10.141.0.3`. Caddy peers use the last address in
`X-Forwarded-For`. Cloudflared peers use `CF-Connecting-IP`. Headers from other
peers are ignored. Invalid configuration stops startup. Do not trust arbitrary
clients or expose a trusted proxy's network address to untrusted containers.
The `caddy=` convention also fits nginx and Traefik when configured to replace
untrusted forwarding headers and append the actual immediate client address.

The shipped Compose file configures those two fixed proxy container addresses on
the dedicated `10.141.0.0/24` proxy network for its `https` and `cloudflare`
profiles. No proxy container is present in the LAN-only profile. The server
and proxy containers must be recreated after updating the Compose configuration.
Dynamic network allocation uses `10.141.0.128/25`, keeping the trusted fixed
proxy addresses outside the dynamic range even when a proxy profile is off.
Set `THEVAULT_PROXY_SUBNET`, `THEVAULT_PROXY_IP_RANGE`, `THEVAULT_PROXY_CADDY_IP`,
`THEVAULT_PROXY_CLOUDFLARED_IP` and `THEVAULT_PROXY_SERVER_IP` together in `.env`
to use another subnet. All three addresses must be distinct, inside the subnet,
and outside the dynamic range. The default `VAULT_TRUSTED_PROXIES` follows the
two proxy IP variables automatically. An explicit override, including an empty
value, is preserved and must be reviewed when changing addresses.
For a custom deployment, set this variable to the actual immediate proxy peer,
not its public client or the whole Docker network. An empty value trusts none.

Two proxy configurations need extra care. Docker can forward a published IPv6
port through `docker-proxy` onto this IPv4-only proxy network. Caddy then sees
the network gateway (`10.141.0.1` by default) instead of the original IPv6 client, so those
clients share one budget. Use an end-to-end IPv6-capable proxy path, or do not
publish an IPv6 route to Caddy in this configuration. With Cloudflare's
orange-cloud DNS proxy in front of Caddy, Caddy sees Cloudflare edge addresses
instead of the clients. Prefer the shipped `cloudflare` tunnel profile, or
configure Caddy to trust only Cloudflare's current published edge ranges and to
forward the real client securely. Do not trust client-supplied forwarding headers
or all peers to work around either case.

### Proxy network upgrades

The old `172.30.141.0/24` default overlaps Docker's built-in `172.17` through
`172.31` pools. Production encountered this when another network already used
`172.30.0.0/16`. The new `10.141.0.0/24` default is outside Docker's built-in
address pools, but can still clash with a LAN, VPN or custom Docker pool.
Before installing or changing it, inspect existing allocations and host routes:

```sh
docker network ls -q | xargs -r docker network inspect --format '{{.Name}} {{json .IPAM.Config}}'
ip route
```

Also check VPN routes and Docker daemon `default-address-pools` settings. Render
the chosen configuration with `docker compose --profile https --profile cloudflare config`
locally, without publishing its output (it can contain secrets). Confirm the subnet,
dynamic range, three static addresses and `VAULT_TRUSTED_PROXIES` agree.

Existing installs can keep their current network by setting all five variables
before replacing the Compose file. For the old shipped network, use:

```dotenv
THEVAULT_PROXY_SUBNET=172.30.141.0/24
THEVAULT_PROXY_IP_RANGE=172.30.141.128/25
THEVAULT_PROXY_CADDY_IP=172.30.141.2
THEVAULT_PROXY_CLOUDFLARED_IP=172.30.141.3
THEVAULT_PROXY_SERVER_IP=172.30.141.4
```

For production's existing `192.168.141.0/24`, use that prefix instead, with
`.128/25`, `.2`, `.3` and `.4` respectively. Preserve any explicit trusted-proxy
entries for custom or host proxies, matching their actual immediate peer IPs.
Do not copy a fresh `.env.example` over an existing `.env`.

Production changes belong to meta/lead, with backup first, not to card workers.
To move a network, schedule downtime and take a vault backup as in [Back up](#back-up),
copy it off the volume, secure the key separately, and save the current Compose
file, `.env`, overrides and network inspection locally for rollback. Then:

1. With the old configuration still in place, run `docker compose down` without
   `-v`. This removes the old Compose network, not the data or config volumes.
   If other containers keep the network in use, stop and investigate rather than
   forcibly disconnecting them.
2. Install the new Compose file and set the five variables to the checked target
   subnet, or leave them unset for the new default. Review explicit trusted-proxy
   overrides. Render `docker compose config` with the active profiles again.
3. Run `docker compose up -d` with the same profiles. Check `docker compose ps`,
   server startup logs, login through each enabled proxy, and the client-address
   behavior of attempt limits. Never test repeated failures against the owner.

If the network or proxy checks fail, run `docker compose down` without `-v`,
restore the saved configuration and start the previous image and profiles.
Network-only changes do not require restoring vault data. If an image upgrade
also changed the database, follow [Roll back an upgrade](#roll-back-an-upgrade).
An older local Compose file without the proxy network or `VAULT_TRUSTED_PROXIES`
trusts no proxy: tunnel clients then share the proxy's attempt-limit bucket.
Update that file and recreate the server and proxies, not just the image.
