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

      # The release the default package installs. Each hash is the SRI form of the asset's line in
      # that release's SHA256SUMS.
      release = {
        version = "0.1.0";
        assets = {
          x86_64-linux = {
            target = "x86_64-unknown-linux-gnu";
            hash = "sha256-dxuxo54rAxI+ka1IMv8rcYciYUZLm+Fddd678F7i5gI=";
          };
          aarch64-linux = {
            target = "aarch64-unknown-linux-gnu";
            hash = "sha256-EJuirB6OfpQ3mxL5FoQSAlp1YTWqOK2vvpTWN5O0Jsc=";
          };
        };
      };

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
      # `default` and `tokativ` are the released binary. `tokativ-source` builds this checkout.
      packages = forEachSystem (pkgs: rec {
        tokativ-source = pkgs.callPackage ./nix/package.nix { };
        tokativ-bin = pkgs.callPackage ./nix/package-bin.nix { inherit release tokativ-source; };
        tokativ = tokativ-bin;
        default = tokativ-bin;
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
            pkgs.spirv-headers
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
