{
  description = "Tokativ - local dictation for Wayland";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs, ... }:
    let
      inherit (nixpkgs) lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forEachSystem = fn: lib.genAttrs systems (system: fn nixpkgs.legacyPackages.${system});

      runtimeLibraries =
        pkgs: with pkgs; [
          vulkan-loader
          wayland
          libxkbcommon
          fontconfig
          freetype
          alsa-lib
          dbus
          onnxruntime
        ];
    in
    {
      packages = forEachSystem (pkgs: rec {
        tokativ = pkgs.callPackage ./nix/package.nix { };
        default = tokativ;
      });

      devShells = forEachSystem (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            pkg-config
            cmake
          ];
          buildInputs = runtimeLibraries pkgs ++ [ pkgs.openssl ];
          LD_LIBRARY_PATH = lib.makeLibraryPath (runtimeLibraries pkgs);
          ORT_LIB_LOCATION = "${pkgs.onnxruntime}/lib";
          ORT_PREFER_DYNAMIC_LINK = "1";
          ALSA_PLUGIN_DIR = "${pkgs.symlinkJoin {
            name = "tokativ-alsa-plugins";
            paths = [
              "${pkgs.alsa-plugins}/lib/alsa-lib"
              "${pkgs.pipewire}/lib/alsa-lib"
            ];
          }}";
        };
      });

      overlays.default = final: _prev: {
        tokativ = self.packages.${final.stdenv.hostPlatform.system}.default;
      };

      homeManagerModules = rec {
        tokativ = import ./nix/hm-module.nix self;
        default = tokativ;
      };
      homeModules = self.homeManagerModules;
    };
}
