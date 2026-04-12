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

    enableEqSink = lib.mkEnableOption "Enable Stratum PipeWire EQ virtual sink (user-level)";

    forceEqRouting = lib.mkEnableOption "Best-effort force routing of playback streams through Stratum EQ sink";

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

        ".config/pipewire/pipewire.conf.d/95-stratum-eq.conf" = lib.mkIf cfg.enableEqSink {
          text = ''
            context.modules = [
              {
                name = libpipewire-module-filter-chain
                args = {
                  node.description = "Stratum Parametric EQ"
                  media.name = "Stratum Parametric EQ"
                  filter.graph = {
                    nodes = [
                      {
                        type = builtin
                        name = eq
                        label = param_eq
                        config = {
                          filters = [
                            { type = bq_peaking freq = 1000.0 gain = 0.0 q = 1.0 }
                          ]
                        }
                      }
                    ]
                    inputs = [ "eq:In 1" "eq:In 2" ]
                    outputs = [ "eq:Out 1" "eq:Out 2" ]
                  }
                  capture.props = {
                    node.name = "effect_input.stratum_eq"
                    media.class = Audio/Sink
                    audio.channels = 2
                    audio.position = [ FL FR ]
                  }
                  playback.props = {
                    node.name = "effect_output.stratum_eq"
                    node.passive = true
                    audio.channels = 2
                    audio.position = [ FL FR ]
                  }
                }
              }
            ]
          '';
        };

        ".config/wireplumber/wireplumber.conf.d/95-stratum-force-eq.conf" = lib.mkIf (cfg.enableEqSink && cfg.forceEqRouting) {
          text = ''
            stream.rules = [
              {
                matches = [
                  {
                    media.class = "Stream/Output/Audio"
                  }
                ]
                actions = {
                  update-props = {
                    target.object = "effect_input.stratum_eq"
                    node.dont-fallback = true
                  }
                }
              }
            ]
          '';
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
          playerctl
          slurp
          grim
          wlr-randr
          netcat-openbsd

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

    systemd.user.services.stratumd = {
      Unit = {
        Description = "Stratum shell daemon";
        After = ["graphical-session.target"];
      };
      Service = {
        ExecStart = "${config.home.profileDirectory}/bin/stratumd";
        Environment = [
          "PATH=${config.home.profileDirectory}/bin:${config.home.homeDirectory}/.local/state/nix/profile/bin:/run/current-system/sw/bin:/usr/bin:/bin"
        ];
        Restart = "always";
        RestartSec = "1";
      };
      Install = {
        WantedBy = ["default.target"];
      };
    };
  };
}
