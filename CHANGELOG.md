# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Tokativ runs on macOS and Windows. It listens for a hotkey of its own there, Option+Space on
  macOS and Ctrl+Alt+Space on Windows, which settings can change, and types or pastes the text into
  the focused app.

### Changed

- Settings leave out the descriptions that only repeated a setting's name, and search results
  show how often a model was downloaded instead of its likes.

## [0.2.0] - 2026-10-17

### Changed

- `nix run github:nolight132/tokativ` and the Home Manager module install the released binary
  instead of compiling Tokativ. `nix run github:nolight132/tokativ#tokativ-source` still builds it
  from source.

## [0.1.0] - 2026-10-17

The first release. Bind `tokativ toggle` to a key, press it and talk, and the text appears in the
focused app as you speak.

### Added

- A pill at the bottom of the screen shows that Tokativ is listening, with a level meter fed by
  your microphone. Enter finishes the dictation and Escape cancels it.
- A live draft of each phrase sits at the cursor while you talk, and the finished phrase replaces
  it. Text goes in through the input method, and Tokativ falls back to typing or pasting where
  there is none.
- Speech is recognized on your machine by transcribe.cpp, on the graphics card through Vulkan or
  on the CPU. Any of its GGUF models works, and settings can search Hugging Face for them and
  download them.
- A setup window on the first start downloads the model you pick, Parakeet by recommendation,
  and shows the keybinding line for your compositor. Nothing is downloaded before that.
- Settings let you pick the model and the microphone, and show how much memory, graphics memory
  and processor time the daemon uses.
- A writing style rewrites each phrase before it is inserted. It decides how sentences start,
  whether the final period stays, how laughter is spelled, and which fixed phrases get replaced.
- A tray icon and a `tokativ` command line control the daemon.
- The Home Manager module runs Tokativ as a user service and can fetch models at build time.

[unreleased]: https://github.com/nolight132/tokativ/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/nolight132/tokativ/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/nolight132/tokativ/releases/tag/v0.1.0
