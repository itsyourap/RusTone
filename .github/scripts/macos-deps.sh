#!/usr/bin/env bash
# Installs what building pjproject and RusTone needs on macOS (Homebrew).
# sdl2-compat provides the SDL2 API (on top of SDL3) that pjproject uses.
set -euo pipefail

export HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1
brew install pkgconf openssl@3 sdl2-compat openh264 opencore-amr vo-amrwbenc

# pjproject's configure and pkg-config put the Homebrew prefix's include
# and lib folders (for SDL and the codecs) before openssl@3's, so another
# OpenSSL linked into the prefix would be compiled and linked instead: the
# arm64 runner image links a newer one whose ASN1_TIME is opaque, which
# pjproject's OpenSSL code doesn't build against. Unlink whichever formula
# owns the prefix's OpenSSL headers unless it's openssl@3.
headers=$(brew --prefix)/include/openssl
target=$(readlink "$headers" 2>/dev/null || readlink "$headers/opensslv.h" 2>/dev/null || true)
owner=$(printf '%s\n' "$target" | sed -n 's#.*/Cellar/\([^/]*\)/.*#\1#p')
if [ -n "$owner" ] && [ "$owner" != openssl@3 ]; then
  echo "Unlinking $owner: its OpenSSL headers in $headers would shadow openssl@3's"
  brew unlink "$owner"
fi
echo "OpenSSL in $(brew --prefix): $(grep -h 'define OPENSSL_VERSION_TEXT' "$headers/opensslv.h" 2>/dev/null || echo 'none linked')"
echo "openssl@3: $(grep -h 'define OPENSSL_VERSION_TEXT' "$(brew --prefix openssl@3)/include/openssl/opensslv.h")"
