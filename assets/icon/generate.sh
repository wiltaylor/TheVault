#!/usr/bin/env bash
# Regenerates every app icon from the three 1024 px sources beside this script.
# Run it from the repository root through the dev container: ./dev assets/icon/generate.sh
set -euo pipefail

cd "$(dirname "$0")/../.."
src=assets/icon
full=$src/icon-full.png
fg=$src/icon-foreground.png
mono=$src/icon-monochrome.png
navy='#0b1020'
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Web: favicons, the PWA icons and the Apple touch icon. The maskable icon puts the foreground
# mark (furthest pixel 36.5% of the width from the centre) on navy, inside the 40% safe circle;
# icon-full.png reaches 41.7% and would lose the corners of its hinges.
pub=web/public
convert "$full" -define icon:auto-resize=48,32,16 "$pub/favicon.ico"
convert "$full" -resize 32x32 PNG32:"$pub/icon-32.png"
convert "$full" -resize 192x192 PNG32:"$pub/icon-192.png"
convert "$full" -resize 512x512 PNG32:"$pub/icon-512.png"
convert "$full" -resize 180x180 PNG24:"$pub/apple-touch-icon.png"
convert -size 1024x1024 xc:"$navy" "$fg" -composite -resize 512x512 PNG24:"$pub/icon-maskable-512.png"

# Desktop and Android bundle icons: Tauri's standard set, from the full-bleed icon.
cargo tauri icon "$full" -o "$tmp/tauri" >/dev/null
for dir in apps/desktop/src-tauri/icons apps/android/src-tauri/icons; do
  cp "$tmp"/tauri/{32x32.png,128x128.png,128x128@2x.png,icon.icns,icon.ico,icon.png,StoreLogo.png} "$dir/"
  cp "$tmp"/tauri/Square*Logo.png "$dir/"
done

# The desktop app installs the 64 px icon too, with the AppImage's application menu entry.
cp "$tmp"/tauri/64x64.png apps/desktop/src-tauri/icons/

# Desktop tray: the mark alone on transparent, 64 px for HiDPI panels. tray.rs draws the unread dot.
convert "$fg" -trim +repage -resize 60x60 -gravity center -background none -extent 64x64 \
  PNG32:apps/desktop/src-tauri/icons/tray.png

# Android launcher. Legacy icons come from Tauri; the adaptive layers are 108 dp, with the mark
# scaled to 83% so its furthest pixel sits inside the 66 dp safe circle. The background layer is
# the ic_launcher_background colour in values/ic_launcher_background.xml.
res=apps/android/src-tauri/gen/android/app/src/main/res
stat=plugins/tauri-plugin-vault-device/android/src/main/res
for pair in mdpi:2 hdpi:3 xhdpi:4 xxhdpi:6 xxxhdpi:8; do
  density=${pair%%:*}
  half=${pair##*:} # the scale in half steps: mdpi is 1x, hdpi 1.5x
  cp "$tmp/tauri/android/mipmap-$density/ic_launcher.png" "$tmp/tauri/android/mipmap-$density/ic_launcher_round.png" \
    "$res/mipmap-$density/"
  layer=$((108 * half / 2))
  mark=$(((layer * 83 + 50) / 100))
  for kind in foreground monochrome; do
    source=$fg
    [ "$kind" = monochrome ] && source=$mono
    convert "$source" -resize "${mark}x${mark}" -gravity center -background none -extent "${layer}x${layer}" \
      PNG32:"$res/mipmap-$density/ic_launcher_$kind.png"
  done
  # The status bar icon: a white silhouette, 24 dp with the mark inside 20 dp.
  mkdir -p "$stat/drawable-$density"
  convert "$mono" -trim +repage -resize "$((20 * half / 2))x$((20 * half / 2))" -gravity center -background none \
    -extent "$((24 * half / 2))x$((24 * half / 2))" PNG32:"$stat/drawable-$density/ic_stat_thevault.png"
done
