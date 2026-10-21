#!/usr/bin/env bash
# Installs what building pjproject and RusTone needs on macOS (Homebrew).
# sdl2-compat provides the SDL2 API (on top of SDL3) that pjproject uses.
set -euo pipefail

export HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1
brew install pkgconf openssl@3 sdl2-compat openh264 opencore-amr vo-amrwbenc
