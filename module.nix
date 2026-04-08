{
  config,
  lib,
  pkgs,
  inputs,
  ...
}: let
  cfg = config.programs.stratum;
  hostSystem = pkgs.stdenv.hostPlatform.system;
in {
  options.programs.stratum = {
    enable = lib.mkEnableOption "A Quickshell config";

    prefer = lib.mkOption {
      type = lib.types.enum ["darkness" "light"];
      default = "darkness";
      description = "The theme preference for Matugen";
    };

    installPortal = lib.mkEnableOption "Use preset xdg-desktp-portal-gtk setup";
  };

  config = lib.mkIf cfg.enable {
    home = {
      file = {
        ".config/quickshell" = {
          source = inputs.stratum;
          recursive = true;
        };
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

          inputs.quickshell.packages.${hostSystem}.default
        ]
        ++ lib.optional cfg.installPortal pkgs.xdg-desktop-portal-gtk;
    };

    xdg = {
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
