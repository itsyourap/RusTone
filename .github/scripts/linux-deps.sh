#!/usr/bin/env bash
# Installs what building pjproject and RusTone needs on Ubuntu 24.04
# (the CI runner and the local build container).
set -euo pipefail

packages=(
  build-essential pkg-config git curl ca-certificates
  # pjproject: TLS, video window, H.264, AMR, sound and camera backends
  libssl-dev libsdl2-dev libopenh264-dev libopencore-amrnb-dev libopencore-amrwb-dev libvo-amrwbenc-dev
  libasound2-dev libv4l-dev uuid-dev
  # bindgen (pjsua-sys regen)
  libclang-dev clang
  # RusTone: eframe (X11/Wayland), tray, file dialogs, HID headsets,
  # Secret Service, idle detection
  libgtk-3-dev libxdo-dev libayatana-appindicator3-dev libudev-dev libdbus-1-dev
  libxss-dev libx11-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev
)

sudo=""
if [ "$(id -u)" -ne 0 ]; then sudo="sudo"; fi
export DEBIAN_FRONTEND=noninteractive
$sudo apt-get update -q
$sudo apt-get install -y -q --no-install-recommends "${packages[@]}"
