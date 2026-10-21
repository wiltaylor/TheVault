# The Linux desktop app

`apps/desktop` is TheVault's Linux app, built with Tauri 2. It runs the server stack in its own process on a local copy of the vault, so it works with no connection, and syncs that copy with the home server (D18, D21). This page describes where the app keeps its data, how it pairs, and when it syncs.

## Files

| What | Where |
|---|---|
| The local vault | `$XDG_DATA_HOME/thevault/vault`, normally `~/.local/share/thevault/vault` |
| The device file: server URL, device id and device name | `$XDG_DATA_HOME/thevault/device.json` |
| The device token | The Secret Service keyring, service `thevault`, user the device id (D2) |
| The sync queue and cursor | The `sync_*` tables of the local vault's `state.db` |
| The app's settings: start on login, update check, a skipped version, install on quit, the application menu entry | `$XDG_DATA_HOME/thevault/settings.json` |
| The autostart entry, while "Start on login" is on | `$XDG_CONFIG_HOME/autostart/dev.wiltaylor.thevault.desktop` |
| An AppImage's installed copy | `~/Applications/TheVault.AppImage` |
| An AppImage's application menu entry and icons, while "Add TheVault to the application menu" is on | `$XDG_DATA_HOME/applications/dev.wiltaylor.thevault.desktop`, `$XDG_DATA_HOME/icons/hicolor/<size>x<size>/apps/dev.wiltaylor.thevault.png` |

The device file holds no secret. Without the keyring the app still opens and edits the local vault, and its sync icon reports that it cannot read the token.

## How the window reaches the vault

The window loads `vault://localhost/` through a custom URI scheme, with no network port (D26). On that scheme:

- `/api/...` goes to the in-process server (`crates/embedded`), acting as the owner. It answers from the local vault, and sends the routes backed by the home server's own state, such as Settings > Devices and the notifications, to the home server with the device token ([`docs/sync.md`](sync.md#what-the-apps-answer-locally)). `/api/events` answers 503 at once, since the scheme cannot stream; the web client polls instead.
- `/_desktop/...` goes to the app itself: the setup page, `GET /_desktop/status`, `POST /_desktop/pair`, `POST /_desktop/sync`, `POST /_desktop/capture`, the update prompt's `POST /_desktop/update/open` and `POST /_desktop/update/skip`, Settings > General's `POST /_desktop/update/check`, which checks now even with the automatic check off, and `POST /_desktop/update/restart`, and the install notice's `POST /_desktop/installed/undo` and `POST /_desktop/installed/dismiss`.
- Every other path serves the bundled `web/dist`, so the web client runs unchanged.

## Setup and pairing

Until it is paired, the app shows its setup page. The user enters the server URL and either the vault password or a pairing code from Settings > Devices; pasting the QR code's payload as the code fills in the server URL too. The app posts them to `POST /api/devices/pair` with platform `linux`, stores the token it gets in the keyring, writes the device file, and runs the first sync before it opens the web client.

While the first sync runs, the setup page shows a progress bar under **Downloading your vault**, with **Notes** `N / M` (every note and definition file the pull reads), **Files** `X / Y (a / b MB)` (the assets it downloads, and their megabytes as they arrive) and **Time left**, such as `about 2 min left`, measured from the last 10 seconds' transfer rates (`docs/sync.md` "Progress"). It asks `GET /_desktop/status` twice a second and opens the web client when the sync finishes. A round that fails shows its error with **Retry**, which runs a round at once; the app also tries again by itself. Closing the window during the first sync hides it in the tray and the sync carries on; quitting the app stops it, and the next start, already paired, opens the web client and carries on syncing there, with the same progress in the sync popover.

## When it syncs

One thread runs the sync rounds of `crates/sync` with every asset kept locally (D19). A round runs:

- when the app starts;
- every 60 seconds;
- 0.75 seconds after a write reaches the local vault, so a burst of saves syncs once;
- 1 second after the live link's event stream reports a `note.changed` for a note version the local vault does not hold, and 1 second after the stream reconnects, so a change made in the browser or on another device shows within seconds. The echo of the app's own pushed write is not news and runs no round;
- when the server answers again: while it does not, the app checks every 3 seconds;
- when the user clicks Sync now in the sync icon's details.

The sync icon beside Archive at the foot of the left sidebar shows where sync stands, as on Android: green when the last round succeeded and nothing waits, yellow while a round runs or changes wait for one, red when the server cannot be reached or sync stopped. Its tooltip says the state in words, and clicking it shows the last sync time, the changes waiting, the server, the last error and a Sync now button. When the server is newer than the app, the icon says to update the app, a notice says so once, and the changes stay queued until then (D27). When the server refuses the token, the device was revoked, and the icon asks for pairing again.

## The tray

The app puts an icon in the tray (D12). Its menu has:

- **Quick capture**: shows the window with the capture dialog open.
- **Open TheVault**: shows the window.
- **Restart to update to `<version>`**: greyed out, as **No update waiting**, until an update is downloaded. Then it installs the update and starts the new version. See [Updates](#updates).
- **Start on login**: a check item. On, the app writes an XDG autostart entry that starts it with `--minimised`, so it starts in the tray with its window hidden. Off, the entry is removed.
- **Check for updates**: a check item, on by default. See [Updates](#updates).
- **Install updates on quit**: a check item, on by default. On, quitting installs an update that is downloaded and waiting, so the next start runs it.
- **Add TheVault to the application menu**: a check item, on by default, shown only when the app runs from an AppImage. See [The icon and the application menu entry](#the-icon-and-the-application-menu-entry).
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

`thevault://note/<id>` opens the note with that id in the window: in the running app when there is one, otherwise in a new one. The packages' desktop file (`apps/desktop/src-tauri/linux/thevault.desktop`) registers the app as the handler of `x-scheme-handler/thevault`. An AppImage installs no desktop file, so when the app runs from one it writes its own entry, which points at the AppImage, and makes it the default handler with `xdg-mime` (see [The icon and the application menu entry](#the-icon-and-the-application-menu-entry)). Any other `thevault://` link is ignored.

## The icon and the application menu entry

On Wayland the compositor and the panel find a window's icon through its `app_id`: they look for a desktop file named `<app_id>.desktop`, or one whose `StartupWMClass` is the `app_id`, and show its `Icon`. GTK 3 sets the `app_id` to the program's name, `g_get_prgname()`, which was the binary's name, `thevault`, up to 0.1.2. The app now names the program `dev.wiltaylor.thevault`, its identifier, before GTK starts (`app_menu::name_program`), so the `app_id` is `dev.wiltaylor.thevault`; on X11 it is the window's `WM_CLASS` name too. `app.enableGTKAppId` in `tauri.conf.json` also registers the GTK application under the identifier, which GNOME reads; the app is single-instance already, so that registration on the session bus changes nothing else.

- The deb and rpm install `/usr/share/applications/TheVault.desktop` from `linux/thevault.desktop`, with `StartupWMClass=dev.wiltaylor.thevault` and `Icon=thevault`, and the icons under `/usr/share/icons/hicolor` as `thevault.png`. They get no entry from the app.
- An AppImage installs itself in `~/Applications` first (see [The AppImage installs itself](#the-appimage-installs-itself)). While **Add TheVault to the application menu** is on, the default, each start of an AppImage (`$APPIMAGE` is set) writes `$XDG_DATA_HOME/applications/dev.wiltaylor.thevault.desktop` (`Name=TheVault`, `Exec="<AppImage>" %u`, `Icon=dev.wiltaylor.thevault`, `StartupWMClass=dev.wiltaylor.thevault`, the `thevault://` MIME type and the **Quick capture** action) and the bundled icons at 32, 64, 128, 256 and 512 px as `$XDG_DATA_HOME/icons/hicolor/<size>x<size>/apps/dev.wiltaylor.thevault.png`. A file already the same is left alone, so a start with nothing to change writes nothing; an AppImage moved to another path gets its `Exec` rewritten at its next start. When anything changed, the app touches the `hicolor` folder and runs `update-desktop-database`, `xdg-mime default` and `kbuildsycoca6`, each only when it is installed. Turning the item off removes the entry and the icons.

The window also sets the 256 px icon itself, which X11, XWayland and some compositors show.

### The AppImage installs itself

When the app starts from an AppImage anywhere but `~/Applications/TheVault.AppImage`, it installs itself there before it opens a window:

1. With no `~/Applications/TheVault.AppImage`, or one older than itself, it copies itself there: to a temporary file in `~/Applications`, made executable, then renamed over the old one. To learn the installed copy's version it runs `~/Applications/TheVault.AppImage --version`, which prints `TheVault <version>` and exits; a copy that answers nothing in 15 seconds counts as older.
2. With an installed copy as new as itself or newer, it copies nothing.
3. It deletes the AppImage the user started when that file is in the download folder (`XDG_DOWNLOAD_DIR` from `~/.config/user-dirs.dirs`, normally `~/Downloads`) or a temporary folder outside the home folder. Anywhere else the file stays.
4. It starts `~/Applications/TheVault.AppImage` with the same arguments, so a `thevault://` link or `--capture` goes through, and exits. Starting it hands the arguments to a copy already running, as any second start does.

The installed copy writes the application menu entry for its own path, and the self-updater replaces that file in place from then on, since it updates the running `$APPIMAGE` ([The AppImage updates itself](#the-appimage-updates-itself)).

After an install, the window shows a notice once, at its top right, on the setup page too: "TheVault was installed to ~/Applications and added to your apps.", with "You can delete <file>." when the original stayed. **Undo** turns **Add TheVault to the application menu** off, which removes the entry and the icons and keeps the installed file; **OK** closes the notice. It waits in `$XDG_DATA_HOME/thevault/installed.json` until one of them is pressed. A deb or rpm install never installs itself. `THEVAULT_NO_SELF_INSTALL=1` turns the install off, which the release smoke test sets, since the second copy would leave tauri-driver driving the one that exited.

After the first start of an AppImage that wrote the entry, the menu lists TheVault and the window and the task manager show its icon. If a panel keeps the generic icon, its icon cache is stale: on KDE Plasma run `kbuildsycoca6` and `plasmashell --replace &`, or log out and back in.

## Updates

The app checks for a newer version about 10 seconds after it starts, every 4 hours after that, and at once when the paired server announces a new build: the server sends `clients.updated` on the event stream the app keeps open whenever a build is published into its clients folder (D27; see [`docs/clients.md`](clients.md#updates)).

### The AppImage updates itself

An app that runs as an AppImage (`$APPIMAGE` is set) and is paired updates itself from its server, with the Tauri updater (`tauri-plugin-updater` 2):

1. It asks the server's updater endpoint, `GET /api/clients/update/linux/<arch>/<version>`. The server answers 204 when it publishes no signed release AppImage newer than the app, or the updater's JSON for the newer one.
2. The app downloads the AppImage in the background and verifies its signature with the public key built into the app (`plugins.updater.pubkey` in `apps/desktop/src-tauri/tauri.conf.json`). The signature must also name the version the server announced (`requireSignedVersion`), so a server cannot pass an older signed build off as a newer one. A build that fails either check is thrown away.
3. The window shows **TheVault `<version>` is ready — restart to update** in its bottom right corner, with **Restart to update**; the tray's **Restart to update to `<version>`** item and its tooltip say the same.
4. **Restart to update** replaces the AppImage file with the new one and starts it. With **Install updates on quit** on, quitting the app replaces it too, and the next start runs the new version.

The server's address may be plain `http` (`dangerousInsecureTransportProtocol` is on): the signature, not the transport, is what proves a build came from you.

### Everything else is offered as a download

When the server has no signed AppImage newer than the app, and for an app that does not run as an AppImage, the check offers a download instead:

1. It asks the paired server for `/api/clients`, the builds the server hands out. When the server publishes an AppImage for this machine's architecture, that AppImage's version is the one compared, and GitHub is not asked.
2. When the server publishes no AppImage, cannot be reached, or the app is not paired yet, it asks GitHub Releases for the latest release of `wiltaylor/thevault`. Drafts and pre-releases are skipped.

When the version found is newer than the app, by semver order, the window shows a prompt in its bottom right corner. A build from the server says "available from your server". An app installed from the deb or rpm (it runs from `/usr` or `/opt`) never replaces itself: the prompt says to update it with the package manager, and keeps the download.

- **Download** opens the server's AppImage download, or the GitHub release page, in the browser.
- **Skip this version** stops offering that version.
- **Later** hides the prompt until the app starts again.

The tray's **Check for updates** item turns the check off, the self-update too. `THEVAULT_UPDATE_URL` replaces the GitHub address, and `THEVAULT_UPDATE_URL=off` turns the whole check off, the server's part too. A build can name another repository with `THEVAULT_RELEASES_REPO` at compile time. `GET /_desktop/status` reports the check under `update` (the offer, with `ready` and `install`) and `updater` (`install`, `checks`, `ready`, `error`).

### The app updates before it syncs

Before every sync round the app compares the server's version (`server_version` in `GET /api/sync/info`) with its own, with the rule Android uses (`crates/sync/src/gate.rs`, D27). When the server is newer and its updater endpoint offers a signed AppImage, the round neither pushes nor pulls: the app downloads and verifies the AppImage, installs it and restarts into it, and the new version syncs. The sync icon says sync waits meanwhile. When the server offers no signed AppImage, the app is a deb or rpm install, or `THEVAULT_UPDATE_URL=off`, sync goes on and Settings > Devices warns on this device's card. A failed check, download or install is tried again after 1, then 4, then 16 minutes; after 3 failures sync goes on with the warning.

### The updater signing key

The AppImage's signature is made with a minisign key pair from `cargo tauri signer generate`. The private key and its password live outside the repository:

| File | What it holds |
|---|---|
| `~/.config/thevault/updater/thevault-updater.key` | The private key, encrypted with the password |
| `~/.config/thevault/updater/thevault-updater.key.pub` | The public key, the same as `plugins.updater.pubkey` in `tauri.conf.json` |
| `~/.config/thevault/updater/updater.env` | `export TAURI_SIGNING_PRIVATE_KEY=<the key file>` and `export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=<its password>` |

`./dev` reads `updater.env` when it exists and `TAURI_SIGNING_PRIVATE_KEY` is not set already, passes the password through by name and mounts the key file read-only; `THEVAULT_NO_SIGNING_KEYS=1` leaves both signing keys out. `just desktop-release` then signs each AppImage, `cargo tauri signer sign --app-version <version>`, writing `<file>.AppImage.sig` beside it, and `just publish-clients` publishes the signature with the AppImage. Without the key, `just desktop-release` says so and builds an AppImage with no signature, which installed apps never update to.

**Back up `~/.config/thevault/updater/`.** Without the private key or its password, no build you make can update the apps already installed: each one must download and install a new AppImage by hand, from a build whose `tauri.conf.json` carries a new public key. Never commit the key, its password or `updater.env`.

## Installing

Each GitHub Release carries the app as an AppImage, a deb and an rpm, with their `SHA256SUMS` (D11). A server can hand out the same files itself: they are on its log-in page under **Get the apps**, and on Settings > Devices (see [`docs/clients.md`](clients.md)). There is no Flatpak: its sandbox gets in the way of the global hotkey and the keyring. The app keeps its device token in the Secret Service, so the desktop needs one running, such as GNOME Keyring or KWallet.

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
- Anywhere else: `chmod +x TheVault_<version>_amd64.AppImage` and run it. It moves itself to `~/Applications` and adds itself to the application menu ([The AppImage installs itself](#the-appimage-installs-itself)).

## Releasing

A release train runs `.github/workflows/release.yml` with the version it cut ([docs/releasing.md](releasing.md)). The workflow writes that version into its checkout, runs `just desktop-release-test` and `just aur-release` in the dev container, and hands the train the AppImage, its updater signature `.AppImage.sig`, the deb, rpm, `SHA256SUMS` and `thevault-bin-aur.tar.gz`, which the train puts on the release. A pre-release's deb and rpm carry the version their package managers sort before the release, `0.1.13~alpha.1` for `0.1.13-alpha.1`, and its AUR `pkgver` is `0.1.13alpha1`.

The workflow signs the AppImage with the updater key from two repository secrets: `TAURI_SIGNING_PRIVATE_KEY`, the text of `thevault-updater.key` (not its path), and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, its password. It fails before building when either is missing, and after building when an AppImage has no `.sig`, so a release never carries an AppImage the installed apps will not update to. `image.yml` downloads the `.sig` with the AppImage, and the server image's clients manifest gives the AppImage its `signature`.

`thevault-bin-aur.tar.gz` holds the release's `PKGBUILD` and `.SRCINFO`, with the version and the deb's checksum filled in. To update the AUR package, copy both into a clone of `ssh://aur@aur.archlinux.org/thevault-bin.git`, then commit and push. `packaging/aur/PKGBUILD` is the template, and `./dev just aur-check` checks that `packaging/aur/.SRCINFO` still matches it.

## Building and testing

```bash
./dev just desktop-build   # apps/desktop/src-tauri/target/debug/thevault
./dev just desktop-test    # unit tests, then S6 under Xvfb through tauri-driver
./dev just desktop-release        # AppImage (signed for the updater with its key), deb and rpm into target/desktop-release
./dev just desktop-release-test   # the bundles, then the AppImage started under Xvfb to the setup page
./dev just aur-check              # packaging/aur/.SRCINFO matches the PKGBUILD
```

S6 (`apps/desktop/e2e`) starts a testkit home server on port 0, a private D-Bus session with an unlocked throwaway gnome-keyring and a stand-in notification daemon (`examples/fake-notifyd.rs`), a stand-in GitHub Releases endpoint on port 0, and the app with its own XDG folders. It pairs, opens a note pulled from the server, edits it with the server stopped, and checks that the edit reaches the server once the server is back on the same address. It then starts the app again and checks that it syncs with the token from the keyring.

The desktop extras' cases: a notification created on the server sets the tray badge and is raised on the desktop; reading it through the API closes it and clears the badge; closing one on the desktop marks it read on the server; clicking one opens its note. A note created and then renamed on the server reaches the local vault within seconds, through the live link. A `thevault://note/<id>` argument opens the note, and a newer release brings up the update prompt, which **Later** hides. The self-update case runs the debug app as an AppImage: `THEVAULT_TEST_APPIMAGE` names a file to treat as the running AppImage and `THEVAULT_TEST_UPDATER_PUBKEY` a throwaway public key, both read by debug builds only. With the app running and its first check done, the test signs a fake 97.0.0 AppImage with the throwaway key, publishes it with `vault clients publish` into the home server's clients folder, and checks that one check, started by the server's `clients.updated`, downloads and verifies it and that the window and the tray offer the restart. Installing and restarting are left to a real install.

The first-sync case (`first-sync.test.mjs`) fills a home server with 400 notes and six 4 MB assets and pairs the app with it through a proxy that lets the server send 2 MB a second, so the first sync takes several seconds. It checks the setup page's notes, files and time left part way through the files, that they change every second, and that the page opens the web client once every byte has arrived. It saves a screenshot of the page to its temporary folder, and copies it to `FIRST_SYNC_SHOT` when that is set.
