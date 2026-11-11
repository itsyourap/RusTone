#!/usr/bin/env bash
# Installs what building pjproject and RusTone needs on macOS (Homebrew).
# sdl2-compat provides the SDL2 API (on top of SDL3) that pjproject uses.
set -euo pipefail

export HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1
brew install pkgconf openssl@3 sdl2-compat openh264 opencore-amr vo-amrwbenc

# pjproject's configure and pkg-config put the Homebrew prefix's include
# and lib folders (for SDL and the codecs) before openssl@3's, so whichever
# OpenSSL is linked into the prefix is the one compiled and linked. The
# arm64 runner image links others (openssl@4, whose ASN1_TIME is opaque and
# which pjproject's OpenSSL code doesn't build against, and openssl@1.1):
# unlink every OpenSSL formula but openssl@3, then link openssl@3 into the
# prefix (it's keg-only next to a newer one), replacing any leftover links,
# so pjproject and RusTone's link both find it there.
for formula in $(brew list --formula -1 | grep -E '^openssl(@[0-9.]+)?$' || true); do
  if [ "$formula" != openssl@3 ]; then
    echo "Unlinking $formula, which would shadow openssl@3"
    brew unlink "$formula"
  fi
done
headers=$(brew --prefix)/include/openssl
owner=$(readlink "$headers" 2>/dev/null || readlink "$headers/opensslv.h" 2>/dev/null || true)
case "$owner" in
  */Cellar/openssl@3/*) ;;
  *) brew link --force --overwrite openssl@3 ;;
esac
echo "OpenSSL in $(brew --prefix): $(grep -h 'define OPENSSL_VERSION_TEXT' "$headers/opensslv.h" 2>/dev/null || echo 'none linked')"
echo "openssl@3: $(grep -h 'define OPENSSL_VERSION_TEXT' "$(brew --prefix openssl@3)/include/openssl/opensslv.h")"
