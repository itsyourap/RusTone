#!/usr/bin/env bash
# Makes packaging/icons/rustone.icns from the rendered PNGs with Apple's
# iconutil. macOS icons come in 16-512 pt at 1x and 2x, so the 64 and
# 1024 px images are the @2x versions of 32 and 512.
set -euo pipefail

icons=$(cd "$(dirname "$0")/../icons" && pwd)
iconset=$(mktemp -d)/rustone.iconset
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  cp "$icons/${size}x${size}.png" "$iconset/icon_${size}x${size}.png"
  double=$((size * 2))
  cp "$icons/${double}x${double}.png" "$iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$iconset" -o "$icons/rustone.icns"
rm -rf "$(dirname "$iconset")"
echo "made $icons/rustone.icns"
