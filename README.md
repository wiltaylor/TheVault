# TheVault

A single-user, self-hosted note app: a Rust server, a SolidJS web client, desktop and Android apps, and the `vault` CLI. The vault is a folder of WCL files.

## Install

[Install TheVault with Docker Compose](docs/install.md) runs the server with Docker Compose, on your home network or Tailscale, at your own domain behind Caddy, or through a Cloudflare Tunnel.

## Documentation

- [Install TheVault with Docker Compose](docs/install.md)
- [Connecting Gmail and Google Calendar](docs/google.md)
- [Handing out the apps from your server](docs/clients.md)
- [The Linux desktop app](docs/desktop.md)
- [Releasing the Android app](docs/android-release.md)
- [Device sync on the server](docs/sync.md)
- [Scripts reference](docs/scripts.md)
- [Vault file format](docs/vault-format.md)

## Develop

Every build, test and lint command runs through the dev container: `./dev just check` is the full gate. `CLAUDE.md` describes the layout and the rules.
