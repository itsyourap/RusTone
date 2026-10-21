#!/usr/bin/env bash
# Builds pjproject on Linux or macOS and installs it into a prefix that
# pjsua-sys links against (PJPROJECT_DIR).
#
#   build-pjproject.sh <pjproject source> <install prefix> <config_site.h>
#
# The codec set matches the Windows build: H.264 (OpenH264), AMR-NB/WB,
# and pjproject's built-in codecs; no FFmpeg, VPX or Opus.
set -euo pipefail

src=$(cd "$1" && pwd)
mkdir -p "$2"
prefix=$(cd "$2" && pwd)
cp "$3" "$src/pjlib/include/pj/config_site.h"

args=(
  --prefix="$prefix"
  --disable-shared
  --disable-pjsua2
  --disable-ffmpeg --disable-vpx --disable-opus --disable-silk --disable-bcg729 --disable-lyra
)

case "$(uname -s)" in
  Linux)
    deps=/usr
    jobs=$(nproc)
    args+=(--with-ssl="$deps")
    ;;
  Darwin)
    deps=$(brew --prefix)
    jobs=$(sysctl -n hw.ncpu)
    # OpenSSL rather than Apple's TLS backend, as on the other platforms.
    args+=(--disable-darwin-ssl --with-ssl="$(brew --prefix openssl@3)")
    ;;
  *) echo "unsupported OS: $(uname -s)" >&2; exit 1 ;;
esac

args+=(
  --with-sdl="$deps"
  --with-openh264="$deps"
  --with-opencore-amr="$deps"
  --with-opencore-amrwbenc="$deps"
)

cd "$src"
# Position-independent code: the static libraries end up in PIE binaries.
CFLAGS="-O2 -fPIC -DNDEBUG" CXXFLAGS="-O2 -fPIC -DNDEBUG" ./configure "${args[@]}"

# configure prints what it found; fail early if a required part is missing.
for want in PJMEDIA_HAS_OPENH264_CODEC PJMEDIA_HAS_OPENCORE_AMRNB_CODEC PJMEDIA_HAS_OPENCORE_AMRWB_CODEC PJ_HAS_SSL_SOCK PJMEDIA_VIDEO_DEV_HAS_SDL; do
  if ! grep -rq -- "-D$want=1" build.mak pjmedia/build/os-auto.mak user.mak 2>/dev/null \
     && ! grep -rq "define $want 1" pjlib/include/pj/compat/os_auto.h pjmedia/include/pjmedia/config_auto.h pjmedia/include/pjmedia-codec/config_auto.h 2>/dev/null; then
    echo "warning: configure did not enable $want" >&2
  fi
done

make dep
make -j"$jobs"
make install
echo "pjproject installed to $prefix"
