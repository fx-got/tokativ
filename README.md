# Tokativ

Dictation for Linux on Wayland that runs entirely on your machine. Press a key, talk, and the text
lands in the focused app phrase by phrase while you speak. A pill at the bottom of the screen shows
your voice and has buttons to cancel or finish.

You pick one speech model and dictate in whatever it understands. Both run on the CPU through
ONNX Runtime:

| Model | Export | Languages |
| --- | --- | --- |
| Parakeet v3 (NVIDIA) | parakeet-tdt-0.6b-v3, int8 | 25 European languages including English and Russian, told apart per phrase |
| GigaAM v3 (Sber) | v3 e2e CTC, int8 | Russian, with punctuation |

Tokativ targets niri. Any compositor with `wlr-layer-shell` and the input method or virtual
keyboard protocols should work. GNOME and X11 are not supported.

## Install

With flakes and Home Manager:

```nix
{
  inputs.tokativ.url = "github:nolight132/tokativ";

  # in your Home Manager configuration
  imports = [ inputs.tokativ.homeModules.default ];
  services.tokativ = {
    enable = true;
    # optional: fetch models at build time instead of from the setup window
    prefetchModels = [ "parakeet" "gigaam" ];
  };
}
```

The module installs the package and runs `tokativ daemon` as a systemd user service tied to
`graphical-session.target`. Without Home Manager, `nix run github:nolight132/tokativ` starts the
daemon. Any CLI command that opens something starts it too if it is not running yet.

On first run a setup window downloads the model you pick (about 670 MB for Parakeet, 225 MB for
GigaAM) into `$XDG_DATA_HOME/tokativ/models`, lets you pick a microphone, and gives you a field
to try dictating into.

## Hotkey

Bind the toggle in your niri config:

```kdl
binds {
    Mod+D { spawn "tokativ" "toggle"; }
    Mod+Shift+D { spawn "tokativ" "cancel"; }
}
```

The first press opens the pill and starts listening. While you speak, a draft of the current
phrase shows at the cursor as underlined text. Each time you pause for about half a second, the
phrase is transcribed in full and inserted. The second press, Enter, or the check mark on the pill
inserts whatever is left and closes it. Escape, the cross, or `tokativ cancel` drop anything not
yet inserted. Text that has already gone in stays.

Escape and Enter work because the session takes the keyboard through the input method protocol.
Any other key goes on to the app and hands the keyboard back, so typing by hand mid-session
behaves normally. Drafts and the two keys need the input method, so they are off when another
input method such as fcitx5 is running.

## Commands

```
tokativ                 run the daemon (same as `tokativ daemon`)
tokativ toggle          start, or finish the running session
tokativ start | stop    start, or finish
tokativ confirm         finish and insert the rest
tokativ cancel          drop what is not inserted and close the pill
tokativ model <id>      switch to parakeet or gigaam
tokativ settings        open the settings window
tokativ quit            stop the daemon
```

The daemon listens on `$XDG_RUNTIME_DIR/tokativ.sock`. Each connection sends one command as a line
of text and gets back `ok` or `error: <reason>`.

## How text gets in

The daemon picks the first of these that works at startup, and the settings window shows which one
it chose and why:

1. **Input method** (`zwp_input_method_v2`). Text is committed as a string, so any script works,
   and a `…` placeholder sits at the cursor while a phrase is transcribed. Only one input method
   can hold the seat. If fcitx5, IBus or kime is running, Tokativ leaves it alone and moves down
   the list. On niri 26.04, binding a second input method detaches the first, so the check happens
   before binding.
2. **Virtual keyboard** (`zwp_virtual_keyboard_v1`). Text is typed wtype-style with a keymap built
   for exactly the characters needed. This is also used for any app that has no text-input-v3
   support, since the input method never activates there.
3. **Clipboard and paste**. The text goes on the clipboard, Ctrl+V is sent (Ctrl+Shift+V in the
   terminals listed in the config), and the previous clipboard text is put back.

## Configuration

Settings live in `$XDG_CONFIG_HOME/tokativ/config.toml`. The settings window writes the file and
the daemon reloads it whenever it changes, whoever wrote it. Every key is optional:

```toml
# input_device = "..."          # a device id picked in settings, omit for the default
model = "parakeet"              # or "gigaam"
launch_at_login = false         # writes an XDG autostart entry
tray = true
live_preview = true            # draft text at the cursor while speaking

[vad]
silence_ms = 500                # the pause that ends a phrase
threshold = 0.5                 # Silero speech probability
max_segment_s = 20              # phrases are cut here even mid-speech
preroll_ms = 200                # audio kept from before speech starts

[insertion]
backend = "auto"                # "input-method", "virtual-keyboard" or "clipboard"
terminals = ["foot", "kitty", "Alacritty", "com.mitchellh.ghostty"]

[pill]
edge = "bottom"                 # or "top"
margin = 48

[models]
unload_after_minutes = 0        # 0 keeps models loaded
```

## Building

```sh
nix develop
cargo run -- daemon
cargo test --workspace
cargo clippy --workspace --all-targets
```

`ort` is pinned to `2.0.0-rc.12` and linked against the onnxruntime from nixpkgs through
`ORT_LIB_LOCATION`, which the dev shell sets. To try the whole pipeline without speaking, point
`TOKATIV_INPUT_FILE` at a 16-bit mono WAV file. The daemon plays it in place of the microphone.
`cargo run -p tokativ-engine --example file -- parakeet speech.wav` runs a 16 kHz file through the
VAD and an engine and prints each phrase.

The workspace is split by job:

- `tokativ-audio` captures audio, computes the bar levels and resamples to 16 kHz. Silero VAD v6
  cuts the speech into phrases.
- `tokativ-engine` holds the `Engine` trait, Parakeet and GigaAM through transcribe-rs, the worker
  thread, model downloads and how phrases are joined.
- `tokativ-input` inserts text over Wayland.
- `tokativ-ui` has the pill, settings and setup windows, drawn with Sonora's `ui` crate on
  Sonora's GPUI fork.
- `tokativ-config` holds the config file and paths shared by all of them.
- `tokativ` is the binary: daemon, socket, CLI and tray.

## Licenses

Tokativ is GPL-3.0-or-later, since it builds on Sonora's UI crate. Inter is under the SIL Open Font
License and the Silero VAD model under MIT. The speech models are downloaded from their authors'
releases under their own licenses: CC-BY-4.0 for Parakeet and MIT for GigaAM.
