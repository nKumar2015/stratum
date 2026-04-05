{
  config,
  lib,
  pkgs,
  inputs,
  ...
}: let
  cfg = config.programs.stratum;
  cacheDir = "${config.home.homeDirectory}/.cache/matugen";
  hostSystem = pkgs.stdenv.hostPlatform.system;
in {
  options.programs.stratum = {
    enable = lib.mkEnableOption "A Quickshell config";

    devSourcePath = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Absolute path to a local Stratum checkout for development. When set, .config/quickshell is sourced from this path instead of the flake input snapshot.";
    };

    prefer = lib.mkOption {
      type = lib.types.enum ["darkness" "light"];
      default = "darkness";
      description = "The theme preference for Matugen";
    };

    fallbackColor = lib.mkOption {
      type = lib.types.str;
      default = "#000000";
      description = "The fallback hex color for Matugen";
    };

    installPortal = lib.mkEnableOption "Use preset xdg-desktp-portal-gtk setup";
  };

  config = lib.mkIf cfg.enable {
    home = {
      file = {
        ".config/quickshell" = {
          source =
            if cfg.devSourcePath != null
            then config.lib.file.mkOutOfStoreSymlink cfg.devSourcePath
            else pkgs.runCommand "patched-stratum" {} ''
              cp -r ${inputs.stratum} $out
              chmod -R +w $out
              rm -f $out/theme/Theme.qml
            '';
          recursive = true;
        };

        ".config/quickshell/theme/Theme.qml".source =
          config.lib.file.mkOutOfStoreSymlink "${cacheDir}/Theme.qml";
      };

      activation = {
        setupMatugenCache = inputs.home-manager.lib.hm.dag.entryAfter ["writeBoundary"] ''
          $DRY_RUN_CMD mkdir -p ${cacheDir}
          $DRY_RUN_CMD touch ${cacheDir}/Theme.qml
        '';
      };

      packages = with pkgs;
        [
          networkmanager
          upower
          brightnessctl
          qt6.qtbase
          qt6.qtdeclarative
          qt6.qt5compat
          pavucontrol
          playerctl
          slurp
          grim
          wlr-randr

          inputs.matugen.packages.${hostSystem}.default
          inputs.quickshell.packages.${hostSystem}.default
        ]
        ++ lib.optional cfg.installPortal pkgs.xdg-desktop-portal-gtk;
    };

    xdg = {
      configFile."matugen/config.toml".text = ''
        [config]
        fallback_color = "${cfg.fallbackColor}"
        prefer = "${cfg.prefer}"
        caching = false

        [templates.stratum_qml]
        input_path = "~/.config/matugen/templates/stratum-theme.qml"
        output_path = "${cacheDir}/Theme.qml"
      '';

      portal = lib.mkIf cfg.installPortal {
        enable = true;
        extraPortals = [
          pkgs.xdg-desktop-portal-gtk
          pkgs.xdg-desktop-portal-hyprland
        ];
        config = {
          common = {
            default = ["gtk"];
            "org.freedesktop.impl.portal.FileChooser" = ["gtk"];
          };
        };
      };
    };
  };
}
