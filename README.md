<!-- The centred header needs HTML, so it comes before the first heading. -->
<!-- markdownlint-disable MD033 MD041 -->
<div align="center">

<img src="packaging/icons/256x256.png" alt="RusTone" width="112">

# RusTone

**A fast, modern SIP softphone for Windows, Linux and macOS, on x64 and ARM.**

Voice and video calls, messaging, presence and contacts over any standard SIP service,
in a native app written in Rust on top of the proven pjproject SIP stack.

[![CI](https://github.com/itsyourap/RusTone/actions/workflows/ci.yml/badge.svg)](https://github.com/itsyourap/RusTone/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue.svg)](LICENSE)
![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20Linux%20%7C%20macOS-555)
![Architectures](https://img.shields.io/badge/arch-x64%20%7C%20ARM64-555)

[Download](#quickstart) · [Features](#features) · [How it works](#how-it-works) · [Build from source](#building-from-source)

</div>
<!-- markdownlint-enable MD033 -->

---

## What is RusTone?

RusTone is a desktop softphone: it turns your computer into a phone for a SIP account from a
PBX (Asterisk, FreeSWITCH, 3CX, …) or a VoIP provider. It registers
your account, places and receives voice and video calls, sends and receives text messages, and
shows who in your contacts is available.

It is built to stay out of the way. RusTone lives in the system tray, rings in a compact window
when a call comes in, and keeps working with its window closed. Everything is on one small screen: the dialer, your call history, contacts and
conversations.

## Features

### Calling

- Voice and video calls, with the remote video and a picture-in-picture preview drawn inside the window; full screen; turn video on during an audio call
- Several calls at once, call waiting, hold, mute, and local conferences
- Blind and attended transfer (SIP REFER or the PBX's feature codes) and call pickup
- DTMF over RFC 2833, SIP INFO or in-band audio (automatic by default)
- Call recording to WAV, by hand or automatically
- Auto-answer (all calls, on the caller's request, after a delay, or only for some numbers), do not disturb, call forwarding and auto hang-up
- Dial plan rules and a dialling prefix; hide your caller ID
- Call quality at a glance: codec, bitrate and an estimated MOS

### Messaging and presence

- SIP text messages, kept per conversation next to that contact's calls
- Presence and busy-lamp field (BLF) for contacts, with pickup of a ringing colleague
- Your own status: Online, Away when you're idle, Busy in do not disturb
- Voicemail indicator (MWI) with a one-click voicemail button

### Contacts and history

- Contacts with search and favourites; vCard (`.vcf`) import and export
- A company users directory (JSON, XML, Yealink or CSV from a URL), kept in sync
- Call history with filters and CSV export, missed-call badges and notifications
- Speed-dial buttons, with presence lights

### Audio

- Microphone, speaker and ringtone devices chosen by name, followed when you plug devices in or out
- Echo cancellation, voice activity detection, codec order, and level meters
- USB telephony headsets (hook switch, mute and LEDs) and media keys or Bluetooth headset buttons

### Desktop integration

- Lives in the tray; optional start with the system, hidden
- Opens `sip:`, `tel:` and `callto:` links from browsers and other apps
- Command line and single instance: `rustone sip:1001@pbx.example`, `rustone /answer`, `rustone /hangupall`, …
- Command hooks that run your own programs on call events (e.g. to open a CRM record)
- Light and dark themes that follow the system, and a built-in SIP debugger with logs, SIP messages, RTP statistics and audio meters

### Accounts and security

- SIP over UDP, TCP or TLS, with SRTP (SDES, DTLS) for encrypted media
- STUN and ICE for NAT traversal, outbound proxies and DNS SRV
- Passwords are kept in the operating system's credential store (Windows Credential Manager, macOS Keychain, the Secret Service on Linux), never in a file
- A local "call by IP" account for direct calls without a server

**Codecs**: G.711 (PCMA/PCMU), G.722, AMR-NB, AMR-WB, Speex, iLBC, GSM and G.722.1 for audio; H.264 for video.

## Quickstart

Download the installer for your system from the
[latest release](https://github.com/itsyourap/RusTone/releases/latest):

| System | Download | Install |
| --- | --- | --- |
| Windows 10/11 | `rustone_<version>_x64-setup.exe` (Intel/AMD) or `rustone_<version>_arm64-setup.exe` (ARM) | Run it; it installs for your user, no administrator rights needed |
| | `rustone_<version>_x64_en-US.msi` or `rustone_<version>_arm64_en-US.msi` | For managed or per-machine deployment |
| Ubuntu 24.04+ / Debian 13+ | `rustone_<version>_amd64.deb` (x64) or `rustone_<version>_arm64.deb` (ARM) | `sudo apt install ./rustone_<version>_<arch>.deb` |
| Other Linux | `rustone_<version>_x86_64.AppImage` (x64) or `rustone_<version>_aarch64.AppImage` (ARM) | `chmod +x rustone_*.AppImage`, then run it |
| macOS 13+ | `RusTone_<version>_aarch64.dmg` (Apple silicon) or `RusTone_<version>_x64.dmg` (Intel) | Open it and drag RusTone to Applications |

Every system is built for both 64-bit Intel/AMD (x64) and 64-bit ARM.

Then:

1. Start **RusTone**. It opens on the dialer and adds an icon to the tray (menu bar on macOS).
2. Open **Settings** (the gear in the header) and add your SIP account under **General → Accounts**:
   server, user name, password and, if your provider needs them, the proxy, domain and transport.
3. When the status turns green (**Registered**), type a number and press the call button.

> [!NOTE]
> Until the release builds are code-signed, Windows SmartScreen may warn about an unknown
> publisher (choose **More info → Run anyway**), and macOS asks you to open the app the first
> time with right-click → **Open**.

## How it works

```text
┌──────────────────────────── rustone (egui + fastframe) ───────────────────────────────┐
│  Dialer · Calls · Contacts · Messages · Settings · Debug         tray · links · hooks │
└─────────────────────────────┬────────▲────────────────────────────────────────────────┘
                              │        │ events
                     commands ▼        │
┌──────────────────────────── sip-engine (engine thread) ────────────────────────────┐
│  owns pjsua: accounts, calls, media, presence, messages, video taps, device watch  │
└──────────────────────────────────────┬─────────────────────────────────────────────┘
                                       │ FFI (pjsua-sys, bindgen)
                                       │
┌──────────────────────────── pjproject (statically linked) ──────────────────────────┐
│  SIP · RTP/SRTP · ICE/STUN · TLS · audio devices · codecs · video conference bridge │
└─────────────────────────────────────────────────────────────────────────────────────┘
```

- **pjproject does the telephony.** RusTone links [pjproject](https://github.com/pjsip/pjproject)'s
  `pjsua` library statically, from [a fork](https://github.com/itsyourap/pjproject/tree/rustone-2.17)
  of release 2.17 with one fix for media offers in early dialogs. The `pjsua-sys` crate holds
  the generated bindings for each platform and a few small C helpers.
- **One thread owns the SIP stack.** `sip-engine` runs pjsua on a dedicated thread. The rest of
  the app sends it commands and receives events, so the interface never waits on the network
  and calls keep working while no window is open.
- **The interface is a view over the app's state.** The window is drawn with
  [egui](https://github.com/emilk/egui); [fastframe](https://github.com/crmne/fastframe) provides
  the tray-resident shell, single instance, notifications, theming and other desktop services.
  Closing the window only closes the window: RusTone keeps running in the tray, and an incoming
  call brings up a ringing window.
- **Video is drawn in the window.** Decoded video frames are tapped from pjsua's video
  conference bridge, converted to RGBA and drawn by egui, so remote video and the preview are
  part of the call screen rather than separate windows.
- **Your data stays local.** Settings are a TOML file; contacts, history and messages are in a
  SQLite database in your profile; passwords are in the system's credential store.

### Repository layout

| Crate | Purpose |
| --- | --- |
| [`crates/rustone-app`](crates/rustone-app) | The desktop app (`rustone`): interface, tray, notifications, integrations |
| [`crates/sip-engine`](crates/sip-engine) | Safe, threaded wrapper around pjsua |
| [`crates/pjsua-sys`](crates/pjsua-sys) | Raw bindings to pjsua, link setup, and the C shim |
| [`crates/rustone-core`](crates/rustone-core) | The app model: settings, accounts, contacts, history, rules (no GUI, no unsafe code) |
| [`crates/rustone-win`](crates/rustone-win) / [`crates/rustone-mac`](crates/rustone-mac) | Windows and macOS integrations that need system APIs (jump list, link handling) |
| [`crates/rustone-cli`](crates/rustone-cli) | `rustone-cli`, a command-line tool to test a SIP account without the app |
| [`packaging`](packaging) | Installer configuration, icons and signing scripts |

## Building from source

RusTone is a Cargo workspace. Building it takes two steps: build pjproject (with its codec and
TLS libraries), then point Cargo at it. The [CI workflow](.github/workflows/platform.yml) does
exactly this for Windows, Linux and macOS on both x64 and ARM64, and is the reference recipe.
The steps below are the same on either architecture.

You need [Rust](https://rustup.rs) 1.98 or newer (stable), Git, and the platform's C toolchain.

### Linux (Ubuntu 24.04 / Debian 13, x64 or ARM64)

```sh
git clone https://github.com/itsyourap/RusTone && cd RusTone

# Build tools and libraries (OpenSSL, SDL2, OpenH264, AMR, ALSA, V4L2, GTK, …)
bash .github/scripts/linux-deps.sh

# pjproject from the fork, installed into ../pjproject-install
git clone --branch rustone-2.17 https://github.com/itsyourap/pjproject ../pjproject
bash .github/scripts/build-pjproject.sh ../pjproject ../pjproject-install crates/pjsua-sys/config_site.h

export PJPROJECT_DIR=$PWD/../pjproject-install
cargo run --release
```

### macOS (Apple silicon or Intel)

Install the Xcode command line tools and [Homebrew](https://brew.sh), then:

```sh
git clone https://github.com/itsyourap/RusTone && cd RusTone
bash .github/scripts/macos-deps.sh

git clone --branch rustone-2.17 https://github.com/itsyourap/pjproject ../pjproject
bash .github/scripts/build-pjproject.sh ../pjproject ../pjproject-install crates/pjsua-sys/config_site.h

export PJPROJECT_DIR=$PWD/../pjproject-install
cargo run --release
```

### Windows

You need Visual Studio 2022 or newer with the **Desktop development with C++** workload (and
its ARM64 build tools on ARM). pjproject is built with MSBuild against a dependencies folder
holding OpenSSL 3, SDL2, OpenH264 (static, `/MT`) and the OpenCORE AMR / VisualOn AMR-WB
libraries (built with MSYS2: GCC on x64, clang on ARM64). The `deps` job in
[platform.yml](.github/workflows/platform.yml) shows how each one is built for each
architecture. With those in a folder laid out as `include/`, `lib/` and `bin/`:

```powershell
git clone https://github.com/itsyourap/RusTone; cd RusTone
git clone --branch rustone-2.17 https://github.com/itsyourap/pjproject ..\pjproject
Copy-Item crates\pjsua-sys\config_site.h ..\pjproject\pjlib\include\pj\config_site.h
```

Add a `Directory.Build.props` next to `pjproject-vs14.sln` that adds the dependencies' `include`
and `lib` folders (see the `pjproject` job in the workflow), then, from a Developer PowerShell:

```powershell
# /p:Platform=ARM64 on Windows on ARM
msbuild ..\pjproject\pjproject-vs14.sln /t:libpjproject /p:Configuration=Release-Static /p:Platform=x64 /m

$env:PJPROJECT_DIR = "$PWD\..\pjproject"     # defaults to ..\pjproject
$env:RUSTONE_DEPS_DIR = "C:\path\to\deps"     # defaults to ..\..\deps
cargo run --release
```

The build copies `SDL2.dll` and the OpenSSL DLLs next to `rustone.exe`.

### Tests and checks

```sh
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

Some engine tests place real calls over the loopback interface or need a camera; they are
ignored by default and run with `cargo test -- --ignored`.

### Installers

Installers are made with [cargo-packager](https://github.com/crabnebula-dev/cargo-packager); see
[packaging/README.md](packaging/README.md). Pushing a version tag (`v0.2.0`) builds every
platform's installers in CI and drafts a GitHub Release with them.

## Acknowledgements

RusTone's design is inspired by [MicroSIP](https://www.microsip.org), the lightweight Windows
softphone, whose focus on a small, fast, no-nonsense phone it follows.

RusTone stands on these projects:

### Telephony and media

- [pjproject](https://www.pjsip.org) by Teluu, the SIP, media and NAT traversal stack, with the libraries it bundles: [libsrtp](https://github.com/cisco/libsrtp), [libyuv](https://chromium.googlesource.com/libyuv/libyuv), [Speex](https://www.speex.org), [WebRTC](https://webrtc.org)'s audio processing, iLBC, GSM and G.722.1
- [OpenH264](https://www.openh264.org) by Cisco, the H.264 codec
- [opencore-amr](https://sourceforge.net/projects/opencore-amr/) and VisualOn's AMR-WB encoder, the AMR codecs
- [OpenSSL](https://www.openssl.org), TLS and SRTP key exchange
- [SDL](https://www.libsdl.org), pjproject's video device layer

### Interface and desktop

- [egui and eframe](https://github.com/emilk/egui) by Emil Ernerfeldt, with [egui_plot](https://github.com/emilk/egui_plot) and [egui_kittest](https://github.com/emilk/egui/tree/master/crates/egui_kittest)
- [fastframe](https://github.com/crmne/fastframe) by Carmine Paolino: the app shell, tray, single instance, notifications, theming, fonts, icons, power and network events, autostart and media keys
- [winit](https://github.com/rust-windowing/winit), windowing
- [Lucide](https://lucide.dev) icons and the [Inter](https://rsms.me/inter/) typeface
- [resvg](https://github.com/linebender/resvg), SVG rendering of the logo
- [rfd](https://github.com/PolyMeilex/rfd), file dialogs

### Platform and data

- [rusqlite](https://github.com/rusqlite/rusqlite) and [SQLite](https://sqlite.org), the local database
- [keyring-core](https://github.com/open-source-cooperative/keyring-core) and its Windows, Apple and Secret Service stores, password storage
- [hidapi](https://github.com/ruabmbua/hidapi-rs), USB headsets
- [windows-rs](https://github.com/microsoft/windows-rs), [winreg](https://github.com/gentoo90/winreg-rs) and [objc2](https://github.com/madsmtm/objc2), Windows and macOS APIs
- [ureq](https://github.com/algesten/ureq), [serde](https://serde.rs), [toml](https://github.com/toml-rs/toml), [roxmltree](https://github.com/RazrFalcon/roxmltree), [regex](https://github.com/rust-lang/regex), [jiff](https://github.com/BurntSushi/jiff), [realfft](https://github.com/HEnquist/realfft), [user-idle](https://github.com/olback/user-idle-rs), [if-addrs](https://github.com/messense/if-addrs), [clap](https://github.com/clap-rs/clap) and [log](https://github.com/rust-lang/log)

### Building and packaging

- [bindgen](https://github.com/rust-lang/rust-bindgen) and [cc](https://github.com/rust-lang/cc-rs), the C bindings and shim
- [cargo-packager](https://github.com/crabnebula-dev/cargo-packager), the installers, with [NSIS](https://nsis.sourceforge.io), [WiX](https://wixtoolset.org), [linuxdeploy](https://github.com/linuxdeploy/linuxdeploy) and [dylibbundler](https://github.com/auriamg/macdylibbundler)
- [winresource](https://github.com/BenjaminRi/winresource), [MSYS2](https://www.msys2.org), [vcpkg](https://vcpkg.io), [meson](https://mesonbuild.com) and [Homebrew](https://brew.sh)

## License

RusTone is free software, released under the [GNU General Public License v3.0](LICENSE).
It statically links pjproject, which is licensed under the GPL. The codecs it includes are
covered by their own licenses, and H.264 and AMR may be subject to patents in some countries;
check what applies to you before distributing builds.
