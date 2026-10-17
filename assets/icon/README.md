# App icon

The three 1024 px sources every app icon is made from:

- `icon-full.png`: the full-bleed icon, the mark on navy `#0b1020`.
- `icon-foreground.png`: the mark alone on transparent, inside the central 66%.
- `icon-monochrome.png`: the same mark as a white silhouette, for Android themed icons and the status bar.

To regenerate every derived icon after changing a source, run from the repository root:

```bash
./dev assets/icon/generate.sh
```

It writes:

| Output | Made from |
| --- | --- |
| `web/public/favicon.ico` (16, 32, 48), `icon-32.png`, `icon-192.png`, `icon-512.png`, `apple-touch-icon.png` (180) | `icon-full.png` |
| `web/public/icon-maskable-512.png` | `icon-foreground.png` on navy, inside the 40% maskable safe circle |
| `apps/{desktop,android}/src-tauri/icons/*` | `cargo tauri icon icon-full.png`; the desktop also keeps its `64x64.png` |
| `apps/desktop/src-tauri/icons/tray.png` (64) | `icon-foreground.png`, trimmed; `tray.rs` draws the unread dot |
| Android `mipmap-*/ic_launcher.png`, `ic_launcher_round.png` | `cargo tauri icon icon-full.png` |
| Android `mipmap-*/ic_launcher_foreground.png`, `ic_launcher_monochrome.png` (108 dp) | `icon-foreground.png`, `icon-monochrome.png` at 83%, inside the 66 dp safe circle |
| Plugin `drawable-*/ic_stat_thevault.png` (24 dp) | `icon-monochrome.png`, trimmed to 20 dp |

The adaptive icon XML (`mipmap-anydpi-v26/ic_launcher*.xml`) and the background colour
(`values/ic_launcher_background.xml`) are written by hand and do not change.
