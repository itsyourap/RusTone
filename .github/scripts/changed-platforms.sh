#!/usr/bin/env bash
# Prints which platforms a change needs to build, as a JSON list of names
# (e.g. ["linux-x64","linux-arm64"]), from the changed paths on stdin
# (one per line, as `git diff --name-only` prints them).
#
#   git diff --name-only BASE HEAD | changed-platforms.sh
#
# Documentation and other files that don't affect a build need nothing;
# files only one OS (or one target) uses build just those platforms;
# anything else (the shared Rust code, Cargo files, the workflows) builds
# all of them.
set -euo pipefail

all=(windows-x64 windows-arm64 linux-x64 linux-arm64 macos-arm64 macos-x64)
windows=(windows-x64 windows-arm64)
linux=(linux-x64 linux-arm64)
macos=(macos-arm64 macos-x64)
declare -A wanted=()

want() {
  for platform in "$@"; do wanted[$platform]=1; done
}

while IFS= read -r path; do
  [ -z "$path" ] && continue
  case "$path" in
    # No build needs these.
    *.md | LICENSE | .gitignore | .gitattributes | .claude/*) ;;

    # One target's bindings.
    crates/pjsua-sys/src/bindings/x86_64_pc_windows_msvc.rs) want windows-x64 ;;
    crates/pjsua-sys/src/bindings/aarch64_pc_windows_msvc.rs) want windows-arm64 ;;
    crates/pjsua-sys/src/bindings/x86_64_unknown_linux_gnu.rs) want linux-x64 ;;
    crates/pjsua-sys/src/bindings/aarch64_unknown_linux_gnu.rs) want linux-arm64 ;;
    crates/pjsua-sys/src/bindings/aarch64_apple_darwin.rs) want macos-arm64 ;;
    crates/pjsua-sys/src/bindings/x86_64_apple_darwin.rs) want macos-x64 ;;

    # Windows only.
    crates/rustone-win/* | packaging/windows/* | packaging/icons/*.ico \
      | .github/actions/msvc/* | .github/vcpkg-triplets/* | crates/sip-engine/src/hotplug/windows.rs)
      want "${windows[@]}" ;;

    # Linux only.
    packaging/linux/* | .github/scripts/linux-deps.sh | crates/sip-engine/src/hotplug/linux.rs)
      want "${linux[@]}" ;;

    # macOS only.
    crates/rustone-mac/* | packaging/macos/* | .github/scripts/macos-deps.sh | crates/sip-engine/src/hotplug/macos.rs)
      want "${macos[@]}" ;;

    # pjproject's configure build (Linux and macOS; Windows uses MSBuild).
    .github/scripts/build-pjproject.sh)
      want "${linux[@]}" "${macos[@]}" ;;

    # Everything else is shared.
    *) want "${all[@]}" ;;
  esac
done

# In the platform list's order.
out=()
for platform in "${all[@]}"; do
  if [ -n "${wanted[$platform]:-}" ]; then out+=("\"$platform\""); fi
done
(IFS=,; echo "[${out[*]}]")
