#!/usr/bin/env bash
# Finishes RusTone.app after `cargo packager -f app` and makes the .dmg.
#
#   bundle.sh <RusTone.app> <output.dmg>
#
# 1. Copies the Homebrew libraries the binary links (OpenSSL, SDL2,
#    OpenH264, AMR, …) into Contents/Frameworks and points the binary at
#    them (dylibbundler), plus SDL3, which sdl2-compat loads at run time,
#    so the app runs without Homebrew.
# 2. Signs the libraries and the app: with MACOS_SIGNING_IDENTITY
#    ("Developer ID Application: …", hardened runtime, entitlements) when
#    set, otherwise ad hoc (Apple Silicon runs no unsigned code).
# 3. Puts the app and an Applications link in a compressed disk image.
set -euo pipefail

app=$1
dmg=$2
here=$(cd "$(dirname "$0")" && pwd)
exe="$app/Contents/MacOS/rustone"
frameworks="$app/Contents/Frameworks"

command -v dylibbundler > /dev/null || brew install dylibbundler
mkdir -p "$frameworks"
dylibbundler --overwrite-files --bundle-deps --create-dir \
  --fix-file "$exe" --dest-dir "$frameworks" --install-path "@executable_path/../Frameworks/"

# Homebrew's SDL2 is sdl2-compat, which loads SDL3 at run time (dlopen, so
# dylibbundler can't see it), first from next to itself.
if sdl3=$(brew --prefix sdl3 2> /dev/null) && [ -f "$sdl3/lib/libSDL3.dylib" ]; then
  cp -L "$sdl3/lib/libSDL3.dylib" "$frameworks/libSDL3.dylib"
  chmod u+w "$frameworks/libSDL3.dylib"
  install_name_tool -id "@executable_path/../Frameworks/libSDL3.dylib" "$frameworks/libSDL3.dylib"
  dylibbundler --overwrite-files --bundle-deps \
    --fix-file "$frameworks/libSDL3.dylib" --dest-dir "$frameworks" --install-path "@executable_path/../Frameworks/"
fi
# Nothing may still point into Homebrew.
if otool -L "$exe" "$frameworks"/*.dylib | grep -E "$(brew --prefix)|/usr/local/(opt|Cellar)"; then
  echo "error: the app still links Homebrew libraries" >&2
  exit 1
fi

identity=${MACOS_SIGNING_IDENTITY:--}
if [ "$identity" = "-" ]; then
  echo "signing ad hoc (no MACOS_SIGNING_IDENTITY)"
  sign=(codesign --force --sign -)
else
  echo "signing as $identity"
  sign=(codesign --force --timestamp --options runtime --sign "$identity")
fi
# Inside out: the libraries first, then the app with its entitlements.
find "$frameworks" -type f -name '*.dylib' -print0 | while IFS= read -r -d '' lib; do
  "${sign[@]}" "$lib"
done
"${sign[@]}" --entitlements "$here/entitlements.plist" "$app"
codesign --verify --strict --verbose=2 "$app"

staging=$(mktemp -d)
cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
rm -f "$dmg"
hdiutil create -volname RusTone -srcfolder "$staging" -ov -format UDZO "$dmg"
rm -rf "$staging"
if [ "$identity" != "-" ]; then
  codesign --force --timestamp --sign "$identity" "$dmg"
fi
echo "made $dmg"
