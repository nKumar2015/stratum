{
  description = "Stratum: a native Wayland shell for Hyprland";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    ...
  }: let
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

    version = (nixpkgs.lib.importTOML ./Cargo.toml).workspace.package.version;

    # What the shell builds and links against. It embeds stratum_sys, whose
    # PipeWire bindings run bindgen, so the PipeWire/SPA headers and libclang
    # must be present too.
    shellNativeInputs = pkgs: with pkgs; [pkg-config clang];
    shellInputs = pkgs:
      with pkgs; [
        wayland
        wayland-protocols
        libxkbcommon
        vulkan-loader
        pipewire
        libpulseaudio
        pam
      ];
    # wgpu and xkbcommon dlopen these at runtime, which linking alone does not
    # put on the search path; without them the shell exits with "no suitable
    # Vulkan adapter".
    dlopenLibs = pkgs: pkgs.lib.makeLibraryPath (with pkgs; [vulkan-loader wayland libxkbcommon]);
    libclang = pkgs: "${pkgs.llvmPackages.libclang.lib}/lib";
  in {
    packages = forAllSystems (pkgs: let
      # Single source tree for the Cargo workspace: crates depend on each other
      # (the shell builds on stratum_sys), so each package must see the whole
      # tree plus the shared lockfile.
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
        inherit version;

        src = workspaceSrc;
        buildAndTestSubdir = "tools/stratum_cli";
        cargoLock.lockFile = ./Cargo.lock;
      };

      stratum-shell = pkgs.rustPlatform.buildRustPackage {
        pname = "stratum-shell";
        inherit version;

        src = workspaceSrc;
        buildAndTestSubdir = "tools/stratum_shell";
        cargoLock.lockFile = ./Cargo.lock;

        nativeBuildInputs = shellNativeInputs pkgs;
        buildInputs = shellInputs pkgs;

        LIBCLANG_PATH = libclang pkgs;

        # A runpath, not a wrapper script: a wrapper renames the process, and
        # `pkill -x stratum-shell` stops matching it.
        postFixup = ''
          patchelf --add-rpath ${dlopenLibs pkgs} $out/bin/stratum-shell
        '';

        # Every default the shell has; the binary finds it at
        # <bin>/../share/stratum and has none of its own.
        postInstall = ''
          install -Dm644 ${./tools/stratum_shell/config.default.toml} \
            $out/share/stratum/config.default.toml
        '';
      };

      default = self.packages.${pkgs.stdenv.hostPlatform.system}.stratum-shell;
    });

    # Home Manager: the shell, its CLI, and a user unit that runs the shell
    # with the graphical session. See module.nix.
    homeManagerModules.default = import ./module.nix self;

    # NixOS: what needs the system. The lock screen and polkit dialog check
    # passwords through PAM's `stratum-shell` service; without it the shell
    # falls back to `login`, or refuses to lock.
    nixosModules.default = {pkgs, ...}: let
      hostSystem = pkgs.stdenv.hostPlatform.system;
    in {
      security.pam.services.stratum-shell = {};
      environment.systemPackages = [
        self.packages.${hostSystem}.stratum-shell
        self.packages.${hostSystem}.stratum-cli
      ];
    };

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        packages =
          [
            (pkgs.rust-bin.stable.latest.default.override {
              extensions = [
                "rust-src"
                "rust-analyzer"
              ];
            })
            # dbus-launch, for private buses in live tests.
            pkgs.dbus
          ]
          ++ shellNativeInputs pkgs
          ++ shellInputs pkgs;

        LIBCLANG_PATH = libclang pkgs;
        # Prepended so the user's own path survives.
        shellHook = ''
          export LD_LIBRARY_PATH="${dlopenLibs pkgs}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        '';
      };
    });
  };
}
