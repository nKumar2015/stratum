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
  # The shell's defaults, which the written file starts from. Read from this
  # flake's source, not from the package, which would be import-from-derivation.
  defaults = builtins.fromTOML (builtins.readFile ./tools/stratum_shell/config.default.toml);
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
        Merged over the shell's defaults (`tools/stratum_shell/config.default.toml`,
        which documents every option) and written whole to
        {file}`$XDG_CONFIG_HOME/stratum/config.toml`, so that file always lists
        every option with its value. Tables merge key by key; a list, such as
        `sidebar.widgets`, replaces the default one. The shell reloads the
        file when a rebuild changes it.
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
        curl # the dashboard's weather and calendar sync
        grim # screenshots
        networkmanager # nmcli
        pciutils # lspci, the dashboard's GPU name
        pipewire # pw-link, and pw-cli for the EQ when the native path is unavailable
        playerctl # media controls
        upower # battery details
        wl-clipboard # copying screenshots
      ]);

    xdg.configFile."stratum/config.toml".source =
      toml.generate "stratum-config.toml" (lib.recursiveUpdate defaults cfg.settings);

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
