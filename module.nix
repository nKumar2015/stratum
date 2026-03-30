{
  config,
  lib,
  pkgs,
  inputs,
  ...
}: let
  cacheDir = "${config.home.homeDirectory}/.cache/matugen";
in {
  # 1. Matugen Configuration
  xdg.configFile."matugen/config.toml".text = ''
    [config]
    fallback_color = "#000000"
    prefer = "darkness"
    caching = false

    [templates.stratum_qml]
    input_path = "~/.config/matugen/templates/stratum-theme.qml"
    output_path = "${cacheDir}/Theme.qml"
  '';

  # 2. Quickshell Directory Patching & Symlink Injection
  home.file = {
    ".config/quickshell" = {
      source = pkgs.runCommand "patched-stratum" {} ''
        cp -r ${inputs.stratum} $out
        chmod -R +w $out
        rm -f $out/theme/Theme.qml
      '';
      recursive = true;
    };

    ".config/quickshell/theme/Theme.qml".source =
      config.lib.file.mkOutOfStoreSymlink "${cacheDir}/Theme.qml";
  };

  # 3. Activation Script for First-Boot Safety
  home.activation = {
    setupMatugenCache = inputs.home-manager.lib.hm.dag.entryAfter ["writeBoundary"] ''
      $DRY_RUN_CMD mkdir -p ${cacheDir}
      $DRY_RUN_CMD touch ${cacheDir}/Theme.qml
    '';
  };
}
