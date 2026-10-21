#!/usr/bin/env bash
# Prints which platforms a change needs to build, as a JSON list of names
# (e.g. ["linux-x64","macos-arm64"]), from the changed paths on stdin
# (one per line, as `git diff --name-only` prints them).
#
#   git diff --name-only BASE HEAD | changed-platforms.sh
#
# Documentation and other files that don't affect a build need nothing;
# files only one platform uses build that platform; anything else (the
# shared Rust code, Cargo files, the workflows) builds all of them.
set -euo pipefail

all=(windows-x64 linux-x64 macos-arm64)
declare -A wanted=()

want() {
  for platform in "$@"; do wanted[$platform]=1; done
}

while IFS= read -r path; do
  [ -z "$path" ] && continue
  case "$path" in
    # No build needs these.
    *.md | LICENSE | .gitignore | .gitattributes | .claude/*) ;;

    # Windows only.
    crates/rustone-win/* | packaging/windows/* | .github/actions/msvc/* | packaging/icons/*.ico \
      | crates/pjsua-sys/src/bindings/x86_64_pc_windows_msvc.rs | crates/sip-engine/src/hotplug/windows.rs)
      want windows-x64 ;;

    # Linux only.
    packaging/linux/* | .github/scripts/linux-deps.sh \
      | crates/pjsua-sys/src/bindings/x86_64_unknown_linux_gnu.rs | crates/sip-engine/src/hotplug/linux.rs)
      want linux-x64 ;;

    # macOS only.
    crates/rustone-mac/* | packaging/macos/* | .github/scripts/macos-deps.sh \
      | crates/pjsua-sys/src/bindings/aarch64_apple_darwin.rs | crates/sip-engine/src/hotplug/macos.rs)
      want macos-arm64 ;;

    # pjproject's configure build (Linux and macOS; Windows uses MSBuild).
    .github/scripts/build-pjproject.sh)
      want linux-x64 macos-arm64 ;;

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
