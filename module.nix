# Home Manager module for the native shell. `self` is this flake, so the
# shell and CLI come from the same revision as the module.
self: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.stratum;
  hostSystem = pkgs.stdenv.hostPlatform.system;
  toml = pkgs.formats.toml {};
in {
  imports = [
    (lib.mkRemovedOptionModule ["programs" "stratum" "setupPolkit"] ''
      The shell is the session's polkit agent itself ([behavior] polkit, on by
      default). polkit-agent-helper-1 still needs its setuid wrapper in the
      NixOS configuration.
    '')
    (lib.mkRemovedOptionModule ["programs" "stratum" "prefer"] "It was never read.")
  ];

  options.programs.stratum = {
    enable = lib.mkEnableOption "the Stratum shell, started with the graphical session";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${hostSystem}.stratum-shell;
      description = "The shell. Build it against the system's nixpkgs, or the lock screen cannot load the system's PAM modules and refuses to lock.";
    };

    cliPackage = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${hostSystem}.stratum-cli;
      description = "`stratum-cli`, which keybinds use to reach the shell.";
    };

    settings = lib.mkOption {
      inherit (toml) type;
      default = {};
      example = lib.literalExpression ''
        {
          theme = "carbon";
          render.gpu = "radeon";
          control_center.invert_scroll = true;
        }
      '';
      description = ''
        Written to {file}`$XDG_CONFIG_HOME/stratum/config.toml` when not empty.
        Leave it empty to keep that file your own (the shell reloads it on
        change). The schema is `tools/stratum_shell/config.example.toml`.
      '';
    };

    installPortal = lib.mkEnableOption "the GTK and Hyprland desktop portals (Save As in the screenshot viewer)";
  };

  config = lib.mkIf cfg.enable {
    home.packages =
      [cfg.package cfg.cliPackage]
      # Programs the shell runs.
      ++ (with pkgs; [
        brightnessctl # brightness, and the OSD's fallback reading
        grim # screenshots
        libnotify # notify-send, for screenshot failures
        networkmanager # nmcli
        pipewire # pw-link, pw-cli
        playerctl # media controls
        pulseaudio # pactl
        upower # battery details
        wireplumber # wpctl
        wl-clipboard # copying screenshots
      ])
      ++ lib.optional cfg.installPortal pkgs.xdg-desktop-portal-gtk;

    xdg.configFile."stratum/config.toml" = lib.mkIf (cfg.settings != {}) {
      source = toml.generate "stratum-config.toml" cfg.settings;
    };

    systemd.user.services.stratum-shell = {
      Unit = {
        Description = "Stratum shell";
        Documentation = "https://github.com/nKumar2015/stratum";
        PartOf = ["graphical-session.target"];
        After = ["graphical-session.target"];
      };
      Service = {
        ExecStart = "${cfg.package}/bin/stratum-shell";
        # A crash brings the bar back; a clean exit (`pkill`) leaves it off.
        Restart = "on-failure";
        RestartSec = 1;
      };
      Install.WantedBy = ["graphical-session.target"];
    };

    xdg.portal = lib.mkIf cfg.installPortal {
      enable = true;
      extraPortals = [
        pkgs.xdg-desktop-portal-gtk
        pkgs.xdg-desktop-portal-hyprland
      ];
      config.common = {
        default = ["gtk"];
        "org.freedesktop.impl.portal.FileChooser" = ["gtk"];
      };
    };
  };
}
