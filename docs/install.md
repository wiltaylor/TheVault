# Install TheVault with Docker Compose

This guide runs the TheVault server on one machine with Docker Compose. It covers the first start, how you reach the server, backups, upgrades, restoring and removing it.

The server is one container, `thevault`. It keeps the vault in the `/data` volume and the vault's secret key in the `/config` volume. Two optional profiles add a front door: `https` adds Caddy for your own domain, and `cloudflare` adds a Cloudflare Tunnel.

## What you need

- A Linux machine with Docker Engine and the Docker Compose plugin, version 2.20 or later. Run `docker compose version` to check.
- The image is `linux/amd64`, so the machine needs an x86-64 processor.
- While the image is private, a GitHub account that can read `ghcr.io/wiltaylor/thevault`.

## Log in to the GitHub container registry

Skip this section when the image is public.

GitHub's container registry accepts a personal access token (classic). It does not accept fine-grained tokens.

1. On GitHub, open **Settings > Developer settings > Personal access tokens > Tokens (classic)** and select **Generate new token (classic)**.
2. Tick only the `read:packages` scope, set an expiry, and select **Generate token**.
3. Copy the token.
4. On the server, log in with your GitHub user name:

   ```sh
   docker login ghcr.io -u <github-user>
   ```

5. Paste the token when Docker asks for the password.

Docker keeps the login in `~/.docker/config.json`, so `docker compose pull` works from then on.

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

To change the password later, run this and type the new password twice:

```sh
docker compose exec thevault vault passwd
```

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

## Back up

Run a backup whenever you want a copy of the vault:

```sh
docker compose exec thevault sh -c 'mkdir -p /data/backups && vault backup /data/backups/manual-$(date -u +%Y%m%dT%H%M%SZ).sqlar'
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

Each release is cut by pushing a `v<version>` tag, and GitHub Actions then publishes it in this order:

1. `release.yml` builds the desktop AppImage, deb, rpm and AUR package and the static `vault` CLI for Linux `x86_64` and `aarch64`, and creates the tag's GitHub Release with them. At the same time `android-release.yml` builds the signed APK and attaches it to that release.
2. When both have finished, `image.yml` downloads the APK, the AppImage and the two CLI builds from the release, checks them against its `SHA256SUMS` files, and builds the image with them baked in. It starts the image, checks that it serves all four, and pushes `ghcr.io/wiltaylor/thevault` tagged `<version>`, `<major>.<minor>` and `latest`. Then it attaches `compose.yaml`, `Caddyfile` and `env.example` to the release.

So `docker compose pull` gets a new version only after its apps are on the release. The image carries the source, version, revision and licence as OCI labels: `docker image inspect ghcr.io/wiltaylor/thevault:latest` shows them. `just image-release <assets>` builds and checks the same image on your machine from a folder of the release's files, without pushing it.

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

`down -v` cannot be undone. Remove the folder with `compose.yaml` afterwards, and the image with `docker image rm ghcr.io/wiltaylor/thevault:latest`.
