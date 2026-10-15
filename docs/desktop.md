# The Linux desktop app

`apps/desktop` is TheVault's Linux app, built with Tauri 2. It runs the server stack in its own process on a local copy of the vault, so it works with no connection, and syncs that copy with the home server (D18, D21). This page describes where the app keeps its data, how it pairs, and when it syncs.

## Files

| What | Where |
|---|---|
| The local vault | `$XDG_DATA_HOME/thevault/vault`, normally `~/.local/share/thevault/vault` |
| The device file: server URL, device id and device name | `$XDG_DATA_HOME/thevault/device.json` |
| The device token | The Secret Service keyring, service `thevault`, user the device id (D2) |
| The sync queue and cursor | The `sync_*` tables of the local vault's `state.db` |
| The app's settings: start on login, update check, a skipped version | `$XDG_DATA_HOME/thevault/settings.json` |
| The autostart entry, while "Start on login" is on | `$XDG_CONFIG_HOME/autostart/dev.wiltaylor.thevault.desktop` |

The device file holds no secret. Without the keyring the app still opens and edits the local vault, and its badge reports that it cannot read the token.

## How the window reaches the vault

The window loads `vault://localhost/` through a custom URI scheme, with no network port (D26). On that scheme:

- `/api/...` goes to the in-process server (`crates/embedded`), acting as the owner.
- `/_desktop/...` goes to the app itself: the setup page, `GET /_desktop/status`, `POST /_desktop/pair`, `POST /_desktop/sync`, `POST /_desktop/capture`, and the update prompt's `POST /_desktop/update/open` and `POST /_desktop/update/skip`.
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

## The tray

The app puts an icon in the tray (D12). Its menu has:

- **Quick capture**: shows the window with the capture dialog open.
- **Open TheVault**: shows the window.
- **Start on login**: a check item. On, the app writes an XDG autostart entry that starts it with `--minimised`, so it starts in the tray with its window hidden. Off, the entry is removed.
- **Check for updates**: a check item, on by default. See [Updates](#updates).
- **Quit**: ends the app.

While the home server has unread notifications, the icon carries a red dot, the count shows beside the icon where the tray shows titles (KDE and most StatusNotifier trays), and the tooltip says how many there are. Closing the window hides it in the tray; the app keeps syncing and mirroring notifications. Without a tray (the desktop has no StatusNotifier host, or `libayatana-appindicator3` is missing), the app says so on stderr and closing the window ends it.

## Notifications

The app keeps `GET /api/events` open on the home server with the device token, and reconnects after a pause of 1 second, doubling up to 30 seconds, when the stream drops. On connecting, and on every `notification.*` event, it reads the unread notifications and brings the tray badge and the desktop in line (D9):

- A notification that turns up unread while the app runs is raised through the desktop's notification daemon, with an **Open** action. That covers notifications that arrived while the stream was down.
- The notifications already unread when the app starts only count towards the badge. Starting the app does not raise them again.
- A raised notification that is read, dismissed or cleared anywhere (the web app, a script, another device, the API) closes on the desktop.
- Closing a raised notification on the desktop marks it read on the server, like a swipe on Android. Clicking it opens its note, or the app page it names, in the window, and marks it read.

The badge and the notifications come from the home server, so they need the connection. Offline, the badge keeps the last count it read.

## Quick capture and the global hotkey

The app binds a global **Quick capture** shortcut through the XDG GlobalShortcuts portal (`org.freedesktop.portal.GlobalShortcuts`), suggesting <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>N</kbd>. The desktop decides the key. `GET /_desktop/status` reports the outcome under `hotkey`: `bound` with the key the portal reports, or `unavailable` with the reason.

Where the portal is not available, bind a key to `thevault --capture` in the desktop's keyboard settings instead (D12). `thevault --capture` opens the capture dialog. When the app is already running, the running app shows its window and opens the dialog; otherwise the app starts with it open. The desktop file also offers **Quick capture** as a launcher action.

### Manual check: the portal hotkey

S6 cannot check the portal hotkey, because the test session has no portal. Check it by hand on KDE Plasma and on Hyprland after any change to `src/hotkey.rs`. Run the app from the installed package or AppImage, not from `target/`, so that the portal matches it to `dev.wiltaylor.thevault.desktop`.

On **KDE Plasma 6** (`xdg-desktop-portal-kde`):

1. Start the app. Plasma shows a dialog asking to allow the shortcut **Quick capture in TheVault**, with <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>N</kbd> filled in. Accept it.
2. Open System Settings > Keyboard > Shortcuts. It lists TheVault with **Quick capture in TheVault**.
3. Focus another application and press the key. The TheVault window comes forward with the capture dialog open.
4. Hide the window with its close button and press the key again. The window shows with the dialog open.
5. Quit from the tray, start the app again, and press the key. It works without asking again.

On **Hyprland** (`xdg-desktop-portal-hyprland`):

1. Add a bind to `hyprland.conf` and reload: `bind = SUPER SHIFT, N, global, dev.wiltaylor.thevault:capture`.
2. Start the app. Run `hyprctl globalshortcuts`; it lists `dev.wiltaylor.thevault:capture`. (On older portals the id may show without the app id prefix; use the name the list shows in the bind.)
3. Focus another window and press <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>N</kbd>. The TheVault window comes forward with the capture dialog open.
4. Hide the window with its close button and press the key again. The window shows with the dialog open.

On either desktop, if step 3 fails, read `hotkey` in `/_desktop/status` (from the window's console, `await (await fetch('/_desktop/status')).json()`). `unavailable` means the portal refused or is missing; bind `thevault --capture` to a key in the desktop's settings and check that the key opens the dialog the same way.

## thevault:// links

`thevault://note/<id>` opens the note with that id in the window: in the running app when there is one, otherwise in a new one. The packages' desktop file (`apps/desktop/src-tauri/linux/thevault.desktop`) registers the app as the handler of `x-scheme-handler/thevault`. An AppImage installs no desktop file, so when the app runs from one it writes `~/.local/share/applications/dev.wiltaylor.thevault.desktop` pointing at the AppImage and makes it the default handler with `xdg-mime`. Any other `thevault://` link is ignored.

## Updates

About 10 seconds after it starts, and once a day after that, the app asks GitHub Releases for the latest release of `wiltaylor/thevault` (D27). Drafts and pre-releases are skipped. When the release is newer than the app, by semver order, the window shows a prompt in its bottom right corner:

- **Download** opens the release page in the browser.
- **Skip this version** stops offering that version.
- **Later** hides the prompt until the app starts again.

The tray's **Check for updates** item turns the check off. `THEVAULT_UPDATE_URL` replaces the GitHub address, and `THEVAULT_UPDATE_URL=off` turns the check off. A build can name another repository with `THEVAULT_RELEASES_REPO` at compile time.

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

S6 (`apps/desktop/e2e`) starts a testkit home server on port 0, a private D-Bus session with an unlocked throwaway gnome-keyring and a stand-in notification daemon (`examples/fake-notifyd.rs`), a stand-in GitHub Releases endpoint on port 0, and the app with its own XDG folders. It pairs, opens a note pulled from the server, edits it with the server stopped, and checks that the edit reaches the server once the server is back on the same address. It then starts the app again and checks that it syncs with the token from the keyring.

The desktop extras' cases: a notification created on the server sets the tray badge and is raised on the desktop; reading it through the API closes it and clears the badge; closing one on the desktop marks it read on the server; clicking one opens its note. A `thevault://note/<id>` argument opens the note, and a newer release brings up the update prompt, which **Later** hides.
