<div align="center">

# Tokativ

Local dictation for Linux, macOS and Windows. Press a key, talk, and the text is typed into the
focused app as you speak.

[![License](https://img.shields.io/badge/license-GPL--3.0-2b2b33?style=flat-square)](COPYING)
[![Rust](https://img.shields.io/badge/rust-1.98-2b2b33?style=flat-square&logo=rust)](rust-toolchain.toml)
[![Wayland](https://img.shields.io/badge/wayland-%E2%99%A1-2b2b33?style=flat-square&logo=wayland)](#use)
[![Nix](https://img.shields.io/badge/nix-flake-2b2b33?style=flat-square&logo=nixos)](#install)
[![Platforms](https://img.shields.io/badge/runs%20on-linux%20%7C%20macos%20%7C%20windows-2b2b33?style=flat-square)](#install)

<img src="docs/preview-glow.png" alt="The Tokativ pill while dictating" width="900">
<br>
<br>

</div>

> [!NOTE]
> The project is vibe-coded as hell.

Speech is recognized on your machine by [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp),
on the graphics card through Vulkan, or Metal on macOS, or on the CPU. Nothing is downloaded until
you pick a model: Parakeet is the recommended one, and settings can search Hugging Face for any
other GGUF model. Whisper and Qwen3-ASR also take a vocabulary of names and terms to spell as
written, and follow the text before each phrase.

## Install

```sh
nix run github:nolight132/tokativ
```

With Home Manager, which also runs it as a user service:

```nix
imports = [ inputs.tokativ.homeModules.default ];
services.tokativ.enable = true;
```

On Linux both install the released binary; `nix run github:nolight132/tokativ#tokativ-source`
builds from source. On Apple silicon Macs, Nix builds from source and Home Manager runs it as a
launchd agent.

The [latest release](https://github.com/nolight132/tokativ/releases/latest) has x86_64 and ARM64
builds for all three platforms:

- **Windows:** run the `-setup.exe` installer for your architecture. A portable `.exe` is also available.
- **macOS:** download the `.app.tar.gz` for Intel or Apple silicon, extract it, and move Tokativ
  to Applications. Open it from the context menu on first launch; builds are not notarized.
- **Linux:** download the binary, make it executable, and put it on your `PATH` as `tokativ`.
  It needs glibc 2.35+, the Vulkan loader, Wayland, libxkbcommon and alsa-lib.

## Use

The first press of the hotkey starts dictating, the second inserts the rest. Enter finishes and
Escape cancels.

On Linux, bind `tokativ toggle` to a key in your compositor. It needs one with layer shell, so niri,
Hyprland, Sway, river or KDE Plasma, not GNOME.

On macOS and Windows, Tokativ listens for the hotkey itself: Option+Space on macOS, Ctrl+Alt+Space
on Windows. To change it, click the hotkey in settings and press the new one. A function key such
as F5 works on its own, any other key needs a modifier. macOS asks for Accessibility access the
first time Tokativ types, and the text only arrives once it is allowed. Neither system lets another
app put a draft at the cursor, so the text appears once each phrase is done.

On every platform, **Settings → General → Live caption** adds a larger floating window with the
current draft and the latest dictated text. It sits below a top pill or above a bottom pill,
and lets clicks through to the app underneath. It is on by default on macOS and Windows.

**Settings → Writing style → Spoken emoji** replaces names such as “thumbs up emoji” with 👍
and “эмодзи красное сердце” with ❤️. It is on by default, works offline, and recognizes the full
Unicode 17 emoji catalogue with names in 140+ languages, including flags and skin tones.
Say “emoji” or its local equivalent (such as “эмодзи”, “絵文字” or “이모지”) before or after the name.
Names without an emoji command remain ordinary text.

Windows and macOS check for updates and show a popup when a new version is available. You can
also check from the tray menu or **Settings → General**. Updates keep your settings and models.

## Build from source

Install Rust through rustup (the checkout selects its version), CMake and a C++ toolchain:

- **Windows:** Visual Studio Build Tools with Desktop development with C++, a Windows SDK,
  NASM and the Vulkan SDK. Use Developer PowerShell; for ARM64, use clang-cl and Ninja.
- **macOS:** Xcode command line tools.
- **Linux:** pkg-config, glslc and development packages for Vulkan, SPIRV-Headers, Wayland,
  libxkbcommon, fontconfig, alsa-lib and OpenSSL. `nix develop` provides them.

```sh
git clone https://github.com/nolight132/tokativ.git
cd tokativ
cargo build --release --locked
```

Run `target/release/tokativ` (Windows: `.\target\release\tokativ.exe`).

## License

GPL-3.0-or-later.
