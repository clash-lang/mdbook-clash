{
  description = "An mdBook preprocessor for checked Clash examples";

  inputs = {
    # Keep this as an independent input so consumers can select another Clash
    # release with `--override-input clash-compiler ...`.
    clash-compiler.url = "github:clash-lang/clash-compiler/v1.10.0";

    # Use the nixpkgs revision against which the selected Clash flake was
    # tested. Overriding clash-compiler therefore also selects a compatible
    # package set for GHC and clash-prelude.
    nixpkgs.follows = "clash-compiler/nixpkgs";
  };

  outputs =
    {
      clash-compiler,
      nixpkgs,
      self,
      ...
    }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];

      forAllSystems = nixpkgs.lib.genAttrs systems;

      mdbookClashOverlay =
        final: _prev:
        let
          clashGhcVersion = clash-compiler.ghcVersion.${final.stdenv.hostPlatform.system};
          clashPackages = final."clashPackages-${clashGhcVersion}";
          clash = clashPackages.clash-ghc;
          ghc = clashPackages.ghcWithPackages (packages: [
            packages.clash-prelude
          ]);
          runtimeDependencies = [
            clash
            ghc
            final.netlistsvg
            final.yosys
          ];

          mdbook-clash = final.rustPlatform.buildRustPackage {
            pname = "mdbook-clash";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
            src = self;

            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [ final.makeWrapper ];
            postInstall = ''
              wrapProgram $out/bin/mdbook-clash \
                --prefix PATH : ${final.lib.makeBinPath runtimeDependencies}
            '';

            passthru = {
              inherit clash ghc runtimeDependencies;
            };

            meta = {
              description = "An mdBook preprocessor for checked Clash examples";
              license = final.lib.licenses.bsd2;
              mainProgram = "mdbook-clash";
            };
          };
        in
        {
          inherit mdbook-clash;
        };

      overlay = nixpkgs.lib.composeManyExtensions [
        clash-compiler.overlays.default
        mdbookClashOverlay
      ];

      pkgsFor =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ overlay ];
        };
    in
    {
      overlays.default = overlay;

      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          default = pkgs.mdbook-clash;
          inherit (pkgs) mdbook-clash;
        }
      );

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/mdbook-clash";
          meta.description = "Run the mdbook-clash preprocessor";
        };
      });

      checks = forAllSystems (system: {
        default = self.packages.${system}.mdbook-clash;
      });

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          inherit (pkgs.mdbook-clash) clash ghc;
        in
        {
          default = pkgs.mkShell {
            packages = [
              pkgs.cargo
              pkgs.clippy
              pkgs.mdbook
              pkgs.netlistsvg
              pkgs.rustc
              pkgs.rustfmt
              pkgs.yosys
              ghc
              clash
            ];

            shellHook = ''
              echo "mdbook-clash development shell (Clash GHC ${clash-compiler.ghcVersion.${system}})"
              echo "Build: cargo build"
              echo "Example: mdbook build example"
            '';
          };
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt);
    };
}
