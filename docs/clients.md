# Handing out the apps from your server

A server can hand out the Android APK, the Linux AppImage, deb and rpm, and static Linux builds of the `vault` CLI itself, so a phone or a new computer gets the app, and an AI agent or a script gets the CLI, straight from the server it will talk to. The builds are public: anyone who reaches the server can download them without logging in. They hold no vault data and no secrets.

## Where the builds live

The server reads one folder, its clients folder:

- `vault-server --clients <dir>`, or `VAULT_CLIENTS_DIR=<dir>`, names it.
- Without either, it is `<vault>/.vault/clients`. `.vault/` is the server's private folder: nothing in it syncs to devices, is indexed, or goes into `vault backup`.

The folder holds the build files and a `manifest.json` that lists them. The server serves only the files the manifest lists, and reads the manifest on every request, so a newly published build shows at once, with no restart. A folder that does not exist is an empty list.

The server also looks at the manifest every 30 seconds, which costs one `stat`. When the builds it lists change, whoever wrote it, the server sends `clients.updated` to the apps, which then update themselves: see [Updates](#updates).

```json
{
  "clients": [
    {
      "platform": "android",
      "kind": "apk",
      "arch": "aarch64",
      "version": "0.1.0",
      "build": "release",
      "size": 31457280,
      "sha256": "9f2c…",
      "filename": "TheVault_0.1.0_aarch64.apk",
      "published": 1760000000000
    }
  ]
}
```

| Field | Meaning |
|---|---|
| `platform` | `android`, `linux`, or `cli` for the `vault` CLI |
| `kind` | `apk`, `appimage`, `deb`, `rpm`, or `binary` for the `vault` CLI |
| `arch` | `aarch64`, `x86_64`, `armv7`, `x86` or `universal` |
| `version` | The app's version |
| `build` | `release`, signed with the release key, or `debug`, signed with the build machine's debug key |
| `size`, `sha256` | The file's size in bytes and its SHA-256 |
| `filename` | The file's name in the folder: `TheVault_<version>_<arch>[-debug].<ext>`, or `vault-<version>-linux-<arch>[-debug]` for the CLI |
| `published` | When it was published, in milliseconds since the epoch |
| `signature` | An AppImage's updater signature, the text of the `.AppImage.sig` file published beside it. Absent for every other build, and for an AppImage published without one |

The folder holds one build per kind and architecture. Publishing a build replaces the one before it and deletes the old file.

## Publish

Build the apps, then publish them into the folder the server reads:

```bash
./dev just publish-clients .scratch/my-vault/.vault/clients
```

`just publish-clients <dir>` publishes the newest builds it finds:

- Android: the signed release APK in `target/android-release/`, only once it has passed `just android-release-smoke`, which builds it and runs it on the emulator ([`docs/android-release.md`](android-release.md#the-release-smoke-test)). A release APK that has not passed is not published, and the recipe fails after publishing the rest; `just publish-clients <dir> <build> skip` overrides this in an emergency. Without a release APK, it publishes the debug APK of the last `just android-build` and labels it `debug`.
- Linux: the AppImage, with its updater signature `<file>.AppImage.sig`, the deb and the rpm in `target/desktop-release/` (`just desktop-release`).
- The `vault` CLI: the static builds for `x86_64` and `aarch64` in `target/cli-release/` (`just cli-release`), see [The vault CLI](#the-vault-cli).

It reads the signing keys from `~/.config/thevault` when they are there and not set already: `android/keystore.env` for the Android release keystore ([`docs/android-release.md`](android-release.md#where-the-keystore-lives)) and `updater/updater.env` for the desktop updater key ([`docs/desktop.md`](desktop.md#the-updater-signing-key)). `./dev` reads them too and passes them into the container.

With `build='missing'`, the default, it builds what is not built yet: the signed release APK, through `just android-release-smoke`, when the release keystore and the Android toolchain are there, else the debug APK, the desktop release bundles, and the two CLI builds. `just publish-clients <dir> all` builds all of them again first, which is how to publish a new version. `just publish-clients <dir> none` publishes only what is already built. It prints what the folder holds when it is done, with `signed` beside an AppImage that carries its updater signature.

`./dev` only sees the checkout, so a `<dir>` outside it must be published from the host with the `vault` binary instead:

```bash
vault clients publish --dir /srv/thevault/.vault/clients target/android-release/TheVault_0.1.0_aarch64.apk
vault clients publish --vault /srv/thevault --build debug --version 0.1.0 app-arm64-debug.apk
```

`vault clients publish` copies each file in, reads its kind from the extension and its version and architecture from the file name, and rewrites the manifest. An AppImage's `<file>.AppImage.sig` beside it goes into the manifest as its `signature`. `--version`, `--arch` and `--build` say what the file name does not; `--dir` defaults to `<vault>/.vault/clients`. It prints the new manifest. The files are copied and the manifest written under temporary names and renamed into place, so a server reading the folder meanwhile sees either the old list or the new one. A server running on the folder announces the publish within 30 seconds.

An APK installs over the app already on a phone only when the same key signed both, and only over the same package: the release app is `dev.wiltaylor.thevault` and the debug app `dev.wiltaylor.thevault.debug`, so a release APK never replaces a debug install. Release APKs are signed with the release keystore. Debug APKs built through `./dev` are signed with the one debug key kept in the `thevault-gradle` volume, so they install over each other; a debug APK built before that key existed, or on another machine, does not, and must be uninstalled first. See [`docs/android-release.md`](android-release.md).

## Download

- **The log-in page** links to **Get the apps**, at `/apps`, which works without logging in.
- **Settings > Devices** shows the same list at the top.

Each build shows its version, `release` or `debug`, architecture, size, a download button and its SHA-256 with a copy button. The APK also has a QR code of its download address: scan it with the phone's camera to download the APK on the phone. The address is the one the page was loaded from, so load the page at the address the phone reaches the server on.

With nothing published, both places explain how to publish.

### The API

| Route | What it does |
|---|---|
| `GET /api/clients` | The manifest's builds, each with `path`, its download path, `url`, its absolute address as the request reached the server, and for an APK `qr_svg`, the QR code of `url`. `enabled` is false on a server with no clients folder. An app's embedded server forwards these routes to its home server |
| `GET /api/clients/{filename}` | One build's bytes, as an attachment with its media type (`application/vnd.android.package-archive` for an APK, `application/octet-stream` for an AppImage, `application/vnd.debian.binary-package`, `application/x-rpm`), `Content-Length`, one `Range` (206), and the SHA-256 as its `ETag` |
| `GET /api/clients/latest/{kind}[?arch=]` | The newest build of `apk`, `appimage`, `deb` or `rpm`, for `arch` or the kind's usual architecture (`aarch64` for an APK, `x86_64` otherwise). The address never changes, so an updater can follow it |
| `GET /api/clients/update/{target}/{arch}/{current_version}` | The desktop app's updater check, in the format the Tauri updater reads. 204 when the manifest has no release AppImage for `target` (`linux`) and `arch` (`x86_64`, `aarch64`) that carries a `signature` and is newer than `current_version` by semver order. Otherwise 200 with `{version, notes, pub_date, url, signature}`: `url` downloads the AppImage itself, at the address the request reached the server on, and `pub_date` is its publish time in RFC 3339 |

All four, and the CLI download below, are public, `GET` and `HEAD` only. A name the manifest does not list, including one that tries to leave the folder, answers 404.

```bash
curl -fOJ https://vault.example/api/clients/latest/appimage
vault clients list --url https://vault.example
```

## The vault CLI

The server hands out the `vault` CLI of its own version for Linux `x86_64` and `aarch64`, so an AI agent, or any machine with `curl`, can install the client that matches the server without a Rust toolchain. Each build is one statically linked executable, built against musl, so it runs on any Linux distribution, Alpine included.

`./dev just cli-release` builds both into `target/cli-release/`, as `vault-<version>-linux-x86_64` and `vault-<version>-linux-aarch64`, with their `SHA256SUMS`. `./dev just cli-release-build x86_64` builds one. The dev image cross-compiles them with `cargo zigbuild`, zig being the C compiler and linker for SQLite, ring and zstd, under the `cli` profile: stripped, with thin LTO. `just publish-clients` publishes them as platform `cli`, kind `binary`; `vault clients publish` reads a file named `vault-<version>-linux-<arch>` the same way.

| Route | What it does |
|---|---|
| `GET /api/clients/cli/linux-{arch}` | The newest CLI build for `arch`, `x86_64` or `aarch64`, served as `GET /api/clients/{filename}` serves it, as `application/octet-stream`. 404 when no build for `arch` is published, or for any target but `linux-<arch>`. Public, like every download |

Install it from the server, with no manifest to parse:

```bash
curl -fsSL https://vault.example/api/clients/cli/linux-$(uname -m) -o ~/.local/bin/vault
chmod +x ~/.local/bin/vault
vault --url https://vault.example health
```

`/api/clients` lists the CLI builds too, with their `sha256`, to check a download against.

### When the server is newer

Every `vault` command that talks to a server first asks it for its version, `GET /api/version`, once per process. When the server's version is newer than the CLI's, by semver order, the CLI prints one line on stderr and carries on:

```text
vault 0.1.10 is older than the server 0.1.11; update with: curl -fsSL https://vault.example/api/clients/cli/linux-x86_64 -o $(command -v vault)
```

stdout, the command's JSON, is unchanged, and so is the exit status. A server of the same or an older version, or one that does not answer `/api/version`, adds nothing. The commands that work on a vault folder directly, such as `init`, `backup` and `clients publish`, ask no server.

## Updates

Publishing a build is all it takes for the apps paired with the server to update themselves:

1. `just publish-clients <dir>` or `vault clients publish` rewrites the clients folder's manifest.
2. Within 30 seconds the server notices the manifest changed and publishes `clients.updated` on `/api/events`, with every build listed now: `{"clients": [{"kind", "arch", "version", "build"}]}`. The push dispatcher sends each paired device with a UnifiedPush endpoint `{"event": "clients.updated"}`, coalesced like `note.changed`, so a burst of publishes is one push per 10 seconds at most.
3. **Linux desktop app**: it hears `clients.updated` on its event stream and checks at once, besides its checks at start and every 4 hours. Running as an AppImage, it downloads a newer signed AppImage from `/api/clients/update/...`, verifies its signature and shows "Update ready — restart to update" in the window and the tray; it installs on restart, or on quit. A deb or rpm install is told to update through its package manager, with the download link. See [`docs/desktop.md`](desktop.md#updates).
4. **Android app**: the push schedules its update job, which also runs once a day. The job reads `/api/clients`, downloads a newer APK of its own build type, checks its SHA-256 and installs it with a PackageInstaller session; once TheVault installed itself the first time, later updates install without asking. See [`docs/android-release.md`](android-release.md).

Publish the builds of a release with the server, or before it: from 0.1.9, an app that finds its server newer than itself at a sync round pauses sync, updates itself to the published build and then syncs, so a server upgrade no longer has to wait for the apps. With no matching build published, the app keeps syncing and Settings > Devices > This device warns. The server publishes each APK's `size` and serves a `Range` request with `206 Partial Content`, so the Android app resumes a short download. See [`docs/android-release.md`](android-release.md#the-app-updates-before-it-syncs).

Each app only ever moves to a newer version. Obtainium still works for the APK: add `https://<server>/api/clients/latest/apk` as a direct APK link, see [`docs/android-release.md`](android-release.md#with-obtainium).

### What to back up

The signing keys live outside the repository, under `~/.config/thevault`, and nothing else can make an update the installed apps accept:

- `android/thevault-release.jks` and `android/keystore.env`: the Android release keystore and its passwords.
- `updater/thevault-updater.key`, `updater/thevault-updater.key.pub` and `updater/updater.env`: the desktop updater key pair and its password.

**Back up that folder somewhere safe and private.** Losing the Android keystore means every phone must uninstall the app, losing its local data, and install a build signed with a new key. Losing the updater key means every desktop app must be updated by hand once, to a build that carries a new public key. Never commit any of them.
