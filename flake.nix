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
        # As for the shell: the tests run in the devshell.
        doCheck = false;
      };

      # The shell and the file manager, built together so their shared
      # dependencies (wgpu, naga, cosmic-text) compile once. The file manager
      # is an output of its own: it reaches a system only when the Home
      # Manager module's `fileManager.enable` asks for it.
      stratum-shell = pkgs.rustPlatform.buildRustPackage {
        pname = "stratum-shell";
        inherit version;
        outputs = ["out" "files"];

        src = workspaceSrc;
        cargoBuildFlags = ["--package" "stratum-shell" "--package" "stratum-files"];
        cargoLock.lockFile = ./Cargo.lock;

        # `cargo test` in checkPhase compiles the whole release build a
        # second time (fat LTO, one codegen unit): 6m51s on top of the 6m41s
        # build. The tests run in the devshell before every commit instead.
        doCheck = false;

        nativeBuildInputs = shellNativeInputs pkgs;
        buildInputs = shellInputs pkgs;

        LIBCLANG_PATH = libclang pkgs;

        # A runpath, not a wrapper script: a wrapper renames the process, and
        # `pkill -x stratum-shell` stops matching it.
        postFixup = ''
          patchelf --add-rpath ${dlopenLibs pkgs} $out/bin/stratum-shell
          patchelf --add-rpath ${dlopenLibs pkgs} $files/bin/stratum-files
        '';

        # Every default the shell has; the binaries find it at
        # <bin>/../share/stratum and have none of their own. The file
        # manager's output gets its own copy, its desktop entry, and what
        # makes it the desktop portal's file chooser (the .portal file and
        # the D-Bus service that starts it).
        postInstall = ''
          install -Dm644 ${./tools/stratum_shell/config.default.toml} \
            $out/share/stratum/config.default.toml
          mkdir -p $files/bin
          mv $out/bin/stratum-files $files/bin/
          install -Dm644 ${./tools/stratum_shell/config.default.toml} \
            $files/share/stratum/config.default.toml
          install -Dm644 ${./tools/stratum_files/data/org.stratum.Files.desktop} \
            $files/share/applications/org.stratum.Files.desktop
          install -Dm644 ${./tools/stratum_files/data/stratum.portal} \
            $files/share/xdg-desktop-portal/portals/stratum.portal
          mkdir -p $files/share/dbus-1/services
          substitute ${./tools/stratum_files/data/org.freedesktop.impl.portal.desktop.stratum.service.in} \
            $files/share/dbus-1/services/org.freedesktop.impl.portal.desktop.stratum.service \
            --subst-var-by bin $files/bin
        '';
      };

      stratum-files = self.packages.${pkgs.stdenv.hostPlatform.system}.stratum-shell.files;

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
