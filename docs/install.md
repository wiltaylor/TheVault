# Install TheVault with Docker Compose

This guide runs the TheVault server on one machine with Docker Compose. It covers the first start, how you reach the server, backups, upgrades, restoring and removing it.

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

The URL is `thevault:8080` because `cloudflared` runs in the same compose network as the server and finds it by its service name. Cloudflare provides the HTTPS certificate.

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

- `/home/agent` is the named volume `thevault_browser-home`. Browser profiles that `playwright-cli` saves, logins and the desktop's settings stay there across restarts and upgrades. `/work`, where commands run and screenshots land, is emptied when the container is re-created.
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

2. Next to `compose.yaml`, create `compose.override.yaml`:

   ```yaml
   services:
     browser:
       image: thevault-android:local
       devices: [/dev/kvm]
       group_add: ["${KVM_GID}"]
   ```

3. In `.env`, set `KVM_GID` to the output of `stat -c %g /dev/kvm`, and set `BROWSER_MEMORY=8g`. The box uses over 5 GB once the emulator is up, so the default 2 GB limit kills it.
4. Run `docker compose up -d`.

The box refuses to start without `/dev/kvm`. The emulator cold-boots on every start, in under a minute on the default 2 CPUs. Its AVD lives in `/home/agent/.android` on the `thevault_browser-home` volume, so Google sign-ins and installed apps persist. Sign in to Google on the desktop yourself, and treat that account like a browser login: every agent with Browser automation can use it. `ANDROID_RAM_MB` sets the emulator's memory, default `4096`.

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

```sh
docker compose cp thevault:/data/backups/manual-20261018T090000Z.sqlar .
```

The container also writes a backup before every upgrade, to `/data/backups/<old>-to-<new>-<time>.sqlar`, and keeps the newest five. Those backups live in the same volume as the vault. Copy them off too if the disk is the thing you are protecting against.

No backup holds the secret key. Without `/config/vault.key` a restored vault cannot read its stored secrets, such as the Google sign-in. Copy the key once and keep it apart from the backups:

```sh
docker compose cp thevault:/config/vault.key ./vault.key
```

To put the key in a backup anyway, add `--include-key` to the `vault backup` command.

## Upgrade

1. Pull the new image and restart:

   ```sh
   docker compose pull && docker compose up -d
   ```

2. Check that `docker compose ps` shows `thevault` as `healthy`.

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
