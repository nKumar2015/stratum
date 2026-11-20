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

    fileManager = {
      enable = lib.mkEnableOption ''
        the file manager, `stratum-files`, and every app's Open and Save
        dialogs through it (the desktop portal's FileChooser, with GTK's as
        the fallback when `installPortal` is on). Its options are `[files]`
        and `[metrics.files]` in `settings`'';

      package = lib.mkOption {
        type = lib.types.package;
        default = cfg.package.files or self.packages.${hostSystem}.stratum-files;
        defaultText = lib.literalExpression "config.programs.stratum.package.files";
        description = "`stratum-files`: the `files` output of the shell's package, built with it.";
      };

      setDefault = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Make it the app that opens folders (`inode/directory`). Needs
          `xdg.mimeApps.enable`, which makes Home Manager own
          {file}`~/.config/mimeapps.list`; without it, run
          `xdg-mime default org.stratum.Files.desktop inode/directory`.
        '';
      };

      thumbnailers = lib.mkOption {
        type = lib.types.listOf lib.types.package;
        default = [];
        example = lib.literalExpression "[pkgs.ffmpegthumbnailer]";
        description = ''
          Thumbnailers for what `stratum-files` does not decode itself
          (it does PNG, JPEG, WebP, GIF, BMP and SVG): any package with a
          {file}`share/thumbnailers/*.thumbnailer` file, such as
          `ffmpegthumbnailer` (videos and audio covers), `papers` or
          `evince` (PDFs), `gnome-font-viewer` (fonts). Installed with the
          file manager; `[files] thumbnail_external = false` stops their
          use. Thumbnails are kept in {file}`~/.cache/thumbnails`, shared
          with other apps.
        '';
      };
    };
  };

  config = lib.mkIf cfg.enable {
    warnings = lib.optional (cfg.fileManager.setDefault && !config.xdg.mimeApps.enable) ''
      programs.stratum.fileManager.setDefault needs xdg.mimeApps.enable; folders
      still open in the previous app.
    '';

    xdg.mimeApps.defaultApplications = lib.mkIf (cfg.fileManager.enable && cfg.fileManager.setDefault) {
      "inode/directory" = ["org.stratum.Files.desktop"];
    };

    home.packages =
      [cfg.package cfg.cliPackage]
      ++ lib.optional cfg.fileManager.enable cfg.fileManager.package
      ++ lib.optionals cfg.fileManager.enable cfg.fileManager.thumbnailers
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
        util-linux # setsid, which the launcher starts apps with
        wl-clipboard # copying screenshots, and restoring clipboard history entries
        xdg-utils # xdg-open, for links in calendar events
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

    xdg.portal = lib.mkIf (cfg.installPortal || cfg.fileManager.enable) {
      enable = true;
      extraPortals =
        lib.optionals cfg.installPortal [
          pkgs.xdg-desktop-portal-gtk
          pkgs.xdg-desktop-portal-hyprland
        ]
        ++ lib.optional cfg.fileManager.enable cfg.fileManager.package;
      config = let
        # The first one found answers: ours, then GTK's.
        fileChooser = {
          "org.freedesktop.impl.portal.FileChooser" =
            lib.optional cfg.fileManager.enable "stratum"
            ++ lib.optional cfg.installPortal "gtk";
        };
      in {
        common = lib.optionalAttrs cfg.installPortal {default = ["gtk"];} // fileChooser;
        # Hyprland's package installs a hyprland-portals.conf
        # (default=hyprland;gtk), and xdg-desktop-portal takes a file
        # named for the desktop over portals.conf wherever each is found,
        # so the common file never applied under Hyprland and every
        # FileChooser call went to GTK. This one, in ~/.config, is found
        # before the package's; it keeps Hyprland's defaults.
        hyprland = lib.mkIf cfg.fileManager.enable ({default = ["hyprland" "gtk"];} // fileChooser);
      };
    };
  };
}
