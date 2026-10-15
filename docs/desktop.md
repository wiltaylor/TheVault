# The Linux desktop app

`apps/desktop` is TheVault's Linux app, built with Tauri 2. It runs the server stack in its own process on a local copy of the vault, so it works with no connection, and syncs that copy with the home server (D18, D21). This page describes where the app keeps its data, how it pairs, and when it syncs.

## Files

| What | Where |
|---|---|
| The local vault | `$XDG_DATA_HOME/thevault/vault`, normally `~/.local/share/thevault/vault` |
| The device file: server URL, device id and device name | `$XDG_DATA_HOME/thevault/device.json` |
| The device token | The Secret Service keyring, service `thevault`, user the device id (D2) |
| The sync queue and cursor | The `sync_*` tables of the local vault's `state.db` |

The device file holds no secret. Without the keyring the app still opens and edits the local vault, and its badge reports that it cannot read the token.

## How the window reaches the vault

The window loads `vault://localhost/` through a custom URI scheme, with no network port (D26). On that scheme:

- `/api/...` goes to the in-process server (`crates/embedded`), acting as the owner.
- `/_desktop/...` goes to the app itself: the setup page, `GET /_desktop/status`, `POST /_desktop/pair`, `POST /_desktop/sync` and `POST /_desktop/capture`.
- Every other path serves the bundled `web/dist`, so the web client runs unchanged.

## Setup and pairing

Until it is paired, the app shows its setup page. The user enters the server URL and either the vault password or a pairing code from Settings > Devices; pasting the QR code's payload as the code fills in the server URL too. The app posts them to `POST /api/devices/pair` with platform `linux`, stores the token it gets in the keyring, writes the device file, and runs the first sync before it opens the web client.

## When it syncs

One thread runs the sync rounds of `crates/sync` with every asset kept locally (D19). A round runs:

- when the app starts;
- every 60 seconds;
- 0.75 seconds after a write reaches the local vault, so a burst of saves syncs once;
- when the server answers again: while it does not, the app checks every 3 seconds;
- when the user clicks the sync badge.

The badge in the bottom left corner of every page shows where sync stands: synced, syncing, offline with the number of changes waiting, or why it stopped. When the server is newer than the app, the badge says to update the app, and the changes stay queued until then (D27). When the server refuses the token, the device was revoked, and the badge asks for pairing again.

## Quick capture from the command line

`thevault --capture` opens the capture dialog. When the app is already running, the running app shows its window and opens the dialog; otherwise the app starts with it open. Bind it to a key in the desktop's keyboard settings where the global shortcuts portal is not available (D12).

## Installing

Each GitHub Release carries the app as an AppImage, a deb and an rpm, with their `SHA256SUMS` (D11). There is no Flatpak: its sandbox gets in the way of the global hotkey and the keyring. The app keeps its device token in the Secret Service, so the desktop needs one running, such as GNOME Keyring or KWallet.

### Arch Linux and CachyOS

Install `thevault-bin` from the AUR with an AUR helper:

```bash
paru -S thevault-bin    # or: yay -S thevault-bin
```

Without a helper, build it by hand:

```bash
git clone https://aur.archlinux.org/thevault-bin.git
cd thevault-bin
makepkg -si
```

The package installs the release's deb contents: `/usr/bin/thevault`, its desktop entry and its icons. It depends on `webkit2gtk-4.1` and `gtk3`, and suggests `gnome-keyring` or `kwallet` for the Secret Service.

### Other distributions

- Debian and Ubuntu: `sudo apt install ./TheVault_<version>_amd64.deb`.
- Fedora: `sudo dnf install ./TheVault-<version>-1.x86_64.rpm`.
- openSUSE: `sudo zypper install ./TheVault-<version>-1.x86_64.rpm`.
- Anywhere else: `chmod +x TheVault_<version>_amd64.AppImage` and run it.

## Releasing

Pushing a tag `v<version>` runs `.github/workflows/release.yml`. The tag must match `version` in `apps/desktop/src-tauri/tauri.conf.json`. The workflow runs `just desktop-release-test` and `just aur-release` in the dev container, then creates the tag's GitHub Release with the AppImage, deb, rpm, `SHA256SUMS` and `thevault-bin-aur.tar.gz`.

`thevault-bin-aur.tar.gz` holds the release's `PKGBUILD` and `.SRCINFO`, with the version and the deb's checksum filled in. To update the AUR package, copy both into a clone of `ssh://aur@aur.archlinux.org/thevault-bin.git`, then commit and push. `packaging/aur/PKGBUILD` is the template, and `./dev just aur-check` checks that `packaging/aur/.SRCINFO` still matches it.

## Building and testing

```bash
./dev just desktop-build   # apps/desktop/src-tauri/target/debug/thevault
./dev just desktop-test    # unit tests, then S6 under Xvfb through tauri-driver
./dev just desktop-release        # AppImage, deb and rpm into target/desktop-release
./dev just desktop-release-test   # the bundles, then the AppImage started under Xvfb to the setup page
./dev just aur-check              # packaging/aur/.SRCINFO matches the PKGBUILD
```

S6 (`apps/desktop/e2e`) starts a testkit home server on port 0, a private D-Bus session with an unlocked throwaway gnome-keyring, and the app with its own XDG folders. It pairs, opens a note pulled from the server, edits it with the server stopped, and checks that the edit reaches the server once the server is back on the same address. It then starts the app again and checks that it syncs with the token from the keyring.
