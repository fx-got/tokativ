self:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.tokativ;
  toml = pkgs.formats.toml { };
  models = import ./models.nix;
in
{
  options.services.tokativ = {
    enable = lib.mkEnableOption "Tokativ, local dictation for Wayland";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "tokativ.packages.\${system}.default";
      description = "The Tokativ package to run.";
    };

    settings = lib.mkOption {
      inherit (toml) type;
      default = { };
      example = lib.literalExpression ''
        {
          language = "auto";
          engines.russian = "gigaam";
          vad.silence_ms = 600;
          pill.margin = 64;
        }
      '';
      description = ''
        Written to {file}`$XDG_CONFIG_HOME/tokativ/config.toml`. Left empty, the file stays
        under the settings window's control. When set, the settings window can still change
        it, and the next activation puts these values back.
      '';
    };

    prefetchModels = lib.mkOption {
      type = lib.types.listOf (
        lib.types.enum [
          "parakeet"
          "gigaam"
        ]
      );
      default = [ ];
      example = [
        "parakeet"
        "gigaam"
      ];
      description = ''
        Models to fetch at build time and link into {file}`$XDG_DATA_HOME/tokativ/models`,
        instead of downloading them from the setup window.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];

    xdg.configFile."tokativ/config.toml" = lib.mkIf (cfg.settings != { }) {
      source = toml.generate "tokativ-config.toml" cfg.settings;
      force = true;
    };

    xdg.dataFile = lib.mkMerge (
      map (
        engine:
        lib.listToAttrs (
          map (file: {
            name = "tokativ/models/${engine}/${file.name}";
            value.source = pkgs.fetchurl {
              inherit (file) url sha256;
              name = "tokativ-${engine}-${file.name}";
            };
          }) models.${engine}
        )
      ) cfg.prefetchModels
    );

    systemd.user.services.tokativ = {
      Unit = {
        Description = "Tokativ dictation daemon";
        PartOf = [ "graphical-session.target" ];
        After = [ "graphical-session.target" ];
      };
      Service = {
        ExecStart = "${lib.getExe cfg.package} daemon";
        Restart = "on-failure";
        RestartSec = 2;
      };
      Install.WantedBy = [ "graphical-session.target" ];
    };
  };
}
