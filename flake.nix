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
    packages = forAllSystems (pkgs: let
      # Single source tree for the Cargo workspace: crates depend on each other
      # (stratumd and stratum-cli build on stratum_sys), so each package must
      # see the whole tree plus the shared lockfile.
      workspaceSrc = pkgs.lib.cleanSourceWith {
        src = ./.;
        filter = path: type: let
          base = baseNameOf path;
        in
          base != "target"
          && base != ".direnv"
          && base != ".git"
          && !(pkgs.lib.hasPrefix "result" base);
      };
    in {
      stratum-cli = pkgs.rustPlatform.buildRustPackage {
        pname = "stratum-cli";
        version = "0.1.0";

        src = workspaceSrc;
        buildAndTestSubdir = "tools/stratum_cli";
        cargoLock.lockFile = ./Cargo.lock;

        nativeBuildInputs = with pkgs; [pkg-config];
        buildInputs = with pkgs; [dbus openssl];
      };

      stratumd = pkgs.rustPlatform.buildRustPackage {
        pname = "stratumd";
        version = "0.1.0";

        src = workspaceSrc;
        buildAndTestSubdir = "tools/stratumd";
        cargoLock.lockFile = ./Cargo.lock;

        nativeBuildInputs = with pkgs; [pkg-config clang];
        buildInputs = with pkgs; [pam dbus pipewire libpulseaudio];

        LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
      };

      stratum-shell = pkgs.rustPlatform.buildRustPackage {
        pname = "stratum-shell";
        version = "0.1.0";

        src = workspaceSrc;
        buildAndTestSubdir = "tools/stratum_shell";
        cargoLock.lockFile = ./Cargo.lock;

        nativeBuildInputs = with pkgs; [pkg-config clang];
        # Wayland client headers + the Vulkan loader are needed at link time.
        # The shell embeds stratum_sys, whose PipeWire bindings run bindgen, so
        # the PipeWire/SPA headers and libclang must be present too.
        buildInputs = with pkgs; [
          wayland wayland-protocols
          libxkbcommon
          vulkan-loader
          libGL
          dbus
          pipewire
          libpulseaudio
          pam
        ];

        LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
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

      # Ensure CLI and daemon are present on PATH when using the exported HM module.
      home.packages = [
        self.packages.${hostSystem}.stratum-cli
        self.packages.${hostSystem}.stratumd
      ];
    };

    # Optional NixOS module that installs the CLI system-wide.
    nixosModules.default = {pkgs, ...}: let
      hostSystem = pkgs.stdenv.hostPlatform.system;
    in {
      environment.systemPackages = [
        self.packages.${hostSystem}.stratum-cli
        self.packages.${hostSystem}.stratumd
      ];
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
          pam
          clang
          libpulseaudio
          pipewire

          # Native shell: Wayland protocol headers, keyboard handling, and the
          # Vulkan loader that wgpu presents through.
          wayland
          wayland-protocols
          libxkbcommon
          vulkan-loader
          libGL
          udev
        ];

        LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
        # wgpu and xkbcommon are loaded with dlopen at runtime, so being on the
        # link line is not enough; without this the shell exits with "no
        # suitable Vulkan adapter". Prepended so the user's own path survives.
        shellHook = ''
          export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath (with pkgs; [vulkan-loader wayland libxkbcommon])}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
          echo "Rust dev shell ready (cargo/rustc/clippy/rustfmt + dbus tooling)."
        '';
      };
    });
  };
}
