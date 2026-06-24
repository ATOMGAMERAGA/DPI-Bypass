#!/usr/bin/env bash
# Generate all icon/derivative assets from the brand master.
#
# Source of truth is logo.svg (vector). When a proper SVG rasterizer
# (rsvg-convert / inkscape) is available we render straight from the vector at
# native resolution. Otherwise we DOWNSCALE the 1600x1600 logo.png — downscaling
# never loses sharpness, and logo.png is itself a high-res render of logo.svg.
# Upscaling a small raster is never done.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SVG="$ROOT/logo.svg"
PNG="$ROOT/logo.png"
ICONS="$ROOT/src-tauri/icons"
ASSETS="$ROOT/assets"
mkdir -p "$ICONS" "$ASSETS"

# render <size> <out> — vector if possible, else high-res downscale.
render() {
  local size="$1" out="$2"
  if command -v rsvg-convert >/dev/null 2>&1; then
    rsvg-convert -w "$size" -h "$size" "$SVG" -o "$out"
  elif command -v inkscape >/dev/null 2>&1; then
    inkscape "$SVG" --export-type=png --export-filename="$out" -w "$size" -h "$size" >/dev/null 2>&1
  else
    convert -background none "$PNG" -resize "${size}x${size}" "$out"
  fi
}

echo "==> Tauri icon set -> $ICONS"
render 32  "$ICONS/32x32.png"
render 128 "$ICONS/128x128.png"
render 256 "$ICONS/128x128@2x.png"
render 512 "$ICONS/icon.png"
render 16  "$ICONS/16x16.png"

# Windows .ico (multi-resolution) and macOS .icns.
convert "$ICONS/16x16.png" "$ICONS/32x32.png" "$ICONS/128x128.png" \
        "$ICONS/128x128@2x.png" "$ICONS/icon.ico"
if ! convert "$ICONS/icon.png" "$ICONS/icon.icns" 2>/dev/null; then
  echo "   (icns: ImageMagick could not write it; copying 512 png as placeholder)"
  cp "$ICONS/icon.png" "$ICONS/icon.icns"
fi

echo "==> Distribution PNG set -> $ASSETS"
for s in 32 128 256 512 1024; do
  render "$s" "$ASSETS/icon-$s.png"
done

echo "==> Tray status variants -> $ASSETS"
# Active = green status dot, inactive = red, composited bottom-right.
make_tray() {
  local color="$1" out="$2"
  render 44 "$out"
  convert "$out" \
    \( -size 18x18 xc:none -fill "$color" -draw "circle 9,9 9,1" \) \
    -gravity SouthEast -geometry +1+1 -composite "$out"
}
make_tray "#2ecc71" "$ASSETS/tray-active.png"
make_tray "#e74c3c" "$ASSETS/tray-inactive.png"

echo "Done. Generated:"
ls -1 "$ICONS" "$ASSETS"
