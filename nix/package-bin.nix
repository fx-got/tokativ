# The binary the release workflow publishes, patched to find its libraries in the Nix store.
# `release` comes from flake.nix and names the version and the hash of each asset.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  makeWrapper,
  alsa-lib,
  alsa-plugins,
  pipewire,
  fontconfig,
  freetype,
  libxkbcommon,
  vulkan-loader,
  wayland,
  symlinkJoin,
  release,
  tokativ-source,
}:

let
  asset = release.assets.${stdenv.hostPlatform.system};
  alsaPlugins = symlinkJoin {
    name = "tokativ-alsa-plugins";
    paths = [
      "${alsa-plugins}/lib/alsa-lib"
      "${pipewire}/lib/alsa-lib"
    ];
  };
in
stdenv.mkDerivation {
  pname = "tokativ-bin";
  inherit (release) version;

  src = fetchurl {
    url = "https://github.com/nolight132/tokativ/releases/download/v${release.version}/tokativ-v${release.version}-${asset.target}";
    inherit (asset) hash;
  };

  dontUnpack = true;
  dontStrip = true;

  nativeBuildInputs = [
    autoPatchelfHook
    makeWrapper
  ];

  buildInputs = [
    alsa-lib
    fontconfig
    freetype
    libxkbcommon
    vulkan-loader
    stdenv.cc.cc.lib
  ];

  # GPUI opens these with dlopen, so they have to be on the runpath explicitly.
  runtimeDependencies = [
    vulkan-loader
    wayland
    libxkbcommon
  ];

  installPhase = ''
    runHook preInstall
    install -Dm755 "$src" "$out/bin/tokativ"
    install -Dm444 ${../assets/linux/tokativ.desktop} "$out/share/applications/tokativ.desktop"
    install -Dm444 ${../COPYING} "$out/share/licenses/tokativ/LICENSE"
    install -Dm444 ${../assets/fonts/LICENSE.txt} "$out/share/licenses/tokativ/LICENSE.Inter"
    install -Dm444 ${../assets/silero/LICENSE} "$out/share/licenses/tokativ/LICENSE.silero-vad"
    install -Dm444 ${../assets/icons/LICENSE} "$out/share/licenses/tokativ/LICENSE.lucide"
    install -Dm444 ${../assets/emoji/LICENSE} "$out/share/licenses/tokativ/LICENSE.Unicode"
    runHook postInstall
  '';

  postFixup = ''
    wrapProgram $out/bin/tokativ --set-default ALSA_PLUGIN_DIR ${alsaPlugins}
  '';

  meta = tokativ-source.meta // {
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
    # This derivation installs the Linux release executables.
    platforms = builtins.attrNames release.assets;
  };
}
