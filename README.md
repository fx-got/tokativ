<div align="center">

# Tokativ

Local dictation for Wayland. Press a key, talk, and the text is typed into the focused app as you
speak.

[![License](https://img.shields.io/badge/license-GPL--3.0-2b2b33?style=flat-square)](COPYING)
[![Rust](https://img.shields.io/badge/rust-1.98-2b2b33?style=flat-square&logo=rust)](rust-toolchain.toml)
[![Wayland](https://img.shields.io/badge/wayland-%E2%99%A1-2b2b33?style=flat-square&logo=wayland)](#use)
[![Nix](https://img.shields.io/badge/nix-flake-2b2b33?style=flat-square&logo=nixos)](#nix)
[![Platforms](https://img.shields.io/badge/linux-x86__64%20%7C%20aarch64-2b2b33?style=flat-square&logo=linux&logoColor=white)](#install)

<img src="docs/preview-glow.png" alt="The Tokativ pill while dictating" width="900">
<br>
<br>

</div>

> [!NOTE]
> The project is vibe-coded as hell.

Speech is recognized on your machine, on the graphics card through Vulkan or on the CPU, by
[transcribe.cpp](https://github.com/handy-computer/transcribe.cpp). Any of its GGUF models works, and
settings can search Hugging Face for them: Parakeet, Whisper, Canary, GigaAM, Moonshine, Qwen3-ASR
and more.

## Install

### Nix

Run it once:

```sh
nix run github:nolight132/tokativ
```

Or install it with Home Manager, which also runs it as a user service:

```nix
# flake inputs
tokativ.url = "github:nolight132/tokativ";

# Home Manager configuration
imports = [ inputs.tokativ.homeModules.default ];
services.tokativ.enable = true;
```

### From source

You need Rust 1.98, a C++ compiler, cmake, pkg-config, glslc (from shaderc), and the development
files for Vulkan, SPIRV-Headers, wayland, libxkbcommon, fontconfig, alsa-lib and openssl.

```sh
git clone https://github.com/nolight132/tokativ
cd tokativ
cargo build --release
./target/release/tokativ
```

## Use

The first start opens a setup window that downloads a model and shows the keybinding line for
your compositor. Bind `tokativ toggle` to a key: the first press starts dictating, the second
inserts the rest. While dictating, Enter finishes and Escape cancels.

Tokativ runs on compositors with layer shell: niri, Hyprland, Sway, river and KDE Plasma. GNOME
is not supported.

`cargo run -- sandbox` runs a separate copy as a new user would see it, to work on the setup flow.

## License

GPL-3.0-or-later.
