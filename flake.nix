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
            gnumake
            shaderc
          ];
          buildInputs = runtimeLibraries pkgs ++ [
            pkgs.openssl
            pkgs.vulkan-headers
          ];
          LD_LIBRARY_PATH = lib.makeLibraryPath (runtimeLibraries pkgs);
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
