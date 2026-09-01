{
  description = "An mdBook preprocessor for checked Clash examples";

  inputs = {
    # Keep this as an independent input so consumers can select another Clash
    # release with `--override-input clash-compiler ...`.
    clash-compiler.url = "github:clash-lang/clash-compiler/v1.10.1";

    # Pin one doctest implementation across all supported Clash/GHC pairs. The
    # driver uses its internal parser and runner APIs so Markdown transcripts
    # have the same behavior on every CI lane.
    doctest-src = {
      url = "https://hackage.haskell.org/package/doctest-0.24.3/doctest-0.24.3.tar.gz";
      flake = false;
    };

    # Use the nixpkgs revision against which the selected Clash flake was
    # tested. Overriding clash-compiler therefore also selects a compatible
    # package set used by the selected Clash release.
    nixpkgs.follows = "clash-compiler/nixpkgs";
  };

  outputs =
    {
      clash-compiler,
      doctest-src,
      nixpkgs,
      self,
      ...
    }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];

      forAllSystems = nixpkgs.lib.genAttrs systems;

      mdbookClashOverlay =
        final: _prev:
        let
          clashGhcVersion = clash-compiler.ghcVersion.${final.stdenv.hostPlatform.system};
          clashPackages = final."clashPackages-${clashGhcVersion}";
          clash = clashPackages.clash-ghc;
          doctest = final.haskell.lib.dontCheck (clashPackages.callCabal2nix "doctest" doctest-src { });
          ghcWithDoctest = clashPackages.ghcWithPackages (_: [ doctest ]);
          mdbook-clash-doctest = clashPackages.callCabal2nix "mdbook-clash-doctest" ./doctest-driver {
            inherit doctest;
          };
          runtimeDependencies = [
            clash
            mdbook-clash-doctest
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
              ln -s ${mdbook-clash-doctest}/bin/mdbook-clash-doctest \
                $out/bin/mdbook-clash-doctest
              wrapProgram $out/bin/mdbook-clash \
                --prefix PATH : ${final.lib.makeBinPath runtimeDependencies}
            '';

            passthru = {
              inherit
                clash
                doctest
                ghcWithDoctest
                mdbook-clash-doctest
                runtimeDependencies
                ;
            };

            meta = {
              description = "An mdBook preprocessor for checked Clash examples";
              license = final.lib.licenses.bsd2;
              mainProgram = "mdbook-clash";
            };
          };
        in
        {
          inherit mdbook-clash mdbook-clash-doctest;
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
          inherit (pkgs) mdbook-clash mdbook-clash-doctest;
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
          inherit (pkgs.mdbook-clash) clash ghcWithDoctest;
        in
        {
          default = pkgs.mkShell {
            packages = [
              pkgs.cargo
              pkgs.cabal-install
              pkgs.clippy
              pkgs.mdbook
              pkgs.netlistsvg
              pkgs.rustc
              pkgs.rustfmt
              pkgs.yosys
              clash
              ghcWithDoctest
            ];

            shellHook = ''
              export PATH="${ghcWithDoctest}/bin:$PWD/scripts:$PWD/.dev-bin:$PATH"
              echo "mdbook-clash development shell (Clash toolchain)"
              echo "Build: cargo build"
              echo "Build doctest driver: build-doctest"
              echo "Example: mdbook build example"
            '';
          };
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt);
    };
}
