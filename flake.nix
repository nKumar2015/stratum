{
  description = "My Quickshell Config";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

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
    ...
  } @ inputs: {
    # Export the module for use in other system configurations
    homeManagerModules.default = {
      config,
      pkgs,
      lib,
      ...
    }: {
      imports = [./module.nix];
      # This passes the inputs (like stratum) down into the module
      _module.args.inputs = inputs;
    };
  };
}
