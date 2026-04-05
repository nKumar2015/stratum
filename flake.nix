{
  description = "My Quickshell Config";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";

    # Replace this with the actual URL/path to the stratum repository
    stratum = {
      url = "github:nkumar2015/stratum";
      flake = false;
    };

    matugen.url = "github:InioX/matugen";
    quickshell.url = "git+https://git.outfoxxed.me/outfoxxed/quickshell";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    ...
  } @ inputs: let
    systems = [
      "x86_64-linux"
      "aarch64-linux"
    ];

    overlays = [(import rust-overlay)];

    forAllSystems = f:
      nixpkgs.lib.genAttrs systems (system:
        f (import nixpkgs {
          inherit system overlays;
        }));
  in {
    packages = forAllSystems (pkgs: {
      stratum-cli = pkgs.rustPlatform.buildRustPackage {
        pname = "stratum-cli";
        version = "0.1.0";

        src = ./tools/stratum_cli;
        cargoLock.lockFile = ./tools/stratum_cli/Cargo.lock;

        nativeBuildInputs = with pkgs; [pkg-config];
        buildInputs = with pkgs; [dbus openssl];
      };

      default = self.packages.${pkgs.stdenv.hostPlatform.system}.stratum-cli;
    });

    # Export the module for use in other system configurations
    homeManagerModules.default = {
      config,
      pkgs,
      lib,
      ...
    }: let
      hostSystem = pkgs.stdenv.hostPlatform.system;
    in {
      imports = [./module.nix];
      # This passes the inputs (like stratum) down into the module
      _module.args.inputs = inputs;

      # Ensure the CLI is present on PATH when using the exported HM module.
      home.packages = [self.packages.${hostSystem}.stratum-cli];
    };

    # Optional NixOS module that installs the CLI system-wide.
    nixosModules.default = {
      pkgs,
      ...
    }: let
      hostSystem = pkgs.stdenv.hostPlatform.system;
    in {
      environment.systemPackages = [self.packages.${hostSystem}.stratum-cli];
    };

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        packages = with pkgs; [
          (rust-bin.stable.latest.default.override {
            extensions = [
              "rust-src"
              "rust-analyzer"
            ];
          })
          pkg-config
          dbus
          glib
          openssl
        ];

        shellHook = ''
          echo "Rust dev shell ready (cargo/rustc/clippy/rustfmt + dbus tooling)."
        '';
      };
    });
  };
}
