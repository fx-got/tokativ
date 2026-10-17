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

On Linux both install the binary from the latest
[release](https://github.com/nolight132/tokativ/releases), built for x86_64 and aarch64 against glibc
2.35, and `nix run github:nolight132/tokativ#tokativ-source` builds it from source instead. On Apple
silicon Macs they build it from source, and Home Manager runs it as a launchd agent.

On Linux without Nix, download the binary for your machine from the release and put it on your
`PATH` as `tokativ`. It needs the Vulkan loader, wayland, libxkbcommon and alsa-lib.

From source you need Rust 1.98, a C++ compiler, cmake, pkg-config, glslc, and the development files
for Vulkan, SPIRV-Headers, wayland, libxkbcommon, fontconfig, alsa-lib and openssl. Then
`cargo build --release`.

macOS and Windows have no release binaries yet. On macOS, a source build needs Rust, CMake and the
Xcode command line tools: `cargo build --release`, then `target/release/tokativ`.

On Windows x64, install these before building:

- [Rust through rustup](https://rust-lang.org/tools/install/) with the MSVC toolchain. The checkout
  selects the Rust version from `rust-toolchain.toml` automatically.
- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/), with
  **Desktop development with C++**, the x64 MSVC tools and a Windows SDK. GPUI's release build uses
  `fxc.exe` from that SDK.
- [CMake](https://cmake.org/download/) and [NASM](https://www.nasm.us/), both on `PATH`.
- [Vulkan SDK](https://vulkan.lunarg.com/sdk/home#windows), including `glslc`.

Open a new **Developer PowerShell for Visual Studio** after installation so it sees `VULKAN_SDK`
and the updated `PATH`. From a short checkout path, such as `C:\src\tokativ`:

```powershell
git clone https://github.com/nolight132/tokativ.git
cd tokativ
cargo build --release --locked
.\target\release\tokativ.exe
```

The first launch opens setup to download a model and choose a microphone. The default hotkey is
Ctrl+Alt+Space. To update an existing build, quit Tokativ before rebuilding its executable:

```powershell
.\target\release\tokativ.exe quit
git pull
cargo build --release --locked
.\target\release\tokativ.exe
```

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

On every platform, **Settings → Pill → Live caption** adds a larger floating window with the
current draft and the latest dictated text. It sits below a top pill or above a bottom pill,
and lets clicks through to the app underneath. It is on by default on macOS and Windows.

**Settings → Writing style → Spoken emoji** replaces names such as “thumbs up emoji” with 👍
and “эмодзи красное сердце” with ❤️. It is on by default, works offline, and recognizes the full
Unicode 17 emoji catalogue with names in 140+ languages, including flags and skin tones.
Say “emoji” or its local equivalent (such as “эмодзи”, “絵文字” or “이모지”) before or after the name.
Names without an emoji command remain ordinary text.

## License

GPL-3.0-or-later.
