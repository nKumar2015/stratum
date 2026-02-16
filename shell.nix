# shell.nix
{pkgs ? import <nixpkgs> {}}: let
  # Define the Qt6 packages we need
  qt6Deps = with pkgs.qt6; [
    qtdeclarative
    qtquickcontrols2
    qtbase
  ];

  # Add quickshell to the list
  allDeps = qt6Deps ++ [pkgs.quickshell];

  # Build the import path string by mapping over the packages
  qmlPath = pkgs.lib.makeSearchPath "lib/qt-6/qml" allDeps;
in
  pkgs.mkShell {
    buildInputs = allDeps;

    shellHook = ''
      export QML_IMPORT_PATH="${qmlPath}"
      echo "QML LSP Environment Loaded!"
      echo "Import Path: $QML_IMPORT_PATH"
    '';
  }
