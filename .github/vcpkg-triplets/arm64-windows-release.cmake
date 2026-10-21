# Release-only, dynamic ARM64 builds (OpenSSL's DLLs), like vcpkg's community
# x64-windows-release triplet, which has no ARM64 counterpart.
set(VCPKG_TARGET_ARCHITECTURE arm64)
set(VCPKG_CRT_LINKAGE dynamic)
set(VCPKG_LIBRARY_LINKAGE dynamic)
set(VCPKG_BUILD_TYPE release)
