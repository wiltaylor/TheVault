# Handing out the apps from your server

A server can hand out the Android APK and the Linux AppImage, deb and rpm itself, so a phone or a new computer gets the app straight from the server it will pair with. The builds are public: anyone who reaches the server can download them without logging in. They hold no vault data and no secrets.

## Where the builds live

The server reads one folder, its clients folder:

- `vault-server --clients <dir>`, or `VAULT_CLIENTS_DIR=<dir>`, names it.
- Without either, it is `<vault>/.vault/clients`. `.vault/` is the server's private folder: nothing in it syncs to devices, is indexed, or goes into `vault backup`.

The folder holds the build files and a `manifest.json` that lists them. The server serves only the files the manifest lists, and reads the manifest on every request, so a newly published build shows at once, with no restart. A folder that does not exist is an empty list.

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
| `platform` | `android` or `linux` |
| `kind` | `apk`, `appimage`, `deb` or `rpm` |
| `arch` | `aarch64`, `x86_64`, `armv7`, `x86` or `universal` |
| `version` | The app's version |
| `build` | `release`, signed with the release key, or `debug`, signed with the build machine's debug key |
| `size`, `sha256` | The file's size in bytes and its SHA-256 |
| `filename` | The file's name in the folder: `TheVault_<version>_<arch>[-debug].<ext>` |
| `published` | When it was published, in milliseconds since the epoch |

The folder holds one build per kind and architecture. Publishing a build replaces the one before it and deletes the old file.

## Publish

Build the apps, then publish them into the folder the server reads:

```bash
./dev just publish-clients .scratch/my-vault/.vault/clients
```

`just publish-clients <dir>` publishes the newest builds it finds:

- Android: the signed APK in `target/android-release/` (`just android-release`). Without one, it publishes the debug APK of the last `just android-build` and labels it `debug`, so it works without a release keystore.
- Linux: the AppImage, deb and rpm in `target/desktop-release/` (`just desktop-release`).

With `build='missing'`, the default, it builds what is not built yet: the debug APK when the Android toolchain is there, and the desktop release bundles. `just publish-clients <dir> none` publishes only what is already built. It prints what the folder holds when it is done.

`./dev` only sees the checkout, so a `<dir>` outside it must be published from the host with the `vault` binary instead:

```bash
vault clients publish --dir /srv/thevault/.vault/clients target/android-release/TheVault_0.1.0_aarch64.apk
vault clients publish --vault /srv/thevault --build debug --version 0.1.0 app-arm64-debug.apk
```

`vault clients publish` copies each file in, reads its kind from the extension and its version and architecture from the file name, and rewrites the manifest. `--version`, `--arch` and `--build` say what the file name does not; `--dir` defaults to `<vault>/.vault/clients`. It prints the new manifest. The files are copied and the manifest written under temporary names and renamed into place, so a server reading the folder meanwhile sees either the old list or the new one.

A debug APK installs over an earlier debug APK only when the same machine built both, and never over a release build, because Android checks that the same key signed them.

## Download

- **The log-in page** links to **Get the apps**, at `/apps`, which works without logging in.
- **Settings > Devices** shows the same list at the top.

Each build shows its version, `release` or `debug`, architecture, size, a download button and its SHA-256 with a copy button. The APK also has a QR code of its download address: scan it with the phone's camera to download the APK on the phone. The address is the one the page was loaded from, so load the page at the address the phone reaches the server on.

With nothing published, both places explain how to publish.

### The API

| Route | What it does |
|---|---|
| `GET /api/clients` | The manifest's builds, each with `path`, its download path, `url`, its absolute address as the request reached the server, and for an APK `qr_svg`, the QR code of `url`. `enabled` is false on an app's own embedded server, which hands out nothing |
| `GET /api/clients/{filename}` | One build's bytes, as an attachment with its media type (`application/vnd.android.package-archive` for an APK, `application/octet-stream` for an AppImage, `application/vnd.debian.binary-package`, `application/x-rpm`), `Content-Length`, one `Range` (206), and the SHA-256 as its `ETag` |
| `GET /api/clients/latest/{kind}[?arch=]` | The newest build of `apk`, `appimage`, `deb` or `rpm`, for `arch` or the kind's usual architecture (`aarch64` for an APK, `x86_64` otherwise). The address never changes, so an updater can follow it |

All three are public, `GET` and `HEAD` only. A name the manifest does not list, including one that tries to leave the folder, answers 404.

```bash
curl -fOJ https://vault.example/api/clients/latest/appimage
vault clients list --url https://vault.example
```

## Updates

- **Linux desktop app**: its update check asks the paired server's `/api/clients` first and offers the server's AppImage when it is newer than the app. It asks GitHub Releases only when the server publishes no AppImage or cannot be reached. See [`docs/desktop.md`](desktop.md#updates).
- **Android app**: add `https://<server>/api/clients/latest/apk` to Obtainium as a direct APK link. See [`docs/android-release.md`](android-release.md#with-obtainium).
