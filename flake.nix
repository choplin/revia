{
  description = "revia development environment";

  inputs = {
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0";
    flake-parts.url = "github:hercules-ci/flake-parts";
    devshell = {
      url = "github:numtide/devshell";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{
      nixpkgs,
      flake-parts,
      devshell,
      fenix,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [ ];

      flake = { };

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];

      perSystem =
        {
          config,
          self',
          inputs',
          pkgs,
          system,
          ...
        }:
        let
          cargoManifest = builtins.fromTOML (builtins.readFile ./Cargo.toml);
          rustToolchain = fenix.packages.${system}.stable.withComponents [
            "cargo"
            "clippy"
            "rust-analyzer"
            "rust-src"
            "rustc"
            "rustfmt"
          ];
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rustToolchain;
            rustc = rustToolchain;
          };
          revia = rustPlatform.buildRustPackage {
            pname = "revia";
            version = cargoManifest.package.version;
            src = pkgs.lib.cleanSource ./.;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ pkgs.makeWrapper ];
            nativeCheckInputs = [ pkgs.git ];

            postInstall = ''
              wrapProgram "$out/bin/revia" \
                --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.git ]}
            '';

            meta = {
              description = "An interactive terminal UI for reviewing Git diffs";
              homepage = "https://github.com/choplin/revia";
              license = pkgs.lib.licenses.mit;
              mainProgram = "revia";
            };
          };
        in
        {
          _module.args.pkgs = import nixpkgs {
            inherit system;
            overlays = [ devshell.overlays.default ];
          };

          devShells.default = pkgs.devshell.mkShell {
            imports = [ (pkgs.devshell.importTOML ./devshell.toml) ];
            packages = [ rustToolchain ];
          };

          packages = {
            inherit revia;
            default = revia;
          };
        };
    };
}
