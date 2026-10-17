{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  cmake,
  makeWrapper,
  autoPatchelfHook,
  openssl,
  shaderc,
  vulkan-headers,
  alsa-lib,
  alsa-plugins,
  pipewire,
  dbus,
  fontconfig,
  freetype,
  libxkbcommon,
  vulkan-loader,
  wayland,
  symlinkJoin,
  stdenv,
}:

let
  manifest = lib.importTOML ../Cargo.toml;
  # Sonora's icons and i18n crates read their assets two folders above themselves, which in the
  # vendored sources is the build directory. Keep the rev in step with Cargo.toml.
  sonora = fetchFromGitHub {
    owner = "sonorahq";
    repo = "sonora";
    rev = "2a68b53b9c375f85928238b35db824eeedb414ed";
    hash = "sha256-lY/Py0lVdSzl9vU6glHTHf6uZHFpiwbH+B9jb9ADDYE=";
  };
  alsaPlugins = symlinkJoin {
    name = "tokativ-alsa-plugins";
    paths = [
      "${alsa-plugins}/lib/alsa-lib"
      "${pipewire}/lib/alsa-lib"
    ];
  };
in
rustPlatform.buildRustPackage {
  pname = "tokativ";
  inherit (manifest.workspace.package) version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
      ../assets
    ];
  };

  cargoLock = {
    lockFile = ../Cargo.lock;
    allowBuiltinFetchGit = true;
  };

  cargoBuildFlags = [ "--package=tokativ" ];
  cargoTestFlags = [ "--workspace" ];

  nativeBuildInputs = [
    pkg-config
    cmake
    # glslc, which compiles ggml's Vulkan shaders
    shaderc
    makeWrapper
    autoPatchelfHook
  ];

  buildInputs = [
    openssl
    vulkan-headers
    vulkan-loader
    alsa-lib
    dbus
    fontconfig
    freetype
    libxkbcommon
    stdenv.cc.cc.lib
  ];

  # GPUI opens these with dlopen, so they have to be on the runpath explicitly.
  runtimeDependencies = [
    vulkan-loader
    wayland
    libxkbcommon
  ];

  preBuild = ''
    ln -s ${sonora}/assets "$NIX_BUILD_TOP/assets"
  '';

  # cmake is only here for transcribe.cpp's build script, not to configure this package.
  dontUseCmakeConfigure = true;

  # The tests need a microphone, a Wayland seat or downloaded models in places.
  doCheck = false;

  postInstall = ''
    install -Dm444 assets/linux/tokativ.desktop $out/share/applications/tokativ.desktop
    install -Dm444 assets/fonts/LICENSE.txt $out/share/licenses/tokativ/LICENSE.Inter
    install -Dm444 assets/silero/LICENSE $out/share/licenses/tokativ/LICENSE.silero-vad
  '';

  postFixup = ''
    wrapProgram $out/bin/tokativ --set-default ALSA_PLUGIN_DIR ${alsaPlugins}
  '';

  meta = {
    description = "Local dictation for Wayland";
    mainProgram = "tokativ";
    license = with lib.licenses; [
      gpl3Plus
      ofl
      mit
    ];
    platforms = [
      "x86_64-linux"
      "aarch64-linux"
    ];
  };
}
